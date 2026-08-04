use crate::events::{
    catalog::catalogued_hooks,
    normalize::{NormalizeContext, SafeIngressEvent, capture_hook_json, normalize_event},
};
use crate::ipc::protocol::{MAX_HOOK_BYTES, MAX_JSON_DEPTH, MAX_JSON_FIELDS, MAX_JSON_NODES};
use crate::model::AgentKind;
use crate::paths::{AgentVersionCacheFile, AppPaths};
use crate::security::permissions::ensure_private_file;
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
    let args: Vec<String> = std::env::args().collect();
    let mut owner = None;
    let mut agent = None;
    let mut event = None;
    let mut i = 1;
    while i + 1 < args.len() {
        match args[i].as_str() {
            "--owner" => owner = Some(args[i + 1].clone()),
            "--agent" => agent = Some(args[i + 1].clone()),
            "--event" => event = Some(args[i + 1].clone()),
            _ => {}
        }
        i += 2;
    }
    if owner.as_deref() != Some("cc-reminder") {
        return Err("owner".into());
    }
    let agent = match agent.as_deref() {
        Some("codex") => AgentKind::Codex,
        Some("claude-code") => AgentKind::ClaudeCode,
        _ => return Err("agent".into()),
    };
    let event = event.ok_or("event")?;
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
    let key = load_key(&paths).ok();
    let Some(key) = key else {
        return Ok(());
    };
    let normalized = normalize_event(
        captured,
        &NormalizeContext {
            correlation_key: key,
            projects: Vec::new(),
            platform: if cfg!(windows) {
                crate::projects::PathPlatform::Windows
            } else {
                crate::projects::PathPlatform::Unix
            },
        },
    )
    .map_err(|_| "normalize")?;
    let safe = SafeIngressEvent {
        event_id: normalized.id,
        source: normalized.source,
        source_version: normalized.source_version,
        source_event: normalized.source_event,
        occurred_at: normalized.occurred_at,
        received_at: normalized.received_at,
        project_id: normalized.project_id,
        project_display_name: normalized.project_display_name,
        cwd_fingerprint: normalized.unmatched_cwd_fingerprint,
        session_ref: normalized.session_ref,
        turn_ref: normalized.turn_ref,
        public_fields: normalized.public_fields,
    };
    if let Ok(executable) = std::env::current_exe() {
        let command = canonical_hook_command(&executable, agent, &event);
        let request = crate::ipc::IngressRequest {
            protocol_version: crate::ipc::IPC_PROTOCOL_VERSION,
            helper_version: env!("CARGO_PKG_VERSION").to_owned(),
            command_fingerprint: command_fingerprint(&command),
            event: captured_for_ipc(&safe),
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

fn captured_for_ipc(safe: &SafeIngressEvent) -> crate::events::normalize::CapturedHookEvent {
    crate::events::normalize::CapturedHookEvent {
        source: safe.source,
        source_version: safe.source_version.clone(),
        source_event: safe.source_event.clone(),
        occurred_at: safe.occurred_at,
        cwd: None,
        session_id: None,
        turn_id: None,
        model: None,
        permission_mode: None,
        public_fields: safe.public_fields.clone(),
        sensitive_fields: Default::default(),
    }
}

fn insert_ingress(path: &std::path::Path, event: &SafeIngressEvent) -> Result<(), String> {
    let connection =
        crate::storage::db::Database::open_ingress_writer(path).map_err(|_| "database")?;
    connection.execute(
        "INSERT OR IGNORE INTO ingress_events(id, safe_envelope_json, received_at, state) VALUES (?1, ?2, ?3, 'pending')",
        rusqlite::params![event.event_id.to_string(), serde_json::to_string(event).map_err(|_| "database")?, event.received_at.to_rfc3339()],
    ).map_err(|_| "database")?;
    Ok(())
}

fn load_cached_version(agent: AgentKind) -> Result<Version, String> {
    let paths = AppPaths::discover().map_err(|_| "paths")?;
    let bytes = std::fs::read(&paths.agent_versions).map_err(|_| "cache")?;
    if bytes.len() > 16 * 1024 {
        return Err("cache".into());
    }
    let cache: AgentVersionCacheFile = serde_json::from_slice(&bytes).map_err(|_| "cache")?;
    cache
        .agents
        .get(&agent)
        .map(|v| v.version.clone())
        .ok_or("cache".into())
}
fn load_key(paths: &AppPaths) -> Result<[u8; 32], String> {
    if let Ok(bytes) = std::fs::read(&paths.correlation_key)
        && bytes.len() == 32
    {
        return bytes.try_into().map_err(|_| "key".into());
    }
    let mut key = [0; 32];
    rand::RngExt::fill(&mut rand::rng(), &mut key);
    if let Some(parent) = paths.correlation_key.parent() {
        std::fs::create_dir_all(parent).map_err(|_| "key")?;
    }
    ensure_private_file(&paths.correlation_key).map_err(|_| "key")?;
    std::fs::write(&paths.correlation_key, key).map_err(|_| "key")?;
    Ok(key)
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
