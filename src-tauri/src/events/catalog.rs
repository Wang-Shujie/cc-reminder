use semver::Version;
use serde::{Deserialize, Serialize};

use crate::model::{AgentKind, EventCategory};

const CLAUDE_CODE_CATALOG: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/resources/capabilities/claude-code-2.1.218.json"
));
const CODEX_CATALOG: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/resources/capabilities/codex-0.145.0.json"
));
const SAFE_EVENTS: [&str; 4] = ["SessionStart", "SessionEnd", "PermissionRequest", "Stop"];

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct CapabilityCatalog {
    pub agent: AgentKind,
    pub verified_version: Version,
    pub hooks: Vec<HookCapability>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct HookCapability {
    pub source_event: String,
    pub label_zh: String,
    pub label_en: String,
    pub category: EventCategory,
    pub phase: String,
    pub supports_matcher: bool,
    pub matcher_target: Option<String>,
    pub input_fields: Vec<InputField>,
    pub sensitivity: Sensitivity,
    pub high_frequency: bool,
    pub neutral_output: NeutralOutput,
    pub status: CapabilityStatus,
    pub min_verified_version: Version,
    pub max_verified_version: Version,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct InputField {
    pub name: String,
    pub sensitivity: Sensitivity,
    pub persist_by_default: bool,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Sensitivity {
    Public,
    Sensitive,
    Forbidden,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum NeutralOutput {
    Empty,
    EmptyObject,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CapabilityStatus {
    Stable,
    Experimental,
    Deprecated,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CatalogVerification {
    Exact,
    CompatibleUnverified,
    UpgradeRequired,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CapabilityResolution {
    pub catalog: CapabilityCatalog,
    pub verification: CatalogVerification,
}

pub fn catalog_for(agent: AgentKind, version: &Version) -> CapabilityResolution {
    let catalogs = embedded_catalogs();

    if let Some(catalog) = catalogs
        .iter()
        .find(|catalog| catalog.agent == agent && catalog.verified_version == *version)
    {
        return CapabilityResolution {
            catalog: catalog.clone(),
            verification: CatalogVerification::Exact,
        };
    }

    // The fixed catalog contract encodes its compatibility line as verified major.minor.
    let same_line = catalogs
        .iter()
        .filter(|catalog| {
            catalog.agent == agent
                && catalog.verified_version.major == version.major
                && catalog.verified_version.minor == version.minor
        })
        .collect::<Vec<_>>();
    let compatible = same_line
        .iter()
        .copied()
        .filter(|catalog| catalog.verified_version <= *version)
        .max_by_key(|catalog| &catalog.verified_version)
        .or_else(|| (same_line.len() == 1).then(|| same_line[0]));

    if let Some(catalog) = compatible {
        return CapabilityResolution {
            catalog: catalog.clone(),
            verification: CatalogVerification::CompatibleUnverified,
        };
    }

    let latest = catalogs
        .iter()
        .filter(|catalog| catalog.agent == agent)
        .max_by_key(|catalog| &catalog.verified_version)
        .expect("every supported agent has an embedded capability catalog");
    let hooks = SAFE_EVENTS
        .iter()
        .map(|event| {
            latest
                .hooks
                .iter()
                .find(|hook| hook.source_event == *event)
                .expect("safe events exist in every embedded catalog")
                .clone()
        })
        .collect();

    CapabilityResolution {
        catalog: CapabilityCatalog {
            agent,
            verified_version: latest.verified_version.clone(),
            hooks,
        },
        verification: CatalogVerification::UpgradeRequired,
    }
}

fn embedded_catalogs() -> [CapabilityCatalog; 2] {
    [
        serde_json::from_str(CLAUDE_CODE_CATALOG)
            .expect("embedded Claude Code capability catalog is valid"),
        serde_json::from_str(CODEX_CATALOG).expect("embedded Codex capability catalog is valid"),
    ]
}

#[cfg(test)]
mod tests {
    use semver::Version;

    use super::{
        CapabilityCatalog, CapabilityStatus, CatalogVerification, HookCapability, NeutralOutput,
        Sensitivity, catalog_for,
    };
    use crate::model::AgentKind;

    #[test]
    fn verified_codex_catalog_has_exact_lifecycle_events() {
        let result = catalog_for(AgentKind::Codex, &Version::parse("0.145.0").unwrap());

        assert_eq!(result.verification, CatalogVerification::Exact);
        assert_eq!(
            event_names(&result.catalog),
            vec![
                "SessionStart",
                "SessionEnd",
                "UserPromptSubmit",
                "PreToolUse",
                "PostToolUse",
                "PermissionRequest",
                "PreCompact",
                "PostCompact",
                "SubagentStart",
                "SubagentStop",
                "Stop",
            ]
        );
    }

    #[test]
    fn verified_claude_catalog_has_all_runtime_events() {
        let result = catalog_for(AgentKind::ClaudeCode, &Version::parse("2.1.218").unwrap());

        assert_eq!(result.verification, CatalogVerification::Exact);
        assert_eq!(
            event_names(&result.catalog),
            vec![
                "SessionStart",
                "SessionEnd",
                "Setup",
                "UserPromptSubmit",
                "UserPromptExpansion",
                "PreToolUse",
                "PostToolUse",
                "PostToolUseFailure",
                "PostToolBatch",
                "PermissionRequest",
                "PermissionDenied",
                "Notification",
                "Stop",
                "StopFailure",
                "SubagentStart",
                "SubagentStop",
                "PreCompact",
                "PostCompact",
                "Elicitation",
                "ElicitationResult",
                "TaskCreated",
                "TaskCompleted",
                "TeammateIdle",
                "ConfigChange",
                "InstructionsLoaded",
                "WorktreeCreate",
                "WorktreeRemove",
                "CwdChanged",
                "FileChanged",
                "MessageDisplay",
            ]
        );
    }

    #[test]
    fn newer_patch_in_declared_line_uses_compatible_catalog() {
        let result = catalog_for(AgentKind::ClaudeCode, &Version::parse("2.1.219").unwrap());

        assert_eq!(
            result.verification,
            CatalogVerification::CompatibleUnverified
        );
        assert_eq!(result.catalog.verified_version, Version::new(2, 1, 218));
    }

    #[test]
    fn unknown_version_line_uses_only_safe_subset() {
        let result = catalog_for(AgentKind::Codex, &Version::parse("0.146.0").unwrap());

        assert_eq!(result.verification, CatalogVerification::UpgradeRequired);
        assert_eq!(
            event_names(&result.catalog),
            vec!["SessionStart", "SessionEnd", "PermissionRequest", "Stop",]
        );
    }

    #[test]
    fn embedded_catalogs_round_trip_through_json() {
        for agent in [AgentKind::ClaudeCode, AgentKind::Codex] {
            let version = match agent {
                AgentKind::ClaudeCode => Version::new(2, 1, 218),
                AgentKind::Codex => Version::new(0, 145, 0),
            };
            let catalog = catalog_for(agent, &version).catalog;

            let json = serde_json::to_string(&catalog).unwrap();
            let decoded: CapabilityCatalog = serde_json::from_str(&json).unwrap();

            assert_eq!(decoded, catalog);
        }
    }

    #[test]
    fn verified_codex_catalog_declares_complete_matcher_and_capture_metadata() {
        type ExpectedHook = (
            &'static str,
            Option<&'static str>,
            &'static [(&'static str, Sensitivity)],
        );

        let catalog = catalog_for(AgentKind::Codex, &Version::new(0, 145, 0)).catalog;
        let cases: [ExpectedHook; 11] = [
            (
                "SessionStart",
                Some("source"),
                &[
                    ("session_id", Sensitivity::Sensitive),
                    ("transcript_path", Sensitivity::Forbidden),
                    ("cwd", Sensitivity::Sensitive),
                    ("hook_event_name", Sensitivity::Public),
                    ("model", Sensitivity::Public),
                    ("permission_mode", Sensitivity::Public),
                    ("source", Sensitivity::Public),
                ],
            ),
            (
                "SessionEnd",
                Some("reason"),
                &[
                    ("session_id", Sensitivity::Sensitive),
                    ("transcript_path", Sensitivity::Forbidden),
                    ("cwd", Sensitivity::Sensitive),
                    ("hook_event_name", Sensitivity::Public),
                    ("model", Sensitivity::Public),
                    ("reason", Sensitivity::Public),
                ],
            ),
            (
                "UserPromptSubmit",
                None,
                &[
                    ("session_id", Sensitivity::Sensitive),
                    ("transcript_path", Sensitivity::Forbidden),
                    ("cwd", Sensitivity::Sensitive),
                    ("hook_event_name", Sensitivity::Public),
                    ("model", Sensitivity::Public),
                    ("permission_mode", Sensitivity::Public),
                    ("turn_id", Sensitivity::Sensitive),
                    ("prompt", Sensitivity::Sensitive),
                ],
            ),
            (
                "PreToolUse",
                Some("tool_name"),
                &[
                    ("session_id", Sensitivity::Sensitive),
                    ("transcript_path", Sensitivity::Forbidden),
                    ("cwd", Sensitivity::Sensitive),
                    ("hook_event_name", Sensitivity::Public),
                    ("model", Sensitivity::Public),
                    ("permission_mode", Sensitivity::Public),
                    ("turn_id", Sensitivity::Sensitive),
                    ("tool_name", Sensitivity::Public),
                    ("tool_use_id", Sensitivity::Sensitive),
                    ("tool_input", Sensitivity::Sensitive),
                ],
            ),
            (
                "PostToolUse",
                Some("tool_name"),
                &[
                    ("session_id", Sensitivity::Sensitive),
                    ("transcript_path", Sensitivity::Forbidden),
                    ("cwd", Sensitivity::Sensitive),
                    ("hook_event_name", Sensitivity::Public),
                    ("model", Sensitivity::Public),
                    ("permission_mode", Sensitivity::Public),
                    ("turn_id", Sensitivity::Sensitive),
                    ("tool_name", Sensitivity::Public),
                    ("tool_use_id", Sensitivity::Sensitive),
                    ("tool_input", Sensitivity::Sensitive),
                    ("tool_response", Sensitivity::Sensitive),
                ],
            ),
            (
                "PermissionRequest",
                Some("tool_name"),
                &[
                    ("session_id", Sensitivity::Sensitive),
                    ("transcript_path", Sensitivity::Forbidden),
                    ("cwd", Sensitivity::Sensitive),
                    ("hook_event_name", Sensitivity::Public),
                    ("model", Sensitivity::Public),
                    ("permission_mode", Sensitivity::Public),
                    ("turn_id", Sensitivity::Sensitive),
                    ("tool_name", Sensitivity::Public),
                    ("tool_input", Sensitivity::Sensitive),
                ],
            ),
            (
                "PreCompact",
                Some("trigger"),
                &[
                    ("session_id", Sensitivity::Sensitive),
                    ("transcript_path", Sensitivity::Forbidden),
                    ("cwd", Sensitivity::Sensitive),
                    ("hook_event_name", Sensitivity::Public),
                    ("model", Sensitivity::Public),
                    ("turn_id", Sensitivity::Sensitive),
                    ("trigger", Sensitivity::Public),
                ],
            ),
            (
                "PostCompact",
                Some("trigger"),
                &[
                    ("session_id", Sensitivity::Sensitive),
                    ("transcript_path", Sensitivity::Forbidden),
                    ("cwd", Sensitivity::Sensitive),
                    ("hook_event_name", Sensitivity::Public),
                    ("model", Sensitivity::Public),
                    ("turn_id", Sensitivity::Sensitive),
                    ("trigger", Sensitivity::Public),
                ],
            ),
            (
                "SubagentStart",
                Some("agent_type"),
                &[
                    ("session_id", Sensitivity::Sensitive),
                    ("transcript_path", Sensitivity::Forbidden),
                    ("cwd", Sensitivity::Sensitive),
                    ("hook_event_name", Sensitivity::Public),
                    ("model", Sensitivity::Public),
                    ("permission_mode", Sensitivity::Public),
                    ("turn_id", Sensitivity::Sensitive),
                    ("agent_id", Sensitivity::Sensitive),
                    ("agent_type", Sensitivity::Public),
                ],
            ),
            (
                "SubagentStop",
                Some("agent_type"),
                &[
                    ("session_id", Sensitivity::Sensitive),
                    ("transcript_path", Sensitivity::Forbidden),
                    ("cwd", Sensitivity::Sensitive),
                    ("hook_event_name", Sensitivity::Public),
                    ("model", Sensitivity::Public),
                    ("permission_mode", Sensitivity::Public),
                    ("turn_id", Sensitivity::Sensitive),
                    ("agent_id", Sensitivity::Sensitive),
                    ("agent_type", Sensitivity::Public),
                    ("agent_transcript_path", Sensitivity::Forbidden),
                    ("stop_hook_active", Sensitivity::Public),
                    ("last_assistant_message", Sensitivity::Sensitive),
                ],
            ),
            (
                "Stop",
                None,
                &[
                    ("session_id", Sensitivity::Sensitive),
                    ("transcript_path", Sensitivity::Forbidden),
                    ("cwd", Sensitivity::Sensitive),
                    ("hook_event_name", Sensitivity::Public),
                    ("model", Sensitivity::Public),
                    ("permission_mode", Sensitivity::Public),
                    ("turn_id", Sensitivity::Sensitive),
                    ("stop_hook_active", Sensitivity::Public),
                    ("last_assistant_message", Sensitivity::Sensitive),
                ],
            ),
        ];

        for (source_event, matcher_target, fields) in cases {
            let hook = hook(&catalog, source_event);

            assert_eq!(
                hook.supports_matcher,
                matcher_target.is_some(),
                "{source_event}"
            );
            assert_eq!(
                hook.matcher_target.as_deref(),
                matcher_target,
                "{source_event}"
            );
            assert_eq!(hook.input_fields.len(), fields.len(), "{source_event}");
            for (actual, (name, sensitivity)) in hook.input_fields.iter().zip(fields) {
                assert_eq!(actual.name, *name, "{source_event}");
                assert_eq!(actual.sensitivity, *sensitivity, "{source_event}.{name}");
                assert!(!actual.persist_by_default, "{source_event}.{name}");
            }
        }
    }

    #[test]
    fn verified_claude_capture_metadata_supports_task_three() {
        let catalog = catalog_for(AgentKind::ClaudeCode, &Version::new(2, 1, 218)).catalog;
        let permission = hook(&catalog, "PermissionRequest");
        let stop = hook(&catalog, "Stop");

        assert_eq!(permission.matcher_target.as_deref(), Some("tool_name"));
        assert!(permission.supports_matcher);
        assert_eq!(
            field_names(permission),
            vec![
                "session_id",
                "transcript_path",
                "cwd",
                "permission_mode",
                "hook_event_name",
                "tool_name",
                "tool_input",
                "permission_suggestions",
            ]
        );
        assert_eq!(
            field(permission, "transcript_path").sensitivity,
            Sensitivity::Forbidden
        );
        assert_eq!(
            field(permission, "tool_input").sensitivity,
            Sensitivity::Sensitive
        );
        assert!(!stop.supports_matcher);
        assert_eq!(
            field_names(stop),
            vec![
                "session_id",
                "transcript_path",
                "cwd",
                "permission_mode",
                "hook_event_name",
                "stop_hook_active",
                "last_assistant_message",
            ]
        );
    }

    #[test]
    fn runtime_catalogs_keep_unverified_metadata_conservative() {
        for (agent, version) in [
            (AgentKind::ClaudeCode, Version::new(2, 1, 218)),
            (AgentKind::Codex, Version::new(0, 145, 0)),
        ] {
            let catalog = catalog_for(agent, &version).catalog;

            assert!(catalog.hooks.iter().all(|hook| {
                hook.status == CapabilityStatus::Stable
                    && !hook.high_frequency
                    && hook.neutral_output == NeutralOutput::Empty
            }));
        }
    }

    fn event_names(catalog: &CapabilityCatalog) -> Vec<&str> {
        catalog
            .hooks
            .iter()
            .map(|hook| hook.source_event.as_str())
            .collect()
    }

    fn hook<'a>(catalog: &'a CapabilityCatalog, source_event: &str) -> &'a HookCapability {
        catalog
            .hooks
            .iter()
            .find(|hook| hook.source_event == source_event)
            .unwrap()
    }

    fn field_names(hook: &HookCapability) -> Vec<&str> {
        hook.input_fields
            .iter()
            .map(|field| field.name.as_str())
            .collect()
    }

    fn field<'a>(hook: &'a HookCapability, name: &str) -> &'a super::InputField {
        hook.input_fields
            .iter()
            .find(|field| field.name == name)
            .unwrap()
    }
}
