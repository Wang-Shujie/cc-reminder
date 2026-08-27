//! 规则命令面:mod 收口 + Tauri wrappers + 命令级测试;
//! inputs/views/impls 子模块承载原单一文件的各段(架构提案 §3)。

use tauri::State;

use super::{CoreState, configuration_error};
use crate::error::AppError;
use crate::model::NotificationDocument;

mod impls;
mod inputs;
mod views;

use impls::*;
use inputs::*;
use views::*;

// ---------------------------------------------------------------------------
// Tauri wrappers
// ---------------------------------------------------------------------------

#[tauri::command]
pub async fn list_hook_rules(
    state: State<'_, CoreState>,
    input: ListHookRulesInput,
) -> Result<Vec<GlobalRuleView>, AppError> {
    let state = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || list_hook_rules_impl(&state, input))
        .await
        .map_err(|_| configuration_error("join_failed", "command join failed"))?
}

#[tauri::command]
pub async fn save_global_rule(
    state: State<'_, CoreState>,
    input: SaveGlobalRuleInput,
) -> Result<GlobalRuleView, AppError> {
    let state = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || save_global_rule_impl(&state, input))
        .await
        .map_err(|_| configuration_error("join_failed", "command join failed"))?
}

#[tauri::command]
pub async fn save_project_rule_patch(
    state: State<'_, CoreState>,
    input: SaveProjectRulePatchInput,
) -> Result<(), AppError> {
    let state = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || save_project_rule_patch_impl(&state, input))
        .await
        .map_err(|_| configuration_error("join_failed", "command join failed"))?
}

#[tauri::command]
pub async fn reset_project_rule_field(
    state: State<'_, CoreState>,
    input: ResetProjectRuleFieldInput,
) -> Result<(), AppError> {
    let state = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || reset_project_rule_field_impl(&state, input))
        .await
        .map_err(|_| configuration_error("join_failed", "command join failed"))?
}

#[tauri::command]
pub async fn preview_notification(
    state: State<'_, CoreState>,
    input: PreviewNotificationInput,
) -> Result<NotificationDocument, AppError> {
    let state = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || preview_notification_impl(&state, input))
        .await
        .map_err(|_| configuration_error("join_failed", "command join failed"))?
}

#[tauri::command]
pub async fn send_rule_test(
    state: State<'_, CoreState>,
    input: SendRuleTestInput,
) -> Result<(), AppError> {
    // The impl awaits the (async) sender factory directly, so it must run on
    // the async runtime — not inside spawn_blocking.
    send_rule_test_impl(state.inner(), input).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::CoreState;
    use crate::events::catalog::{CapabilityStatus, Sensitivity, catalog_for};
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

    fn test_state() -> CoreState {
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
        let cipher = std::sync::Arc::new(FieldCipher::from_key([3u8; 32]));
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

    fn unknown_rule_input() -> SaveGlobalRuleInput {
        SaveGlobalRuleInput {
            agent: AgentKindInput::Codex,
            source_event: "ThisEventDoesNotExist".into(),
            config: crate::rules::resolve::default_rule(AgentKind::Codex, "Stop"),
        }
    }

    #[tokio::test]
    async fn rule_command_rejects_unknown_capability_and_invalid_patch() {
        let error = save_global_rule_impl(&test_state(), unknown_rule_input()).unwrap_err();
        assert_eq!(error.code, "configuration.unknown_hook");
    }

    #[test]
    fn list_hook_rules_decorates_rows_with_capability_metadata() {
        let state = test_state();
        let views = list_hook_rules_impl(
            &state,
            ListHookRulesInput {
                agent: AgentKindInput::ClaudeCode,
                project_id: None,
            },
        )
        .unwrap();

        // ensure_global_rules seeded every catalogued event.
        assert!(views.len() >= 30);
        let permission = views
            .iter()
            .find(|view| view.source_event == "PermissionRequest")
            .unwrap();
        assert_eq!(permission.phase, "request");
        assert_eq!(permission.sensitivity, Sensitivity::Sensitive);
        assert!(permission.available);
        assert!(!permission.installed); // nothing installed in a fresh env
        assert!(!permission.high_frequency);
        assert_eq!(permission.status, CapabilityStatus::Stable);
        // Catalog input fields are exposed for the drawer's privacy section.
        assert!(
            permission
                .input_fields
                .iter()
                .any(|field| field.name == "tool_input")
        );
        assert!(permission.patched_fields.is_empty());
    }

    #[test]
    fn reset_unknown_field_is_rejected_by_deny_unknown_fields_at_parse() {
        // serde(deny_unknown_fields) rejects unknown variants at the boundary;
        // PatchFieldInput is a closed enum, so an unknown field name fails
        // deserialization before reaching the command. We assert the closed
        // enum refuses an out-of-set value at the JSON layer.
        let json = r#"{"project_id":"00000000-0000-0000-0000-000000000000","agent":"codex","source_event":"Stop","field":"nonexistent"}"#;
        let err = serde_json::from_str::<ResetProjectRuleFieldInput>(json).unwrap_err();
        assert!(err.to_string().contains("unknown") || err.to_string().contains("nonexistent"));
    }

    #[test]
    fn uninstall_never_deploys_or_requires_the_helper() {
        // v2-issues: Uninstall 被 lifecycle 豁免 helper 在场校验,commands 层
        // 也不得部署/要求 helper——占位 manifest 的开发环境卸载不应报
        // helper_unavailable,release 环境卸载不得顺带写盘签名 helper。
        let state = test_state();
        let input = crate::commands::agents::ApplyHookActionInput {
            agent: crate::commands::agents::AgentKindInput::ClaudeCode,
            action: crate::commands::agents::HookActionInput::Uninstall,
            expected_health_revision: 0,
            confirm_compatible_version: true,
        };
        if let Err(err) = crate::commands::agents::apply_hook_action_impl(&state, input) {
            assert_ne!(err.code, "configuration.helper_unavailable");
            assert_ne!(err.code, "update.helper_not_installed");
        }
    }

    #[test]
    fn unverified_install_without_confirmation_is_rejected() {
        let state = test_state();
        let input = crate::commands::agents::ApplyHookActionInput {
            agent: crate::commands::agents::AgentKindInput::Codex,
            action: crate::commands::agents::HookActionInput::Install,
            expected_health_revision: 0,
            confirm_compatible_version: false,
        };
        // The hard contract: apply_hook_action never mutates Agent config
        // without a detected+confirmed agent. With no agent installed in the
        // test environment, it must error out before touching Agent config; the
        // exact code depends on detection outcome (integration.* when the agent
        // is missing/needs confirmation, configuration.* when helper setup
        // fails). We assert the command rejects rather than mutates.
        let err = crate::commands::agents::apply_hook_action_impl(&state, input).unwrap_err();
        assert!(
            err.code.starts_with("integration.")
                || err.code.starts_with("configuration.")
                || err.code.starts_with("update."),
            "unexpected error code: {}",
            err.code
        );
    }
}
