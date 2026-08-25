//! Updater commands (stub). Release signing + endpoint config is Task 22.

use serde::{Deserialize, Serialize};
use tauri::State;

use super::{CoreState, configuration_error, update_error};
use crate::error::AppError;

#[derive(Clone, Debug, Serialize)]
pub struct UpdateStatusView {
    pub available: bool,
    pub version: Option<String>,
    pub notes: Option<String>,
    /// Always false here: the install command must not run without a signed
    /// endpoint (Task 22). The frontend surfaces "updater not configured".
    pub installable: bool,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InstallUpdateInput {
    /// Must be `true` to confirm the install. Mirrors the explicit-confirm
    /// contract the plan requires.
    pub confirmed: bool,
}

pub(crate) fn check_for_updates_impl(_state: &CoreState) -> Result<UpdateStatusView, AppError> {
    // ponytail: the updater plugin is wired in lib.rs but its endpoint is not
    // configured until Task 22. We surface a stable "not configured" status so
    // the frontend can render the settings page without a runtime error.
    Ok(UpdateStatusView {
        available: false,
        version: None,
        notes: None,
        installable: false,
    })
}

pub(crate) fn install_update_impl(
    _state: &CoreState,
    input: InstallUpdateInput,
) -> Result<(), AppError> {
    if !input.confirmed {
        return Err(update_error(
            "confirmation_required",
            "install requires explicit confirmation",
        ));
    }
    Err(update_error(
        "endpoint_not_configured",
        "signed update endpoint is configured in release step (Task 22)",
    ))
}

#[tauri::command]
pub async fn check_for_updates(state: State<'_, CoreState>) -> Result<UpdateStatusView, AppError> {
    let state = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || check_for_updates_impl(&state))
        .await
        .map_err(|_| configuration_error("join_failed", "command join failed"))?
}

#[tauri::command]
pub async fn install_update(
    state: State<'_, CoreState>,
    input: InstallUpdateInput,
) -> Result<(), AppError> {
    let state = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || install_update_impl(&state, input))
        .await
        .map_err(|_| configuration_error("join_failed", "command join failed"))?
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::CoreState;
    use crate::events::catalog::catalog_for;
    use crate::model::AgentKind;
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
        let cipher = std::sync::Arc::new(FieldCipher::from_key([6u8; 32]));
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
    fn check_for_updates_reports_not_available_without_endpoint() {
        let st = state();
        let view = check_for_updates_impl(&st).unwrap();
        assert!(!view.available);
        assert!(!view.installable);
    }

    #[test]
    fn install_without_confirmation_is_rejected() {
        let st = state();
        let err = install_update_impl(&st, InstallUpdateInput { confirmed: false }).unwrap_err();
        assert_eq!(err.code, "update.confirmation_required");
    }

    #[test]
    fn install_with_confirmation_still_defers_to_endpoint_config() {
        let st = state();
        let err = install_update_impl(&st, InstallUpdateInput { confirmed: true }).unwrap_err();
        assert_eq!(err.code, "update.endpoint_not_configured");
    }
}
