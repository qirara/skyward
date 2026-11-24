//! The package Samsung's Odin, and Heimdall after it, flash a phone in
//! download mode from: a tar whose files are named after the partitions
//! they go to, followed by the line `md5sum` writes of the tar itself,
//! which Odin checks the tar against before it writes anything. Odin reads
//! the name on that line as the tar's own, without a folder before it.

use anyhow::{ensure, Result};
use md5::{Digest, Md5};

/// The tar's unit: each header, and each file's bytes rounded up to it.
const BLOCK: usize = 512;
/// The record tar rounds an archive up to, of twenty blocks.
const RECORD: usize = 20 * BLOCK;

/// The package holding `bytes` under `name`, its MD5 line naming the tar
/// `tar_name`: a POSIX tar of the one file, owned by root, mode 0644, with
/// no time of its own so that the same image makes the same package.
pub fn package(name: &str, bytes: &[u8], tar_name: &str) -> Result<Vec<u8>> {
    ensure!(!name.is_empty() && name.len() < 100 && name.is_ascii() && !name.contains('/'), "{name:?} is not a name Odin finds a partition by");
    ensure!(!tar_name.is_empty() && !tar_name.contains(['/', '\n']), "{tar_name:?} is not the name of a tar without its folder");
    // Eleven octal digits, as the header has room for.
    ensure!((bytes.len() as u64) < 1 << 33, "an image of {} bytes is more than a tar's header can say", bytes.len());
    let mut header = [0u8; BLOCK];
    header[..name.len()].copy_from_slice(name.as_bytes());
    let mut field = |at: usize, text: &[u8]| header[at..at + text.len()].copy_from_slice(text);
    field(100, b"0000644\0");
    field(108, b"0000000\0");
    field(116, b"0000000\0");
    field(124, format!("{:011o}\0", bytes.len()).as_bytes());
    field(136, b"00000000000\0");
    field(148, b"        ");
    field(156, b"0");
    field(257, b"ustar\0");
    field(263, b"00");
    field(329, b"0000000\0");
    field(337, b"0000000\0");
    // The checksum is the sum of the header's bytes, its own field read as
    // spaces, in six octal digits, a NUL and a space.
    let sum: u32 = header.iter().map(|&b| u32::from(b)).sum();
    header[148..156].copy_from_slice(format!("{sum:06o}\0 ").as_bytes());

    let mut tar = Vec::with_capacity(BLOCK + bytes.len().next_multiple_of(BLOCK) + RECORD);
    tar.extend_from_slice(&header);
    tar.extend_from_slice(bytes);
    tar.resize(tar.len().next_multiple_of(BLOCK), 0);
    // Two blocks of zeros end the archive, which is then made a whole
    // number of records, as tar makes it.
    tar.resize(tar.len() + 2 * BLOCK, 0);
    tar.resize(tar.len().next_multiple_of(RECORD), 0);
    let line = format!("{}  {tar_name}\n", hex::encode(Md5::digest(&tar)));
    tar.extend_from_slice(line.as_bytes());
    Ok(tar)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read;

    #[test]
    fn the_package_is_a_tar_of_the_image_and_the_line_odin_checks_it_by() {
        let image: Vec<u8> = (0..70_000u32).map(|i| (i % 251) as u8).collect();
        let package = package("recovery.img", &image, "skyward-p3q.tar").unwrap();
        // The line: 32 hex digits, two spaces, the tar's name and a newline.
        let (tar, line) = package.split_at(package.len() - (32 + 2 + "skyward-p3q.tar".len() + 1));
        assert_eq!(tar.len() % RECORD, 0, "whole records, then the line");
        let line = std::str::from_utf8(line).unwrap();
        assert_eq!(line, format!("{}  skyward-p3q.tar\n", hex::encode(Md5::digest(tar))), "md5sum's line of the tar, named without a folder");

        let mut archive = tar::Archive::new(tar);
        let mut entries = archive.entries().unwrap();
        let mut entry = entries.next().unwrap().unwrap();
        assert_eq!(entry.path().unwrap().to_str(), Some("recovery.img"));
        assert_eq!((entry.header().mode().unwrap(), entry.header().uid().unwrap(), entry.size()), (0o644, 0, 70_000));
        assert_eq!(entry.header().as_ustar().map(|h| h.magic), Some(*b"ustar\0"), "a POSIX tar");
        let mut back = Vec::new();
        entry.read_to_end(&mut back).unwrap();
        assert_eq!(back, image);
        drop(entry);
        assert!(entries.next().is_none(), "the image alone");
    }

    #[test]
    fn the_same_image_makes_the_same_package() {
        assert_eq!(package("recovery.img", b"abc", "a.tar").unwrap(), package("recovery.img", b"abc", "a.tar").unwrap());
    }

    #[test]
    fn a_name_odin_cannot_take_is_refused() {
        assert!(package("images/recovery.img", b"x", "a.tar").is_err(), "a file in a folder");
        assert!(package("", b"x", "a.tar").is_err());
        assert!(package("recovery.img", b"x", "out/a.tar").is_err(), "the tar named with its folder");
    }
}
