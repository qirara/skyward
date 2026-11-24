//! The footer a recovery image ends in for the bootloader to verify it by:
//! a vbmeta image holding the image's digest, laid out over the whole
//! partition as avbtool's `add_hash_footer` lays it. An unlocked bootloader
//! boots an unsigned one, `--algorithm NONE`, whatever its digest; it gives
//! up on the whole slot when the footer at the partition's end points at
//! anything but a vbmeta image, as a stock footer left behind an image
//! written over the start of the partition does. Some bootloaders read the
//! signature as well, as Samsung's reads the one of the algorithm its
//! vbmeta partition names for the recovery: the vbmeta image is then signed
//! with a key of the builder's, as avbtool signs it with
//! `--algorithm SHA256_RSA<bits> --key`.

use anyhow::{bail, ensure, Context, Result};
use rsa::pkcs1v15::Pkcs1v15Sign;
use rsa::traits::PublicKeyParts;
use rsa::{BoxedUint, PrivateKey};
use sha2::{Digest, Sha256};
use std::path::Path;

/// The unit avbtool lays an image and its vbmeta image out in.
const BLOCK: usize = 4096;
const FOOTER_LEN: usize = 64;
const HEADER_LEN: usize = 256;
const TAG_HASH: u64 = 2;
const RELEASE: &[u8] = b"skyward-pack";

/// `image` grown to fill a `partition_size`-byte partition named
/// `partition`: the image, its vbmeta image from the next block on, and the
/// footer locating that in the partition's last bytes, zeros between.
pub fn footed(image: &[u8], partition: &str, partition_size: usize, salt: &[u8], key: Option<&Key>) -> Result<Vec<u8>> {
    let digest = Sha256::new().chain_update(salt).chain_update(image).finalize();
    let vbmeta = vbmeta(&hash_descriptor(image.len() as u64, partition, salt, &digest), key)?;
    let offset = image.len().next_multiple_of(BLOCK);
    let end = offset + vbmeta.len().next_multiple_of(BLOCK);
    ensure!(
        end + BLOCK <= partition_size,
        "a {}-byte image and its vbmeta image do not fit a {partition_size}-byte {partition} partition beside its footer",
        image.len()
    );
    let mut out = Vec::with_capacity(partition_size);
    out.extend_from_slice(image);
    out.resize(offset, 0);
    out.extend_from_slice(&vbmeta);
    out.resize(partition_size - FOOTER_LEN, 0);
    out.extend_from_slice(b"AVBf");
    out.extend_from_slice(&1u32.to_be_bytes());
    out.extend_from_slice(&0u32.to_be_bytes());
    for n in [image.len(), offset, vbmeta.len()] {
        out.extend_from_slice(&(n as u64).to_be_bytes());
    }
    out.resize(partition_size, 0);
    Ok(out)
}

/// A descriptor saying the first `image_size` bytes of `partition`, salted
/// with `salt`, hash to `digest` under SHA-256.
fn hash_descriptor(image_size: u64, partition: &str, salt: &[u8], digest: &[u8]) -> Vec<u8> {
    let mut algorithm = [0u8; 32];
    algorithm[..6].copy_from_slice(b"sha256");
    let mut d = Vec::new();
    d.extend_from_slice(&TAG_HASH.to_be_bytes());
    // How many bytes follow these first sixteen, once they are known.
    d.extend_from_slice(&0u64.to_be_bytes());
    d.extend_from_slice(&image_size.to_be_bytes());
    d.extend_from_slice(&algorithm);
    // The lengths of the name, salt and digest, then the flags.
    for n in [partition.len(), salt.len(), digest.len(), 0] {
        d.extend_from_slice(&(n as u32).to_be_bytes());
    }
    d.extend_from_slice(&[0; 60]);
    d.extend_from_slice(partition.as_bytes());
    d.extend_from_slice(salt);
    d.extend_from_slice(digest);
    d.resize(d.len().next_multiple_of(8), 0);
    let following = (d.len() - 16) as u64;
    d[8..16].copy_from_slice(&following.to_be_bytes());
    d
}

/// An RSA key a vbmeta image is signed with, under the algorithm its size
/// names: SHA256_RSA2048, SHA256_RSA4096 or SHA256_RSA8192.
pub struct Key {
    private: PrivateKey,
    bits: usize,
}

impl Key {
    /// The private key in the PEM file `path`, PKCS #1 as avbtool's test
    /// keys are or PKCS #8 as `openssl genpkey` writes one.
    pub fn read(path: &Path) -> Result<Key> {
        use rsa::pkcs1::DecodeRsaPrivateKey;
        use rsa::pkcs8::DecodePrivateKey;
        let pem = std::fs::read_to_string(path).with_context(|| format!("read {}", path.display()))?;
        let private = PrivateKey::from_pkcs1_pem(&pem)
            .or_else(|_| PrivateKey::from_pkcs8_pem(&pem))
            .with_context(|| format!("{} holds no RSA private key in PEM", path.display()))?;
        Key::new(private)
    }

    fn new(private: PrivateKey) -> Result<Key> {
        let bits = private.n().bit_width() as usize;
        ensure!([2048, 4096, 8192].contains(&bits), "an RSA key of {bits} bits; libavb takes keys of 2048, 4096 or 8192 bits");
        ensure!(*private.e_bytes() == [1, 0, 1], "libavb takes RSA keys whose exponent is 65537 alone");
        Ok(Key { private, bits })
    }

    /// libavb's number for the algorithm: SHA-256 with RSA of the key's size.
    fn algorithm(&self) -> u32 {
        match self.bits {
            2048 => 1,
            4096 => 2,
            _ => 3,
        }
    }

    fn signature_len(&self) -> usize {
        self.bits / 8
    }

    /// The public key as libavb reads it: the key's size in bits and
    /// -1/n mod 2^32 as big-endian u32s, then the modulus and r² mod n for
    /// r = 2^bits, each as many big-endian bytes as the key has bits.
    fn public(&self) -> Vec<u8> {
        let n = self.private.n_bytes();
        let n0 = u32::from_be_bytes(n[n.len() - 4..].try_into().expect("a modulus of 2048 bits or more"));
        // The inverse of an odd number mod 2^32, by Newton's iteration: each
        // step doubles the low bits that are right, from the three an odd
        // number is its own inverse in.
        let mut inverse = n0;
        for _ in 0..5 {
            inverse = inverse.wrapping_mul(2u32.wrapping_sub(n0.wrapping_mul(inverse)));
        }
        let rr = r_squared(&self.private, self.bits);
        let mut out = Vec::with_capacity(8 + 2 * self.signature_len());
        out.extend_from_slice(&(self.bits as u32).to_be_bytes());
        out.extend_from_slice(&inverse.wrapping_neg().to_be_bytes());
        for bytes in [&n, &rr] {
            out.resize(out.len() + self.signature_len() - bytes.len(), 0);
            out.extend_from_slice(bytes);
        }
        out
    }

    /// The PKCS #1 v1.5 signature of `data`'s SHA-256, as avbtool's padding
    /// and `openssl rsautl -sign -raw` make it.
    fn sign(&self, data: &[u8]) -> Result<Vec<u8>> {
        let signature = self.private.sign(Pkcs1v15Sign::new::<Sha256>(), &Sha256::digest(data)).context("sign the vbmeta image")?;
        if signature.len() != self.signature_len() {
            bail!("a signature of {} bytes from a key of {} bits", signature.len(), self.bits);
        }
        Ok(signature)
    }
}

/// r² mod n for the modulus n of `key` and r = 2^`bits`, in big-endian
/// bytes without leading zeros. The modulus is public, so the time this
/// takes tells nothing.
fn r_squared(key: &impl PublicKeyParts, bits: usize) -> Box<[u8]> {
    let bits = u32::try_from(bits).expect("a key of a size libavb takes");
    BoxedUint::one_with_precision(2 * bits + 1).shl(2 * bits).rem_vartime(key.n()).to_be_bytes_trimmed_vartime()
}

/// A vbmeta image holding `descriptors`: its header, the authentication
/// block and the descriptors as its auxiliary block. Unsigned, the
/// authentication block is empty; signed with `key`, it holds the SHA-256
/// of the header and the auxiliary block and its signature, and the
/// auxiliary block holds the public key after the descriptors.
fn vbmeta(descriptors: &[u8], key: Option<&Key>) -> Result<Vec<u8>> {
    let public = key.map(Key::public).unwrap_or_default();
    let (hash_len, signature_len) = match key {
        Some(key) => (32, key.signature_len()),
        None => (0, 0),
    };
    let n = descriptors.len();
    let aux = (n + public.len()).next_multiple_of(64);
    let auth = (hash_len + signature_len).next_multiple_of(64);
    let mut v = Vec::with_capacity(HEADER_LEN + auth + aux);
    v.extend_from_slice(b"AVB0");
    // The libavb version it needs, 1.0.
    v.extend_from_slice(&1u32.to_be_bytes());
    v.extend_from_slice(&0u32.to_be_bytes());
    v.extend_from_slice(&(auth as u64).to_be_bytes());
    v.extend_from_slice(&(aux as u64).to_be_bytes());
    v.extend_from_slice(&key.map_or(0, Key::algorithm).to_be_bytes());
    // The offsets and sizes of the hash and the signature in the
    // authentication block, of the public key, its metadata and the
    // descriptors in the auxiliary one, then the rollback index.
    for field in [0, hash_len, hash_len, signature_len, n, public.len(), n + public.len(), 0, 0, n, 0] {
        v.extend_from_slice(&(field as u64).to_be_bytes());
    }
    // The flags and the rollback index location.
    v.extend_from_slice(&0u32.to_be_bytes());
    v.extend_from_slice(&0u32.to_be_bytes());
    v.extend_from_slice(RELEASE);
    v.resize(HEADER_LEN, 0);
    let mut aux_block = Vec::with_capacity(aux);
    aux_block.extend_from_slice(descriptors);
    aux_block.extend_from_slice(&public);
    aux_block.resize(aux, 0);
    let mut auth_block = Vec::with_capacity(auth);
    if let Some(key) = key {
        let signed = [&v[..], &aux_block].concat();
        auth_block.extend_from_slice(&Sha256::digest(&signed));
        auth_block.extend_from_slice(&key.sign(&signed)?);
    }
    auth_block.resize(auth, 0);
    v.extend_from_slice(&auth_block);
    v.extend_from_slice(&aux_block);
    Ok(v)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SALT: [u8; 32] = [0x5a; 32];

    /// What avbtool 1.4.0 makes of `len` bytes of `S`, given by
    /// `add_hash_footer --partition_size 81920 --partition_name recovery
    /// --algorithm NONE --salt 5a…5a --internal_release_string skyward-pack`.
    fn avbtool(len: usize) -> &'static [u8] {
        match len {
            4096 => include_bytes!("../tests/avb-footer-4096.bin"),
            1000 => include_bytes!("../tests/avb-footer-1000.bin"),
            _ => unreachable!(),
        }
    }

    #[test]
    fn the_footer_is_avbtools_byte_for_byte() {
        for len in [4096, 1000] {
            let got = footed(&vec![b'S'; len], "recovery", 81920, &SALT, None).unwrap();
            assert!(got == avbtool(len), "a {len}-byte image");
        }
    }

    /// AOSP's own test keys, which avbtool's tests sign with; public, and
    /// for nothing but tests.
    fn test_key(bits: usize) -> Key {
        Key::read(&Path::new(env!("CARGO_MANIFEST_DIR")).join(format!("tests/testkey_rsa{bits}.pem"))).unwrap()
    }

    fn be64(b: &[u8], at: usize) -> usize {
        u64::from_be_bytes(b[at..at + 8].try_into().unwrap()) as usize
    }

    /// The vbmeta image a footed partition points at.
    fn vbmeta_of(partition: &[u8]) -> &[u8] {
        let footer = &partition[partition.len() - FOOTER_LEN..];
        assert_eq!(&footer[..4], b"AVBf");
        let (at, len) = (be64(footer, 20), be64(footer, 28));
        &partition[at..at + len]
    }

    #[test]
    fn a_signed_footer_carries_its_digest_signature_and_key_where_libavb_reads_them() {
        use rsa::PublicKey;
        for (bits, algorithm) in [(2048, 1u32), (4096, 2)] {
            let key = test_key(bits);
            let image = vec![b'S'; 4096];
            let partition = footed(&image, "recovery", 81920, &SALT, Some(&key)).unwrap();
            assert_eq!(&partition[..4096], &image[..], "the image itself is left as it was");
            let v = vbmeta_of(&partition);
            assert_eq!(&v[..4], b"AVB0");
            let (auth, aux) = (be64(v, 12), be64(v, 20));
            assert_eq!(u32::from_be_bytes(v[28..32].try_into().unwrap()), algorithm, "SHA256_RSA{bits}");
            let field = |i: usize| be64(v, 32 + 8 * i);
            let (hash_at, hash_len, sig_at, sig_len) = (field(0), field(1), field(2), field(3));
            let (key_at, key_len, desc_at, desc_len) = (field(4), field(5), field(8), field(9));
            assert_eq!((hash_at, hash_len, sig_at, sig_len), (0, 32, 32, bits / 8));
            assert_eq!((auth % 64, aux % 64, desc_at, key_at), (0, 0, 0, desc_len), "the key right after the descriptors");
            assert_eq!(v.len(), HEADER_LEN + auth + aux);
            let (header, auth_block, aux_block) = (&v[..HEADER_LEN], &v[HEADER_LEN..HEADER_LEN + auth], &v[HEADER_LEN + auth..]);
            let signed = [header, aux_block].concat();
            assert_eq!(&auth_block[..32], &Sha256::digest(&signed)[..], "the hash covers the header and the auxiliary block");
            let public = PublicKey::from(&key.private);
            public.verify(Pkcs1v15Sign::new::<Sha256>(), &auth_block[..32], &auth_block[32..32 + sig_len]).expect("the signature verifies");
            assert_eq!(&aux_block[key_at..key_at + key_len], &key.public()[..], "the key as libavb reads it");
        }
    }

    /// The public key in libavb's form, with its numbers for Montgomery
    /// multiplication, as avbtool's extract_public_key writes it: worked out
    /// for AOSP's test keys from the moduli openssl reads in them, apart
    /// from this code.
    #[test]
    fn the_public_key_is_the_one_avbtool_extracts() {
        for bits in [2048, 4096] {
            let expected = std::fs::read(Path::new(env!("CARGO_MANIFEST_DIR")).join(format!("tests/testkey_rsa{bits}_pub.bin"))).unwrap();
            assert_eq!(test_key(bits).public(), expected, "{bits} bits");
        }
    }

    #[test]
    fn signing_changes_only_the_vbmeta_image_and_needs_a_key_libavb_takes() {
        let image = vec![b'S'; 1000];
        let unsigned = footed(&image, "recovery", 81920, &SALT, None).unwrap();
        let signed = footed(&image, "recovery", 81920, &SALT, Some(&test_key(2048))).unwrap();
        assert_eq!(unsigned.len(), signed.len());
        assert_eq!(&unsigned[..4096], &signed[..4096], "the image and its padding");
        let (u, s) = (vbmeta_of(&unsigned), vbmeta_of(&signed));
        let descriptors = be64(u, 32 + 8 * 9);
        assert_eq!(descriptors, be64(s, 32 + 8 * 9));
        let auth = be64(s, 12);
        assert_eq!(&u[HEADER_LEN..HEADER_LEN + descriptors], &s[HEADER_LEN + auth..HEADER_LEN + auth + descriptors], "the same descriptors");
        let dir = tempfile::tempdir().unwrap();
        let not_a_key = dir.path().join("key.pem");
        std::fs::write(&not_a_key, "-----BEGIN CERTIFICATE-----\nMA==\n-----END CERTIFICATE-----\n").unwrap();
        assert!(Key::read(&not_a_key).is_err(), "a file that holds no private key");
    }

    #[test]
    fn an_image_that_leaves_no_room_for_its_vbmeta_and_footer_is_refused() {
        let image = vec![b'S'; 8192];
        assert!(footed(&image, "recovery", 16384, &SALT, None).is_ok(), "a block each for the vbmeta image and the footer");
        assert!(footed(&image, "recovery", 12288, &SALT, None).is_err());
    }
}
