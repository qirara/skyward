//! A phone's own kernel taken into another image: the device trees a
//! build may have put after it, the gzip a recovery's kernel is carried
//! in, and the table of overlays its dtbo partition holds.

use anyhow::{ensure, Context, Result};
use flate2::write::GzEncoder;
use std::io::Write;

/// How a flattened device tree begins.
const DTB_MAGIC: [u8; 4] = [0xd0, 0x0d, 0xfe, 0xed];

/// The header of a flattened device tree, of which the length of the
/// whole tree is the second field.
const DTB_HEADER: usize = 40;

/// How the table of device tree overlays a dtbo partition holds begins.
const DTBO_MAGIC: [u8; 4] = [0xd7, 0xb7, 0xab, 0x1e];

/// A kernel and the device trees a build put after it, as `cat Image dtb >
/// Image-dtb` joins them: flattened trees, each giving its own length, one
/// after another to the very end. A kernel with none after it is given
/// back whole, with no trees.
pub fn split_appended_dtbs(kernel: &[u8]) -> (&[u8], &[u8]) {
    // Whether trees run from `at` to the end, one after another.
    let trees_to_the_end = |mut at: usize| loop {
        if at == kernel.len() {
            return true;
        }
        let Some(header) = kernel.get(at..at + DTB_HEADER) else { return false };
        let length = u32::from_be_bytes(header[4..8].try_into().expect("four bytes")) as usize;
        let version = u32::from_be_bytes(header[20..24].try_into().expect("four bytes"));
        if header[..4] != DTB_MAGIC || length < DTB_HEADER || length > kernel.len() - at || version < 16 {
            return false;
        }
        at += length;
    };
    let mut from = 0;
    while let Some(found) = kernel[from..].windows(DTB_MAGIC.len()).position(|w| w == DTB_MAGIC) {
        let at = from + found;
        if trees_to_the_end(at) {
            return kernel.split_at(at);
        }
        from = at + 1;
    }
    (kernel, &[])
}

/// `data` packed as gzip at its highest compression, as a recovery's
/// kernel is carried for the bootloader to unpack.
pub fn gzip(data: &[u8]) -> Result<Vec<u8>> {
    let mut gz = GzEncoder::new(Vec::new(), flate2::Compression::best());
    gz.write_all(data).context("gzip encode")?;
    gz.finish().context("gzip encode")
}

/// The table of device tree overlays at the start of what a dtbo
/// partition holds, without the rest of the partition after it.
pub fn dtbo_table(partition: &[u8]) -> Result<&[u8]> {
    ensure!(partition.len() >= 8 && partition[..4] == DTBO_MAGIC, "this is not a table of device tree overlays: it starts {:02x?}", &partition[..partition.len().min(4)]);
    let length = u32::from_be_bytes(partition[4..8].try_into().expect("four bytes")) as usize;
    partition.get(..length).with_context(|| format!("the table of overlays says it is {length} bytes, more than the {} there are", partition.len()))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A flattened device tree of `length` bytes, of version 17.
    fn dtb(length: usize) -> Vec<u8> {
        let mut tree = vec![0u8; length];
        tree[..4].copy_from_slice(&DTB_MAGIC);
        tree[4..8].copy_from_slice(&(length as u32).to_be_bytes());
        tree[20..24].copy_from_slice(&17u32.to_be_bytes());
        tree
    }

    #[test]
    fn the_trees_after_a_kernel_are_split_off_where_they_run_to_the_end() {
        // A kernel that happens to hold the magic itself, not followed by
        // trees running to the end.
        let mut image = b"ARMd kernel".to_vec();
        image.extend_from_slice(&DTB_MAGIC);
        image.extend_from_slice(&[0xff; 64]);
        let trees = [dtb(64), dtb(100)].concat();
        let joined = [image.clone(), trees.clone()].concat();
        assert_eq!(split_appended_dtbs(&joined), (&image[..], &trees[..]));
        assert_eq!(split_appended_dtbs(&image), (&image[..], &[][..]), "no trees after it");
        let cut = &joined[..joined.len() - 1];
        assert_eq!(split_appended_dtbs(cut).1, &[][..], "a tree cut short is no tree");
    }

    #[test]
    fn a_kernel_packed_as_gzip_unpacks_to_itself() {
        use std::io::Read;
        let kernel = b"MZ an arm64 kernel".repeat(1000);
        let packed = gzip(&kernel).unwrap();
        assert!(packed.len() < kernel.len() && packed.starts_with(&[0x1f, 0x8b]));
        let mut back = Vec::new();
        flate2::read::GzDecoder::new(&packed[..]).read_to_end(&mut back).unwrap();
        assert_eq!(back, kernel);
    }

    #[test]
    fn a_dtbo_partition_gives_its_table_alone() {
        let mut partition = vec![0u8; 4096];
        partition[..4].copy_from_slice(&DTBO_MAGIC);
        partition[4..8].copy_from_slice(&1000u32.to_be_bytes());
        assert_eq!(dtbo_table(&partition).unwrap().len(), 1000);
        partition[4..8].copy_from_slice(&5000u32.to_be_bytes());
        assert!(dtbo_table(&partition).is_err(), "longer than the partition");
        assert!(dtbo_table(&dtb(64)).is_err(), "a device tree, not a table of overlays");
    }
}
