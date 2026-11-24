use anyhow::{ensure, Context, Result};
use clap::{Parser, Subcommand};
use sha2::{Digest, Sha256};
use skyward_config::ramdisk as paths;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

mod avb;
mod described;
mod odin;

use skyward_bootimg::{BootHeader, BootImage, Compression, Ramdisk};

/// Builds Skyward recovery images from a device's own stock images.
#[derive(Parser)]
#[command(name = "skyward-pack")]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Print an image's header and the files in its ramdisk.
    List { image: PathBuf },
    /// Write one ramdisk file to stdout.
    Cat { image: PathBuf, path: String },
    /// Unpack and pack an image's ramdisk unchanged, to check that a rebuilt
    /// archive keeps every entry.
    Repack { image: PathBuf, out: PathBuf },
    /// Join a kernel and another image's ramdisk into one boot image.
    Boot {
        /// Image holding the kernel (the device's boot partition).
        #[arg(long)]
        kernel: PathBuf,
        /// Image holding the ramdisk.
        #[arg(long)]
        ramdisk: PathBuf,
        #[arg(long)]
        out: PathBuf,
    },
    /// Build a Skyward recovery image on top of the device's stock recovery,
    /// or where its configuration's [base] says so, on a recovery of another
    /// make: its init, adbd, SELinux policy and libraries stay, the recovery
    /// service becomes Skyward. The image fills the whole partition and ends
    /// in an AVB footer of its own, unsigned unless a key is given.
    Recovery {
        /// The recovery image to build on: the whole partition, as the phone
        /// or a full update package holds it, ending in its AVB footer, or
        /// an image of the size the configuration's [base] gives.
        #[arg(long, alias = "base")]
        stock: PathBuf,
        /// Skyward built for the device (aarch64-linux-android).
        #[arg(long)]
        binary: PathBuf,
        /// The first program of a system installed beside the installed
        /// one, skyward-system-init built for the device and linked
        /// statically.
        #[arg(long)]
        system_init: PathBuf,
        /// The device configuration, devices/<codename>.toml.
        #[arg(long)]
        config: PathBuf,
        /// The UI typeface.
        #[arg(long)]
        font: PathBuf,
        /// The terminal's monospaced typeface.
        #[arg(long)]
        mono: PathBuf,
        /// The monospaced typeface's licence, which goes with it.
        #[arg(long)]
        mono_license: PathBuf,
        /// A file to put in the ramdisk, as HOST=PATH with PATH inside the
        /// ramdisk: a kernel module the recovery's kernel does not carry,
        /// or firmware or a library its drivers and programs ask for there.
        #[arg(long = "add-file", value_name = "HOST=PATH")]
        add_files: Vec<String>,
        /// An RSA private key, in PEM, to sign the image's vbmeta footer
        /// with as avbtool signs one, for a bootloader that reads the
        /// signature; the footer is left unsigned without one.
        #[arg(long)]
        avb_key: Option<PathBuf>,
        #[arg(long)]
        out: PathBuf,
    },
    /// Pack an image as Samsung's Odin and Heimdall take it: a tar holding
    /// it under the name of the partition it goes to, ending in the line of
    /// the tar's MD5 that Odin checks it by.
    Odin {
        #[arg(long)]
        image: PathBuf,
        /// The name it goes by in the tar, which tells Odin the partition.
        #[arg(long, default_value = "recovery.img")]
        name: String,
        /// The .tar.md5 to write.
        #[arg(long)]
        out: PathBuf,
    },
    /// Print the stock build a device's recovery is made on, as lines of
    /// name=value: its version, where its full update package is, and the
    /// SHA-256 of the recovery partition in that package. Given somewhere
    /// to write it, also take that partition out of the package on its
    /// server, reading only what the partition takes, and write it there
    /// once its SHA-256 is that one.
    Stock {
        /// The device configuration, devices/<codename>.toml.
        #[arg(long)]
        config: PathBuf,
        /// Where to write the stock recovery image.
        #[arg(long)]
        out: Option<PathBuf>,
    },
    /// Take what a device's own partitions say of the programs they carry
    /// out of its stock build's full update package on its server: the
    /// init scripts, the service descriptions, the accounts and the build
    /// properties of each partition that carries such programs, written
    /// under `out` as the partition and then the path in it. Only the
    /// parts of the package that hold these files are downloaded, each
    /// checked against the package's own sum of it.
    Described {
        /// The device configuration, devices/<codename>.toml.
        #[arg(long)]
        config: PathBuf,
        /// A partition to take them out of; vendor, odm and system_ext
        /// when none is named.
        #[arg(long = "partition")]
        partitions: Vec<String>,
        /// The folder to write the files under, which is not there yet.
        #[arg(long)]
        out: PathBuf,
    },
}

fn main() -> Result<()> {
    match Cli::parse().cmd {
        Cmd::List { image } => list(&image),
        Cmd::Cat { image, path } => {
            let rd = Ramdisk::decode(&BootImage::read(&image)?.ramdisk)?;
            let e = rd.find(&path).with_context(|| format!("{path} is not in the ramdisk"))?;
            std::io::stdout().write_all(&e.data)?;
            Ok(())
        }
        Cmd::Repack { image, out } => {
            let mut img = BootImage::read(&image)?;
            let compression = Compression::of(&img.ramdisk)?;
            img.ramdisk = Ramdisk::decode(&img.ramdisk)?.encode_as(compression)?;
            write(&out, &img, &img.to_bytes()?)
        }
        Cmd::Odin { image, name, out } => {
            let bytes = std::fs::read(&image).with_context(|| format!("read {}", image.display()))?;
            let tar_name = out
                .file_name()
                .and_then(|n| n.to_str())
                .and_then(|n| n.strip_suffix(".md5"))
                .filter(|n| n.ends_with(".tar"))
                .with_context(|| format!("{} is not named as Odin takes a package, <name>.tar.md5", out.display()))?;
            std::fs::write(&out, odin::package(&name, &bytes, tar_name)?).with_context(|| format!("write {}", out.display()))?;
            println!("wrote {}, holding {} as {name}", out.display(), image.display());
            Ok(())
        }
        Cmd::Boot { kernel, ramdisk, out } => {
            let mut img = BootImage::read(&ramdisk)?;
            img.kernel = BootImage::read(&kernel)?.kernel;
            write(&out, &img, &img.to_bytes()?)
        }
        Cmd::Recovery { stock, binary, system_init, config, font, mono, mono_license, add_files, avb_key, out } => {
            let cfg = skyward_config::DeviceConfig::load(&config)?;
            let base = &cfg.base;
            let read = |p: &PathBuf| std::fs::read(p).with_context(|| format!("read {}", p.display()));
            let whole = read(&stock)?;
            match &base.sha256 {
                Some(sum) => {
                    let found = hex::encode(Sha256::digest(&whole));
                    ensure!(&found == sum, "{} has the SHA-256 {found}, where the configuration's [base] names {sum}", stock.display());
                }
                None if base.third_party => {
                    eprintln!("warning: the configuration's [base] names no SHA-256 to check {} against", stock.display());
                }
                None => {}
            }
            let size = match base.partition_size {
                Some(size) => {
                    ensure!(whole.len() as u64 <= size, "{} is {} bytes, more than the {size}-byte partition [base] names", stock.display(), whole.len());
                    usize::try_from(size)?
                }
                None => partition_size(&stock)?,
            };
            let mut img = BootImage::parse(&whole).with_context(|| format!("read {}", stock.display()))?;
            let compression = Compression::of(&img.ramdisk).with_context(|| format!("read the ramdisk of {}", stock.display()))?;
            let mut rd = Ramdisk::decode(&img.ramdisk)?;
            // A stock recovery ramdisk has no third-party tree; refuse to build
            // on top of another custom recovery unless the configuration says
            // the phone is built on one.
            let foreign = rd.entries.iter().find(|e| e.name == "FFiles" || e.name.starts_with("twres")).map(|e| e.name.clone());
            match (base.third_party, foreign) {
                (false, Some(name)) => anyhow::bail!("{} holds {name}, which is not a stock recovery", stock.display()),
                (true, None) => anyhow::bail!("{} holds no TWRP tree, where the configuration's [base] names a recovery of another make", stock.display()),
                _ => {}
            }
            // What only the base's own program and theme used.
            for name in &base.drop {
                match name.strip_suffix('/') {
                    Some(dir) => rd.remove_tree(dir)?,
                    None => rd.remove(name)?,
                }
            }
            rd.replace(paths::RECOVERY, read(&binary)?)?;
            // Install scripts look for the commands a stock recovery lacks
            // among its programs, and its unzip there too, a link to ziptool
            // until it points here; the recovery binary is each of them. A
            // recovery of another make carries programs of its own under
            // some of those names, which give way to the links.
            let (_, recovery) = paths::RECOVERY.rsplit_once('/').expect("the recovery binary is in a folder");
            for command in paths::COMMANDS {
                match base.third_party {
                    true => rd.link_over(command, recovery)?,
                    false => rd.link(command, recovery)?,
                }
            }
            rd.add(paths::SYSTEM_INIT, 0o755, read(&system_init)?)?;
            rd.add(paths::CONFIG, 0o644, read(&config)?)?;
            rd.add(paths::FONT, 0o644, read(&font)?)?;
            rd.add(paths::MONO, 0o644, read(&mono)?)?;
            rd.add(paths::MONO_LICENSE, 0o644, read(&mono_license)?)?;
            // A stock ramdisk may run logd without any scheduling profiles,
            // and logd stops as soon as it cannot find the one it asks for.
            if rd.find(paths::TASK_PROFILES).is_none() {
                rd.add(paths::TASK_PROFILES, 0o644, TASK_PROFILES.to_vec())?;
            }
            if base.mtp {
                rd.add(paths::MTP_RC, 0o644, MTP_RC.to_vec())?;
            }
            for file in &add_files {
                let (host, into) = added(file)?;
                rd.add(into, 0o644, read(&PathBuf::from(host))?).with_context(|| format!("add {host} as {into}"))?;
            }
            // On an unlocked device adbd keeps root when ro.secure is 0 and
            // lets any computer in without authorization when ro.adb.secure is
            // 0; init lets a later line of a property file override an earlier
            // one. update_engine refuses a package built before the recovery
            // itself unless both of the last two are set; the recovery checks
            // packages against the installed system instead.
            let stock_props = &rd
                .find(paths::PROP_DEFAULT)
                .with_context(|| format!("{} is not in the ramdisk", paths::PROP_DEFAULT))?
                .data;
            let mut props = without(stock_props, &BUILD_VERSION);
            props.extend_from_slice(b"\nro.secure=0\nro.adb.secure=0\nro.debuggable=1\nro.ota.allow_downgrade=true\n");
            rd.replace(paths::PROP_DEFAULT, props)?;
            // The kernel the base carries unpacks the compression it came in.
            img.ramdisk = rd.encode_as(compression)?;
            let bytes = img.to_bytes()?;
            let key = avb_key.as_deref().map(avb::Key::read).transpose()?;
            write(&out, &img, &avb::footed(&bytes, "recovery", size, &Sha256::digest(&bytes), key.as_ref())?)
        }
        Cmd::Stock { config, out } => {
            let stock = skyward_config::DeviceConfig::load(&config)?.stock.with_context(|| format!("{} names no stock build", config.display()))?;
            println!("version={}\npackage={}\nrecovery={}", stock.version, stock.package, stock.recovery);
            match out {
                Some(out) => take_stock_recovery(&stock, &out),
                None => Ok(()),
            }
        }
        Cmd::Described { config, partitions, out } => {
            let stock = skyward_config::DeviceConfig::load(&config)?.stock.with_context(|| format!("{} names no stock build", config.display()))?;
            let partitions = if partitions.is_empty() { skyward_programs::phone::PARTITIONS.map(str::to_string).to_vec() } else { partitions };
            described::take(&stock.package, &partitions, &out)
        }
    }
}

/// The partition a recovery is made on, as a full update package names it.
const RECOVERY: &str = "recovery";

/// Take the recovery partition out of the stock build's full update package
/// on its server, reading only what that partition takes of it, and write it
/// to `out` once its SHA-256 is the one the configuration names.
fn take_stock_recovery(stock: &skyward_config::Stock, out: &Path) -> Result<()> {
    let parent = out.parent().filter(|p| !p.as_os_str().is_empty()).unwrap_or(Path::new("."));
    std::fs::create_dir_all(parent).with_context(|| format!("make {}", parent.display()))?;
    // Taken out beside where it goes, so it is moved there whole.
    let work = tempfile::Builder::new().prefix(".stock").tempdir_in(parent).with_context(|| format!("make a folder in {}", parent.display()))?;
    let payload = unpayload::Payload::open_url(&stock.package).with_context(|| format!("open {}", stock.package))?;
    let options = unpayload::ExtractOptions { output_dir: work.path().to_path_buf(), partitions: vec![RECOVERY.into()], ..Default::default() };
    let report = |event: unpayload::Event<'_>| match event {
        unpayload::Event::Started { partitions } => {
            for (name, bytes) in partitions {
                println!("taking {name} out of the package: {bytes} bytes");
            }
        }
        unpayload::Event::Extracted(image) => println!("took {} out: {} bytes", image.partition, image.size),
        unpayload::Event::Progress { .. } => {}
    };
    let taken = payload.extract(&options, &report).with_context(|| format!("take {RECOVERY} out of {}", stock.package))?;
    let image = taken.iter().find(|i| i.partition == RECOVERY).with_context(|| format!("{} holds no {RECOVERY} partition", stock.package))?;
    let sum = hex::encode(image.sha256.as_deref().context("the package's own sum of the image was not checked")?);
    ensure!(sum == stock.recovery, "the {RECOVERY} partition in {} has the SHA-256 {sum}, where the configuration names {}", stock.package, stock.recovery);
    std::fs::rename(&image.path, out).with_context(|| format!("move {} to {}", image.path.display(), out.display()))?;
    println!("wrote {} ({} bytes), its SHA-256 the one the configuration names", out.display(), image.size);
    Ok(())
}

/// The size of the partition `stock` fills: all of it, as the phone and a
/// full update package hold it, which its AVB footer ends.
fn partition_size(stock: &Path) -> Result<usize> {
    let mut file = File::open(stock).with_context(|| format!("open {}", stock.display()))?;
    let size = file.metadata().with_context(|| format!("stat {}", stock.display()))?.len();
    let mut magic = [0u8; 4];
    if size >= 64 {
        file.seek(SeekFrom::Start(size - 64)).with_context(|| format!("seek in {}", stock.display()))?;
        file.read_exact(&mut magic).with_context(|| format!("read {}", stock.display()))?;
    }
    ensure!(
        &magic == b"AVBf",
        "{} does not end in an AVB footer; give the whole recovery partition, as the phone or a full update package holds it",
        stock.display()
    );
    Ok(usize::try_from(size)?)
}

/// What a recovery image says about the build it is: the version and patch
/// levels of its own ramdisk's system and vendor partition, which are older
/// than those installed beside it whenever the phone has taken an update.
/// The phone's secure services read these to tell the secure world which
/// build is running, and keys the installed system made are refused when
/// they say the wrong one, so they are left out here for the recovery to
/// fill in from the installed system once it has that mounted.
const BUILD_VERSION: [&str; 3] = ["ro.build.version.release", "ro.build.version.security_patch", "ro.vendor.build.security_patch"];

/// The scheduling profiles a recovery's programs look up by name. A
/// recovery's cgroup setup mounts no controllers, so of all a system's
/// profiles do, only the timer slack applies here.
const TASK_PROFILES: &[u8] = include_bytes!("task_profiles.json");

/// The init rules that give the USB port file transfer beside adb, which a
/// stock recovery does not offer.
const MTP_RC: &[u8] = include_bytes!("mtp.rc");

/// A property file without the lines that set any of `names`.
fn without(props: &[u8], names: &[&str]) -> Vec<u8> {
    let mut out = Vec::with_capacity(props.len());
    for line in props.split_inclusive(|b| *b == b'\n') {
        let key = line.split(|b| *b == b'=').next().unwrap_or_default();
        if !names.iter().any(|n| n.as_bytes() == key) {
            out.extend_from_slice(line);
        }
    }
    out
}

/// A file `--add-file` puts in the ramdisk, given as HOST=PATH: the file on
/// this computer, and where it goes in the ramdisk, from its top.
fn added(given: &str) -> Result<(&str, &str)> {
    let (host, into) = given.split_once('=').with_context(|| format!("{given} is not HOST=PATH"))?;
    let into = into.trim_start_matches('/');
    let plain = Path::new(into).components().all(|c| matches!(c, std::path::Component::Normal(_)));
    ensure!(!host.is_empty() && plain && Path::new(into).file_name().is_some(), "{given}: {into} is not a path inside the ramdisk");
    Ok((host, into))
}

/// Write `bytes`, the image `img` makes, to `out`.
fn write(out: &Path, img: &BootImage, bytes: &[u8]) -> Result<()> {
    std::fs::write(out, bytes).with_context(|| format!("write {}", out.display()))?;
    println!(
        "wrote {} ({} bytes kernel, {} bytes ramdisk, {} bytes in all)",
        out.display(),
        img.kernel.len(),
        img.ramdisk.len(),
        bytes.len()
    );
    Ok(())
}

fn list(image: &Path) -> Result<()> {
    let img = BootImage::read(image)?;
    let h = &img.header;
    print!("header v{}  kernel {} bytes  ramdisk {} bytes  {:?}", h.header_version(), img.kernel.len(), img.ramdisk.len(), h.osversionpatch());
    if let BootHeader::Legacy(legacy) = h {
        print!("  page {} bytes  dtb {} bytes", legacy.page_size, img.dtb.len());
    }
    println!();
    if img.ramdisk.is_empty() {
        return Ok(());
    }
    println!("ramdisk compressed as {:?}", Compression::of(&img.ramdisk)?);
    let rd = Ramdisk::decode(&img.ramdisk)?;
    for e in &rd.entries {
        println!("{:06o} {:>5}:{:<5} {:>10}  {}", e.mode, e.uid, e.gid, e.data.len(), e.name);
    }
    println!("{} entries", rd.entries.len());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_lines_that_set_a_name_are_the_only_ones_left_out() {
        let props = b"ro.build.id=TQ\nro.build.version.release=13\nro.build.version.release_or_codename=13\nro.product.name=ishtar\n\
ro.vendor.build.security_patch=2026-10-01\nro.vendor.build.security_patch_level=1\n";
        let out = without(props, &BUILD_VERSION);
        let text = String::from_utf8(out).expect("property files are text");
        assert!(!text.contains("ro.build.version.release=13"));
        assert!(!text.contains("ro.vendor.build.security_patch=2026-10-01"), "the vendor partition's patch level is left out too");
        assert!(text.contains("ro.build.version.release_or_codename=13"), "a longer name is left alone");
        assert!(text.contains("ro.vendor.build.security_patch_level=1"), "a longer name is left alone");
        assert!(text.contains("ro.build.id=TQ") && text.contains("ro.product.name=ishtar"));
    }

    #[test]
    fn the_profiles_name_all_the_recovery_asks_for() {
        let profiles: serde_json::Value = serde_json::from_slice(TASK_PROFILES).expect("the profiles are JSON");
        let names = |key: &str| -> Vec<String> {
            profiles[key].as_array().expect("a list").iter().map(|p| p["Name"].as_str().expect("a name").to_string()).collect()
        };
        let (plain, aggregate) = (names("Profiles"), names("AggregateProfiles"));
        // What libprocessgroup asks for when a program sets its scheduling
        // policy, and what the recovery's own init scripts give logd.
        let asked = [
            "SCHED_SP_DEFAULT",
            "SCHED_SP_BACKGROUND",
            "SCHED_SP_FOREGROUND",
            "SCHED_SP_TOP_APP",
            "SCHED_SP_SYSTEM",
            "SCHED_SP_RT_APP",
            "CPUSET_SP_DEFAULT",
            "CPUSET_SP_BACKGROUND",
            "CPUSET_SP_FOREGROUND",
            "CPUSET_SP_TOP_APP",
            "CPUSET_SP_SYSTEM",
            "CPUSET_SP_RESTRICTED",
            "BlkIOBackground",
            "BlkIOForeground",
            "ServiceCapacityLow",
        ];
        for name in asked {
            assert!(plain.iter().chain(&aggregate).any(|n| n == name), "{name} is missing");
        }
        for a in profiles["AggregateProfiles"].as_array().expect("a list") {
            for part in a["Profiles"].as_array().expect("a list") {
                let part = part.as_str().expect("a name");
                assert!(plain.iter().any(|n| n == part), "{} is made of {part}, which is not a profile", a["Name"]);
            }
        }
    }

    #[test]
    fn the_partition_is_as_large_as_a_stock_image_ending_in_its_footer() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("recovery.img");
        let mut whole = vec![0u8; 8192];
        whole[8192 - 64..8192 - 60].copy_from_slice(b"AVBf");
        std::fs::write(&path, &whole).unwrap();
        assert_eq!(partition_size(&path).unwrap(), 8192);
        std::fs::write(&path, &whole[..8000]).unwrap();
        assert!(partition_size(&path).is_err(), "an image cut short of its partition's end");
        std::fs::write(&path, b"AVBf").unwrap();
        assert!(partition_size(&path).is_err(), "too short to end in a footer");
    }

    #[test]
    fn an_added_file_goes_to_a_path_inside_the_ramdisk() {
        assert_eq!(added("work/dhd.ko=lib/modules/dhd.ko").unwrap(), ("work/dhd.ko", "lib/modules/dhd.ko"));
        assert_eq!(added("a=/lib/firmware/y792_p3.bin").unwrap(), ("a", "lib/firmware/y792_p3.bin"), "from the ramdisk's top");
        for bad in ["dhd.ko", "a=../etc/passwd", "a=lib/../../x", "a=", "=lib/x", "a=/"] {
            assert!(added(bad).is_err(), "{bad} is refused");
        }
    }

    #[test]
    fn a_file_without_a_last_newline_keeps_its_last_line() {
        let out = without(b"a=1\nro.build.version.release=13", &BUILD_VERSION);
        assert_eq!(out, b"a=1\n");
        let kept = without(b"a=1\nb=2", &BUILD_VERSION);
        assert_eq!(kept, b"a=1\nb=2");
    }
}
