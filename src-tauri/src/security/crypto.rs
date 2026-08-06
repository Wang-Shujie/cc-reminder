use std::collections::BTreeMap;
use std::fmt;
use std::fs::{File, OpenOptions};
use std::io::{ErrorKind, Read, Write};
use std::path::Path;
use std::sync::Mutex;

use chacha20poly1305::aead::{Aead, KeyInit, Payload};
use chacha20poly1305::{XChaCha20Poly1305, XNonce};
use rand::{TryRng, rngs::SysRng};
use serde::{Deserialize, Serialize};
use uuid::Uuid;
use zeroize::{Zeroize, Zeroizing};

use crate::error::{AppError, ErrorDomain};
use crate::model::EncryptedBlobRef;
use crate::security::credentials::CredentialStore;
use crate::security::permissions::{ensure_current_user_dacl, ensure_private_directory};

const DATA_KEY_USERNAME: &str = "data-key";
const CORRELATION_KEY_FILE: &str = "correlation.key";
const KEY_BYTES: usize = 32;
const NONCE_BYTES: usize = 24;
const TAG_BYTES: usize = 16;
const MAX_ENCRYPTED_FIELDS: usize = 256;
const MAX_FIELD_NAME_BYTES: usize = 256;
const MAX_FIELD_PLAINTEXT_BYTES: usize = 1_048_576;
static DATA_KEY_INIT_LOCK: Mutex<()> = Mutex::new(());

#[derive(Clone)]
pub struct FieldCipher {
    key: Zeroizing<[u8; KEY_BYTES]>,
}

impl FieldCipher {
    pub fn load_or_create() -> Result<Self, AppError> {
        Self::load_or_create_with_store(&CredentialStore::system())
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn from_key(key: [u8; KEY_BYTES]) -> Self {
        Self::with_key(key)
    }

    fn with_key(key: [u8; KEY_BYTES]) -> Self {
        Self {
            key: Zeroizing::new(key),
        }
    }

    pub fn encrypt_fields(
        &self,
        event_id: Uuid,
        plaintext_fields: &BTreeMap<String, String>,
    ) -> Result<EncryptedFields, AppError> {
        validate_plaintext_fields(plaintext_fields)?;
        let mut fields = BTreeMap::new();
        for (field_name, plaintext) in plaintext_fields {
            fields.insert(
                field_name.clone(),
                self.encrypt(
                    plaintext.as_bytes(),
                    event_field_aad(event_id, field_name).as_bytes(),
                )?,
            );
        }
        Ok(EncryptedFields {
            event_id,
            blob_id: Uuid::now_v7(),
            fields,
        })
    }

    pub fn decrypt_fields(
        &self,
        event_id: Uuid,
        encrypted_fields: &EncryptedFields,
    ) -> Result<BTreeMap<String, String>, AppError> {
        validate_encrypted_fields(&encrypted_fields.fields)?;
        encrypted_fields
            .fields
            .iter()
            .map(|(field_name, encrypted)| {
                let plaintext =
                    self.decrypt(encrypted, event_field_aad(event_id, field_name).as_bytes())?;
                let value = match String::from_utf8(plaintext) {
                    Ok(value) => value,
                    Err(error) => {
                        let mut plaintext = error.into_bytes();
                        plaintext.zeroize();
                        return Err(crypto_error(
                            "security.decryption_failed",
                            "sensitive field authentication failed",
                        ));
                    }
                };
                Ok((field_name.clone(), value))
            })
            .collect()
    }

    pub(crate) fn encrypt_snapshot(
        &self,
        snapshot_id: Uuid,
        plaintext: &[u8],
    ) -> Result<EncryptedValue, AppError> {
        if plaintext.len() > MAX_FIELD_PLAINTEXT_BYTES {
            return Err(crypto_error(
                "security.invalid_snapshot",
                "snapshot plaintext exceeds the supported size",
            ));
        }
        self.encrypt(plaintext, snapshot_aad(snapshot_id).as_bytes())
    }

    /// Decrypt a config snapshot. Reserved for the explicit disaster-recovery
    /// flow (design 9.4); not yet called from the install path, which only
    /// writes snapshots.
    #[allow(dead_code)]
    pub(crate) fn decrypt_snapshot(
        &self,
        snapshot_id: Uuid,
        encrypted: &EncryptedValue,
    ) -> Result<Vec<u8>, AppError> {
        validate_encrypted_value(encrypted)?;
        self.decrypt(encrypted, snapshot_aad(snapshot_id).as_bytes())
    }

    fn load_or_create_with_store(store: &CredentialStore) -> Result<Self, AppError> {
        let _guard = DATA_KEY_INIT_LOCK.lock().map_err(|_| {
            crypto_error(
                "security.data_key_unavailable",
                "data encryption key is unavailable",
            )
        })?;
        match store.get_named_secret(DATA_KEY_USERNAME) {
            Ok(secret) => Self::from_stored_key(secret),
            Err(error) if error.code == "secret_store.not_found" => {
                let mut key = Zeroizing::new([0_u8; KEY_BYTES]);
                fill_random(&mut key[..])?;
                store.set_named_secret(DATA_KEY_USERNAME, &key[..])?;
                Ok(Self::with_key(*key))
            }
            Err(error) => Err(error),
        }
    }

    fn from_stored_key(secret: Vec<u8>) -> Result<Self, AppError> {
        let mut secret = Zeroizing::new(secret);
        if secret.len() != KEY_BYTES {
            return Err(crypto_error(
                "security.invalid_data_key",
                "stored data key is invalid",
            ));
        }
        let mut key = [0_u8; KEY_BYTES];
        key.copy_from_slice(&secret);
        secret.zeroize();
        Ok(Self::with_key(key))
    }

    fn encrypt(&self, plaintext: &[u8], aad: &[u8]) -> Result<EncryptedValue, AppError> {
        let cipher = XChaCha20Poly1305::new_from_slice(&self.key[..]).map_err(|_| {
            crypto_error(
                "security.encryption_failed",
                "sensitive field encryption failed",
            )
        })?;
        let mut nonce = [0_u8; NONCE_BYTES];
        fill_random(&mut nonce)?;
        let ciphertext = cipher
            .encrypt(
                &XNonce::from(nonce),
                Payload {
                    msg: plaintext,
                    aad,
                },
            )
            .map_err(|_| {
                crypto_error(
                    "security.encryption_failed",
                    "sensitive field encryption failed",
                )
            })?;
        Ok(EncryptedValue {
            nonce: nonce.to_vec(),
            ciphertext,
        })
    }

    fn decrypt(&self, encrypted: &EncryptedValue, aad: &[u8]) -> Result<Vec<u8>, AppError> {
        let cipher = XChaCha20Poly1305::new_from_slice(&self.key[..]).map_err(|_| {
            crypto_error(
                "security.decryption_failed",
                "sensitive field authentication failed",
            )
        })?;
        let nonce: [u8; NONCE_BYTES] = encrypted.nonce.as_slice().try_into().map_err(|_| {
            crypto_error(
                "security.decryption_failed",
                "sensitive field authentication failed",
            )
        })?;
        cipher
            .decrypt(
                &XNonce::from(nonce),
                Payload {
                    msg: &encrypted.ciphertext,
                    aad,
                },
            )
            .map_err(|_| {
                crypto_error(
                    "security.decryption_failed",
                    "sensitive field authentication failed",
                )
            })
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct EncryptedFields {
    event_id: Uuid,
    blob_id: Uuid,
    fields: BTreeMap<String, EncryptedValue>,
}

impl fmt::Debug for EncryptedFields {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("EncryptedFields")
            .field("blob_id", &self.blob_id)
            .finish_non_exhaustive()
    }
}

impl EncryptedFields {
    pub fn blob_ref(&self) -> EncryptedBlobRef {
        EncryptedBlobRef {
            blob_id: self.blob_id,
        }
    }

    pub(crate) fn to_blob(&self) -> Result<Vec<u8>, AppError> {
        validate_encrypted_fields(&self.fields)?;
        serde_json::to_vec(&self.fields).map_err(|_| {
            crypto_error(
                "security.serialization_failed",
                "encrypted fields could not be serialized",
            )
        })
    }

    pub(crate) fn event_id(&self) -> Uuid {
        self.event_id
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub(crate) struct EncryptedValue {
    pub(crate) nonce: Vec<u8>,
    pub(crate) ciphertext: Vec<u8>,
}

pub struct CorrelationKey {
    key: Zeroizing<[u8; KEY_BYTES]>,
}

impl CorrelationKey {
    pub fn load_or_create(data_dir: &Path) -> Result<Self, AppError> {
        validate_data_directory(data_dir)?;
        ensure_private_directory(data_dir)?;
        ensure_current_user_dacl(data_dir)?;
        let path = data_dir.join(CORRELATION_KEY_FILE);
        match read_correlation_key(&path) {
            Ok(key) => {
                ensure_current_user_dacl(&path)?;
                return Ok(Self { key });
            }
            Err(error) if error.kind() != ErrorKind::NotFound => return Err(correlation_error()),
            Err(_) => {}
        }

        let mut key = Zeroizing::new([0_u8; KEY_BYTES]);
        fill_random(&mut key[..])?;
        let published = publish_correlation_key_with(data_dir, &path, &key[..], |file, key| {
            file.write_all(key)
        });
        match published {
            Ok(()) => Ok(Self { key }),
            Err(_) => read_correlation_key(&path)
                .map(|key| Self { key })
                .map_err(|_| correlation_error()),
        }
    }

    pub fn expose_for_hmac(&self) -> &[u8; KEY_BYTES] {
        &self.key
    }
}

fn validate_data_directory(data_dir: &Path) -> Result<(), AppError> {
    if data_dir.file_name().and_then(|name| name.to_str()) == Some("com.ccreminder.app") {
        Ok(())
    } else {
        Err(crypto_error(
            "security.invalid_data_directory",
            "application data directory is invalid",
        ))
    }
}

fn read_correlation_key(path: &Path) -> std::io::Result<Zeroizing<[u8; KEY_BYTES]>> {
    let mut file = File::open(path)?;
    if file.metadata()?.len() != KEY_BYTES as u64 {
        return Err(std::io::Error::new(
            ErrorKind::InvalidData,
            "invalid correlation key length",
        ));
    }
    let mut key = Zeroizing::new([0_u8; KEY_BYTES]);
    file.read_exact(&mut key[..])?;
    Ok(key)
}

fn publish_correlation_key_with<F>(
    data_dir: &Path,
    final_path: &Path,
    key: &[u8],
    write_key: F,
) -> Result<(), AppError>
where
    F: FnOnce(&mut File, &[u8]) -> std::io::Result<()>,
{
    let temporary_path = data_dir.join(format!(".correlation-{}.tmp", Uuid::now_v7()));
    let result = (|| -> Result<(), AppError> {
        let mut file = private_new_file(&temporary_path).map_err(|_| correlation_error())?;
        ensure_current_user_dacl(&temporary_path)?;
        write_key(&mut file, key).map_err(|_| correlation_error())?;
        file.sync_all().map_err(|_| correlation_error())?;
        drop(file);
        std::fs::hard_link(&temporary_path, final_path).map_err(|_| correlation_error())?;
        Ok(())
    })();
    let _ = std::fs::remove_file(&temporary_path);
    result
}

#[cfg(unix)]
fn private_new_file(path: &Path) -> std::io::Result<File> {
    use std::os::unix::fs::OpenOptionsExt;

    OpenOptions::new()
        .create_new(true)
        .write(true)
        .mode(0o600)
        .open(path)
}

#[cfg(not(unix))]
fn private_new_file(path: &Path) -> std::io::Result<File> {
    OpenOptions::new().create_new(true).write(true).open(path)
}

fn fill_random(bytes: &mut [u8]) -> Result<(), AppError> {
    SysRng.try_fill_bytes(bytes).map_err(|_| {
        crypto_error(
            "security.random_failed",
            "cryptographic randomness is unavailable",
        )
    })
}

fn validate_plaintext_fields(fields: &BTreeMap<String, String>) -> Result<(), AppError> {
    if fields.len() > MAX_ENCRYPTED_FIELDS {
        return Err(invalid_fields_error());
    }
    let mut total_bytes = 0_usize;
    for (name, value) in fields {
        total_bytes = total_bytes
            .checked_add(value.len())
            .ok_or_else(invalid_fields_error)?;
        if name.is_empty()
            || name.len() > MAX_FIELD_NAME_BYTES
            || total_bytes > MAX_FIELD_PLAINTEXT_BYTES
        {
            return Err(invalid_fields_error());
        }
    }
    Ok(())
}

fn validate_encrypted_fields(fields: &BTreeMap<String, EncryptedValue>) -> Result<(), AppError> {
    if fields.len() > MAX_ENCRYPTED_FIELDS {
        return Err(invalid_fields_error());
    }
    let mut total_plaintext_bytes = 0_usize;
    for (name, value) in fields {
        validate_encrypted_value(value)?;
        total_plaintext_bytes = total_plaintext_bytes
            .checked_add(value.ciphertext.len() - TAG_BYTES)
            .ok_or_else(invalid_fields_error)?;
        if name.is_empty()
            || name.len() > MAX_FIELD_NAME_BYTES
            || total_plaintext_bytes > MAX_FIELD_PLAINTEXT_BYTES
        {
            return Err(invalid_fields_error());
        }
    }
    Ok(())
}

fn validate_encrypted_value(value: &EncryptedValue) -> Result<(), AppError> {
    if value.nonce.len() != NONCE_BYTES
        || !(TAG_BYTES..=MAX_FIELD_PLAINTEXT_BYTES + TAG_BYTES).contains(&value.ciphertext.len())
    {
        return Err(invalid_fields_error());
    }
    Ok(())
}

fn event_field_aad(event_id: Uuid, field_name: &str) -> String {
    format!("cc-reminder:event:{event_id}:field:{field_name}")
}

pub(crate) fn snapshot_aad(snapshot_id: Uuid) -> String {
    format!("cc-reminder:snapshot:{snapshot_id}:hooks")
}

fn invalid_fields_error() -> AppError {
    crypto_error(
        "security.invalid_sensitive_fields",
        "sensitive fields exceed the supported limits",
    )
}

fn correlation_error() -> AppError {
    crypto_error(
        "security.correlation_key_unavailable",
        "correlation key is unavailable",
    )
}

fn crypto_error(code: &str, message: &str) -> AppError {
    AppError {
        domain: ErrorDomain::Storage,
        code: code.to_owned(),
        message: message.to_owned(),
        suggested_action: None,
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    #[cfg(unix)]
    use std::os::unix::fs::PermissionsExt;
    use std::sync::{Arc, Barrier};
    use std::thread;
    use std::time::Duration;

    use tempfile::tempdir;
    use uuid::Uuid;

    use super::{CorrelationKey, FieldCipher, publish_correlation_key_with};
    use crate::security::credentials::CredentialStore;

    #[test]
    fn sensitive_fields_round_trip_only_with_matching_event_and_field_aad() {
        let cipher = FieldCipher::from_key([4_u8; 32]);
        let event_id = Uuid::now_v7();
        let encrypted = cipher
            .encrypt_fields(
                event_id,
                &BTreeMap::from([("prompt".into(), "secret text".into())]),
            )
            .unwrap();

        assert_eq!(
            cipher.decrypt_fields(event_id, &encrypted).unwrap()["prompt"],
            "secret text"
        );
        assert!(cipher.decrypt_fields(Uuid::now_v7(), &encrypted).is_err());

        let mut renamed = encrypted.clone();
        let prompt = renamed.fields.remove("prompt").unwrap();
        renamed.fields.insert("renamed".into(), prompt);
        assert!(cipher.decrypt_fields(event_id, &renamed).is_err());
    }

    #[test]
    fn altered_ciphertext_is_rejected() {
        let cipher = FieldCipher::from_key([5_u8; 32]);
        let event_id = Uuid::now_v7();
        let mut encrypted = cipher
            .encrypt_fields(
                event_id,
                &BTreeMap::from([("prompt".into(), "secret text".into())]),
            )
            .unwrap();
        encrypted.fields.get_mut("prompt").unwrap().ciphertext[0] ^= 1;

        assert!(cipher.decrypt_fields(event_id, &encrypted).is_err());
    }

    #[test]
    fn repeated_encryption_uses_different_nonces() {
        let cipher = FieldCipher::from_key([6_u8; 32]);
        let event_id = Uuid::now_v7();
        let fields = BTreeMap::from([("prompt".into(), "same text".into())]);

        let first = cipher.encrypt_fields(event_id, &fields).unwrap();
        let second = cipher.encrypt_fields(event_id, &fields).unwrap();

        assert_ne!(first.fields["prompt"].nonce, second.fields["prompt"].nonce);
        assert_ne!(
            first.fields["prompt"].ciphertext,
            second.fields["prompt"].ciphertext
        );
    }

    #[test]
    fn field_encryption_rejects_unbounded_input() {
        let cipher = FieldCipher::from_key([7_u8; 32]);
        let fields = (0..257)
            .map(|index| (format!("field-{index}"), "value".to_owned()))
            .collect();

        let error = cipher.encrypt_fields(Uuid::now_v7(), &fields).unwrap_err();

        assert_eq!(error.code, "security.invalid_sensitive_fields");
    }

    #[test]
    fn field_encryption_rejects_invalid_names_and_oversized_values() {
        let cipher = FieldCipher::from_key([7_u8; 32]);
        let invalid_name = BTreeMap::from([("".to_owned(), "value".to_owned())]);
        let oversized = BTreeMap::from([("prompt".to_owned(), "x".repeat(1_048_576 + 1))]);

        assert_eq!(
            cipher
                .encrypt_fields(Uuid::now_v7(), &invalid_name)
                .unwrap_err()
                .code,
            "security.invalid_sensitive_fields"
        );
        assert_eq!(
            cipher
                .encrypt_fields(Uuid::now_v7(), &oversized)
                .unwrap_err()
                .code,
            "security.invalid_sensitive_fields"
        );
    }

    #[test]
    fn field_encryption_rejects_an_oversized_aggregate() {
        let cipher = FieldCipher::from_key([7_u8; 32]);
        let fields = BTreeMap::from([
            ("prompt".to_owned(), "x".repeat(600 * 1024)),
            ("transcript".to_owned(), "y".repeat(600 * 1024)),
        ]);

        let error = cipher.encrypt_fields(Uuid::now_v7(), &fields).unwrap_err();

        assert_eq!(error.code, "security.invalid_sensitive_fields");
    }

    #[test]
    fn encrypted_field_blob_contains_no_plaintext_and_has_an_opaque_reference() {
        let cipher = FieldCipher::from_key([9_u8; 32]);
        let encrypted = cipher
            .encrypt_fields(
                Uuid::now_v7(),
                &BTreeMap::from([(
                    "prompt".to_owned(),
                    "known-sensitive-plaintext-4197".to_owned(),
                )]),
            )
            .unwrap();

        let blob = encrypted.to_blob().unwrap();

        assert!(!String::from_utf8_lossy(&blob).contains("known-sensitive-plaintext-4197"));
        assert_ne!(encrypted.blob_ref().blob_id, Uuid::nil());
    }

    #[test]
    fn data_key_is_randomly_created_once_in_secure_storage() {
        let store = CredentialStore::memory_for_test();
        let first = FieldCipher::load_or_create_with_store(&store).unwrap();
        let event_id = Uuid::now_v7();
        let encrypted = first
            .encrypt_fields(
                event_id,
                &BTreeMap::from([("prompt".to_owned(), "secret text".to_owned())]),
            )
            .unwrap();

        let second = FieldCipher::load_or_create_with_store(&store).unwrap();

        assert_eq!(
            second.decrypt_fields(event_id, &encrypted).unwrap()["prompt"],
            "secret text"
        );
    }

    #[test]
    fn concurrent_data_key_initialization_returns_compatible_ciphers() {
        let store = CredentialStore::delayed_memory_for_test(Duration::from_millis(50));
        let start = Arc::new(Barrier::new(3));
        let handles: Vec<_> = (0..2)
            .map(|_| {
                let store = store.clone();
                let start = Arc::clone(&start);
                thread::spawn(move || {
                    start.wait();
                    FieldCipher::load_or_create_with_store(&store).unwrap()
                })
            })
            .collect();
        start.wait();
        let mut ciphers = handles.into_iter().map(|handle| handle.join().unwrap());
        let first = ciphers.next().unwrap();
        let second = ciphers.next().unwrap();
        let event_id = Uuid::now_v7();
        let encrypted = first
            .encrypt_fields(
                event_id,
                &BTreeMap::from([("prompt".to_owned(), "secret text".to_owned())]),
            )
            .unwrap();

        assert_eq!(
            second.decrypt_fields(event_id, &encrypted).unwrap()["prompt"],
            "secret text"
        );
        assert_eq!(
            FieldCipher::load_or_create_with_store(&store)
                .unwrap()
                .decrypt_fields(event_id, &encrypted)
                .unwrap()["prompt"],
            "secret text"
        );
    }

    #[test]
    fn snapshots_round_trip_only_with_matching_snapshot_aad() {
        let cipher = FieldCipher::from_key([8_u8; 32]);
        let snapshot_id = Uuid::now_v7();
        let encrypted = cipher
            .encrypt_snapshot(snapshot_id, b"hook subtree")
            .unwrap();

        assert_eq!(
            cipher.decrypt_snapshot(snapshot_id, &encrypted).unwrap(),
            b"hook subtree"
        );
        assert!(cipher.decrypt_snapshot(Uuid::now_v7(), &encrypted).is_err());
    }

    #[test]
    fn correlation_key_file_is_random_and_not_credential_encryption_material() {
        let root = tempdir().unwrap();
        let directory = root.path().join("com.ccreminder.app");
        let first = CorrelationKey::load_or_create(&directory).unwrap();
        let second = CorrelationKey::load_or_create(&directory).unwrap();

        assert_eq!(first.expose_for_hmac(), second.expose_for_hmac());
        assert_ne!(first.expose_for_hmac(), &[0_u8; 32]);
        assert_eq!(
            std::fs::read(directory.join("correlation.key"))
                .unwrap()
                .len(),
            32
        );
        #[cfg(unix)]
        assert_eq!(
            std::fs::metadata(directory.join("correlation.key"))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
    }

    #[test]
    fn correlation_key_creates_a_missing_private_data_directory() {
        let root = tempdir().unwrap();
        let data_dir = root.path().join("com.ccreminder.app");

        CorrelationKey::load_or_create(&data_dir).unwrap();

        assert!(data_dir.is_dir());
        assert_eq!(
            std::fs::read(data_dir.join("correlation.key"))
                .unwrap()
                .len(),
            32
        );
        #[cfg(unix)]
        assert_eq!(
            std::fs::metadata(&data_dir).unwrap().permissions().mode() & 0o777,
            0o700
        );
    }

    #[test]
    fn correlation_key_rejects_shared_directory_without_changing_permissions() {
        let root = tempdir().unwrap();
        let shared = root.path().join("shared");
        std::fs::create_dir(&shared).unwrap();
        #[cfg(unix)]
        std::fs::set_permissions(&shared, std::fs::Permissions::from_mode(0o755)).unwrap();

        let error = match CorrelationKey::load_or_create(&shared) {
            Ok(_) => panic!("shared directory must be rejected"),
            Err(error) => error,
        };

        assert_eq!(error.code, "security.invalid_data_directory");
        #[cfg(unix)]
        assert_eq!(
            std::fs::metadata(&shared).unwrap().permissions().mode() & 0o777,
            0o755
        );
        assert!(!shared.join("correlation.key").exists());
    }

    #[test]
    fn failed_correlation_key_write_does_not_publish_a_partial_file() {
        let root = tempdir().unwrap();
        let data_dir = root.path().join("com.ccreminder.app");
        std::fs::create_dir(&data_dir).unwrap();
        let final_path = data_dir.join("correlation.key");

        let error =
            publish_correlation_key_with(&data_dir, &final_path, &[17_u8; 32], |file, key| {
                use std::io::Write;

                file.write_all(&key[..8])?;
                Err(std::io::Error::other("injected write failure"))
            })
            .unwrap_err();

        assert_eq!(error.code, "security.correlation_key_unavailable");
        assert!(!final_path.exists());
        assert_eq!(std::fs::read_dir(data_dir).unwrap().count(), 0);
    }
}
