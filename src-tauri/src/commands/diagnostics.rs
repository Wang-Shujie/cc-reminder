//! Diagnostic export + clear-history commands (Task 20).

use serde::Deserialize;
use sha2::{Digest, Sha256};
use tauri::State;

use super::{CoreState, configuration_error};
use crate::diagnostics::Diagnostics;
use crate::error::{AppError, ErrorDomain};

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExportDiagnosticsInput {
    /// User-selected save path from the frontend dialog.
    pub selected_path: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ClearHistoryInput {
    /// v1 exposes only the preserve-active-work form; the confirmation is the
    /// exact dialog text on the UI side.
    pub preserve_active_jobs: bool,
}

pub(crate) fn export_diagnostics_impl(
    state: &CoreState,
    input: ExportDiagnosticsInput,
) -> Result<String, AppError> {
    if input.selected_path.trim().is_empty() {
        return Err(configuration_error(
            "diagnostics.path_invalid",
            "a save path is required",
        ));
    }
    let diagnostics = Diagnostics::init(
        &crate::paths::AppPaths::discover()
            .map_err(|_| configuration_error("diagnostics.unavailable", "app paths unavailable"))?
            .logs,
    )?;
    let settings = state.config.get_settings();
    let manifest = build_manifest(state, settings.as_ref().ok())?;
    // health.json: a fresh typed snapshot from the shared projection.
    let snapshot = crate::commands::health_snapshot(state)?;
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
    let archive = diagnostics.export(&[
        ("manifest.json", manifest),
        ("health.json", health),
        ("queue-stats.json", queue_stats),
    ])?;
    std::fs::write(&input.selected_path, archive).map_err(|_| AppError {
        domain: ErrorDomain::Storage,
        code: "diagnostics.write_failed".to_owned(),
        message: "diagnostic archive could not be written".to_owned(),
        suggested_action: Some("choose a different save location".to_owned()),
    })?;
    Ok(input.selected_path)
}

/// manifest.json: versions, OS/architecture, SHA-256 hashes of non-sensitive
/// serialized settings/rules — never the values — schema version, export time.
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
    manifest.insert("schema_version".to_owned(), serde_json::json!(1));
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
    crate::storage::retention::clear_history(
        &crate::storage::db::Database::open(std::path::Path::new(state.queue.database_path()))
            .map_err(|_| {
                configuration_error("diagnostics.unavailable", "database could not be opened")
            })?,
        chrono::Utc::now(),
    )
}

#[tauri::command]
pub async fn export_diagnostics(
    state: State<'_, CoreState>,
    input: ExportDiagnosticsInput,
) -> Result<String, AppError> {
    let state = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || export_diagnostics_impl(&state, input))
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
