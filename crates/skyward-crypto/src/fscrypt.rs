//! Letting a key into a filesystem, so the files it was used on can be
//! read. The kernel keeps the key for the filesystem, not for any one
//! file, and names it by a hash it works out itself.

use anyhow::{bail, Context, Result};
use std::path::Path;

/// How long the name the kernel gives a key is.
pub const NAME_BYTES: usize = 16;

/// Let `raw` into the filesystem mounted at `mount`, and give the name the
/// kernel knows it by. Every file whose directory names that key can be
/// read from then on.
#[cfg(target_os = "android")]
pub fn add_key(mount: &Path, raw: &[u8], wrapped: bool) -> Result<[u8; NAME_BYTES]> {
    use std::os::fd::AsRawFd;
    /// The request's fixed part, before the key itself.
    const HEADER_BYTES: usize = 80;
    /// The key is named by what the kernel makes of it rather than by
    /// anything this side chooses.
    const BY_IDENTIFIER: u32 = 2;
    /// The key is one only the storage hardware can unwrap.
    const HARDWARE_WRAPPED: u32 = 1;
    const ADD_KEY: u32 = 0xc050_6617;

    let mut arg = vec![0u8; HEADER_BYTES + raw.len()];
    arg[0..4].copy_from_slice(&BY_IDENTIFIER.to_ne_bytes());
    arg[40..44].copy_from_slice(&(raw.len() as u32).to_ne_bytes());
    if wrapped {
        arg[76..80].copy_from_slice(&HARDWARE_WRAPPED.to_ne_bytes());
    }
    arg[HEADER_BYTES..].copy_from_slice(raw);
    let dir = std::fs::File::open(mount).with_context(|| format!("open {}", mount.display()))?;
    if unsafe { libc::ioctl(dir.as_raw_fd(), ADD_KEY as libc::Ioctl, arg.as_mut_ptr()) } != 0 {
        return Err(std::io::Error::last_os_error()).with_context(|| format!("let a key into {}", mount.display()));
    }
    let mut name = [0u8; NAME_BYTES];
    name.copy_from_slice(&arg[8..8 + NAME_BYTES]);
    Ok(name)
}

/// The name the kernel knows the key a folder is encrypted under by: the
/// first kind of policy names it by a descriptor its maker chose, the
/// second by what the kernel works out from the key itself.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyName {
    Descriptor([u8; 8]),
    Identifier([u8; NAME_BYTES]),
}

impl std::fmt::Display for KeyName {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let bytes: &[u8] = match self {
            KeyName::Descriptor(d) => d,
            KeyName::Identifier(i) => i,
        };
        bytes.iter().try_for_each(|b| write!(f, "{b:02x}"))
    }
}

/// The key the folder at `dir` is encrypted under, or none where it is not
/// encrypted, or its filesystem was made without the means to encrypt any,
/// as one that holds a partition the phone keeps plain may be.
#[cfg(target_os = "android")]
pub fn key_of(dir: &Path) -> Result<Option<KeyName>> {
    use std::os::fd::AsRawFd;
    const GET_POLICY: u32 = 0xc009_6616;
    /// The second kind of policy is the largest: its version, three bytes
    /// of how it encrypts, four reserved, and the key's name.
    const POLICY_BYTES: usize = 24;

    let folder = std::fs::File::open(dir).with_context(|| format!("open {}", dir.display()))?;
    let mut policy = [0u8; 8 + POLICY_BYTES];
    policy[..8].copy_from_slice(&(POLICY_BYTES as u64).to_ne_bytes());
    if unsafe { libc::ioctl(folder.as_raw_fd(), GET_POLICY as libc::Ioctl, policy.as_mut_ptr()) } != 0 {
        let e = std::io::Error::last_os_error();
        if matches!(e.raw_os_error(), Some(libc::ENODATA | libc::EOPNOTSUPP)) {
            return Ok(None);
        }
        return Err(e).with_context(|| format!("read how {} is encrypted", dir.display()));
    }
    match policy[8] {
        0 => Ok(Some(KeyName::Descriptor(policy[12..20].try_into().expect("eight bytes")))),
        2 => Ok(Some(KeyName::Identifier(policy[16..16 + NAME_BYTES].try_into().expect("sixteen bytes")))),
        other => bail!("{} is encrypted under a policy of version {other}, which is not known here", dir.display()),
    }
}

/// Whether the names and contents under `dir` read as they are: it is not
/// encrypted, or the key it is encrypted with is in the kernel. A user's
/// own files read only once that user's key is let in.
#[cfg(target_os = "android")]
pub fn unlocked(dir: &Path) -> Result<bool> {
    use std::os::fd::AsRawFd;
    const GET_KEY_STATUS: u32 = 0xc080_661a;
    /// How the first kind of policy names its key, and how the second does.
    const BY_DESCRIPTOR: u32 = 1;
    const BY_IDENTIFIER: u32 = 2;
    const PRESENT: u32 = 2;

    let Some(key) = key_of(dir)? else { return Ok(true) };
    let mut status = [0u8; 128];
    match key {
        KeyName::Descriptor(descriptor) => {
            status[..4].copy_from_slice(&BY_DESCRIPTOR.to_ne_bytes());
            status[8..16].copy_from_slice(&descriptor);
        }
        KeyName::Identifier(identifier) => {
            status[..4].copy_from_slice(&BY_IDENTIFIER.to_ne_bytes());
            status[8..8 + NAME_BYTES].copy_from_slice(&identifier);
        }
    }
    let folder = std::fs::File::open(dir).with_context(|| format!("open {}", dir.display()))?;
    if unsafe { libc::ioctl(folder.as_raw_fd(), GET_KEY_STATUS as libc::Ioctl, status.as_mut_ptr()) } != 0 {
        return Err(std::io::Error::last_os_error()).with_context(|| format!("ask whether the key for {} is in", dir.display()));
    }
    Ok(u32::from_ne_bytes(status[64..68].try_into().expect("four bytes")) == PRESENT)
}

/// Whether what a phone says about how it encrypts its files means the
/// keys are ones only the storage hardware can unwrap.
pub fn hardware_wrapped(mode: &str) -> bool {
    mode.split(['+', ':']).any(|part| part == "wrappedkey_v0")
}

/// Read the name the phone recorded for a key it let in, to check against
/// the name the kernel works out now.
pub fn recorded_name(path: &Path) -> Result<[u8; NAME_BYTES]> {
    let raw = std::fs::read(path).with_context(|| format!("read {}", path.display()))?;
    match <[u8; NAME_BYTES]>::try_from(&raw[..]) {
        Ok(name) => Ok(name),
        Err(_) => bail!("{} holds {} bytes, and a key's name is {NAME_BYTES}", path.display(), raw.len()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_phone_says_whether_the_hardware_holds_the_keys() {
        assert!(hardware_wrapped("aes-256-xts:aes-256-cts:v2+inlinecrypt_optimized+wrappedkey_v0"));
        assert!(!hardware_wrapped("aes-256-xts:aes-256-cts:v2"));
        assert!(!hardware_wrapped("aes-256-xts:aes-256-cts:v2+wrappedkey_v1"));
    }

    #[test]
    fn a_name_is_read_back_only_at_its_own_length() {
        let dir = std::env::temp_dir().join(format!("skyward-fscrypt-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let good = dir.join("ref");
        std::fs::write(&good, [7u8; NAME_BYTES]).unwrap();
        assert_eq!(recorded_name(&good).unwrap(), [7u8; NAME_BYTES]);
        let short = dir.join("short");
        std::fs::write(&short, [7u8; 8]).unwrap();
        assert!(recorded_name(&short).is_err());
    }

    #[test]
    fn a_key_is_named_in_the_log_as_the_kernel_names_it_in_hex() {
        let identifier = KeyName::Identifier([0x0a, 0xbc, 0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 0xff]);
        assert_eq!(identifier.to_string(), "0abc000102030405060708090a0b0cff");
        assert_eq!(KeyName::Descriptor([1, 2, 3, 4, 5, 6, 7, 0x80]).to_string(), "0102030405060780");
    }
}
