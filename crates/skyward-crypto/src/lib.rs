pub mod encryption;
pub mod fscrypt;
pub mod gatekeeper;
pub mod keystore;
pub mod locksettings;
pub mod sharedsecret;
pub mod token;
pub mod user;
pub mod weaver;

#[cfg(target_os = "android")]
pub mod data;
#[cfg(target_os = "android")]
pub mod keymaster;
#[cfg(target_os = "android")]
pub mod keymint;
#[cfg(target_os = "android")]
pub mod keystore2;

/// The phone's key service, of whichever kind its vendor partition runs:
/// KeyMint, registered with the service directory, or the Keymaster 4 of
/// a vendor built for Android 11 or before, as Samsung's for the S21 is,
/// reached over the driver the hardware services talk on. What the
/// recovery asks of either is the same.
#[cfg(target_os = "android")]
pub enum Service<'a> {
    KeyMint(keymint::KeyMint<'a>),
    Keymaster(keymaster::Keymaster<'a>),
}

#[cfg(target_os = "android")]
impl<'a> Service<'a> {
    /// Reach the running key service: KeyMint where it runs, and the
    /// Keymaster on `hardware` where it does not, or say that neither does.
    pub fn open(binder: &'a skyward_binder::driver::Binder, hardware: &'a skyward_binder::driver::Binder) -> anyhow::Result<Service<'a>> {
        let newer = match keymint::KeyMint::open(binder) {
            Ok(service) => return Ok(Service::KeyMint(service)),
            Err(e) => e,
        };
        match keymaster::Keymaster::open(hardware) {
            Ok(service) => Ok(Service::Keymaster(service)),
            Err(older) => Err(older.context(format!("nor is the key service of the older kind: {newer:#}"))),
        }
    }

    /// Turn a key only the storage hardware can use into the form it takes
    /// for this start of the phone.
    pub fn ephemeral_storage_key(&self, key: &[u8]) -> anyhow::Result<Vec<u8>> {
        match self {
            Service::KeyMint(s) => s.ephemeral_storage_key(key),
            Service::Keymaster(s) => s.ephemeral_storage_key(key),
        }
    }

    /// Undo an AES-GCM encryption the key service did with `key_blob`; a
    /// key guarded by a screen lock takes the note the secure world wrote
    /// when it was answered.
    pub fn gcm_decrypt(&self, key_blob: &[u8], app_id: &[u8], nonce: &[u8], body: &[u8], note: Option<&token::Token>) -> anyhow::Result<Vec<u8>> {
        match self {
            Service::KeyMint(s) => s.gcm_decrypt(key_blob, app_id, nonce, body, note),
            Service::Keymaster(s) => s.gcm_decrypt(key_blob, app_id, nonce, body, note),
        }
    }

    /// What the key service says a key may be used for: each entry is a tag
    /// and the number or length that came with it.
    pub fn authorizations(&self, key_blob: &[u8], app_id: &[u8]) -> anyhow::Result<Vec<(i32, i64)>> {
        match self {
            Service::KeyMint(s) => s.authorizations(key_blob, app_id),
            Service::Keymaster(s) => s.authorizations(key_blob, app_id),
        }
    }

    /// Whether the secure world will unwrap the key at all.
    pub fn usable(&self, key_blob: &[u8], app_id: &[u8]) -> anyhow::Result<()> {
        match self {
            Service::KeyMint(s) => s.usable(key_blob, app_id),
            Service::Keymaster(s) => s.usable(key_blob, app_id),
        }
    }
}

#[cfg(target_os = "android")]
impl keystore::Decryptor for Service<'_> {
    fn gcm_decrypt(&self, key_blob: &[u8], app_id: &[u8], nonce: &[u8], body: &[u8]) -> anyhow::Result<Vec<u8>> {
        // The keys unwrapped this way belong to the phone rather than to
        // any user, so none of them is guarded by a screen lock.
        Service::gcm_decrypt(self, key_blob, app_id, nonce, body, None)
    }
}
