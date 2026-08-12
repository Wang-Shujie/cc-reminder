use std::fmt;

#[cfg(any(test, feature = "test-support"))]
use std::collections::BTreeMap;
#[cfg(any(test, feature = "test-support"))]
use std::sync::{Arc, Mutex};
#[cfg(any(test, feature = "test-support"))]
use std::time::Duration;

use secrecy::{ExposeSecret, SecretString};
use serde::{Deserialize, Serialize};
use uuid::Uuid;
use zeroize::Zeroize;

use crate::error::{AppError, ErrorDomain};
use crate::model::ChannelId;

pub use crate::model::CredentialAvailability;

const SERVICE: &str = "cc-reminder";
const CHANNEL_USERNAME_PREFIX: &str = "channel/";
const CREDENTIAL_REFERENCE_PREFIX: &str = "cc-reminder/channel/";
const MAX_CREDENTIAL_BYTES: usize = 16 * 1024;

/// Secret credential material that deliberately has no `Serialize` implementation.
///
/// ```compile_fail
/// use cc_reminder_lib::security::credentials::CredentialPayload;
/// use secrecy::SecretString;
///
/// let payload = CredentialPayload::WeCom {
///     webhook: SecretString::from("secret"),
/// };
/// serde_json::to_vec(&payload).unwrap();
/// ```
#[derive(Clone)]
pub enum CredentialPayload {
    DingTalk {
        webhook: SecretString,
        signing_secret: Option<SecretString>,
    },
    WeCom {
        webhook: SecretString,
    },
}

impl CredentialPayload {
    pub fn expose_wecom_webhook_for_use(&self) -> &SecretString {
        match self {
            Self::WeCom { webhook } => webhook,
            Self::DingTalk { .. } => panic!("credential payload kind mismatch"),
        }
    }
}

impl fmt::Debug for CredentialPayload {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::DingTalk { signing_secret, .. } => formatter
                .debug_struct("DingTalk")
                .field("webhook", &"[REDACTED]")
                .field(
                    "signing_secret",
                    &signing_secret.as_ref().map(|_| "[REDACTED]"),
                )
                .finish(),
            Self::WeCom { .. } => formatter
                .debug_struct("WeCom")
                .field("webhook", &"[REDACTED]")
                .finish(),
        }
    }
}

#[derive(Clone)]
pub struct CredentialStore {
    backend: CredentialBackend,
}

#[derive(Clone)]
#[allow(dead_code)] // test-only variants are not all exercised under test-support builds.
enum CredentialBackend {
    System,
    #[cfg(any(test, feature = "test-support"))]
    Memory(Arc<Mutex<BTreeMap<String, Vec<u8>>>>),
    #[cfg(any(test, feature = "test-support"))]
    Unavailable,
    #[cfg(any(test, feature = "test-support"))]
    OperationallyUnavailable,
    #[cfg(any(test, feature = "test-support"))]
    DelayedMemory {
        values: Arc<Mutex<BTreeMap<String, Vec<u8>>>>,
        miss_delay: Duration,
    },
}

impl CredentialStore {
    pub const fn system() -> Self {
        Self {
            backend: CredentialBackend::System,
        }
    }

    pub fn availability(&self) -> CredentialAvailability {
        if self.probe().is_ok() {
            CredentialAvailability::Available
        } else {
            unavailable_availability()
        }
    }

    pub fn put(
        &self,
        channel_id: ChannelId,
        payload: &CredentialPayload,
    ) -> Result<String, AppError> {
        let username = format!("{CHANNEL_USERNAME_PREFIX}{channel_id}");
        let mut serialized = encode_payload(payload)?;
        let result = self.set_secret(&username, &serialized);
        serialized.zeroize();
        result?;
        Ok(format!("{CREDENTIAL_REFERENCE_PREFIX}{channel_id}"))
    }

    pub fn get(&self, credential_ref: &str) -> Result<CredentialPayload, AppError> {
        let username = username_from_reference(credential_ref)?;
        let mut serialized = self.get_secret(&username)?;
        let result = decode_payload(&serialized);
        serialized.zeroize();
        result
    }

    pub fn delete(&self, credential_ref: &str) -> Result<(), AppError> {
        let username = username_from_reference(credential_ref)?;
        self.delete_secret(&username)
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn memory_for_test() -> Self {
        Self {
            backend: CredentialBackend::Memory(Arc::new(Mutex::new(BTreeMap::new()))),
        }
    }

    #[cfg(test)]
    fn unavailable_for_test(_reason: &str) -> Self {
        Self {
            backend: CredentialBackend::Unavailable,
        }
    }

    #[cfg(test)]
    fn operationally_unavailable_for_test() -> Self {
        Self {
            backend: CredentialBackend::OperationallyUnavailable,
        }
    }

    #[cfg(test)]
    pub(crate) fn delayed_memory_for_test(miss_delay: Duration) -> Self {
        Self {
            backend: CredentialBackend::DelayedMemory {
                values: Arc::new(Mutex::new(BTreeMap::new())),
                miss_delay,
            },
        }
    }

    fn probe(&self) -> Result<(), AppError> {
        match &self.backend {
            CredentialBackend::System => probe_system_store(),
            #[cfg(any(test, feature = "test-support"))]
            CredentialBackend::Memory(_) | CredentialBackend::DelayedMemory { .. } => Ok(()),
            #[cfg(any(test, feature = "test-support"))]
            CredentialBackend::Unavailable | CredentialBackend::OperationallyUnavailable => {
                Err(unavailable_error())
            }
        }
    }

    fn set_secret(&self, username: &str, secret: &[u8]) -> Result<(), AppError> {
        match &self.backend {
            CredentialBackend::System => set_system_secret(username, secret),
            #[cfg(any(test, feature = "test-support"))]
            CredentialBackend::Memory(values) => {
                values
                    .lock()
                    .map_err(|_| unavailable_error())?
                    .insert(username.to_owned(), secret.to_vec());
                Ok(())
            }
            #[cfg(any(test, feature = "test-support"))]
            CredentialBackend::DelayedMemory { values, .. } => {
                values
                    .lock()
                    .map_err(|_| unavailable_error())?
                    .insert(username.to_owned(), secret.to_vec());
                Ok(())
            }
            #[cfg(any(test, feature = "test-support"))]
            CredentialBackend::Unavailable | CredentialBackend::OperationallyUnavailable => {
                Err(unavailable_error())
            }
        }
    }

    pub(super) fn set_named_secret(&self, username: &str, secret: &[u8]) -> Result<(), AppError> {
        self.set_secret(username, secret)
    }

    pub(super) fn get_named_secret(&self, username: &str) -> Result<Vec<u8>, AppError> {
        self.get_secret(username)
    }

    fn get_secret(&self, username: &str) -> Result<Vec<u8>, AppError> {
        match &self.backend {
            CredentialBackend::System => get_system_secret(username),
            #[cfg(any(test, feature = "test-support"))]
            CredentialBackend::Memory(values) => values
                .lock()
                .map_err(|_| unavailable_error())?
                .get(username)
                .cloned()
                .ok_or_else(not_found_error),
            #[cfg(any(test, feature = "test-support"))]
            CredentialBackend::DelayedMemory { values, miss_delay } => {
                let value = values
                    .lock()
                    .map_err(|_| unavailable_error())?
                    .get(username)
                    .cloned();
                if value.is_none() {
                    std::thread::sleep(*miss_delay);
                }
                value.ok_or_else(not_found_error)
            }
            #[cfg(any(test, feature = "test-support"))]
            CredentialBackend::Unavailable | CredentialBackend::OperationallyUnavailable => {
                Err(unavailable_error())
            }
        }
    }

    fn delete_secret(&self, username: &str) -> Result<(), AppError> {
        match &self.backend {
            CredentialBackend::System => delete_system_secret(username),
            #[cfg(any(test, feature = "test-support"))]
            CredentialBackend::Memory(values) => values
                .lock()
                .map_err(|_| unavailable_error())?
                .remove(username)
                .map(|mut secret| secret.zeroize())
                .ok_or_else(not_found_error),
            #[cfg(any(test, feature = "test-support"))]
            CredentialBackend::DelayedMemory { values, .. } => values
                .lock()
                .map_err(|_| unavailable_error())?
                .remove(username)
                .map(|mut secret| secret.zeroize())
                .ok_or_else(not_found_error),
            #[cfg(any(test, feature = "test-support"))]
            CredentialBackend::Unavailable | CredentialBackend::OperationallyUnavailable => {
                Err(unavailable_error())
            }
        }
    }
}

impl Default for CredentialStore {
    fn default() -> Self {
        Self::system()
    }
}

pub(super) fn set_system_secret(username: &str, secret: &[u8]) -> Result<(), AppError> {
    keyring::Entry::new(SERVICE, username)
        .and_then(|entry| entry.set_secret(secret))
        .map_err(map_keyring_error)
}

pub(super) fn get_system_secret(username: &str) -> Result<Vec<u8>, AppError> {
    keyring::Entry::new(SERVICE, username)
        .and_then(|entry| entry.get_secret())
        .map_err(map_keyring_error)
}

pub(super) fn delete_system_secret(username: &str) -> Result<(), AppError> {
    keyring::Entry::new(SERVICE, username)
        .and_then(|entry| entry.delete_credential())
        .map_err(map_keyring_error)
}

fn probe_system_store() -> Result<(), AppError> {
    match keyring::Entry::new(SERVICE, "availability-probe").and_then(|entry| entry.get_secret()) {
        Ok(mut secret) => {
            secret.zeroize();
            Ok(())
        }
        Err(keyring::Error::NoEntry) => Ok(()),
        Err(_) => Err(unavailable_error()),
    }
}

#[derive(Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum CredentialRecord {
    DingTalk {
        webhook: String,
        signing_secret: Option<String>,
    },
    WeCom {
        webhook: String,
    },
}

impl Drop for CredentialRecord {
    fn drop(&mut self) {
        match self {
            Self::DingTalk {
                webhook,
                signing_secret,
            } => {
                webhook.zeroize();
                signing_secret.zeroize();
            }
            Self::WeCom { webhook } => webhook.zeroize(),
        }
    }
}

fn encode_payload(payload: &CredentialPayload) -> Result<Vec<u8>, AppError> {
    let record = match payload {
        CredentialPayload::DingTalk {
            webhook,
            signing_secret,
        } => CredentialRecord::DingTalk {
            webhook: webhook.expose_secret().to_owned(),
            signing_secret: signing_secret
                .as_ref()
                .map(|value| value.expose_secret().to_owned()),
        },
        CredentialPayload::WeCom { webhook } => CredentialRecord::WeCom {
            webhook: webhook.expose_secret().to_owned(),
        },
    };
    let mut serialized = serde_json::to_vec(&record).map_err(|_| serialization_error())?;
    if serialized.len() > MAX_CREDENTIAL_BYTES {
        serialized.zeroize();
        return Err(invalid_record_error());
    }
    Ok(serialized)
}

fn decode_payload(serialized: &[u8]) -> Result<CredentialPayload, AppError> {
    if serialized.len() > MAX_CREDENTIAL_BYTES {
        return Err(invalid_record_error());
    }
    let mut record: CredentialRecord =
        serde_json::from_slice(serialized).map_err(|_| invalid_record_error())?;
    Ok(match &mut record {
        CredentialRecord::DingTalk {
            webhook,
            signing_secret,
        } => CredentialPayload::DingTalk {
            webhook: SecretString::from(std::mem::take(webhook)),
            signing_secret: signing_secret.take().map(SecretString::from),
        },
        CredentialRecord::WeCom { webhook } => CredentialPayload::WeCom {
            webhook: SecretString::from(std::mem::take(webhook)),
        },
    })
}

fn username_from_reference(credential_ref: &str) -> Result<String, AppError> {
    let id = credential_ref
        .strip_prefix(CREDENTIAL_REFERENCE_PREFIX)
        .and_then(|value| Uuid::parse_str(value).ok())
        .ok_or_else(invalid_reference_error)?;
    Ok(format!("{CHANNEL_USERNAME_PREFIX}{id}"))
}

fn map_keyring_error(error: keyring::Error) -> AppError {
    if matches!(error, keyring::Error::NoEntry) {
        not_found_error()
    } else {
        unavailable_error()
    }
}

fn unavailable_availability() -> CredentialAvailability {
    CredentialAvailability::Unavailable {
        reason_code: "secret_store.unavailable".to_owned(),
    }
}

fn secret_store_error(code: &str, message: &str) -> AppError {
    AppError {
        domain: ErrorDomain::SecretStore,
        code: code.to_owned(),
        message: message.to_owned(),
        suggested_action: None,
    }
}

fn unavailable_error() -> AppError {
    secret_store_error(
        "secret_store.unavailable",
        "secure credential storage is unavailable",
    )
}

fn not_found_error() -> AppError {
    secret_store_error("secret_store.not_found", "credential was not found")
}

fn invalid_reference_error() -> AppError {
    secret_store_error(
        "secret_store.invalid_reference",
        "credential reference is invalid",
    )
}

fn serialization_error() -> AppError {
    secret_store_error(
        "secret_store.serialization_failed",
        "credential could not be encoded",
    )
}

fn invalid_record_error() -> AppError {
    secret_store_error(
        "secret_store.invalid_record",
        "stored credential could not be decoded",
    )
}

#[cfg(test)]
mod tests {
    use secrecy::{ExposeSecret, SecretString};
    use uuid::Uuid;

    use super::{CredentialAvailability, CredentialPayload, CredentialStore};

    #[test]
    fn saved_credential_returns_only_an_opaque_reference() {
        let store = CredentialStore::memory_for_test();
        let reference = store.put(channel_id(), &wecom_payload("fake-key")).unwrap();

        assert!(reference.starts_with("cc-reminder/channel/"));
        let loaded = store.get(&reference).unwrap();
        assert_eq!(
            loaded.expose_wecom_webhook_for_use().expose_secret(),
            "fake-key"
        );
        assert!(!format!("{reference:?}").contains("fake-key"));
        assert_eq!(store.availability(), CredentialAvailability::Available);
    }

    #[test]
    fn unavailable_secure_storage_refuses_persistence() {
        let store = CredentialStore::unavailable_for_test("Secret Service unavailable");

        let error = store
            .put(channel_id(), &wecom_payload("fake-key"))
            .unwrap_err();

        assert_eq!(error.code, "secret_store.unavailable");
        assert!(!format!("{error:?}").contains("fake-key"));
        assert_eq!(
            store.availability(),
            CredentialAvailability::Unavailable {
                reason_code: "secret_store.unavailable".to_owned(),
            }
        );
    }

    #[test]
    fn initialized_but_inaccessible_secure_storage_is_reported_unavailable() {
        let store = CredentialStore::operationally_unavailable_for_test();

        assert_eq!(
            store.availability(),
            CredentialAvailability::Unavailable {
                reason_code: "secret_store.unavailable".to_owned(),
            }
        );
    }

    #[test]
    fn replacing_a_credential_keeps_the_same_opaque_reference() {
        let store = CredentialStore::memory_for_test();
        let channel_id = channel_id();
        let first = store.put(channel_id, &wecom_payload("first-key")).unwrap();

        let second = store
            .put(channel_id, &wecom_payload("replacement-key"))
            .unwrap();

        assert_eq!(first, second);
        assert_eq!(
            store
                .get(&second)
                .unwrap()
                .expose_wecom_webhook_for_use()
                .expose_secret(),
            "replacement-key"
        );
    }

    #[test]
    fn deleting_a_credential_removes_the_secret() {
        let store = CredentialStore::memory_for_test();
        let reference = store.put(channel_id(), &wecom_payload("fake-key")).unwrap();

        store.delete(&reference).unwrap();

        assert_eq!(
            store.get(&reference).unwrap_err().code,
            "secret_store.not_found"
        );
    }

    #[test]
    fn credential_debug_output_redacts_every_secret() {
        let payload = CredentialPayload::DingTalk {
            webhook: SecretString::from("debug-webhook"),
            signing_secret: Some(SecretString::from("debug-signing-secret")),
        };

        let output = format!("{payload:?}");

        assert!(!output.contains("debug-webhook"));
        assert!(!output.contains("debug-signing-secret"));
    }

    fn channel_id() -> Uuid {
        Uuid::now_v7()
    }

    fn wecom_payload(webhook: &str) -> CredentialPayload {
        CredentialPayload::WeCom {
            webhook: SecretString::from(webhook.to_owned()),
        }
    }
}
