use crate::snapshots::{self, Merge, SnapshotStatus, Snapshots};
use crate::{snapshot_state, MergeStatus, Slot, SnapshotState, METADATA};
use anyhow::{ensure, Context, Result};
use skyward_block::{devnum, dm, mount, BY_NAME};
use skyward_lp::{Extent, Metadata, Partition, ATTR_DISABLED};
use std::fs;
use std::path::{Path, PathBuf};

/// A logical partition of super, mapped and mounted read-only.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Mounted {
    /// The name in super, such as vendor_dlkm_b; also the device-mapper name.
    pub partition: String,
    pub device: PathBuf,
    pub mount_point: PathBuf,
    pub filesystem: String,
}

#[derive(Debug, Clone, Default)]
pub struct Mounts {
    pub partitions: Vec<Mounted>,
    /// Damaged metadata copies in super that their backups stood in for.
    pub damaged: Vec<String>,
}

/// Where a logical partition goes in recovery. System goes to /mnt/system,
/// since /system holds recovery itself; a partition whose mount point
/// directory is in the ramdisk root uses it, as /vendor or /odm; any other
/// goes under /mnt.
pub fn mount_point(base: &str) -> PathBuf {
    let root = Path::new("/").join(base);
    if base != "system" && fs::symlink_metadata(&root).is_ok_and(|m| m.is_dir()) {
        root
    } else {
        Path::new("/mnt").join(base)
    }
}

fn super_device() -> PathBuf {
    Path::new(BY_NAME).join("super")
}

/// The device numbers of super and its extra block devices, in the order
/// the metadata lists them, so an extent's device index resolves to one.
fn block_devices(md: &Metadata, super_dev: &Path) -> Result<Vec<(u32, u32)>> {
    md.block_devices
        .iter()
        .enumerate()
        .map(|(i, d)| if i == 0 { devnum(super_dev) } else { devnum(Path::new(BY_NAME).join(&d.partition_name)) })
        .collect()
}

/// The device-mapper targets that lay partition `p` out over super.
pub(crate) fn targets(p: &Partition, devices: &[(u32, u32)]) -> Vec<dm::Target> {
    p.extents
        .iter()
        .map(|e| match *e {
            Extent::Linear { sectors, device, sector } => dm::Target::Linear { sectors, device: devices[device], offset: sector },
            Extent::Zero { sectors } => dm::Target::Zero { sectors },
        })
        .collect()
}

/// How many bytes partition `p` holds.
fn bytes(p: &Partition) -> u64 {
    p.sectors() * 512
}

/// Map partition `p` from super read-only with device-mapper and mount it
/// over its point. A device that does not mount is removed again.
fn map_and_mount(p: &Partition, devices: &[(u32, u32)], slot: Slot) -> Result<Mounted> {
    // A recovery of another make maps super's partitions as its own init
    // starts, before the recovery service runs; such a mapping is used as
    // it is, mapping the partition again failing while it is there.
    if let Some((_, minor)) = dm::status(&p.name)? {
        return mount_mapped(p, Path::new("/dev/block").join(format!("dm-{minor}")), slot);
    }
    let device = dm::create(&p.name, &targets(p, devices), true)?;
    let mount_point = mount_point(base(p, slot));
    let mounted = (|| -> Result<String> {
        fs::create_dir_all(&mount_point).with_context(|| format!("create {}", mount_point.display()))?;
        Ok(mount::mount_ro_over(&device, &mount_point).with_context(|| format!("mount {}", p.name))?.to_string())
    })();
    match mounted {
        Ok(filesystem) => Ok(Mounted { partition: p.name.clone(), device, mount_point, filesystem }),
        Err(e) => Err(match dm::remove(&p.name) {
            Ok(()) => e,
            Err(r) => e.context(format!("removing {} afterwards failed too: {r:#}", p.name)),
        }),
    }
}

/// Whether super names its partitions after the slot each is of, as an A/B
/// phone's does. A phone without slots keeps one set of partitions, named
/// without a suffix, which this reads as slot A's.
fn slotted(md: &Metadata) -> bool {
    md.partitions.iter().any(|p| [Slot::A, Slot::B].iter().any(|slot| p.name.ends_with(slot.suffix())))
}

/// The name the slot's partition `base_name` goes by in super.
fn named(md: &Metadata, base_name: &str, slot: Slot) -> String {
    match slotted(md) {
        true => format!("{base_name}{}", slot.suffix()),
        false => base_name.to_string(),
    }
}

/// The slot's partitions that hold data. Copy-on-write partitions such as
/// odm_b-cow end in -cow and stay out.
fn slot_partitions(md: &Metadata, slot: Slot) -> Vec<&Partition> {
    let slotted = slotted(md);
    md.partitions
        .iter()
        .filter(|p| if slotted { p.name.ends_with(slot.suffix()) } else { !p.name.ends_with("-cow") })
        .filter(|p| !p.extents.is_empty() && p.attributes & ATTR_DISABLED == 0)
        .collect()
}

/// The name a partition of the slot goes by without the slot's suffix, as
/// a phone without slots names it already.
fn base(p: &Partition, slot: Slot) -> &str {
    p.name.strip_suffix(slot.suffix()).unwrap_or(&p.name)
}

/// Partitions are mapped only while no update snapshot is outstanding,
/// since a snapshotted partition reads correctly only through its snapshot.
fn ensure_no_snapshot() -> Result<()> {
    let state = snapshot_state()?;
    ensure!(state == SnapshotState::None, "an update snapshot is outstanding ({state:?})");
    Ok(())
}

/// Mount /metadata, then map the slot's logical partitions from super and
/// mount them, all read-only. Where an update went to the slot and is not
/// merged yet, each partition it has a snapshot of is mapped through that
/// snapshot, as the system's own first stage maps it, merging where the
/// merge is under way. A partition an earlier run of the recovery service
/// mounted, and that still shows, is taken as it is.
pub fn mount_partitions(slot: Slot) -> Result<Mounts> {
    let metadata = Path::new(METADATA);
    if !mount::is_mount_point(metadata)? {
        // A recovery of another make may carry no folder to mount it on.
        fs::create_dir_all(metadata).with_context(|| format!("make {}", metadata.display()))?;
        mount::mount_ro(&Path::new(BY_NAME).join("metadata"), metadata)?;
    }
    let super_dev = super_device();
    let md = skyward_lp::read(&super_dev, slot.number())?;
    let devices = block_devices(&md, &super_dev)?;
    let snaps = Snapshots::lock(true)?;
    let through = match &snaps {
        Some(s) if s.needed(slot)? => Some(s),
        _ => None,
    };

    let shown = mounted_partitions(slot)?;
    let mut mounts = Mounts { partitions: Vec::new(), damaged: md.damaged.clone() };
    for p in slot_partitions(&md, slot) {
        if let Some(m) = shown.iter().find(|m| m.partition == p.name) {
            mounts.partitions.push(m.clone());
            continue;
        }
        let mounted = match through.map(|s| Ok::<_, anyhow::Error>((s, s.live(p)?))).transpose()? {
            Some((s, Some(status))) => mount_snapshot(s, &md, &devices, p, &status, slot)?,
            _ => map_and_mount(p, &devices, slot)?,
        };
        mounts.partitions.push(mounted);
    }
    Ok(mounts)
}

/// Start merging the update into `slot`, the slot it went to and the phone
/// runs from, its partitions mapped through their snapshots; see
/// [`snapshots::initiate`]. `tell` tells the bootloader where the update
/// stands.
pub fn start_merge(slot: Slot, tell: &dyn Fn(MergeStatus) -> Result<()>) -> Result<()> {
    let md = skyward_lp::read(&super_device(), slot.number())?;
    let snaps = Snapshots::lock(true)?.context("no update was ever applied")?;
    snapshots::initiate(&snaps, slot, &md, tell)
}

/// Whether every snapshot the slot reads through is mapped as a kernel
/// snapshot, as following a merge needs; `false` where it reads through
/// none.
pub fn snapshots_mapped(slot: Slot) -> Result<bool> {
    let md = skyward_lp::read(&super_device(), slot.number())?;
    let Some(snaps) = Snapshots::lock(false)? else { return Ok(false) };
    if !snaps.needed(slot)? {
        return Ok(false);
    }
    let mut any = false;
    for p in slot_partitions(&md, slot) {
        if snaps.live(p)?.is_some() {
            if !snapshots::is_snapshot(&p.name)? {
                return Ok(false);
            }
            any = true;
        }
    }
    Ok(any)
}

/// Look at the merge into `slot`, cleaning up after each snapshot merged;
/// see [`snapshots::check`].
pub fn check_merge(slot: Slot, tell: &dyn Fn(MergeStatus) -> Result<()>) -> Result<Merge> {
    let super_dev = super_device();
    let md = skyward_lp::read(&super_dev, slot.number())?;
    let devices = block_devices(&md, &super_dev)?;
    let snaps = Snapshots::lock(true)?.context("no update was ever applied")?;
    snapshots::check(&snaps, slot, &md, &devices, tell)
}

/// Map partition `p` through its snapshot `status` and mount it read-only;
/// a snapshot that does not mount is taken apart again.
fn mount_snapshot(snaps: &Snapshots, md: &Metadata, devices: &[(u32, u32)], p: &Partition, status: &SnapshotStatus, slot: Slot) -> Result<Mounted> {
    let device = snapshots::map_snapshot(snaps, md, devices, p, status)?;
    mount_mapped(p, device, slot)
}

/// Mount partition `p`, mapped as its snapshot at `device`, read-only over
/// its point; the snapshot is taken apart again if it does not mount.
fn mount_mapped(p: &Partition, device: PathBuf, slot: Slot) -> Result<Mounted> {
    let mount_point = mount_point(base(p, slot));
    let mounted = (|| -> Result<String> {
        fs::create_dir_all(&mount_point).with_context(|| format!("create {}", mount_point.display()))?;
        Ok(mount::mount_ro_over(&device, &mount_point).with_context(|| format!("mount {}", p.name))?.to_string())
    })();
    match mounted {
        Ok(filesystem) => Ok(Mounted { partition: p.name.clone(), device, mount_point, filesystem }),
        Err(e) => Err(match snapshots::unmap_snapshot(&p.name) {
            Ok(()) => e,
            Err(r) => e.context(format!("taking {} apart afterwards failed too: {r:#}", p.name)),
        }),
    }
}

/// The base names of the slot's logical partitions (such as `system`,
/// `vendor`), in the order super lists them, whether or not they are mounted.
pub fn slot_bases(slot: Slot) -> Result<Vec<String>> {
    let md = skyward_lp::read(&super_device(), slot.number())?;
    Ok(slot_partitions(&md, slot).into_iter().map(|p| base(p, slot).to_string()).collect())
}

/// The slot's logical partitions by base name with the bytes each holds, in
/// the order super lists them.
pub fn slot_sizes(slot: Slot) -> Result<Vec<(String, u64)>> {
    let md = skyward_lp::read(&super_device(), slot.number())?;
    Ok(slot_partitions(&md, slot).into_iter().map(|p| (base(p, slot).to_string(), bytes(p))).collect())
}

/// One of the slot's logical partitions mapped whole under a name of its
/// own, beside the mapping the recovery mounts, for a backup to read it or
/// a restore to write it.
#[derive(Debug)]
pub struct Whole {
    pub name: String,
    pub device: PathBuf,
    pub size: u64,
}

/// Map the slot's logical partition `base_name` whole as `name`, read-only
/// or not, while no update snapshot is outstanding.
pub fn map_whole(slot: Slot, base_name: &str, name: &str, readonly: bool) -> Result<Whole> {
    ensure_no_snapshot()?;
    let super_dev = super_device();
    let md = skyward_lp::read(&super_dev, slot.number())?;
    let devices = block_devices(&md, &super_dev)?;
    let full = named(&md, base_name, slot);
    let p = slot_partitions(&md, slot).into_iter().find(|p| p.name == full).with_context(|| format!("super holds no {full}"))?;
    let device = dm::create(name, &targets(p, &devices), readonly)?;
    Ok(Whole { name: name.into(), device, size: bytes(p) })
}

/// Remove a mapping [`map_whole`] made.
pub fn unmap_whole(whole: &Whole) -> Result<()> {
    dm::remove(&whole.name)
}

/// The base names of the logical partitions super lists for the slot, in
/// its order, those that hold nothing yet among them: the ones an image
/// can be written into.
pub fn slot_names(slot: Slot) -> Result<Vec<String>> {
    let md = skyward_lp::read(&super_device(), slot.number())?;
    Ok(match slotted(&md) {
        true => md.partitions.iter().filter_map(|p| p.name.strip_suffix(slot.suffix())).map(str::to_string).collect(),
        false => md.partitions.iter().filter(|p| !p.name.ends_with("-cow")).map(|p| p.name.clone()).collect(),
    })
}

/// A logical partition [`remake`] readied for an image, mapped writable as
/// `device` and as large as the image.
#[derive(Debug)]
pub struct Remade {
    pub name: String,
    pub device: PathBuf,
    pub size: u64,
    /// Its table, where the recovery had it mapped already and something
    /// still holds it: it stays mapped, read-only again, once written.
    held: Option<Vec<dm::Target>>,
}

/// Check that the slot's logical partition `base_name` can be made to hold
/// `bytes`, changing nothing: no update snapshot is outstanding, super
/// lists the partition, and has the room.
pub fn can_remake(slot: Slot, base_name: &str, bytes: u64) -> Result<()> {
    ensure_no_snapshot()?;
    let mut md = skyward_lp::read(&super_device(), slot.number())?;
    let name = named(&md, base_name, slot);
    md.resize(&name, bytes)
}

/// Make the slot's logical partition `base_name` hold `bytes` and map it
/// writable under its own name, for an image to be written into it, as
/// fastbootd readies one: super's metadata is written anew, for both slots
/// alike, with the partition as large as the image. No update snapshot may
/// be outstanding, and the partition has to be off its mount point, as it
/// is once the recovery has moved off the installed system; one a program
/// still holds is mapped already, and takes its new table where it is.
pub fn remake(slot: Slot, base_name: &str, bytes: u64) -> Result<Remade> {
    ensure_no_snapshot()?;
    remake_on(&super_device(), slot, base_name, bytes)
}

fn remake_on(super_dev: &Path, slot: Slot, base_name: &str, bytes: u64) -> Result<Remade> {
    let mut md = skyward_lp::read(super_dev, slot.number())?;
    let name = named(&md, base_name, slot);
    let mapped = dm::status(&name)?;
    if let Some(device) = mapped {
        let points = mount::points_of(device)?;
        ensure!(points.is_empty(), "{name} is mounted on {}, and is written only once it is off", points[0].display());
    }
    let before = md.partition(&name).with_context(|| format!("super holds no {name}"))?.extents.clone();
    md.resize(&name, bytes)?;
    let p = md.partition(&name).expect("resizing keeps the partition");
    if p.extents != before {
        skyward_lp::write(super_dev, &md, &[Slot::A.number(), Slot::B.number()])?;
    }
    let table = targets(p, &block_devices(&md, super_dev)?);
    let (device, held) = match mapped {
        None => (dm::create(&name, &table, false)?, None),
        Some((_, minor)) => {
            dm::reload(&name, &table, false)?;
            (PathBuf::from(format!("/dev/block/dm-{minor}")), Some(table))
        }
    };
    Ok(Remade { name, device, size: p.bytes(), held })
}

/// Take away the mapping [`remake`] made once the image is written; one
/// the recovery had already, and something still holds, stays as it was
/// found, read-only.
pub fn settle(remade: &Remade) -> Result<()> {
    match &remade.held {
        None => dm::remove(&remade.name),
        Some(table) => dm::reload(&remade.name, table, true),
    }
}

/// What [`map_slot`] did to the slot's logical partitions, for
/// [`unmap_slot`] to undo: those it mapped, and those the recovery had
/// mapped read-only already, and still uses, that it made writable.
#[derive(Debug, Default)]
pub struct SlotMaps {
    made: Vec<String>,
    opened: Vec<(String, Vec<dm::Target>)>,
}

/// Map each of the slot's logical partitions writable and under its own
/// name, as recoveries map them for install scripts, which look for them
/// at /dev/block/mapper/<name>; while no update snapshot is outstanding.
/// One the recovery has mapped read-only already is made writable where
/// it is, in use or not.
pub fn map_slot(slot: Slot) -> Result<SlotMaps> {
    ensure_no_snapshot()?;
    let super_dev = super_device();
    let md = skyward_lp::read(&super_dev, slot.number())?;
    let devices = block_devices(&md, &super_dev)?;
    let mut maps = SlotMaps::default();
    for p in slot_partitions(&md, slot) {
        if let Err(e) = map_writable(p, targets(p, &devices), &mut maps) {
            return Err(match unmap_slot(&maps) {
                Ok(()) => e,
                Err(r) => e.context(format!("undoing what was mapped before failed too: {r:#}")),
            });
        }
    }
    Ok(maps)
}

/// Map partition `p` writable with `table`, or make the read-only mapping
/// of it writable, keeping in `maps` what was done.
fn map_writable(p: &Partition, table: Vec<dm::Target>, maps: &mut SlotMaps) -> Result<()> {
    if dm::status(&p.name)?.is_none() {
        dm::create(&p.name, &table, false)?;
        maps.made.push(p.name.clone());
    } else if dm::readonly(&p.name)? {
        dm::reload(&p.name, &table, false)?;
        maps.opened.push((p.name.clone(), table));
    }
    Ok(())
}

/// Undo [`map_slot`]: remove the mappings it made, and make those it made
/// writable read-only again.
pub fn unmap_slot(maps: &SlotMaps) -> Result<()> {
    for name in &maps.made {
        dm::remove(name)?;
    }
    for (name, table) in &maps.opened {
        dm::reload(name, table, true)?;
    }
    Ok(())
}

/// Map and mount one of the slot's logical partitions read-only by its base
/// name (such as `system` or `vendor`): through its snapshot where the slot
/// reads through an update's snapshots, from the snapshot already mapped
/// where it was only unmounted.
pub fn mount_one(slot: Slot, base_name: &str) -> Result<Mounted> {
    let super_dev = super_device();
    let md = skyward_lp::read(&super_dev, slot.number())?;
    let devices = block_devices(&md, &super_dev)?;
    let name = named(&md, base_name, slot);
    let p = slot_partitions(&md, slot)
        .into_iter()
        .find(|p| p.name == name)
        .with_context(|| format!("super holds no {name}"))?;
    let snaps = Snapshots::lock(true)?;
    let status = match &snaps {
        Some(s) if s.needed(slot)? => s.live(p)?.map(|status| (s, status)),
        _ => None,
    };
    match status {
        Some(_) if dm::status(&name)?.is_some() => mount_mapped(p, Path::new("/dev/block").join(dm_node(&name)?), slot),
        Some((s, status)) => mount_snapshot(s, &md, &devices, p, &status, slot),
        None => map_and_mount(p, &devices, slot),
    }
}

/// The node device-mapper device `name` has under /dev/block, as dm-N.
fn dm_node(name: &str) -> Result<String> {
    let (_, minor) = dm::status(name)?.with_context(|| format!("{name} is not mapped"))?;
    Ok(format!("dm-{minor}"))
}

/// Unmount the partition mapped as `name` where its own device is what
/// shows on `point`, and take it apart: its device-mapper device, and where
/// it reads through a snapshot, the snapshot's own devices too. A partition
/// that is not mapped is left as it is.
fn unmap(name: &str, point: &Path) -> Result<()> {
    let Some(dev) = dm::status(name)? else { return Ok(()) };
    if mount::visible(point)?.is_some_and(|v| v.device == dev) {
        mount::umount_over(point)?;
    }
    snapshots::unmap_snapshot(name)
}

/// Unmount one of the slot's logical partitions by its base name and remove
/// its device-mapper device. One that reads through a snapshot is only
/// unmounted: its snapshot stays mapped, for its merge to go on and for it
/// to be mounted again.
pub fn unmount_one(slot: Slot, base_name: &str) -> Result<()> {
    let name = crate::partition_name(base_name, slot)?;
    let point = mount_point(base_name);
    if !snapshots::is_snapshot(&name)? {
        return unmap(&name, &point);
    }
    let dev = dm::status(&name)?.with_context(|| format!("{name} is not mapped"))?;
    if mount::visible(&point)?.is_some_and(|v| v.device == dev) {
        mount::umount_over(&point)?;
    }
    Ok(())
}

/// The slot's partitions that are mapped and show on their mount points, in
/// the order they were mapped.
pub fn mounted_partitions(slot: Slot) -> Result<Vec<Mounted>> {
    let slots = crate::current_slot()?.is_some();
    let mut found = Vec::new();
    for entry in fs::read_dir("/sys/block").context("read /sys/block")? {
        let entry = entry?;
        let node = entry.file_name().to_string_lossy().into_owned();
        let Some(minor) = node.strip_prefix("dm-") else { continue };
        let name_file = entry.path().join("dm/name");
        let name = fs::read_to_string(&name_file).with_context(|| format!("read {}", name_file.display()))?;
        let name = name.trim();
        // A phone without slots maps its partitions under their own names;
        // only a mapping that shows on the mount point its name gives is
        // taken for one.
        let Some(base) = name.strip_suffix(slot.suffix()).or_else(|| (!slots).then_some(name)) else { continue };
        let device = Path::new("/dev/block").join(&node);
        let mount_point = mount_point(base);
        let Some(shown) = mount::visible(&mount_point)? else { continue };
        if shown.device != devnum(&device)? {
            continue;
        }
        let m = Mounted { partition: name.to_string(), device, mount_point, filesystem: shown.filesystem };
        found.push((minor.parse::<u32>()?, m));
    }
    found.sort_by_key(|(minor, _)| *minor);
    Ok(found.into_iter().map(|(_, m)| m).collect())
}

/// Take the slot's mounted partitions away before an update writes over
/// them: unmount them and remove their device-mapper devices. A partition
/// whose mount point is in `busy` still has files in use; it only leaves
/// the tree, and those files and its device stay until let go. /metadata
/// stays mounted for the update.
pub fn release_partitions(slot: Slot, busy: &[PathBuf]) -> Result<()> {
    for m in mounted_partitions(slot)?.into_iter().rev() {
        if busy.contains(&m.mount_point) {
            mount::umount_detach_over(&m.mount_point)?;
        } else {
            mount::umount_over(&m.mount_point)?;
            snapshots::unmap_snapshot(&m.partition)?;
        }
    }
    Ok(())
}

/// Take the slot's mounted partitions away for an update that writes them
/// in place, as an update does on a phone without slots: each must be off
/// its mount point and out of device-mapper before the update maps it
/// anew. What still runs from one is ended first, the programs the phone's
/// own partitions brought up among them, and the loop devices whose files
/// lie on it are let go, wherever their mounts are. A partition still held
/// after that is left out of the tree and mapped, as on a phone with slots,
/// where the update writes the other slot.
#[cfg(target_os = "android")]
pub fn free_partitions(slot: Slot) -> Result<()> {
    use skyward_block::loopdev;
    for m in mounted_partitions(slot)?.into_iter().rev() {
        let device = devnum(&m.device)?;
        let loops = loopdev::backed_by(device)?;
        for node in &loops {
            for point in mount::points_of(devnum(node)?)? {
                end_users(&point);
                if let Err(e) = mount::umount(&point) {
                    mount::umount_detach(&point).with_context(|| format!("take {} off {} ({e:#})", node.display(), point.display()))?;
                }
            }
            loopdev::clear(node)?;
        }
        let mut freed = false;
        for _ in 0..20 {
            end_users(&m.mount_point);
            match mount::umount_over(&m.mount_point) {
                Ok(()) => {
                    freed = true;
                    break;
                }
                Err(e) if busy(&e) => std::thread::sleep(std::time::Duration::from_millis(50)),
                Err(e) => return Err(e),
            }
        }
        match freed {
            true => snapshots::unmap_snapshot(&m.partition)?,
            false => {
                let _ = skyward_sys::kmsg(&format!("{} is still in use, and only leaves the tree", m.partition));
                mount::umount_detach_over(&m.mount_point)?;
            }
        }
    }
    Ok(())
}

/// Whether an unmount failed for something still using the filesystem.
#[cfg(target_os = "android")]
fn busy(e: &anyhow::Error) -> bool {
    e.chain().filter_map(|e| e.downcast_ref::<std::io::Error>()).any(|e| e.raw_os_error() == Some(libc::EBUSY))
}

/// End every process that has a file mapped or open from under `point`,
/// but init and this one: it would run on from what is about to be
/// written over.
#[cfg(target_os = "android")]
fn end_users(point: &Path) {
    let under = |path: &Path| path.starts_with(point) && path != point;
    let me = std::process::id();
    let Ok(processes) = fs::read_dir("/proc") else { return };
    for process in processes.flatten() {
        let Some(pid) = process.file_name().to_str().and_then(|n| n.parse::<u32>().ok()) else { continue };
        if pid == 1 || pid == me {
            continue;
        }
        let dir = process.path();
        let mapped = fs::read_to_string(dir.join("maps"))
            .is_ok_and(|maps| maps.lines().filter_map(|line| line.split_whitespace().nth(5)).any(|file| under(Path::new(file))));
        let open = || {
            fs::read_dir(dir.join("fd")).is_ok_and(|fds| fds.flatten().any(|fd| fs::read_link(fd.path()).is_ok_and(|file| under(&file))))
        };
        if mapped || open() {
            let name = fs::read_to_string(dir.join("comm")).unwrap_or_default();
            let _ = skyward_sys::kmsg(&format!("ended {pid} ({}), which ran from {}", name.trim(), point.display()));
            unsafe { libc::kill(pid as libc::pid_t, libc::SIGKILL) };
        }
    }
}

/// Undo `mount_partitions`: unmount the slot's partitions where their own
/// device is what shows, remove their device-mapper devices, and unmount
/// /metadata.
pub fn unmount_partitions(slot: Slot) -> Result<()> {
    let md = skyward_lp::read(&super_device(), slot.number())?;
    for p in slot_partitions(&md, slot).into_iter().rev() {
        unmap(&p.name, &mount_point(base(p, slot)))?;
    }
    let metadata = Path::new(METADATA);
    let metadata_dev = devnum(Path::new(BY_NAME).join("metadata"))?;
    if mount::visible(metadata)?.is_some_and(|v| v.device == metadata_dev) {
        mount::umount(metadata)?;
    }
    Ok(())
}

#[cfg(test)]
mod naming {
    use super::*;
    use skyward_lp::{BlockDevice, Geometry, Group};

    fn super_of(names: &[&str]) -> Metadata {
        let extent = Extent::Linear { sectors: 8, device: 0, sector: 2048 };
        Metadata {
            geometry: Geometry { metadata_max_size: 65536, metadata_slot_count: 2, logical_block_size: 4096 },
            major_version: 10,
            minor_version: 0,
            header_flags: 0,
            partitions: names.iter().map(|n| Partition { name: n.to_string(), attributes: 0, group: 0, extents: vec![extent] }).collect(),
            groups: vec![Group { name: "default".into(), maximum_size: 0 }],
            block_devices: vec![BlockDevice { partition_name: "super".into(), first_logical_sector: 2048, alignment: 0, alignment_offset: 0, size: 1 << 30 }],
            damaged: Vec::new(),
        }
    }

    #[test]
    fn an_ab_phones_super_names_each_partition_after_its_slot() {
        let md = super_of(&["system_a", "system_b", "vendor_a", "vendor_b", "system_b-cow"]);
        assert!(slotted(&md));
        let b: Vec<&str> = slot_partitions(&md, Slot::B).into_iter().map(|p| base(p, Slot::B)).collect();
        assert_eq!(b, ["system", "vendor"], "the slot's own, without the copy-on-write one");
        assert_eq!(named(&md, "vendor", Slot::B), "vendor_b");
    }

    #[test]
    fn a_phone_without_slots_keeps_one_set_named_bare() {
        let md = super_of(&["system", "vendor", "product", "odm"]);
        assert!(!slotted(&md));
        let a: Vec<&str> = slot_partitions(&md, Slot::A).into_iter().map(|p| base(p, Slot::A)).collect();
        assert_eq!(a, ["system", "vendor", "product", "odm"]);
        assert_eq!(named(&md, "vendor", Slot::A), "vendor", "no suffix to add");
        let mut empty = md.clone();
        empty.partitions[3].extents.clear();
        assert_eq!(slot_partitions(&empty, Slot::A).len(), 3, "a partition that holds nothing stays out");
    }
}

#[cfg(all(test, target_os = "android"))]
mod tests {
    use super::*;
    use crate::current_slot;
    use std::io::{Read, Seek, SeekFrom, Write};

    fn run(program: &str, args: &[&str]) -> String {
        let out = std::process::Command::new(program).args(args).output().unwrap();
        assert!(out.status.success(), "{program} {args:?}: {}", String::from_utf8_lossy(&out.stderr));
        String::from_utf8(out.stdout).unwrap().trim().to_string()
    }

    /// What `device` holds from `at`, `len` bytes of it.
    fn read_at(device: &Path, at: u64, len: usize) -> Vec<u8> {
        let mut file = fs::File::open(device).unwrap();
        skyward_block::drop_cache(&file).unwrap();
        file.seek(SeekFrom::Start(at)).unwrap();
        let mut buf = vec![0u8; len];
        file.read_exact(&mut buf).unwrap();
        buf
    }

    /// The bytes an image of `len` bytes holds, told apart by `seed`.
    fn image(len: usize, seed: u8) -> Vec<u8> {
        (0..len).map(|i| (i % 251) as u8 ^ seed).collect()
    }

    fn write_whole(device: &Path, data: &[u8]) {
        let mut file = fs::OpenOptions::new().write(true).open(device).unwrap();
        file.write_all(data).unwrap();
        file.sync_all().unwrap();
    }

    /// A copy of the phone's own super, its metadata alone, in a file as
    /// large as super whose rest is a hole, so that nothing here writes to
    /// the phone's partitions: one of its partitions is resized, mapped and
    /// written as an image goes into it.
    #[test]
    fn on_the_phone_a_logical_partition_takes_the_size_of_an_image_and_is_mapped_for_it() {
        let slot = current_slot().unwrap().expect("an A/B phone");
        let real = super_device();
        let md = skyward_lp::read(&real, slot.number()).unwrap();
        let head = read_at(&real, 0, (md.block_devices[0].first_logical_sector * 512) as usize);
        let file = std::env::temp_dir().join(format!("skyward-super-{}.img", std::process::id()));
        let mut copy = fs::File::create(&file).unwrap();
        copy.write_all(&head).unwrap();
        copy.set_len(md.block_devices[0].size).unwrap();
        drop(copy);
        let device = PathBuf::from(run("/system/bin/losetup", &["-f", "--show", file.to_str().unwrap()]));

        // A name of its own, which nothing the recovery mapped has.
        let mut md = skyward_lp::read(&device, slot.number()).unwrap();
        let taken = md.partitions.iter().position(|p| p.name == format!("vendor_dlkm{}", slot.suffix())).expect("super lists vendor_dlkm");
        let name = format!("scratch{}", slot.suffix());
        md.partitions[taken].name = name.clone();
        skyward_lp::write(&device, &md, &[slot.number()]).unwrap();
        let was = md.partitions[taken].bytes();

        // Smaller: it gives up its last sectors, and is mapped anew.
        let small = image(8 << 20, 0x5a);
        let remade = remake_on(&device, slot, "scratch", small.len() as u64 - 100).unwrap();
        assert_eq!((remade.name.as_str(), remade.size), (name.as_str(), 8 << 20));
        assert_eq!(skyward_block::size_bytes(&remade.device).unwrap(), 8 << 20);
        write_whole(&remade.device, &small);
        let Extent::Linear { sector, .. } = md.partitions[taken].extents[0] else { panic!("its first extent lies on super") };
        assert_eq!(read_at(&device, sector * 512, small.len()), small, "what is written to it lies where its first extent is");
        settle(&remade).unwrap();
        assert!(dm::status(&name).unwrap().is_none(), "a mapping made for the image goes with it");
        for number in [0, 1] {
            let written = skyward_lp::read(&device, number).unwrap();
            assert_eq!(written.partition(&name).unwrap().bytes(), 8 << 20, "both slots read the new size");
        }

        // Larger again, while something holds the mapping it had: the
        // mapping takes the new table where it is, and stays read-only.
        let before = skyward_lp::read(&device, slot.number()).unwrap();
        let p = before.partition(&name).unwrap();
        let held = dm::create(&name, &targets(p, &block_devices(&before, &device).unwrap()), true).unwrap();
        let large = image(24 << 20, 0xa5);
        assert!(was >= 24 << 20, "the room it gave up holds it again");
        let remade = remake_on(&device, slot, "scratch", large.len() as u64).unwrap();
        assert_eq!(remade.device, held);
        assert_eq!(skyward_block::size_bytes(&held).unwrap(), 24 << 20);
        write_whole(&held, &large);
        assert_eq!(read_at(&held, 0, large.len()), large);
        settle(&remade).unwrap();
        assert!(dm::readonly(&name).unwrap(), "it is read-only as it was found");
        assert_eq!(read_at(&held, 0, large.len()), large);

        // More than super has free changes nothing.
        let unchanged = skyward_lp::read(&device, slot.number()).unwrap();
        dm::remove(&name).unwrap();
        let err = remake_on(&device, slot, "scratch", md.block_devices[0].size).unwrap_err();
        assert!(format!("{err:#}").contains("too few free") || format!("{err:#}").contains("its group"), "{err:#}");
        assert_eq!(skyward_lp::read(&device, slot.number()).unwrap(), unchanged);
        assert!(dm::status(&name).unwrap().is_none());

        run("/system/bin/losetup", &["-d", device.to_str().unwrap()]);
        fs::remove_file(&file).unwrap();
    }
}
