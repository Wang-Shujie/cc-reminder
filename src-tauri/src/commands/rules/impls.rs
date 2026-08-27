// 规则命令的 typed 实现(无 Tauri 类型,原样移出)。

use super::super::{CoreState, configuration_error, parse_uuid_input};
use super::inputs::*;
use super::views::*;
use crate::error::AppError;
use crate::events::catalog::{
    CapabilityResolution, CatalogVerification, catalog_for, reference_catalog,
};
use crate::model::{AgentKind, NotificationDocument, Severity};
use crate::rules::resolve::{StoredGlobalRule, required_hook_selection};
use crate::worker::ChannelSenderFactory;
// ---------------------------------------------------------------------------
// Bodies
// ---------------------------------------------------------------------------

pub(crate) fn list_hook_rules_impl(
    state: &CoreState,
    input: ListHookRulesInput,
) -> Result<Vec<GlobalRuleView>, AppError> {
    let agent = input.agent.into_kind();
    let project_id = match input.project_id.as_deref() {
        Some(id) => Some(parse_uuid_input(id)?),
        None => None,
    };
    // `available` reflects the CURRENT version resolution; metadata comes from
    // the newest verified reference catalog.
    let resolution = resolve_catalog(state, agent);
    let reference = reference_catalog(agent);
    let mut views = Vec::new();
    for rule in state
        .storage
        .config
        .list_global_rules()?
        .into_iter()
        .filter(|rule| rule.agent == agent)
    {
        let patch = match project_id {
            Some(pid) => state
                .storage
                .config
                .get_project_patch(pid, agent, &rule.source_event)
                .ok(),
            None => None,
        };
        let available = resolution
            .catalog
            .hooks
            .iter()
            .any(|hook| hook.source_event == rule.source_event);
        let cap = reference
            .hooks
            .iter()
            .find(|hook| hook.source_event == rule.source_event);
        let installed = state
            .storage
            .integrations
            .hook(agent, &rule.source_event)
            .is_ok();
        views.push(build_rule_view(
            rule,
            patch.as_ref(),
            installed,
            cap,
            available,
        ));
    }
    Ok(views)
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
        .storage
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
    state.storage.config.save_global_rule(&stored)?;
    // Recompute selection and emit a revision if it differs from installed
    // rows — but do NOT mutate Agent config (only apply_hook_action does).
    let _ = recompute_selection_health(state)?;
    let fresh = state
        .storage
        .config
        .get_global_rule(agent, &stored.source_event)?;
    let resolution = resolve_catalog(state, agent);
    let reference = reference_catalog(agent);
    let available = resolution
        .catalog
        .hooks
        .iter()
        .any(|hook| hook.source_event == fresh.source_event);
    let cap = reference
        .hooks
        .iter()
        .find(|hook| hook.source_event == fresh.source_event);
    let installed = state
        .storage
        .integrations
        .hook(agent, &fresh.source_event)
        .is_ok();
    Ok(build_rule_view(fresh, None, installed, cap, available))
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
    state.storage.config.save_project_patch(
        project_id,
        agent,
        &input.source_event,
        &input.patch,
    )?;
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
    state.storage.config.reset_project_patch_field(
        project_id,
        agent,
        &input.source_event,
        field,
    )?;
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
                .storage
                .config
                .get_project_patch(pid, agent, &input.source_event)
                .ok()
            {
                Some(patch) => crate::rules::resolve::resolve_rule(
                    &state
                        .storage
                        .config
                        .get_global_rule(agent, &input.source_event)?
                        .config,
                    Some(&patch.patch),
                ),
                None => {
                    state
                        .storage
                        .config
                        .get_global_rule(agent, &input.source_event)?
                        .config
                }
            }
        }
        None => {
            state
                .storage
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
    let channel = state.storage.config.get_channel(channel_id)?;
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
            .storage
            .config
            .list_global_rules()?
            .iter()
            .any(|r| r.agent == agent && r.source_event == event))
}

fn resolve_catalog(state: &CoreState, agent: AgentKind) -> CapabilityResolution {
    let version = state
        .storage
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
/// Every rule save that reaches here mutated selection state, so a
/// `core://health-changed` revision is pushed through the shared sink (the
/// forwarder task in lib.rs turns it into a revision-only WebView emit; in
/// pure command tests the default sink is disconnected and drops it).
fn recompute_selection_health(state: &CoreState) -> Result<SelectionOutOfDateView, AppError> {
    let globals = state.storage.config.list_global_rules()?;
    let mut overrides = Vec::new();
    for project in state.storage.config.list_projects()? {
        for g in &globals {
            if let Ok(patch) =
                state
                    .storage
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
        if state.storage.integrations.hook(*agent, event).is_err() {
            out_of_date = true;
            break;
        }
    }
    // Board-wide health revision bump (no single channel triggered it). The
    // payload carries only the revision — subscribers refetch details.
    crate::worker::emit(
        &state.runtime.core_events,
        crate::worker::CoreEvent::HealthChanged { channel_id: None },
    );
    let _ = CatalogVerification::Exact; // suppress unused import on some cfgs
    Ok(SelectionOutOfDateView {
        selection_out_of_date: out_of_date,
        health_revision: 0,
    })
}
