//! Agent integration commands: detect, list integrations, apply hook action.

use serde::{Deserialize, Serialize};
use tauri::State;

use super::{CoreState, configuration_error, integration_error};
use crate::agents::EntryHealth;
use crate::error::AppError;
use crate::installer::lifecycle::{HookAction, HookInstaller};
use crate::model::{AgentKind, TrustStatus};
use crate::rules::resolve::required_hook_selection;

// ---------------------------------------------------------------------------
// Inputs / outputs
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DetectAgentsInput {
    /// When true, allow installing/inspecting against a `CompatibleUnverified`
    /// catalog version. Mirrors the AgentIntegration confirmation flag.
    pub confirm_compatible_version: bool,
}

#[derive(Clone, Debug, Serialize)]
pub struct AgentIntegrationView {
    pub agent: String,
    pub installed: bool,
    pub version: Option<String>,
    pub executable_path: Option<String>,
    pub health: String,
    pub needs_compatible_version_confirmation: bool,
}

#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HookActionInput {
    Install,
    Repair,
    UpgradeHelper,
    Uninstall,
}

impl HookActionInput {
    fn into_lifecycle(self) -> HookAction {
        match self {
            Self::Install => HookAction::Install,
            Self::Repair => HookAction::Repair,
            Self::UpgradeHelper => HookAction::UpgradeHelper,
            Self::Uninstall => HookAction::Uninstall,
        }
    }
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ApplyHookActionInput {
    pub agent: AgentKindInput,
    pub action: HookActionInput,
    /// Expected health revision. Frontend passes the revision it last saw so
    /// a stale request can be rejected; we do not yet track revisions across
    /// processes, so today this is accepted but logged.
    pub expected_health_revision: u64,
    /// Must be `true` when the catalog verification is
    /// `CompatibleUnverified`. The frontend cannot install without
    /// acknowledging the version mismatch.
    pub confirm_compatible_version: bool,
}

#[derive(Clone, Copy, Debug, Deserialize)]
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

#[derive(Clone, Debug, Serialize)]
pub struct HookHealthView {
    pub agent: String,
    pub selection_out_of_date: bool,
    pub entries: Vec<HookEntryView>,
}

/// Serialized with the enums' own serde snake_case renames — never
/// `format!("{:?}")`, which would leak PascalCase Debug strings to the UI.
#[derive(Clone, Debug, Serialize)]
pub struct HookEntryView {
    pub source_event: String,
    pub trust_status: TrustStatus,
    pub health: EntryHealth,
}

// ---------------------------------------------------------------------------
// Bodies
// ---------------------------------------------------------------------------

pub(crate) fn detect_agents_impl(
    state: &CoreState,
    input: DetectAgentsInput,
) -> Result<Vec<AgentIntegrationView>, AppError> {
    let _ = input;
    let mut views = Vec::new();
    for agent in [AgentKind::ClaudeCode, AgentKind::Codex] {
        let detection = crate::agents::detect_agent(agent, None);
        // Persist detection result so health reflects it.
        let _ = crate::agents::persist_detection(&state.integrations, &detection);
        views.push(build_agent_view(agent, &detection));
    }
    Ok(views)
}

fn detection_state_summary(state: &crate::agents::DetectionState) -> String {
    use crate::agents::DetectionState;
    match state {
        DetectionState::Detected => "detected".into(),
        DetectionState::Missing => "missing".into(),
        DetectionState::InvalidVersion => "invalid_version".into(),
        DetectionState::ProcessFailed => "process_failed".into(),
        DetectionState::TimedOut => "timed_out".into(),
    }
}

fn build_agent_view(
    agent: AgentKind,
    detection: &crate::agents::Detection,
) -> AgentIntegrationView {
    let version = detection
        .version
        .clone()
        .unwrap_or_else(|| semver::Version::new(0, 0, 0));
    let resolution = crate::events::catalog::catalog_for(agent, &version);
    let needs_confirm = matches!(
        resolution.verification,
        crate::events::catalog::CatalogVerification::CompatibleUnverified
    );
    AgentIntegrationView {
        agent: agent.as_str().into(),
        installed: detection.state == crate::agents::DetectionState::Detected,
        version: detection.version.as_ref().map(|v| v.to_string()),
        executable_path: detection
            .executable_path
            .as_ref()
            .map(|p| p.to_string_lossy().into_owned()),
        health: detection_state_summary(&detection.state),
        needs_compatible_version_confirmation: needs_confirm,
    }
}

pub(crate) fn list_agent_integrations_impl(
    state: &CoreState,
) -> Result<Vec<AgentIntegrationView>, AppError> {
    let mut views = Vec::new();
    for agent in [AgentKind::ClaudeCode, AgentKind::Codex] {
        let detection = crate::agents::detect_agent(agent, None);
        let _ = state;
        views.push(build_agent_view(agent, &detection));
    }
    Ok(views)
}

pub(crate) fn apply_hook_action_impl(
    state: &CoreState,
    input: ApplyHookActionInput,
) -> Result<HookHealthView, AppError> {
    let agent = input.agent.into_kind();
    let action = input.action.into_lifecycle();
    let _ = input.expected_health_revision;

    let detection = crate::agents::detect_agent(agent, None);
    let resolution = crate::events::catalog::catalog_for(
        agent,
        &detection
            .version
            .clone()
            .unwrap_or_else(|| semver::Version::new(0, 0, 0)),
    );
    if matches!(
        resolution.verification,
        crate::events::catalog::CatalogVerification::CompatibleUnverified
    ) && !input.confirm_compatible_version
    {
        return Err(integration_error(
            "agent_confirmation_required",
            "agent version requires explicit confirmation",
        ));
    }

    // Derive the required hook selection from the current rules — the frontend
    // cannot supply events/paths/fingerprints.
    let global = state.config.list_global_rules()?;
    let overrides: Vec<_> = state
        .config
        .list_projects()?
        .into_iter()
        .flat_map(|p| collect_project_patches(state, p.id))
        .flatten()
        .collect();
    let required = required_hook_selection(&global, &overrides);
    // Deploy the bundled signed helper to its stable path (idempotent) and
    // derive the selection from the WIRED installer: the lifecycle requires
    // the selection's helper path/version to equal the stable binary's, so a
    // hand-built path here would drift (e.g. the Windows `.exe` suffix).
    let env = build_hook_environment(state)?;
    let selection = crate::agents::HookSelection {
        events: required
            .into_iter()
            .filter(|(a, _)| *a == agent)
            .map(|(_, e)| e)
            .collect(),
        helper_path: env.helper.stable_path(),
        helper_version: env.helper.manifest_version().clone(),
    };
    let installer = build_installer(agent, &detection, state, &env);
    installer.apply(action, &selection)?;
    let health = installer.inspect(&selection)?;
    Ok(HookHealthView {
        agent: agent.as_str().into(),
        selection_out_of_date: health.selection_out_of_date,
        entries: health
            .entries
            .into_iter()
            .map(|e| HookEntryView {
                source_event: e.source_event,
                trust_status: e.trust_status,
                health: e.health,
            })
            .collect(),
    })
}

fn collect_project_patches(
    state: &CoreState,
    project_id: uuid::Uuid,
) -> Result<Vec<crate::rules::resolve::StoredRulePatch>, AppError> {
    // ponytail: there is no list_project_patches API; we iterate global rules
    // and probe per-project. Acceptable at the small list sizes here; promote
    // to a dedicated query if the rule table grows.
    let globals = state.config.list_global_rules()?;
    let mut out = Vec::new();
    for g in globals {
        if let Ok(patch) = state
            .config
            .get_project_patch(project_id, g.agent, &g.source_event)
        {
            out.push(patch);
        }
    }
    Ok(out)
}

/// Wire the hook environment for a real mutation. The signed helper is loaded
/// from the bundled resources (fixed paths under the shell-resolved resource
/// directory — never caller input) and DEPLOYED to its stable per-user path
/// first, idempotently (`ensure_installed` skips the copy when identical bytes
/// are already in place). Without this bridge every Install/Repair would fail
/// with `update.helper_not_installed` because nothing else ever populates the
/// stable path. Development builds carry the placeholder manifest and surface
/// the typed `configuration.helper_unavailable` error here.
fn build_hook_environment(
    state: &CoreState,
) -> Result<crate::installer::lifecycle::HookEnvironment, AppError> {
    let bin_dir = state
        .integrations
        .database_path()
        .parent()
        .unwrap_or_else(|| std::path::Path::new("."))
        .join("bin");
    let resources_dir = state.resources_dir.as_deref().ok_or_else(|| {
        crate::installer::helper::helper_unavailable_error(
            "the application resource directory is unavailable",
        )
    })?;
    let helper = crate::installer::helper::load_bundled_installer(resources_dir, &bin_dir)?;
    helper.ensure_installed()?;
    let home = directories::BaseDirs::new()
        .map(|d| d.home_dir().to_path_buf())
        .unwrap_or_else(|| std::path::PathBuf::from("."));
    Ok(crate::installer::lifecycle::HookEnvironment {
        repository: state.integrations.clone(),
        cipher: Some((*state.cipher).clone()),
        helper,
        home,
        codex_home: None,
    })
}

fn build_installer(
    agent: AgentKind,
    _detection: &crate::agents::Detection,
    _state: &CoreState,
    env: &crate::installer::lifecycle::HookEnvironment,
) -> HookInstaller {
    let config_path = match agent {
        AgentKind::ClaudeCode => env.home.join(".claude").join("settings.json"),
        AgentKind::Codex => env
            .codex_home
            .clone()
            .unwrap_or_else(|| env.home.join(".codex"))
            .join("hooks.json"),
    };
    HookInstaller::new(
        agent,
        config_path,
        env.repository.clone(),
        env.cipher.clone(),
        env.helper.clone(),
    )
}

// ---------------------------------------------------------------------------
// Tauri wrappers
// ---------------------------------------------------------------------------

#[tauri::command]
pub async fn detect_agents(
    state: State<'_, CoreState>,
    input: DetectAgentsInput,
) -> Result<Vec<AgentIntegrationView>, AppError> {
    let state = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || detect_agents_impl(&state, input))
        .await
        .map_err(|_| configuration_error("join_failed", "command join failed"))?
}

#[tauri::command]
pub async fn list_agent_integrations(
    state: State<'_, CoreState>,
) -> Result<Vec<AgentIntegrationView>, AppError> {
    let state = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || list_agent_integrations_impl(&state))
        .await
        .map_err(|_| configuration_error("join_failed", "command join failed"))?
}

#[tauri::command]
pub async fn apply_hook_action(
    state: State<'_, CoreState>,
    input: ApplyHookActionInput,
) -> Result<HookHealthView, AppError> {
    let state = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || apply_hook_action_impl(&state, input))
        .await
        .map_err(|_| configuration_error("join_failed", "command join failed"))?
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The frontend mirrors these as snake_case codes; Debug formatting would
    /// emit PascalCase (`NotRequired`/`Healthy`) and break every consumer.
    #[test]
    fn hook_entry_view_serializes_snake_case_codes() {
        let view = HookEntryView {
            source_event: "Stop".into(),
            trust_status: TrustStatus::NeedsUserConfirmation,
            health: EntryHealth::AgentUpgradeRequired,
        };
        let json = serde_json::to_value(&view).unwrap();
        assert_eq!(json["trust_status"], "needs_user_confirmation");
        assert_eq!(json["health"], "agent_upgrade_required");

        let clean = HookEntryView {
            source_event: "Stop".into(),
            trust_status: TrustStatus::NotRequired,
            health: EntryHealth::Healthy,
        };
        let json = serde_json::to_value(&clean).unwrap();
        assert_eq!(json["trust_status"], "not_required");
        assert_eq!(json["health"], "healthy");
    }
}
