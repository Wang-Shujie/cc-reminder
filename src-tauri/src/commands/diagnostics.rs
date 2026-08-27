//! Diagnostic export + clear-history + debug-logging commands (Task 20).
//!
//! Export security contract (review round 1): the native save dialog is opened
//! FROM Rust via [`tauri_plugin_dialog::DialogExt`] on the injected
//! [`tauri::AppHandle`] — the frontend supplies no path at all, and Rust-side
//! dialog calls do not go through the capability table (the WebView keeps no
//! filesystem permission). Only bytes assembled here are written, create +
//! truncate, to the path the user picked in that dialog, and the command
//! answers with a tagged enum carrying just the final filename — never an
//! absolute path.

use std::collections::BTreeSet;

use semver::Version;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tauri::State;
use tauri_plugin_dialog::DialogExt;

use super::{CoreState, configuration_error};
use crate::error::{AppError, ErrorDomain};
use crate::model::AgentKind;

/// Default suggestion offered by the save dialog; the user may rename or
/// relocate freely before confirming.
const EXPORT_DEFAULT_FILENAME: &str = "cc-reminder-diagnostics.zip";

/// Serde tagged result: `{ "status": "saved", "filename": .. } | { "status":
/// "cancelled" }` (mirrored as `DiagnosticExportResult` in contracts.ts).
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum DiagnosticExportResult {
    Saved {
        /// Final path component only — never the absolute location.
        filename: String,
    },
    Cancelled,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ClearHistoryInput {
    /// v1 exposes only the preserve-active-work form; the confirmation is the
    /// exact dialog text on the UI side.
    pub preserve_active_jobs: bool,
}

#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SetDebugLoggingInput {
    /// 0 closes any open debug window; 15 / 60 open a bounded window in
    /// minutes. Every other value is rejected.
    pub duration_minutes: u16,
}

pub(crate) fn export_diagnostics_impl(
    state: &CoreState,
    target: Option<&std::path::Path>,
) -> Result<DiagnosticExportResult, AppError> {
    let Some(target) = target else {
        return Ok(DiagnosticExportResult::Cancelled);
    };
    let archive = build_archive(state)?;
    write_archive(target, &archive)?;
    let filename = target
        .file_name()
        .and_then(|name| name.to_str())
        .map(str::to_owned)
        .ok_or_else(|| {
            configuration_error(
                "diagnostics.path_invalid",
                "save target has no usable file name",
            )
        })?;
    Ok(DiagnosticExportResult::Saved { filename })
}

fn build_archive(state: &CoreState) -> Result<Vec<u8>, AppError> {
    let settings = state.config.get_settings();
    let manifest = build_manifest(state, settings.as_ref().ok())?;
    // health.json: a fresh typed snapshot from the shared projection.
    let snapshot = super::health_snapshot(state)?;
    let health = serde_json::to_vec(&snapshot).map_err(|_| AppError {
        domain: ErrorDomain::Storage,
        code: "diagnostics.unavailable".to_owned(),
        message: "health snapshot could not be serialized".to_owned(),
        suggested_action: None,
    })?;
    let stats = state.queue.queue_stats()?;
    let queue_stats = serde_json::to_vec(&serde_json::json!({
        "pending": stats.pending,
        "sending": stats.sending,
        "retry_wait": stats.retry_wait,
        "succeeded": stats.succeeded,
        "failed": stats.failed,
        "expired": stats.expired,
    }))
    .map_err(|_| AppError {
        domain: ErrorDomain::Storage,
        code: "diagnostics.unavailable".to_owned(),
        message: "queue stats could not be serialized".to_owned(),
        suggested_action: None,
    })?;
    // The shared logger instance writes the runtime log files that ship in
    // the archive (already redacted at write time).
    state.diagnostics.export(&[
        ("manifest.json", manifest),
        ("health.json", health),
        ("queue-stats.json", queue_stats),
    ])
}

fn write_archive(path: &std::path::Path, bytes: &[u8]) -> Result<(), AppError> {
    #[cfg(unix)]
    {
        use std::io::Write;
        use std::os::unix::fs::OpenOptionsExt;
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .mode(0o600)
            .open(path)
            .map_err(|_| write_failed())?;
        file.write_all(bytes).map_err(|_| write_failed())
    }
    #[cfg(not(unix))]
    {
        std::fs::write(path, bytes).map_err(|_| write_failed())
    }
}

fn write_failed() -> AppError {
    AppError {
        domain: ErrorDomain::Storage,
        code: "diagnostics.write_failed".to_owned(),
        message: "diagnostic archive could not be written".to_owned(),
        suggested_action: Some("choose a different save location".to_owned()),
    }
}

/// Native save dialog, invoked from RUST. Returns `None` when the user
/// cancels. Never called on the main thread (the command wrapper runs inside
/// `spawn_blocking`), matching the plugin's documented usage.
fn save_dialog_target(app: &tauri::AppHandle) -> Result<Option<std::path::PathBuf>, AppError> {
    app.dialog()
        .file()
        .add_filter("ZIP archive", &["zip"])
        .set_file_name(EXPORT_DEFAULT_FILENAME)
        .blocking_save_file()
        .map(|file_path| {
            file_path.into_path().map_err(|_| {
                configuration_error(
                    "diagnostics.path_invalid",
                    "selected save location is unusable",
                )
            })
        })
        .transpose()
}

/// manifest.json: versions (app/helper/agents/capabilities), OS/architecture,
/// SHA-256 hashes of non-sensitive serialized settings/rules — never the
/// values — database schema version read from the applied migrations, and
/// export time. No raw paths, credentials, or rule values.
fn build_manifest(
    state: &CoreState,
    settings: Option<&crate::model::AppSettings>,
) -> Result<Vec<u8>, AppError> {
    let mut manifest = serde_json::Map::new();
    manifest.insert(
        "app_version".to_owned(),
        serde_json::json!(env!("CARGO_PKG_VERSION")),
    );
    manifest.insert("os".to_owned(), serde_json::json!(std::env::consts::OS));
    manifest.insert(
        "architecture".to_owned(),
        serde_json::json!(std::env::consts::ARCH),
    );
    manifest.insert(
        "exported_at".to_owned(),
        serde_json::json!(chrono::Utc::now().to_rfc3339()),
    );
    // Schema version comes straight from the applied-migrations reader so it
    // can never go stale at migration 0002+.
    let schema_version =
        crate::storage::db::Database::open(std::path::Path::new(state.queue.database_path()))
            .and_then(|database| database.schema_version())
            .map_err(|_| {
                configuration_error("diagnostics.unavailable", "schema version unavailable")
            })?;
    manifest.insert(
        "schema_version".to_owned(),
        serde_json::json!(schema_version),
    );

    // Per-agent detected versions + capability catalog versions + the
    // verification verdict, exactly as installed/detected in the repositories.
    let mut agents = serde_json::Map::new();
    let mut helper_versions: BTreeSet<String> = BTreeSet::new();
    for agent in [AgentKind::ClaudeCode, AgentKind::Codex] {
        if let Ok(hooks) = state.integrations.list_hooks(agent) {
            helper_versions.extend(hooks.iter().map(|hook| hook.helper_version.clone()));
        }
        let record = state.integrations.agent(agent).ok();
        let detected_version = record.as_ref().and_then(|r| r.version.clone());
        let verification = match &record {
            Some(record) => Some(
                serde_json::to_value(record.capability_verification).map_err(|_| {
                    configuration_error(
                        "diagnostics.unavailable",
                        "capability verification could not be encoded",
                    )
                })?,
            ),
            None => None,
        };
        // A zero version resolves to the newest embedded catalog for the
        // agent, i.e. the capability set this app actually ships.
        let capability_version = crate::events::catalog::catalog_for(
            agent,
            &fallback_catalog_version(&detected_version),
        )
        .catalog
        .verified_version;
        agents.insert(
            agent.as_str().to_owned(),
            serde_json::json!({
                "detected_version": detected_version.map(|v| v.to_string()),
                "capability_version": capability_version.to_string(),
                "capability_verification": verification,
            }),
        );
    }
    manifest.insert("agents".to_owned(), serde_json::Value::Object(agents));
    if helper_versions.is_empty() {
        helper_versions.insert(env!("CARGO_PKG_VERSION").to_owned());
    }
    manifest.insert(
        "helper_version".to_owned(),
        serde_json::json!(helper_versions),
    );

    if let Some(settings) = settings {
        // Hash of the serialized settings — the values never enter the archive.
        let serialized = serde_json::to_vec(settings)
            .map_err(|_| configuration_error("diagnostics.unavailable", "settings hash failed"))?;
        manifest.insert(
            "settings_sha256".to_owned(),
            serde_json::json!(hex::encode(Sha256::digest(&serialized))),
        );
    }
    for rule in state.config.list_global_rules()? {
        let serialized = serde_json::to_vec(&rule.config)
            .map_err(|_| configuration_error("diagnostics.unavailable", "rule hash failed"))?;
        manifest.insert(
            format!("rule_{}_{}_sha256", rule.agent.as_str(), rule.source_event),
            serde_json::json!(hex::encode(Sha256::digest(&serialized))),
        );
    }
    serde_json::to_vec(&manifest).map_err(|_| {
        configuration_error("diagnostics.unavailable", "manifest serialization failed")
    })
}

/// Detected version if present, else a version no catalog claims — which
/// resolves to the latest embedded catalog for the agent.
fn fallback_catalog_version(detected: &Option<Version>) -> Version {
    detected.clone().unwrap_or(Version::new(0, 0, 0))
}

pub(crate) fn clear_history_impl(
    state: &CoreState,
    input: ClearHistoryInput,
) -> Result<u64, AppError> {
    if !input.preserve_active_jobs {
        return Err(configuration_error(
            "diagnostics.clear_requires_preserve",
            "v1 supports only the preserve-active-work form",
        ));
    }
    let cleared = crate::storage::retention::clear_history(
        &crate::storage::db::Database::open(std::path::Path::new(state.queue.database_path()))
            .map_err(|_| {
                configuration_error("diagnostics.unavailable", "database could not be opened")
            })?,
        chrono::Utc::now(),
    )?;
    // v2-issues: 清空历史后推送 history-changed,订阅端刷新而非停留在
    // 已被清除的列表上。清了 0 行则不打扰。
    if cleared > 0 {
        crate::worker::emit(&state.core_events, crate::worker::CoreEvent::HistoryChanged);
    }
    Ok(cleared)
}

pub(crate) fn set_debug_logging_impl(
    state: &CoreState,
    input: SetDebugLoggingInput,
) -> Result<super::settings::SettingsView, AppError> {
    let until = match input.duration_minutes {
        0 => None,
        15 => Some(chrono::Utc::now() + chrono::Duration::minutes(15)),
        60 => Some(chrono::Utc::now() + chrono::Duration::minutes(60)),
        _ => {
            return Err(configuration_error(
                "diagnostics.invalid_debug_duration",
                "debug duration must be 0, 15, or 60 minutes",
            ));
        }
    };
    state.diagnostics.set_debug_until(until)?;
    super::settings::get_settings_impl(state)
}

#[tauri::command]
pub async fn export_diagnostics(
    state: State<'_, CoreState>,
    app: tauri::AppHandle,
) -> Result<DiagnosticExportResult, AppError> {
    let state = state.inner().clone();
    // spawn_blocking keeps the blocking native dialog off the main thread and
    // off async workers, per the dialog plugin's documented usage.
    tauri::async_runtime::spawn_blocking(move || {
        let target = save_dialog_target(&app)?;
        export_diagnostics_impl(&state, target.as_deref())
    })
    .await
    .map_err(|_| configuration_error("join_failed", "command join failed"))?
}

#[tauri::command]
pub async fn clear_history(
    state: State<'_, CoreState>,
    input: ClearHistoryInput,
) -> Result<u64, AppError> {
    let state = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || clear_history_impl(&state, input))
        .await
        .map_err(|_| configuration_error("join_failed", "command join failed"))?
}

#[tauri::command]
pub async fn set_debug_logging(
    state: State<'_, CoreState>,
    input: SetDebugLoggingInput,
) -> Result<super::settings::SettingsView, AppError> {
    let state = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || set_debug_logging_impl(&state, input))
        .await
        .map_err(|_| configuration_error("join_failed", "command join failed"))?
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::CoreState;
    use crate::events::catalog::catalog_for;
    use crate::security::credentials::CredentialStore;
    use crate::security::crypto::FieldCipher;
    use crate::storage::config::ConfigRepository;
    use crate::storage::db::Database;
    use crate::storage::events::EventRepository;
    use crate::storage::integrations::IntegrationRepository;
    use crate::storage::queue::QueueRepository;
    use semver::Version;
    use tempfile::tempdir;

    fn state() -> CoreState {
        let root = tempdir().unwrap();
        let database_path = root.path().join("com.ccreminder.app/cc-reminder.sqlite3");
        std::fs::create_dir_all(database_path.parent().unwrap()).unwrap();
        std::mem::forget(root);
        let database = Database::open(&database_path).unwrap();
        let config = ConfigRepository::new(database.clone());
        config
            .ensure_global_rules(&[
                catalog_for(AgentKind::ClaudeCode, &Version::new(2, 1, 218)).catalog,
                catalog_for(AgentKind::Codex, &Version::new(0, 145, 0)).catalog,
            ])
            .unwrap();
        let events = EventRepository::new(database.clone());
        let queue = QueueRepository::new(database.clone());
        let integrations = IntegrationRepository::new(database.clone());
        let credentials = CredentialStore::memory_for_test();
        let cipher = std::sync::Arc::new(FieldCipher::from_key([4u8; 32]));
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
    fn cancelled_export_returns_cancelled_and_writes_nothing() {
        let st = state();
        let result = export_diagnostics_impl(&st, None).unwrap();
        assert_eq!(result, DiagnosticExportResult::Cancelled);
        let serialized = serde_json::to_string(&result).unwrap();
        assert_eq!(serialized, r#"{"status":"cancelled"}"#);
    }

    #[test]
    fn export_writes_to_the_dialog_chosen_path_and_answers_with_the_filename_only() {
        let st = state();
        let directory = tempdir().unwrap();
        let target = directory.path().join("chosen-name.zip");

        // A planted secret reaches the log through the redactor and must be
        // scrubbed in the exported archive too.
        st.diagnostics
            .info("test", "token=never-export-this must stay out");

        let result = export_diagnostics_impl(&st, Some(&target)).unwrap();

        assert_eq!(
            result,
            DiagnosticExportResult::Saved {
                filename: "chosen-name.zip".to_owned(),
            }
        );
        let serialized = serde_json::to_string(&result).unwrap();
        assert_eq!(
            serialized,
            r#"{"status":"saved","filename":"chosen-name.zip"}"#
        );
        // The answer never carries the absolute location.
        assert!(!serialized.contains(directory.path().to_str().unwrap()));

        let bytes = std::fs::read(&target).unwrap();
        let text = String::from_utf8_lossy(&bytes);
        assert!(text.contains("manifest.json"));
        assert!(text.contains("health.json"));
        assert!(text.contains("queue-stats.json"));
        assert!(text.contains("cc-reminder.log"));
        assert!(!text.contains("never-export-this"));
        assert!(!text.to_lowercase().contains("sqlite"));

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&target).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o600, "the archive lands user-only");
        }
    }

    #[test]
    fn manifest_reads_schema_from_the_database_and_lists_agent_capability_versions() {
        let st = state();
        // One agent detected with exact catalog match + one hook installed by
        // helper 0.1.0; Codex stays undetected.
        st.integrations
            .upsert_agent(&crate::model::AgentInstallationRecord {
                agent: AgentKind::ClaudeCode,
                executable_path: Some("/usr/local/bin/claude".into()),
                version: Some(Version::new(2, 1, 218)),
                capability_verification: crate::events::catalog::CatalogVerification::Exact,
                health_status: crate::model::InstallationHealth::Healthy,
                last_checked_at: chrono::Utc::now(),
            })
            .unwrap();
        st.integrations
            .replace_hooks(
                AgentKind::ClaudeCode,
                &[crate::model::HookInstallationRecord {
                    agent: AgentKind::ClaudeCode,
                    source_event: "Stop".to_owned(),
                    command_fingerprint: "fp".to_owned(),
                    definition_fingerprint: "dfp".to_owned(),
                    helper_version: env!("CARGO_PKG_VERSION").to_owned(),
                    config_hash: "hash".to_owned(),
                    trust_status: crate::model::TrustStatus::ObservedWorking,
                    health_status: crate::model::InstallationHealth::Healthy,
                    last_seen_at: None,
                }],
            )
            .unwrap();

        let settings = st.config.get_settings().ok();
        let manifest: serde_json::Value =
            serde_json::from_slice(&build_manifest(&st, settings.as_ref()).unwrap()).unwrap();

        // Schema version comes from the applied-migrations reader, not a literal.
        let database = Database::open(std::path::Path::new(st.queue.database_path())).unwrap();
        assert_eq!(
            manifest["schema_version"],
            serde_json::json!(database.schema_version().unwrap())
        );

        // Helper version from hook_installations.
        assert_eq!(
            manifest["helper_version"],
            serde_json::json!([env!("CARGO_PKG_VERSION")])
        );

        let claude = &manifest["agents"]["claude-code"];
        assert_eq!(claude["detected_version"], serde_json::json!("2.1.218"));
        assert_eq!(claude["capability_version"], serde_json::json!("2.1.218"));
        assert_eq!(
            claude["capability_verification"],
            serde_json::json!("exact")
        );
        let codex = &manifest["agents"]["codex"];
        assert_eq!(codex["detected_version"], serde_json::Value::Null);
        assert_eq!(codex["capability_version"], serde_json::json!("0.145.0"));
        assert_eq!(codex["capability_verification"], serde_json::Value::Null);

        // Privacy: no executable path or other unregistered raw path lands in
        // the manifest.
        let text = serde_json::to_string(&manifest).unwrap();
        assert!(!text.contains("/usr/local/bin"));
    }

    #[test]
    fn set_debug_logging_maps_zero_fifteen_sixty_and_rejects_other_values() {
        let st = state();

        // 15 minutes opens a live debug window.
        set_debug_logging_impl(
            &st,
            SetDebugLoggingInput {
                duration_minutes: 15,
            },
        )
        .unwrap();
        assert!(st.diagnostics.debug_active());

        // 0 turns it back off.
        set_debug_logging_impl(
            &st,
            SetDebugLoggingInput {
                duration_minutes: 0,
            },
        )
        .unwrap();
        assert!(!st.diagnostics.debug_active());

        // 60 minutes opens one too.
        set_debug_logging_impl(
            &st,
            SetDebugLoggingInput {
                duration_minutes: 60,
            },
        )
        .unwrap();
        assert!(st.diagnostics.debug_active());

        // Anything else is a typed rejection.
        let error = set_debug_logging_impl(
            &st,
            SetDebugLoggingInput {
                duration_minutes: 30,
            },
        )
        .unwrap_err();
        assert_eq!(
            error.code,
            "configuration.diagnostics.invalid_debug_duration"
        );

        // The command answers with the current settings view.
        let view = set_debug_logging_impl(
            &st,
            SetDebugLoggingInput {
                duration_minutes: 15,
            },
        )
        .unwrap();
        assert_eq!(view.event_retention_days, 30);
    }
}
