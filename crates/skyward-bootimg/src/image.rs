use abootimg_oxide::{binrw::io::Cursor, Header, HeaderV0, HeaderV0Versioned, HeaderV3};
use anyhow::{anyhow, ensure, Context, Result};
use sha1::{Digest, Sha1};
use std::fs;
use std::path::Path;

/// The page the ramdisk-only layout lays its sections out in.
const PAGE: usize = 4096;

/// The header of a boot image: the ramdisk-only layout of versions 3 and 4,
/// which the `boot`, `init_boot` and `recovery` images of phones launched
/// with a GKI kernel carry, or the legacy layout of versions 0 to 2, which
/// names physical load addresses and a page size of its own, and in
/// version 2 carries the device tree: the recovery image of a phone that is
/// not A/B, such as Samsung's, still has it.
#[derive(Clone, Debug)]
pub enum BootHeader {
    Legacy(HeaderV0),
    RamdiskOnly(HeaderV3),
}

impl BootHeader {
    /// The header's version number.
    pub fn header_version(&self) -> u32 {
        match self {
            BootHeader::Legacy(h) => h.header_version(),
            BootHeader::RamdiskOnly(h) => h.header_version(),
        }
    }

    /// The version of Android and the patch level the header says the
    /// image was built for.
    pub fn osversionpatch(&self) -> abootimg_oxide::OsVersionPatch {
        match self {
            BootHeader::Legacy(h) => h.osversionpatch,
            BootHeader::RamdiskOnly(h) => h.osversionpatch,
        }
    }
}

/// A boot image split into its header and its sections: the kernel and the
/// ramdisk, and where the legacy layout carries them, the second-stage
/// bootloader, the recovery's device tree overlay and the device tree.
pub struct BootImage {
    pub header: BootHeader,
    pub kernel: Vec<u8>,
    pub ramdisk: Vec<u8>,
    pub second: Vec<u8>,
    pub recovery_dtbo: Vec<u8>,
    pub dtb: Vec<u8>,
}

fn section(bytes: &[u8], at: usize, len: usize, what: &str) -> Result<Vec<u8>> {
    let end = at.checked_add(len).ok_or_else(|| anyhow!("{what} runs past the end of the image"))?;
    bytes.get(at..end).map(<[u8]>::to_vec).ok_or_else(|| anyhow!("{what} runs past the end of the image"))
}

/// `size` rounded up to a whole number of `page`s.
fn aligned(size: usize, page: usize) -> usize {
    size.div_ceil(page) * page
}

/// The page sizes mkbootimg lays a legacy image out in.
fn legacy_page(h: &HeaderV0) -> Result<usize> {
    let page = h.page_size as usize;
    ensure!(page.is_power_of_two() && (2048..=16384).contains(&page), "the header lays the image out in pages of {page} bytes");
    Ok(page)
}

impl BootImage {
    pub fn read(path: &Path) -> Result<Self> {
        let bytes = fs::read(path).with_context(|| format!("read {}", path.display()))?;
        BootImage::parse(&bytes).with_context(|| format!("read {}", path.display()))
    }

    /// The image `bytes` hold, as a partition holds it: the image, then
    /// whatever fills the partition after it.
    pub fn parse(bytes: &[u8]) -> Result<Self> {
        match Header::parse(&mut Cursor::new(bytes)).context("header")? {
            Header::V3(h) => {
                let kernel = section(bytes, HeaderV3::kernel_position(), h.kernel_size as usize, "kernel")?;
                let ramdisk = section(bytes, h.ramdisk_position(), h.ramdisk_size as usize, "ramdisk")?;
                Ok(BootImage { header: BootHeader::RamdiskOnly(h), kernel, ramdisk, second: Vec::new(), recovery_dtbo: Vec::new(), dtb: Vec::new() })
            }
            Header::V0(h) => {
                // Each section follows the one before it from a page boundary
                // on, the header taking the first page; a section of no size
                // takes no page.
                let page = legacy_page(&h)?;
                let (dtbo_size, dtb_size) = match h.versioned {
                    HeaderV0Versioned::V0 => (0, 0),
                    HeaderV0Versioned::V1 { recovery_dtbo_size, .. } => (recovery_dtbo_size as usize, 0),
                    HeaderV0Versioned::V2 { recovery_dtbo_size, dtb_size, .. } => (recovery_dtbo_size as usize, dtb_size as usize),
                };
                let sizes = [h.kernel_size as usize, h.ramdisk_size as usize, h.second_bootloader_size as usize, dtbo_size, dtb_size];
                let names = ["kernel", "ramdisk", "second-stage bootloader", "recovery dtbo", "dtb"];
                let mut at = page;
                let mut sections = Vec::with_capacity(sizes.len());
                for (size, name) in sizes.into_iter().zip(names) {
                    sections.push(section(bytes, at, size, name)?);
                    at = at.checked_add(aligned(size, page)).with_context(|| format!("the {name} runs past the end of the image"))?;
                }
                let [kernel, ramdisk, second, recovery_dtbo, dtb] = sections.try_into().expect("five sections");
                Ok(BootImage { header: BootHeader::Legacy(h), kernel, ramdisk, second, recovery_dtbo, dtb })
            }
        }
    }

    /// The image with sizes taken from its parts. A version 4 header keeps its
    /// version but carries no boot signature; unlocked bootloaders do not ask
    /// for one. A legacy header keeps its load addresses, its command line
    /// and its page size, and is given the place of the recovery dtbo in the
    /// image written and the digest of its sections anew.
    pub fn to_bytes(&self) -> Result<Vec<u8>> {
        let mut out = Vec::new();
        match &self.header {
            BootHeader::RamdiskOnly(header) => {
                let mut header = header.clone();
                header.kernel_size = u32::try_from(self.kernel.len()).context("kernel size")?;
                header.ramdisk_size = u32::try_from(self.ramdisk.len()).context("ramdisk size")?;
                header.v4_signature_size = header.v4_signature_size.map(|_| 0);
                Header::V3(header).write(&mut out).context("write header")?;
                pad(&mut out, PAGE);
                out.extend_from_slice(&self.kernel);
                pad(&mut out, PAGE);
                out.extend_from_slice(&self.ramdisk);
                pad(&mut out, PAGE);
            }
            BootHeader::Legacy(header) => {
                let page = legacy_page(header)?;
                let mut header = header.clone();
                let size = |part: &[u8], what: &str| u32::try_from(part.len()).with_context(|| format!("{what} size"));
                header.kernel_size = size(&self.kernel, "kernel")?;
                header.ramdisk_size = size(&self.ramdisk, "ramdisk")?;
                header.second_bootloader_size = size(&self.second, "second-stage bootloader")?;
                // The sections a version carries: the first three in every
                // one, the recovery dtbo from version 1 on, the dtb in 2.
                let carried = match &mut header.versioned {
                    HeaderV0Versioned::V0 => {
                        ensure!(self.recovery_dtbo.is_empty() && self.dtb.is_empty(), "a version 0 header carries no dtbo or dtb");
                        3
                    }
                    HeaderV0Versioned::V1 { recovery_dtbo_size, recovery_dtbo_addr } => {
                        ensure!(self.dtb.is_empty(), "a version 1 header carries no dtb");
                        *recovery_dtbo_size = size(&self.recovery_dtbo, "recovery dtbo")?;
                        *recovery_dtbo_addr = self.dtbo_offset(page);
                        4
                    }
                    HeaderV0Versioned::V2 { recovery_dtbo_size, recovery_dtbo_addr, dtb_size, .. } => {
                        *recovery_dtbo_size = size(&self.recovery_dtbo, "recovery dtbo")?;
                        *recovery_dtbo_addr = self.dtbo_offset(page);
                        *dtb_size = size(&self.dtb, "dtb")?;
                        5
                    }
                };
                // mkbootimg's digest: SHA-1 over each section the version
                // carries, then its size as a little-endian u32, the size
                // hashed even where the section is empty. A bootloader that
                // reads it would find an old one stale.
                let mut hash = Sha1::new();
                for part in [&self.kernel, &self.ramdisk, &self.second, &self.recovery_dtbo, &self.dtb].into_iter().take(carried) {
                    hash.update(part);
                    hash.update((part.len() as u32).to_le_bytes());
                }
                let digest = hash.finalize();
                header.hash_digest = [0; 32];
                header.hash_digest[..digest.len()].copy_from_slice(&digest);
                Header::V0(header).write(&mut out).context("write header")?;
                pad(&mut out, page);
                for part in [&self.kernel, &self.ramdisk, &self.second, &self.recovery_dtbo, &self.dtb] {
                    out.extend_from_slice(part);
                    pad(&mut out, page);
                }
            }
        }
        Ok(out)
    }

    /// Where the recovery dtbo starts in the legacy image `to_bytes` writes,
    /// which the header names as an offset into the image; none where it
    /// carries no dtbo.
    fn dtbo_offset(&self, page: usize) -> u64 {
        if self.recovery_dtbo.is_empty() {
            return 0;
        }
        (page + aligned(self.kernel.len(), page) + aligned(self.ramdisk.len(), page) + aligned(self.second.len(), page)) as u64
    }
}

fn pad(out: &mut Vec<u8>, page: usize) {
    out.resize(aligned(out.len(), page), 0);
}

#[cfg(test)]
mod tests {
    use super::*;
    use abootimg_oxide::OsVersionPatch;

    fn legacy(versioned: HeaderV0Versioned, page_size: u32) -> HeaderV0 {
        HeaderV0 {
            kernel_size: 0,
            kernel_addr: 0x0000_8000,
            ramdisk_size: 0,
            ramdisk_addr: 0x0200_0000,
            second_bootloader_size: 0,
            second_bootloader_addr: 0,
            tags_addr: 0x01e0_0000,
            page_size,
            osversionpatch: OsVersionPatch(0),
            board_name: *b"SRPTH05A001\0\0\0\0\0",
            cmdline: Box::new([0; 512 + 1024]),
            hash_digest: [0; 32],
            versioned,
        }
    }

    fn image(header: BootHeader, dtbo: &[u8], dtb: &[u8]) -> BootImage {
        BootImage {
            header,
            kernel: vec![0x4d, 0x5a, 0x00, 0x91, 0xff, 0x7f, 0x9c, 0x14],
            ramdisk: vec![1, 2, 3, 4, 5],
            second: Vec::new(),
            recovery_dtbo: dtbo.to_vec(),
            dtb: dtb.to_vec(),
        }
    }

    #[test]
    fn a_version_2_image_round_trips_with_its_sections_where_the_bootloader_reads_them() {
        let header = legacy(HeaderV0Versioned::V2 { recovery_dtbo_size: 0, recovery_dtbo_addr: 0, dtb_size: 0, dtb_addr: 0x0100_0000 }, 4096);
        let bytes = image(BootHeader::Legacy(header), &[7; 3000], &[0xd0, 0x0d, 0xfe, 0xed]).to_bytes().unwrap();
        let back = BootImage::parse(&bytes).unwrap();
        assert_eq!(back.header.header_version(), 2);
        assert_eq!(back.kernel, [0x4d, 0x5a, 0x00, 0x91, 0xff, 0x7f, 0x9c, 0x14]);
        assert_eq!(back.ramdisk, [1, 2, 3, 4, 5]);
        assert_eq!((back.recovery_dtbo.len(), back.dtb.as_slice()), (3000, &[0xd0, 0x0d, 0xfe, 0xed][..]));
        assert_eq!(&bytes[4096..4100], &[0x4d, 0x5a, 0x00, 0x91], "the kernel starts on the page after the header");
        assert_eq!(&bytes[8192..8197], &[1, 2, 3, 4, 5], "the ramdisk on the page after the kernel");
        let BootHeader::Legacy(h) = back.header else { panic!("a legacy header") };
        let HeaderV0Versioned::V2 { recovery_dtbo_addr, dtb_addr, .. } = h.versioned else { panic!("version 2") };
        assert_eq!(recovery_dtbo_addr, 12288, "the dtbo's place in the image, after the kernel and the ramdisk");
        assert_eq!(dtb_addr, 0x0100_0000, "the dtb's load address stays");
        assert_eq!(&bytes[12288..12291], &[7, 7, 7]);
        assert_eq!(bytes.len(), 4096 * 5, "every section padded to a page");
    }

    #[test]
    fn a_legacy_header_carries_mkbootimgs_digest_of_its_sections() {
        let header = legacy(HeaderV0Versioned::V2 { recovery_dtbo_size: 0, recovery_dtbo_addr: 0, dtb_size: 0, dtb_addr: 0 }, 2048);
        let img = image(BootHeader::Legacy(header), &[], &[9; 10]);
        let bytes = img.to_bytes().unwrap();
        let BootHeader::Legacy(h) = BootImage::parse(&bytes).unwrap().header else { panic!("a legacy header") };
        let mut expected = Sha1::new();
        for part in [&img.kernel[..], &img.ramdisk, &[], &[], &img.dtb] {
            expected.update(part);
            expected.update((part.len() as u32).to_le_bytes());
        }
        assert_eq!(h.hash_digest[..20], expected.finalize()[..]);
        assert_eq!(h.hash_digest[20..], [0; 12]);
        assert_eq!(&bytes[2048..2052], &[0x4d, 0x5a, 0x00, 0x91], "laid out in the header's own pages");
    }

    #[test]
    fn versions_0_and_1_round_trip_and_hold_only_what_they_carry() {
        let v0 = image(BootHeader::Legacy(legacy(HeaderV0Versioned::V0, 4096)), &[], &[]);
        let back = BootImage::parse(&v0.to_bytes().unwrap()).unwrap();
        assert_eq!((back.header.header_version(), back.ramdisk.as_slice()), (0, &[1, 2, 3, 4, 5][..]));
        assert!(image(BootHeader::Legacy(legacy(HeaderV0Versioned::V0, 4096)), &[], &[1]).to_bytes().is_err(), "no dtb in version 0");
        let v1 = image(BootHeader::Legacy(legacy(HeaderV0Versioned::V1 { recovery_dtbo_size: 0, recovery_dtbo_addr: 0 }, 4096)), &[3; 5], &[]);
        let back = BootImage::parse(&v1.to_bytes().unwrap()).unwrap();
        assert_eq!((back.header.header_version(), back.recovery_dtbo.as_slice()), (1, &[3; 5][..]));
    }

    #[test]
    fn a_page_size_mkbootimg_does_not_lay_out_is_refused() {
        for page in [0, 1000, 4097, 1 << 20] {
            let mut bytes = image(BootHeader::Legacy(legacy(HeaderV0Versioned::V0, 4096)), &[], &[]).to_bytes().unwrap();
            // The page size is the eighth field after the magic.
            bytes[36..40].copy_from_slice(&(page as u32).to_le_bytes());
            assert!(BootImage::parse(&bytes).is_err(), "pages of {page} bytes");
        }
    }

    #[test]
    fn a_section_running_past_the_image_is_refused() {
        let bytes = image(BootHeader::Legacy(legacy(HeaderV0Versioned::V0, 4096)), &[], &[]).to_bytes().unwrap();
        assert!(BootImage::parse(&bytes[..4096 + 4]).is_err(), "the kernel cut short");
    }

    #[test]
    fn a_ramdisk_only_image_keeps_its_layout() {
        let header = HeaderV3 {
            kernel_size: 0,
            ramdisk_size: 0,
            osversionpatch: OsVersionPatch(0),
            cmdline: Box::new([0; 1536]),
            v4_signature_size: Some(0),
        };
        let bytes = image(BootHeader::RamdiskOnly(header), &[], &[]).to_bytes().unwrap();
        let back = BootImage::parse(&bytes).unwrap();
        assert_eq!(back.header.header_version(), 4);
        assert_eq!(back.ramdisk, [1, 2, 3, 4, 5]);
        assert!(back.dtb.is_empty() && back.second.is_empty());
        assert_eq!(bytes.len(), 3 * PAGE);
    }
}
