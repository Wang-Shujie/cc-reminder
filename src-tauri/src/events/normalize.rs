use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use hmac::{Hmac, Mac};
use semver::Version;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::Sha256;
use uuid::Uuid;

use crate::actions::new_v1_action_fields;
use crate::error::{AppError, ErrorDomain};
use crate::events::catalog::{Sensitivity, catalog_for};
use crate::model::{AgentKind, EventEnvelope, ProjectId, ScalarValue, Severity};
use crate::projects::{
    PathPlatform, ProjectMatch, ProjectRegistration, path_leaf, resolve_project,
};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CapturedHookEvent {
    pub source: AgentKind,
    pub source_version: Version,
    pub source_event: String,
    pub occurred_at: DateTime<Utc>,
    pub cwd: Option<PathBuf>,
    pub session_id: Option<String>,
    pub turn_id: Option<String>,
    pub model: Option<String>,
    pub permission_mode: Option<String>,
    pub public_fields: BTreeMap<String, ScalarValue>,
    pub sensitive_fields: BTreeMap<String, String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SafeIngressEvent {
    pub event_id: Uuid,
    pub source: AgentKind,
    pub source_version: Version,
    pub source_event: String,
    pub occurred_at: DateTime<Utc>,
    pub received_at: DateTime<Utc>,
    pub project_id: Option<ProjectId>,
    pub project_display_name: Option<String>,
    pub cwd_fingerprint: Option<String>,
    pub session_ref: Option<String>,
    pub turn_ref: Option<String>,
    pub public_fields: BTreeMap<String, ScalarValue>,
}

#[derive(Clone, Debug)]
pub struct NormalizeContext {
    pub correlation_key: [u8; 32],
    pub projects: Vec<ProjectRegistration>,
    pub platform: PathPlatform,
}

pub fn capture_hook_json(
    agent: AgentKind,
    event: &str,
    source_version: Version,
    raw: Value,
) -> Result<CapturedHookEvent, AppError> {
    let object = raw
        .as_object()
        .ok_or_else(|| invalid("hook payload must be an object"))?;
    let hook = catalog_for(agent, &source_version)
        .catalog
        .hooks
        .into_iter()
        .find(|hook| hook.source_event == event)
        .ok_or_else(|| invalid("hook event is not supported by the selected catalog"))?;
    let matcher_target = hook.matcher_target.clone();
    let phase = hook.phase.clone();
    let mut captured = CapturedHookEvent {
        source: agent,
        source_version,
        source_event: event.into(),
        occurred_at: Utc::now(),
        cwd: None,
        session_id: None,
        turn_id: None,
        model: None,
        permission_mode: None,
        public_fields: BTreeMap::new(),
        sensitive_fields: BTreeMap::new(),
    };

    for field in hook.input_fields {
        let Some(value) = object.get(&field.name) else {
            continue;
        };
        match field.sensitivity {
            Sensitivity::Forbidden => {}
            Sensitivity::Public => match field.name.as_str() {
                "model" => captured.model = string(value),
                "permission_mode" => captured.permission_mode = string(value),
                "hook_event_name" => {}
                _ => {
                    if let Some(value) = scalar(value) {
                        captured.public_fields.insert(field.name, value);
                    }
                }
            },
            Sensitivity::Sensitive => match field.name.as_str() {
                "cwd" => captured.cwd = string(value).map(PathBuf::from),
                "session_id" => captured.session_id = string(value),
                "turn_id" => captured.turn_id = string(value),
                _ => {
                    captured
                        .sensitive_fields
                        .insert(field.name, sensitive_string(value));
                }
            },
        }
    }

    if !captured.public_fields.contains_key("event_subtype")
        && let Some(ScalarValue::String(value)) = matcher_target
            .as_ref()
            .and_then(|target| captured.public_fields.get(target))
    {
        captured
            .public_fields
            .insert("event_subtype".into(), ScalarValue::String(value.clone()));
    }
    captured
        .public_fields
        .entry("status".into())
        .or_insert(ScalarValue::String(phase));

    Ok(captured)
}

pub fn normalize_event(
    event: CapturedHookEvent,
    context: &NormalizeContext,
) -> Result<EventEnvelope, AppError> {
    let category = catalog_for(event.source, &event.source_version)
        .catalog
        .hooks
        .into_iter()
        .find(|hook| hook.source_event == event.source_event)
        .map(|hook| hook.category)
        .ok_or_else(|| invalid("hook event is not supported by the selected catalog"))?;
    let project = event
        .cwd
        .as_deref()
        .map(|cwd| resolve_project(cwd, &context.projects, context.platform));
    let (project_id, project_display_name, unmatched_cwd_fingerprint) = project_fields(
        project,
        event.cwd.as_deref(),
        context.platform,
        &context.correlation_key,
    );
    let (correlation_id, action_id, action_capabilities) = new_v1_action_fields();

    Ok(EventEnvelope {
        id: Uuid::now_v7(),
        source: event.source,
        source_version: event.source_version,
        source_event: event.source_event,
        category,
        occurred_at: event.occurred_at,
        received_at: Utc::now(),
        project_id,
        project_display_name,
        unmatched_cwd_fingerprint,
        session_ref: event
            .session_id
            .as_deref()
            .map(|value| reference(&context.correlation_key, b"session", value)),
        turn_ref: event
            .turn_id
            .as_deref()
            .map(|value| reference(&context.correlation_key, b"turn", value)),
        model: event.model,
        permission_mode: event.permission_mode,
        severity: Severity::Info,
        public_fields: event.public_fields,
        encrypted_sensitive_fields: None,
        correlation_id,
        action_id,
        action_capabilities,
    })
}

fn project_fields(
    project: Option<ProjectMatch>,
    cwd: Option<&Path>,
    platform: PathPlatform,
    key: &[u8; 32],
) -> (Option<ProjectId>, Option<String>, Option<String>) {
    match project {
        Some(ProjectMatch::Matched {
            project_id,
            display_name,
        }) => (Some(project_id), Some(display_name), None),
        Some(ProjectMatch::Unmatched) | None => {
            let display_name = cwd.and_then(|path| path_leaf(path, platform));
            let fingerprint = cwd.map(|path| reference(key, b"cwd", &path.to_string_lossy()));
            (None, display_name, fingerprint)
        }
    }
}

fn reference(key: &[u8; 32], domain: &[u8], value: &str) -> String {
    let mut mac = Hmac::<Sha256>::new_from_slice(key).expect("HMAC accepts a 32-byte key");
    mac.update(domain);
    mac.update(&[0]);
    mac.update(value.as_bytes());
    hex::encode(mac.finalize().into_bytes())
}

fn scalar(value: &Value) -> Option<ScalarValue> {
    match value {
        Value::String(value) => Some(ScalarValue::String(value.clone())),
        Value::Number(value) => value.as_f64().map(ScalarValue::Number),
        Value::Bool(value) => Some(ScalarValue::Bool(*value)),
        Value::Null => Some(ScalarValue::Null),
        Value::Array(_) | Value::Object(_) => None,
    }
}

fn string(value: &Value) -> Option<String> {
    value.as_str().map(str::to_owned)
}

fn sensitive_string(value: &Value) -> String {
    match value {
        Value::String(value) => value.clone(),
        _ => value.to_string(),
    }
}

fn invalid(message: &str) -> AppError {
    AppError {
        domain: ErrorDomain::Integration,
        code: "invalid_hook_event".into(),
        message: message.into(),
        suggested_action: None,
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use semver::Version;
    use serde_json::Value;

    use super::{NormalizeContext, capture_hook_json, normalize_event};
    use crate::model::{AgentKind, EventCategory, ScalarValue};
    use crate::projects::{PathPlatform, ProjectRegistration};

    #[test]
    fn codex_permission_keeps_source_name_and_hmac_references() {
        let raw = fixture("codex/0.145.0/permission-request.json");
        let captured = capture_hook_json(
            AgentKind::Codex,
            "PermissionRequest",
            Version::new(0, 145, 0),
            raw,
        )
        .unwrap();
        let event = normalize_event(captured, &context_with_key([7_u8; 32])).unwrap();

        assert_eq!(event.source_event, "PermissionRequest");
        assert_eq!(event.category, EventCategory::Permission);
        assert_ne!(event.session_ref.as_deref(), Some("raw-session-id"));
        assert_ne!(event.turn_ref.as_deref(), Some("raw-turn-id"));
        assert_eq!(
            event.public_fields.get("tool_name"),
            Some(&ScalarValue::String("shell".into()))
        );
        assert!(!event.public_fields.contains_key("unknown_field"));
        assert!(event.action_id.is_none());
        assert!(event.action_capabilities.is_empty());
    }

    #[test]
    fn capture_discards_forbidden_fields_before_building_event() {
        let captured = capture_hook_json(
            AgentKind::Codex,
            "PermissionRequest",
            Version::new(0, 145, 0),
            fixture("codex/0.145.0/permission-request.json"),
        )
        .unwrap();

        assert_eq!(captured.session_id.as_deref(), Some("raw-session-id"));
        assert_eq!(
            captured.cwd.as_deref().unwrap().to_string_lossy(),
            "/workspace/demo-app"
        );
        assert!(captured.sensitive_fields.contains_key("tool_input"));
        assert!(!captured.sensitive_fields.contains_key("transcript_path"));
        assert!(!captured.public_fields.contains_key("unknown_field"));
    }

    #[test]
    fn capture_preserves_sensitive_string_value_without_json_quotes() {
        let captured = capture_hook_json(
            AgentKind::Codex,
            "Stop",
            Version::new(0, 145, 0),
            fixture("codex/0.145.0/stop.json"),
        )
        .unwrap();

        assert_eq!(
            captured
                .sensitive_fields
                .get("last_assistant_message")
                .map(String::as_str),
            Some("private completion text")
        );
    }

    #[test]
    fn unmatched_cwd_keeps_only_leaf_and_hmac_fingerprint() {
        let event = normalize_event(
            capture_hook_json(
                AgentKind::Codex,
                "PermissionRequest",
                Version::new(0, 145, 0),
                serde_json::json!({ "cwd": "/Users/alice/secret/client" }),
            )
            .unwrap(),
            &context_with_key([9_u8; 32]),
        )
        .unwrap();

        assert_eq!(event.project_display_name.as_deref(), Some("client"));
        assert!(
            !event
                .unmatched_cwd_fingerprint
                .as_deref()
                .unwrap()
                .contains("/Users/alice")
        );
    }

    #[test]
    fn unmatched_windows_cwd_preserves_leaf_case_without_exposing_the_full_path() {
        let event = normalize_event(
            capture_hook_json(
                AgentKind::Codex,
                "PermissionRequest",
                Version::new(0, 145, 0),
                serde_json::json!({ "cwd": "C:\\Work\\MyProject" }),
            )
            .unwrap(),
            &NormalizeContext {
                correlation_key: [11_u8; 32],
                projects: Vec::new(),
                platform: PathPlatform::Windows,
            },
        )
        .unwrap();

        assert_eq!(event.project_display_name.as_deref(), Some("MyProject"));
        assert!(!event.project_display_name.unwrap().contains("Work"));
    }

    fn context_with_key(key: [u8; 32]) -> NormalizeContext {
        NormalizeContext {
            correlation_key: key,
            projects: Vec::<ProjectRegistration>::new(),
            platform: PathPlatform::Unix,
        }
    }

    fn fixture(path: &str) -> Value {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../tests/fixtures")
            .join(path);
        serde_json::from_str(&fs::read_to_string(path).unwrap()).unwrap()
    }
}
