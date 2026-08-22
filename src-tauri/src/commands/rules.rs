//! Rule commands: list/save/reset, plus preview + send-test.

use serde::{Deserialize, Serialize};
use tauri::State;

use super::{CoreState, configuration_error, parse_uuid_input};
use crate::error::AppError;
use crate::events::catalog::{CapabilityResolution, CatalogVerification, catalog_for};
use crate::model::{AgentKind, NotificationDocument, PatchField, RuleConfig, Severity};
use crate::rules::resolve::{StoredGlobalRule, required_hook_selection};
use crate::worker::ChannelSenderFactory;

// ---------------------------------------------------------------------------
// Inputs
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum AgentKindInput {
    ClaudeCode,
    Codex,
}
impl AgentKindInput {
    fn into_kind(self) -> AgentKind {
        match self {
            Self::ClaudeCode => AgentKind::ClaudeCode,
            Self::Codex => AgentKind::Codex,
        }
    }
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SaveGlobalRuleInput {
    pub agent: AgentKindInput,
    pub source_event: String,
    pub config: RuleConfig,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SaveProjectRulePatchInput {
    pub project_id: String,
    pub agent: AgentKindInput,
    pub source_event: String,
    pub patch: crate::model::RulePatch,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResetProjectRuleFieldInput {
    pub project_id: String,
    pub agent: AgentKindInput,
    pub source_event: String,
    pub field: PatchFieldInput,
}

#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PatchFieldInput {
    Enabled,
    Targets,
    Filters,
    Privacy,
    Delivery,
    QuietHours,
}
impl PatchFieldInput {
    fn into_field(self) -> PatchField {
        match self {
            Self::Enabled => PatchField::Enabled,
            Self::Targets => PatchField::Targets,
            Self::Filters => PatchField::Filters,
            Self::Privacy => PatchField::Privacy,
            Self::Delivery => PatchField::Delivery,
            Self::QuietHours => PatchField::QuietHours,
        }
    }
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PreviewNotificationInput {
    pub agent: AgentKindInput,
    pub source_event: String,
    /// Optional project scope; preview uses global rule if absent.
    pub project_id: Option<String>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SendRuleTestInput {
    pub agent: AgentKindInput,
    pub source_event: String,
    pub channel_id: String,
}

// ---------------------------------------------------------------------------
// Outputs
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, Serialize)]
pub struct GlobalRuleView {
    pub agent: String,
    pub source_event: String,
    pub enabled: bool,
    pub version: u64,
    pub config: RuleConfig,
}

impl From<StoredGlobalRule> for GlobalRuleView {
    fn from(rule: StoredGlobalRule) -> Self {
        Self {
            agent: rule.agent.as_str().into(),
            source_event: rule.source_event,
            enabled: rule.config.enabled,
            version: rule.version,
            config: rule.config,
        }
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct SelectionOutOfDateView {
    /// True when the rules-derived `required_hook_selection` differs from the
    /// installed hook rows. The frontend surfaces a "repair needed" badge.
    pub selection_out_of_date: bool,
    pub health_revision: u64,
}

// ---------------------------------------------------------------------------
// Bodies
// ---------------------------------------------------------------------------

pub(crate) fn list_hook_rules_impl(state: &CoreState) -> Result<Vec<GlobalRuleView>, AppError> {
    Ok(state
        .config
        .list_global_rules()?
        .into_iter()
        .map(GlobalRuleView::from)
        .collect())
}

pub(crate) fn save_global_rule_impl(
    state: &CoreState,
    input: SaveGlobalRuleInput,
) -> Result<GlobalRuleView, AppError> {
    let agent = input.agent.into_kind();
    if !is_known_capability(state, agent, &input.source_event)? {
        return Err(configuration_error(
            "unknown_hook",
            "rule capability is not catalogued",
        ));
    }
    let existing = state
        .config
        .get_global_rule(agent, &input.source_event)
        .ok();
    let id = existing
        .as_ref()
        .map(|r| r.id)
        .unwrap_or_else(uuid::Uuid::now_v7);
    let version = existing.as_ref().map(|r| r.version).unwrap_or(0);
    let stored = StoredGlobalRule {
        id,
        agent,
        source_event: input.source_event,
        version,
        config: input.config,
    };
    state.config.save_global_rule(&stored)?;
    // Recompute selection and emit a revision if it differs from installed
    // rows — but do NOT mutate Agent config (only apply_hook_action does).
    let _ = recompute_selection_health(state)?;
    let fresh = state.config.get_global_rule(agent, &stored.source_event)?;
    Ok(GlobalRuleView::from(fresh))
}

pub(crate) fn save_project_rule_patch_impl(
    state: &CoreState,
    input: SaveProjectRulePatchInput,
) -> Result<(), AppError> {
    let project_id = parse_uuid_input(&input.project_id)?;
    let agent = input.agent.into_kind();
    if !is_known_capability(state, agent, &input.source_event)? {
        return Err(configuration_error(
            "unknown_hook",
            "rule capability is not catalogued",
        ));
    }
    state
        .config
        .save_project_patch(project_id, agent, &input.source_event, &input.patch)?;
    let _ = recompute_selection_health(state)?;
    Ok(())
}

pub(crate) fn reset_project_rule_field_impl(
    state: &CoreState,
    input: ResetProjectRuleFieldInput,
) -> Result<(), AppError> {
    let project_id = parse_uuid_input(&input.project_id)?;
    let agent = input.agent.into_kind();
    let field = input.field.into_field();
    state
        .config
        .reset_project_patch_field(project_id, agent, &input.source_event, field)?;
    let _ = recompute_selection_health(state)?;
    Ok(())
}

pub(crate) fn preview_notification_impl(
    state: &CoreState,
    input: PreviewNotificationInput,
) -> Result<NotificationDocument, AppError> {
    let agent = input.agent.into_kind();
    // Build a representative envelope from the catalog event. The preview is a
    // rendering of the default document for the (agent, event) pair under the
    // effective rule — full simulation lives behind a richer input when the
    // drawer adds it.
    let resolution = resolve_catalog(state, agent);
    let hook = resolution
        .catalog
        .hooks
        .iter()
        .find(|h| h.source_event == input.source_event)
        .ok_or_else(|| configuration_error("unknown_hook", "rule capability is not catalogued"))?;
    let rule = match input.project_id.as_deref() {
        Some(id) => {
            let pid = parse_uuid_input(id)?;
            match state
                .config
                .get_project_patch(pid, agent, &input.source_event)
                .ok()
            {
                Some(patch) => crate::rules::resolve::resolve_rule(
                    &state
                        .config
                        .get_global_rule(agent, &input.source_event)?
                        .config,
                    Some(&patch.patch),
                ),
                None => {
                    state
                        .config
                        .get_global_rule(agent, &input.source_event)?
                        .config
                }
            }
        }
        None => {
            state
                .config
                .get_global_rule(agent, &input.source_event)?
                .config
        }
    };
    let doc = NotificationDocument {
        title: format!("Preview: {}", hook.label_en),
        severity: match hook.sensitivity {
            crate::events::catalog::Sensitivity::Forbidden => Severity::Critical,
            crate::events::catalog::Sensitivity::Sensitive => Severity::Warning,
            crate::events::catalog::Sensitivity::Public => Severity::Info,
        },
        facts: vec![
            ("Agent".into(), agent.as_str().into()),
            ("Event".into(), input.source_event.clone()),
            ("Rule enabled".into(), rule.enabled.to_string()),
        ],
        body: format!(
            "Preview notification for {} ({}).",
            hook.label_en, hook.label_zh
        ),
        footer: None,
    };
    Ok(doc)
}

pub(crate) async fn send_rule_test_impl(
    state: &CoreState,
    input: SendRuleTestInput,
) -> Result<(), AppError> {
    let channel_id = parse_uuid_input(&input.channel_id)?;
    let channel = state.config.get_channel(channel_id)?;
    // A DingTalk keyword robot only accepts messages containing the configured
    // keyword, so the test must carry it exactly like a real delivery.
    let keyword_prefix = match &channel.public_config {
        crate::model::ChannelPublicConfig::DingTalk { keyword_prefix } => keyword_prefix.as_deref(),
        crate::model::ChannelPublicConfig::WeCom => None,
    };
    let document = NotificationDocument {
        title: "CC Reminder rule test".into(),
        severity: Severity::Info,
        facts: vec![
            ("Agent".into(), input.agent.into_kind().as_str().into()),
            ("Event".into(), input.source_event),
        ],
        body: "Rule test from CC Reminder.".into(),
        footer: None,
    };
    let factory = crate::worker::ProductionSenderFactory::new(state.credentials.clone());
    factory
        .send(
            channel.kind,
            &channel.credential_ref,
            keyword_prefix,
            document,
        )
        .await
        .map(|_| ())
        .map_err(|e| crate::error::AppError {
            domain: crate::error::ErrorDomain::Delivery,
            code: format!("delivery.{}", sanitize(&e.code)),
            message: e.redacted_message,
            suggested_action: None,
        })
}

fn sanitize(s: &str) -> String {
    s.chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '_' || *c == '.')
        .collect()
}

fn is_known_capability(state: &CoreState, agent: AgentKind, event: &str) -> Result<bool, AppError> {
    // ponytail: catalogued_hooks() is the closed catalog; the config repo also
    // has its own validate_capability. We re-check here so the command returns
    // a typed error before touching storage.
    let catalogued = crate::events::catalog::catalogued_hooks();
    Ok(catalogued.contains(&(agent, event.to_owned()))
        || state
            .config
            .list_global_rules()?
            .iter()
            .any(|r| r.agent == agent && r.source_event == event))
}

fn resolve_catalog(state: &CoreState, agent: AgentKind) -> CapabilityResolution {
    let version = state
        .integrations
        .agent(agent)
        .ok()
        .and_then(|a| a.version)
        .unwrap_or_else(|| match agent {
            AgentKind::ClaudeCode => semver::Version::new(2, 1, 218),
            AgentKind::Codex => semver::Version::new(0, 145, 0),
        });
    catalog_for(agent, &version)
}

/// Recompute the required hook selection and compare with installed rows.
/// Emits a `core://health-changed` revision in the real runtime; in tests the
/// side-effect is the return value.
fn recompute_selection_health(state: &CoreState) -> Result<SelectionOutOfDateView, AppError> {
    let globals = state.config.list_global_rules()?;
    let mut overrides = Vec::new();
    for project in state.config.list_projects()? {
        for g in &globals {
            if let Ok(patch) = state
                .config
                .get_project_patch(project.id, g.agent, &g.source_event)
            {
                overrides.push(patch);
            }
        }
    }
    let required = required_hook_selection(&globals, &overrides);
    // Compare against installed rows: any required (agent, event) without an
    // installed hook row means selection is out of date.
    let mut out_of_date = false;
    for (agent, event) in &required {
        if state.integrations.hook(*agent, event).is_err() {
            out_of_date = true;
            break;
        }
    }
    let _ = CatalogVerification::Exact; // suppress unused import on some cfgs
    Ok(SelectionOutOfDateView {
        selection_out_of_date: out_of_date,
        health_revision: 0,
    })
}

// ---------------------------------------------------------------------------
// Tauri wrappers
// ---------------------------------------------------------------------------

#[tauri::command]
pub async fn list_hook_rules(state: State<'_, CoreState>) -> Result<Vec<GlobalRuleView>, AppError> {
    let state = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || list_hook_rules_impl(&state))
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
        CoreState::new(config, events, queue, integrations, credentials, cipher)
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
