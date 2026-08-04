use crate::events::{
    catalog::catalogued_hooks,
    normalize::{SafeIngressEvent, capture_hook_json, normalize_safe_ingress},
};
use crate::ipc::protocol::{
    MAX_HOOK_BYTES, MAX_JSON_DEPTH, MAX_JSON_FIELDS, MAX_JSON_NODES, MAX_SAFE_ENVELOPE_BYTES,
};
use crate::model::{AgentKind, ProjectMatchCacheFile};
use crate::paths::{AgentVersionCacheFile, AppPaths};
use crate::projects::ProjectRegistration;
use crate::security::crypto::CorrelationKey;
use crate::security::permissions::validate_private_file;
use semver::Version;
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::io::{Read, Write};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HookCommand {
    pub command: String,
    pub command_windows: Option<String>,
}

pub fn canonical_hook_command(
    path: &std::path::Path,
    agent: AgentKind,
    event: &str,
) -> HookCommand {
    let args = [
        "--owner",
        "cc-reminder",
        "--agent",
        agent.as_str(),
        "--event",
        event,
    ];
    HookCommand {
        command: std::iter::once(posix_quote(&path.to_string_lossy()))
            .chain(args.iter().map(|a| posix_quote(a)))
            .collect::<Vec<_>>()
            .join(" "),
        command_windows: Some(
            std::iter::once(windows_quote(&path.to_string_lossy()))
                .chain(args.iter().map(|a| windows_quote(a)))
                .collect::<Vec<_>>()
                .join(" "),
        ),
    }
}
pub fn command_fingerprint(command: &HookCommand) -> String {
    let mut b = Vec::new();
    for s in [
        &command.command,
        command.command_windows.as_deref().unwrap_or(""),
    ] {
        b.extend_from_slice(&(s.len() as u32).to_be_bytes());
        b.extend_from_slice(s.as_bytes());
    }
    hex::encode(Sha256::digest(b))
}
fn posix_quote(v: &str) -> String {
    format!("'{}'", v.replace('\'', "'\\''"))
}
fn windows_quote(v: &str) -> String {
    if v.is_empty() || v.chars().any(char::is_whitespace) || v.contains(['&', '|', '<', '>', '^']) {
        format!("\"{}\"", v.replace('"', "\\\""))
    } else {
        v.into()
    }
}

pub fn run_helper() {
    let result = process();
    let _ = result;
    let _ = std::io::stdout().write_all(b"{}\n");
}

fn process() -> Result<(), String> {
    let mut args = std::env::args().skip(1);
    if args.next().as_deref() != Some("--owner")
        || args.next().as_deref() != Some("cc-reminder")
        || args.next().as_deref() != Some("--agent")
    {
        return Err("owner".into());
    }
    let agent = match args.next().as_deref() {
        Some("codex") => AgentKind::Codex,
        Some("claude-code") => AgentKind::ClaudeCode,
        _ => return Err("agent".into()),
    };
    if args.next().as_deref() != Some("--event") {
        return Err("event".into());
    }
    let event = args.next().ok_or("event")?;
    if args.next().is_some() {
        return Err("arguments".into());
    }
    if !catalogued_hooks().contains(&(agent, event.clone())) {
        return Err("event".into());
    }
    let mut input = Vec::new();
    std::io::stdin()
        .take((MAX_HOOK_BYTES + 1) as u64)
        .read_to_end(&mut input)
        .map_err(|_| "stdin")?;
    if input.len() > MAX_HOOK_BYTES {
        return Err("oversize".into());
    }
    let raw: Value = serde_json::from_slice(&input).map_err(|_| "json")?;
    let object = raw.as_object().ok_or("object")?;
    if let Some(name) = object.get("hook_event_name").and_then(Value::as_str)
        && name != event
    {
        return Err("event mismatch".into());
    }
    walk_limits(&raw)?;
    let version = object
        .get("source_version")
        .and_then(Value::as_str)
        .and_then(|v| Version::parse(v).ok())
        .or_else(|| {
            object
                .get("version")
                .and_then(Value::as_str)
                .and_then(|v| Version::parse(v).ok())
        })
        .or_else(|| load_cached_version(agent).ok());
    let Some(version) = version else {
        return Ok(());
    };
    let captured = capture_hook_json(agent, &event, version, raw).map_err(|_| "capture")?;
    let paths = AppPaths::discover().map_err(|_| "paths")?;
    let key = CorrelationKey::load_or_create(&paths.data_dir).ok();
    let projects = load_project_cache(&paths).unwrap_or_default();
    let platform = if cfg!(windows) {
        crate::projects::PathPlatform::Windows
    } else {
        crate::projects::PathPlatform::Unix
    };
    let safe = normalize_safe_ingress(
        captured.clone(),
        &projects,
        platform,
        key.as_ref().map(CorrelationKey::expose_for_hmac),
    );
    validate_safe_envelope(&safe)?;
    if let Ok(executable) = std::env::current_exe() {
        let command = canonical_hook_command(&executable, agent, &event);
        let request = crate::ipc::IngressRequest {
            protocol_version: crate::ipc::IPC_PROTOCOL_VERSION,
            helper_version: env!("CARGO_PKG_VERSION").to_owned(),
            command_fingerprint: command_fingerprint(&command),
            event: captured,
        };
        if matches!(
            crate::ipc::send_ingress(&paths.endpoint(), &request),
            Ok(crate::ipc::IngressResponse::Accepted { .. })
        ) {
            return Ok(());
        }
    }
    if insert_ingress(&paths.database, &safe).is_ok() {
        return Ok(());
    }
    let spool = crate::storage::spool::Spool::new(paths.spool).map_err(|_| "spool")?;
    spool.write_exclusive(&safe).map_err(|_| "spool")?;
    Ok(())
}

pub(crate) fn insert_ingress(
    path: &std::path::Path,
    event: &SafeIngressEvent,
) -> Result<(), String> {
    let envelope = validate_safe_envelope(event)?;
    let connection =
        crate::storage::db::Database::open_ingress_writer(path).map_err(|_| "database")?;
    connection.execute(
        "INSERT OR IGNORE INTO ingress_events(id, safe_envelope_json, received_at, state) VALUES (?1, ?2, ?3, 'pending')",
        rusqlite::params![event.event_id.to_string(), envelope, event.received_at.to_rfc3339()],
    ).map_err(|_| "database")?;
    Ok(())
}

pub(crate) fn persist_ipc_request(
    paths: &AppPaths,
    request: crate::ipc::IngressRequest,
) -> Result<uuid::Uuid, String> {
    let key = CorrelationKey::load_or_create(&paths.data_dir).ok();
    let projects = load_project_cache(paths).unwrap_or_default();
    let platform = if cfg!(windows) {
        crate::projects::PathPlatform::Windows
    } else {
        crate::projects::PathPlatform::Unix
    };
    let safe = normalize_safe_ingress(
        request.event,
        &projects,
        platform,
        key.as_ref().map(CorrelationKey::expose_for_hmac),
    );
    insert_ingress(&paths.database, &safe)?;
    Ok(safe.event_id)
}

#[cfg(feature = "test-support")]
pub fn persist_ipc_request_for_test(
    paths: &AppPaths,
    request: crate::ipc::IngressRequest,
) -> Result<uuid::Uuid, String> {
    persist_ipc_request(paths, request)
}

fn load_cached_version(agent: AgentKind) -> Result<Version, String> {
    let paths = AppPaths::discover().map_err(|_| "paths")?;
    validate_private_file(&paths.agent_versions).map_err(|_| "cache")?;
    let bytes = read_bounded_file(&paths.agent_versions, 16 * 1024).map_err(|_| "cache")?;
    let cache: AgentVersionCacheFile = serde_json::from_slice(&bytes).map_err(|_| "cache")?;
    if cache.schema_version != 1 {
        return Err("cache".into());
    }
    cache
        .agents
        .get(&agent)
        .map(|v| v.version.clone())
        .ok_or("cache".into())
}

fn load_project_cache(paths: &AppPaths) -> Result<Vec<ProjectRegistration>, String> {
    const MAX_PROJECT_CACHE_BYTES: usize = 1_048_576;
    validate_private_file(&paths.project_paths).map_err(|_| "project_cache")?;
    let bytes = read_bounded_file(&paths.project_paths, MAX_PROJECT_CACHE_BYTES)
        .map_err(|_| "project_cache")?;
    let cache: ProjectMatchCacheFile =
        serde_json::from_slice(&bytes).map_err(|_| "project_cache")?;
    if cache.version != 1 || cache.projects.len() > 200 {
        return Err("project_cache".into());
    }
    cache
        .projects
        .into_iter()
        .map(|project| {
            if project.canonical_paths.is_empty() || project.canonical_paths.len() > 200 {
                return Err("project_cache".into());
            }
            let mut paths = project.canonical_paths.into_iter();
            Ok(ProjectRegistration {
                id: project.id,
                display_name: project.display_name,
                canonical_root: paths.next().expect("checked nonempty paths"),
                aliases: paths.collect(),
            })
        })
        .collect()
}

fn read_bounded_file(path: &std::path::Path, limit: usize) -> Result<Vec<u8>, String> {
    let mut bytes = Vec::new();
    std::fs::File::open(path)
        .map_err(|_| "bounded_read")?
        .take((limit + 1) as u64)
        .read_to_end(&mut bytes)
        .map_err(|_| "bounded_read")?;
    if bytes.len() > limit {
        return Err("bounded_read".into());
    }
    Ok(bytes)
}

fn validate_safe_envelope(event: &SafeIngressEvent) -> Result<String, String> {
    let envelope = serde_json::to_string(event).map_err(|_| "safe_envelope")?;
    if envelope.len() > MAX_SAFE_ENVELOPE_BYTES {
        return Err("safe_envelope_too_large".into());
    }
    Ok(envelope)
}
fn walk_limits(value: &Value) -> Result<(), String> {
    let mut stack = vec![(value, 0)];
    let mut nodes = 0;
    let mut fields = 0;
    while let Some((v, d)) = stack.pop() {
        nodes += 1;
        if nodes > MAX_JSON_NODES || d > MAX_JSON_DEPTH {
            return Err("limits".into());
        }
        match v {
            Value::Object(m) => {
                fields += m.len();
                if fields > MAX_JSON_FIELDS {
                    return Err("fields".into());
                }
                for child in m.values() {
                    stack.push((child, d + 1));
                }
            }
            Value::Array(a) => {
                for child in a {
                    stack.push((child, d + 1))
                }
            }
            _ => {}
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::path::Path;

    use chrono::Utc;
    use semver::Version;
    use tempfile::tempdir;
    use uuid::Uuid;

    use super::{canonical_hook_command, command_fingerprint, insert_ingress};
    use crate::events::normalize::SafeIngressEvent;
    use crate::ipc::protocol::MAX_SAFE_ENVELOPE_BYTES;
    use crate::model::{AgentKind, ScalarValue};
    use crate::storage::db::Database;

    #[test]
    fn canonical_commands_quote_metacharacters_without_changing_fingerprints() {
        let posix = canonical_hook_command(
            Path::new("/Users/a b/it's/bin/cc-reminder-hook"),
            AgentKind::Codex,
            "Stop",
        );
        assert!(
            posix
                .command
                .starts_with("'/Users/a b/it'\\''s/bin/cc-reminder-hook' ")
        );
        let windows = canonical_hook_command(
            Path::new(r"C:\Users\a & b\cc-reminder-hook.exe"),
            AgentKind::Codex,
            "Stop",
        );
        assert!(
            windows
                .command_windows
                .as_deref()
                .unwrap()
                .starts_with(r#""C:\Users\a & b\cc-reminder-hook.exe" "#)
        );
        assert_eq!(
            command_fingerprint(&windows),
            command_fingerprint(&canonical_hook_command(
                Path::new(r"C:\Users\a & b\cc-reminder-hook.exe"),
                AgentKind::Codex,
                "Stop",
            ))
        );
    }

    #[test]
    fn oversized_safe_envelope_never_reaches_sqlite() {
        let root = tempdir().unwrap();
        let database = root.path().join("com.ccreminder.app/cc-reminder.sqlite3");
        Database::open(&database).unwrap();
        let event = SafeIngressEvent {
            event_id: Uuid::now_v7(),
            source: AgentKind::Codex,
            source_version: Version::new(0, 145, 0),
            source_event: "Stop".into(),
            occurred_at: Utc::now(),
            received_at: Utc::now(),
            project_id: None,
            project_display_name: None,
            cwd_fingerprint: None,
            session_ref: None,
            turn_ref: None,
            public_fields: BTreeMap::from([(
                "summary".into(),
                ScalarValue::String("x".repeat(MAX_SAFE_ENVELOPE_BYTES)),
            )]),
        };

        assert_eq!(
            insert_ingress(&database, &event).unwrap_err(),
            "safe_envelope_too_large"
        );
        let connection = Database::open_ingress_writer(&database).unwrap();
        let count: i64 = connection
            .query_row("SELECT COUNT(*) FROM ingress_events", [], |row| row.get(0))
            .unwrap();
        assert_eq!(count, 0);
    }
}
