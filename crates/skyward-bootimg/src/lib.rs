//! Android boot images, with the ramdisk-only header of versions 3 and 4
//! or the legacy one of versions 0 to 2 that recoveries of phones without
//! A/B slots still carry, and the newc archives their ramdisks are: read,
//! changed and written again, by the image packer on a computer and by the
//! recovery on the phone.

mod image;
pub mod kernel;
mod ramdisk;

pub use image::{BootHeader, BootImage};
pub use ramdisk::{lz4_legacy, Compression, Entry, Ramdisk};
