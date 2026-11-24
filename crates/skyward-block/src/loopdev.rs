//! Making a block device out of part of a file, which is how the bundles a
//! phone keeps its shared libraries in are opened.

#[cfg(target_os = "android")]
pub use android::*;

#[cfg(target_os = "android")]
mod android {
    use anyhow::{Context, Result};
    use std::fs::File;
    use std::os::fd::AsRawFd;
    use std::path::{Path, PathBuf};

    const CONTROL: &str = "/dev/loop-control";

    const GET_FREE: u32 = 0x4C82;
    const CONFIGURE: u32 = 0x4C0A;
    const GET_STATUS64: u32 = 0x4C05;
    const CLEAR_FD: u32 = 0x4C01;

    /// The device only ever reads. It stays pointed at the file until the
    /// recovery ends, which is as long as anything mounted from it lasts.
    const READ_ONLY: u32 = 1;

    /// Every field is a plain number or a run of bytes, so the request is
    /// made by zeroing it and filling in what matters.
    #[repr(C)]
    struct Info {
        device: u64,
        inode: u64,
        rdevice: u64,
        offset: u64,
        size_limit: u64,
        number: u32,
        encrypt_type: u32,
        encrypt_key_size: u32,
        flags: u32,
        file_name: [u8; 64],
        crypt_name: [u8; 64],
        encrypt_key: [u8; 32],
        init: [u64; 2],
    }

    #[repr(C)]
    struct Config {
        fd: u32,
        block_size: u32,
        info: Info,
        reserved: [u64; 8],
    }

    /// Open `file` from `offset` for `length` bytes as a block device, and
    /// give its path. The device goes away once whatever is mounted from it
    /// is unmounted.
    pub fn attach(file: &Path, offset: u64, length: u64) -> Result<PathBuf> {
        let backing = File::open(file).with_context(|| format!("open {}", file.display()))?;
        let control = File::open(CONTROL).with_context(|| format!("open {CONTROL}"))?;
        let number = unsafe { libc::ioctl(control.as_raw_fd(), GET_FREE as libc::Ioctl) };
        if number < 0 {
            return Err(std::io::Error::last_os_error()).context("ask for a free loop device");
        }
        let path = free_device(number as u32)?;
        let device = File::options()
            .read(true)
            .write(true)
            .open(&path)
            .with_context(|| format!("open {}", path.display()))?;
        let mut config: Config = unsafe { std::mem::zeroed() };
        config.fd = backing.as_raw_fd() as u32;
        config.info.offset = offset;
        config.info.size_limit = length;
        config.info.flags = READ_ONLY;
        if unsafe { libc::ioctl(device.as_raw_fd(), CONFIGURE as libc::Ioctl, &mut config) } < 0 {
            return Err(std::io::Error::last_os_error()).with_context(|| format!("point {} at {}", path.display(), file.display()));
        }
        Ok(path)
    }

    /// The loop devices, by their nodes, whose backing file lies on the
    /// filesystem of the block device `device`: whatever path the file was
    /// opened by, and in whichever mount namespace, the kernel says which
    /// device holds it.
    pub fn backed_by(device: (u32, u32)) -> Result<Vec<PathBuf>> {
        let wanted = libc::makedev(device.0, device.1) as u64;
        let mut found = Vec::new();
        for entry in std::fs::read_dir("/sys/block").context("read /sys/block")? {
            let entry = entry.context("read /sys/block")?;
            let name = entry.file_name().to_string_lossy().into_owned();
            let Some(number) = name.strip_prefix("loop").and_then(|n| n.parse::<u32>().ok()) else { continue };
            // A loop device pointed at nothing has no such folder.
            if !entry.path().join("loop").exists() {
                continue;
            }
            let Ok(node) = free_device(number) else { continue };
            let Ok(file) = File::open(&node) else { continue };
            let mut info: Info = unsafe { std::mem::zeroed() };
            if unsafe { libc::ioctl(file.as_raw_fd(), GET_STATUS64 as libc::Ioctl, &mut info) } < 0 {
                continue;
            }
            if info.device == wanted {
                found.push(node);
            }
        }
        Ok(found)
    }

    /// Let the loop device at `node` go: pointed at nothing at once where
    /// nothing has it open, and otherwise as soon as the last user lets it
    /// go, its backing file with it.
    pub fn clear(node: &Path) -> Result<()> {
        let file = File::open(node).with_context(|| format!("open {}", node.display()))?;
        if unsafe { libc::ioctl(file.as_raw_fd(), CLEAR_FD as libc::Ioctl, 0) } < 0 {
            let e = std::io::Error::last_os_error();
            // One pointed at nothing already has nothing to let go.
            if e.raw_os_error() != Some(libc::ENXIO) {
                return Err(e).with_context(|| format!("let {} go", node.display()));
            }
        }
        Ok(())
    }

    /// Where the kernel put the loop device it just handed out: Android
    /// keeps its block devices in their own directory.
    fn free_device(number: u32) -> Result<PathBuf> {
        for dir in ["/dev/block", "/dev"] {
            let path = Path::new(dir).join(format!("loop{number}"));
            if path.exists() {
                return Ok(path);
            }
        }
        anyhow::bail!("the kernel handed out loop device {number}, which is in neither /dev/block nor /dev")
    }
}
