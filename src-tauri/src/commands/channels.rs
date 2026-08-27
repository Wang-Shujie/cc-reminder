//! Channel commands: list/save/delete/test + credential replace.
//!
//! Privacy: the read surface exposes a [`ChannelView`] that never serializes
//! the credential reference verbatim. URLs are validated via
//! [`channels::validate_official_webhook`] BEFORE the credential enters the
//! keyring, so an invalid webhook is rejected with no keyring side effect.

use chrono::Utc;
use secrecy::SecretString;
use serde::{Deserialize, Serialize};
use tauri::State;
use uuid::Uuid;

use super::{CoreState, configuration_error, parse_uuid_input, secret_store_error};
use crate::error::{AppError, ErrorDomain};
use crate::model::{ChannelHealth, ChannelKind, ChannelPublicConfig, ChannelRecord};
use crate::security::credentials::CredentialPayload;
use crate::worker::ChannelSenderFactory;

// ---------------------------------------------------------------------------
// Inputs
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ChannelCredentialInput {
    DingTalk {
        webhook: String,
        signing_secret: Option<String>,
    },
    WeCom {
        webhook: String,
    },
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SaveChannelInput {
    pub channel_id: Option<String>,
    pub name: String,
    pub credential: ChannelCredentialInput,
    /// DingTalk-only public config. Omitted (or null) for WeCom.
    pub keyword_prefix: Option<String>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReplaceCredentialInput {
    pub channel_id: String,
    pub credential: ChannelCredentialInput,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DeleteChannelInput {
    pub channel_id: String,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TestChannelInput {
    pub channel_id: String,
}

// ---------------------------------------------------------------------------
// Output: typed view, no credential material
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, Serialize)]
pub struct ChannelView {
    pub id: String,
    pub kind: String,
    pub name: String,
    /// `true` when a credential is stored for this channel. The reference
    /// itself is never returned.
    pub credential_present: bool,
    pub health: String,
    pub paused: bool,
    pub last_succeeded_at: Option<chrono::DateTime<Utc>>,
}

impl ChannelView {
    pub(crate) fn from_record(record: &ChannelRecord, present: bool) -> Self {
        Self {
            id: record.id.to_string(),
            kind: super::channel_kind_code(record.kind),
            name: record.name.clone(),
            credential_present: present,
            health: super::channel_summary(
                record.health_status,
                record.paused_reason_code.as_deref(),
            ),
            paused: record.paused_reason_code.is_some(),
            last_succeeded_at: record.last_succeeded_at,
        }
    }
}

// ---------------------------------------------------------------------------
// Validation
// ---------------------------------------------------------------------------

/// Validate a webhook URL for the given channel kind BEFORE any credential is
/// stored. Returns the typed credential payload on success.
pub(crate) fn validate_credential_input(
    input: &ChannelCredentialInput,
) -> Result<(CredentialPayload, ChannelKind, ChannelPublicConfig), AppError> {
    match input {
        ChannelCredentialInput::DingTalk {
            webhook,
            signing_secret,
        } => {
            validate_webhook(ChannelKind::DingTalk, webhook)?;
            let payload = CredentialPayload::DingTalk {
                webhook: SecretString::new(webhook.clone().into()),
                signing_secret: signing_secret
                    .as_deref()
                    .filter(|s| !s.is_empty())
                    .map(|s| SecretString::new(s.to_owned().into())),
            };
            Ok((
                payload,
                ChannelKind::DingTalk,
                ChannelPublicConfig::DingTalk {
                    keyword_prefix: None,
                },
            ))
        }
        ChannelCredentialInput::WeCom { webhook } => {
            validate_webhook(ChannelKind::WeCom, webhook)?;
            Ok((
                CredentialPayload::WeCom {
                    webhook: SecretString::new(webhook.clone().into()),
                },
                ChannelKind::WeCom,
                ChannelPublicConfig::WeCom,
            ))
        }
    }
}

fn validate_webhook(kind: ChannelKind, raw: &str) -> Result<(), AppError> {
    crate::channels::validate_official_webhook(kind, raw).map_err(|err| {
        configuration_error("invalid_webhook", &format!("webhook rejected: {err:?}"))
    })
}

fn apply_public_config(
    public: ChannelPublicConfig,
    kind: ChannelKind,
    keyword_prefix: Option<&str>,
) -> ChannelPublicConfig {
    let prefix = keyword_prefix
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|s| s.to_owned());
    match public {
        ChannelPublicConfig::DingTalk { .. } => ChannelPublicConfig::DingTalk {
            keyword_prefix: prefix,
        },
        other => {
            let _ = kind;
            other
        }
    }
}

// ---------------------------------------------------------------------------
// Command bodies (testable without Tauri)
// ---------------------------------------------------------------------------

pub(crate) fn list_channels_impl(state: &CoreState) -> Result<Vec<ChannelView>, AppError> {
    let records = state.storage.config.list_channels()?;
    let mut views = Vec::with_capacity(records.len());
    for record in &records {
        let present = credential_present(state, &record.credential_ref);
        views.push(ChannelView::from_record(record, present));
    }
    Ok(views)
}

fn credential_present(state: &CoreState, credential_ref: &str) -> bool {
    state.credentials.get(credential_ref).is_ok()
}

pub(crate) fn save_channel_impl(
    state: &CoreState,
    input: SaveChannelInput,
) -> Result<ChannelView, AppError> {
    if input.name.trim().is_empty() {
        return Err(configuration_error(
            "channel_invalid",
            "channel name is empty",
        ));
    }
    let (payload, kind, public) = validate_credential_input(&input.credential)?;
    let public = apply_public_config(public, kind, input.keyword_prefix.as_deref());

    let id = match input.channel_id.as_deref() {
        Some(id) => parse_uuid_input(id)?,
        None => Uuid::now_v7(),
    };

    // If updating, delete the prior credential first so we never leave an
    // orphaned keyring entry when the new payload fails to store.
    let new_credential_ref = state
        .credentials
        .put(id, &payload)
        .map_err(credential_store_failure)?;
    let cleanup_ref = new_credential_ref.clone();

    let record = ChannelRecord {
        id,
        kind,
        name: input.name,
        credential_ref: new_credential_ref,
        public_config: public,
        health_status: ChannelHealth::Unknown,
        paused_reason_code: None,
        consecutive_auth_failures: 0,
        last_succeeded_at: None,
        next_allowed_at: None,
    };
    match state.storage.config.save_channel(&record) {
        Ok(()) => Ok(ChannelView::from_record(&record, true)),
        Err(e) => {
            // best-effort cleanup of the credential we just stored
            let _ = state.credentials.delete(&cleanup_ref);
            Err(e)
        }
    }
}

pub(crate) fn replace_channel_credential_impl(
    state: &CoreState,
    input: ReplaceCredentialInput,
) -> Result<ChannelView, AppError> {
    let channel_id = parse_uuid_input(&input.channel_id)?;
    let existing = state.storage.config.get_channel(channel_id)?;
    let (payload, kind, _public) = validate_credential_input(&input.credential)?;
    if kind != existing.kind {
        return Err(configuration_error(
            "channel_kind_mismatch",
            "credential kind does not match channel",
        ));
    }
    let prior_ref = existing.credential_ref.clone();
    let new_ref = state
        .credentials
        .put(channel_id, &payload)
        .map_err(credential_store_failure)?;
    let mut updated = existing;
    updated.credential_ref = new_ref.clone();
    updated.health_status = ChannelHealth::Unknown;
    updated.paused_reason_code = None;
    updated.consecutive_auth_failures = 0;
    updated.next_allowed_at = None;
    state.storage.config.save_channel(&updated)?;
    let _ = state.credentials.delete(&prior_ref);
    Ok(ChannelView::from_record(&updated, true))
}

pub(crate) fn delete_channel_impl(
    state: &CoreState,
    input: DeleteChannelInput,
) -> Result<(), AppError> {
    let channel_id = parse_uuid_input(&input.channel_id)?;
    let existing = state.storage.config.get_channel(channel_id)?;
    let credential_ref = existing.credential_ref.clone();
    state.storage.config.delete_channel(channel_id)?;
    // best-effort: deleting the channel row already refuses targeted channels,
    // so any leftover credential is now orphaned.
    let _ = state.credentials.delete(&credential_ref);
    Ok(())
}

pub(crate) async fn test_channel_impl(
    state: &CoreState,
    input: TestChannelInput,
) -> Result<TestChannelResult, AppError> {
    let channel_id = parse_uuid_input(&input.channel_id)?;
    let existing = state.storage.config.get_channel(channel_id)?;
    // A DingTalk keyword robot only accepts messages containing the configured
    // keyword, so the connection test must carry it exactly like a real send.
    let keyword_prefix = match &existing.public_config {
        crate::model::ChannelPublicConfig::DingTalk { keyword_prefix } => keyword_prefix.as_deref(),
        crate::model::ChannelPublicConfig::WeCom => None,
    };
    let factory = crate::worker::ProductionSenderFactory::new(state.credentials.clone());
    let document = crate::model::NotificationDocument {
        title: "CC Reminder connection test".into(),
        severity: crate::model::Severity::Info,
        facts: vec![("Source".into(), "test_channel".into())],
        body: "This is a connection test from CC Reminder.".into(),
        footer: None,
    };
    match factory
        .send(
            existing.kind,
            &existing.credential_ref,
            keyword_prefix,
            document,
        )
        .await
    {
        Ok(receipt) => Ok(TestChannelResult {
            http_status: receipt.http_status,
            platform_code: receipt.platform_code,
        }),
        Err(e) => Err(AppError {
            domain: ErrorDomain::Delivery,
            code: format!("delivery.{}", sanitize_code(&e.code)),
            message: e.redacted_message,
            suggested_action: None,
        }),
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct TestChannelResult {
    pub http_status: u16,
    pub platform_code: Option<String>,
}

fn sanitize_code(code: &str) -> String {
    code.chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '_' || *c == '.')
        .collect()
}

fn credential_store_failure(_: AppError) -> AppError {
    secret_store_error("unavailable", "credential store rejected the operation")
}

// ---------------------------------------------------------------------------
// Tauri wrappers
// ---------------------------------------------------------------------------

#[tauri::command]
pub async fn list_channels(state: State<'_, CoreState>) -> Result<Vec<ChannelView>, AppError> {
    let state = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || list_channels_impl(&state))
        .await
        .map_err(|_| configuration_error("join_failed", "command join failed"))?
}

#[tauri::command]
pub async fn save_channel(
    state: State<'_, CoreState>,
    input: SaveChannelInput,
) -> Result<ChannelView, AppError> {
    let state = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || save_channel_impl(&state, input))
        .await
        .map_err(|_| configuration_error("join_failed", "command join failed"))?
}

#[tauri::command]
pub async fn replace_channel_credential(
    state: State<'_, CoreState>,
    input: ReplaceCredentialInput,
) -> Result<ChannelView, AppError> {
    let state = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || replace_channel_credential_impl(&state, input))
        .await
        .map_err(|_| configuration_error("join_failed", "command join failed"))?
}

#[tauri::command]
pub async fn delete_channel(
    state: State<'_, CoreState>,
    input: DeleteChannelInput,
) -> Result<(), AppError> {
    let state = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || delete_channel_impl(&state, input))
        .await
        .map_err(|_| configuration_error("join_failed", "command join failed"))?
}

#[tauri::command]
pub async fn test_channel(
    state: State<'_, CoreState>,
    input: TestChannelInput,
) -> Result<TestChannelResult, AppError> {
    // The impl awaits the (async) sender factory directly, so it must run on
    // the async runtime — not inside spawn_blocking.
    test_channel_impl(state.inner(), input).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::CoreState;
    use crate::events::catalog::catalog_for;
    use crate::security::crypto::FieldCipher;
    use crate::storage::config::ConfigRepository;
    use crate::storage::db::Database;
    use crate::storage::events::EventRepository;
    use crate::storage::integrations::IntegrationRepository;
    use crate::storage::queue::QueueRepository;
    use semver::Version;
    use tempfile::tempdir;

    fn state_with_wecom_credential(secret_marker: &str) -> CoreState {
        let root = tempdir().unwrap();
        // ponytail: leak the TempDir so its on-disk DB outlives the helper. The
        // process is the test binary, which exits immediately after; a per-test
        // directory would be dropped before list_channels_impl reopens the DB.
        let database_path = root.path().join("com.ccreminder.app/cc-reminder.sqlite3");
        std::fs::create_dir_all(database_path.parent().unwrap()).unwrap();
        let database = Database::open(&database_path).unwrap();
        let config = ConfigRepository::new(database.clone());
        config
            .ensure_global_rules(&[
                catalog_for(
                    crate::model::AgentKind::ClaudeCode,
                    &Version::new(2, 1, 218),
                )
                .catalog,
                catalog_for(crate::model::AgentKind::Codex, &Version::new(0, 145, 0)).catalog,
            ])
            .unwrap();
        let events = EventRepository::new(database.clone());
        let queue = QueueRepository::new(database.clone());
        let integrations = IntegrationRepository::new(database.clone());
        let credentials = crate::security::credentials::CredentialStore::memory_for_test();
        let cipher = std::sync::Arc::new(FieldCipher::from_key([7u8; 32]));
        std::mem::forget(root);
        let diagnostics = std::sync::Arc::new(crate::diagnostics::Diagnostics::test(
            &database_path.parent().unwrap().join("logs"),
            1024 * 1024,
            3,
        ));

        // Insert a channel whose credential payload contains the secret marker.
        let state = CoreState::new(
            config,
            events,
            queue,
            integrations,
            credentials,
            cipher,
            diagnostics,
        );
        let input = SaveChannelInput {
            channel_id: None,
            name: "Engineering".into(),
            credential: ChannelCredentialInput::DingTalk {
                webhook: format!(
                    "https://oapi.dingtalk.com/robot/send?access_token={secret_marker}"
                ),
                signing_secret: None,
            },
            keyword_prefix: Some("CC".into()),
        };
        save_channel_impl(&state, input).unwrap();
        state
    }

    #[tokio::test]
    async fn list_channels_never_serializes_saved_credentials() {
        let state = command_state_with_wecom_credential("never-return-this");
        let response = super::list_channels_impl(&state).unwrap();
        let json = serde_json::to_string(&response).unwrap();
        assert!(!json.contains("never-return-this"), "json was: {json}");
        assert!(response[0].credential_present);
    }

    fn command_state_with_wecom_credential(secret_marker: &str) -> CoreState {
        state_with_wecom_credential(secret_marker)
    }

    #[test]
    fn invalid_webhook_is_rejected_before_credential_store() {
        let _state = invalid_state();
        // arbitrary URL / wrong host must be rejected by the validator
        let input = SaveChannelInput {
            channel_id: None,
            name: "bad".into(),
            credential: ChannelCredentialInput::WeCom {
                webhook: "https://example.com/".into(),
            },
            keyword_prefix: None,
        };
        let err = validate_credential_input(&input.credential).unwrap_err();
        assert_eq!(err.code, "configuration.invalid_webhook");
    }

    fn invalid_state() -> CoreState {
        let root = tempdir().unwrap();
        let database_path = root.path().join("com.ccreminder.app/cc-reminder.sqlite3");
        std::fs::create_dir_all(database_path.parent().unwrap()).unwrap();
        let database = Database::open(&database_path).unwrap();
        let config = ConfigRepository::new(database.clone());
        config
            .ensure_global_rules(&[
                catalog_for(
                    crate::model::AgentKind::ClaudeCode,
                    &Version::new(2, 1, 218),
                )
                .catalog,
                catalog_for(crate::model::AgentKind::Codex, &Version::new(0, 145, 0)).catalog,
            ])
            .unwrap();
        let events = EventRepository::new(database.clone());
        let queue = QueueRepository::new(database.clone());
        let integrations = IntegrationRepository::new(database.clone());
        let credentials = crate::security::credentials::CredentialStore::memory_for_test();
        let cipher = std::sync::Arc::new(FieldCipher::from_key([9u8; 32]));
        std::mem::forget(root);
        let diagnostics = std::sync::Arc::new(crate::diagnostics::Diagnostics::test(
            &database_path.parent().unwrap().join("logs"),
            1024 * 1024,
            3,
        ));
        CoreState::new(
            config,
            events,
            queue,
            integrations,
            credentials,
            cipher,
            diagnostics,
        )
    }

    #[test]
    fn delete_channel_refuses_channel_targeted_by_active_rule() {
        let state = invalid_state();
        // seed a channel then a rule that targets it
        let saved = save_channel_impl(
            &state,
            SaveChannelInput {
                channel_id: None,
                name: "eng".into(),
                credential: ChannelCredentialInput::WeCom {
                    webhook: "https://qyapi.weixin.qq.com/cgi-bin/webhook/send?key=abcdef".into(),
                },
                keyword_prefix: None,
            },
        )
        .unwrap();
        let id = Uuid::parse_str(&saved.id).unwrap();
        let mut rule = state
            .storage
            .config
            .get_global_rule(crate::model::AgentKind::Codex, "Stop")
            .unwrap();
        rule.config.enabled = true;
        rule.config.targets = vec![crate::model::TargetConfig {
            channel_id: id,
            template: None,
        }];
        state.storage.config.save_global_rule(&rule).unwrap();

        let err = delete_channel_impl(
            &state,
            DeleteChannelInput {
                channel_id: saved.id.clone(),
            },
        )
        .unwrap_err();
        assert_eq!(err.code, "configuration.channel_in_use");
    }

    #[test]
    fn malformed_uuid_is_rejected_by_delete_channel() {
        let state = invalid_state();
        let err = delete_channel_impl(
            &state,
            DeleteChannelInput {
                channel_id: "not-a-uuid".into(),
            },
        )
        .unwrap_err();
        assert_eq!(err.code, "configuration.malformed_uuid");
    }
}
