//! Structured patching of Agent hook configuration (Task 10).
//!
//! Produces minimal, AST-driven text patches of Claude Code's JSONC
//! `settings.json` and Codex's JSON `hooks.json` plus checked atomic
//! replacement. Both Agents share the same documented hook shape — a top-level
//! object whose `hooks` property maps an event name to an array of matcher
//! groups, each group holding a `hooks` array of command handlers:
//!
//! ```jsonc
//! { "hooks": { "Stop": [ { "matcher": "", "hooks": [ { "type": "command", "command": "...", "timeout": 1 } ] } ] } }
//! ```
//!
//! Codex additionally documents a `commandWindows` override on the handler;
//! Claude does not, so it is omitted from Claude output. Ownership of an
//! installed entry is recognised only by the triad canonical helper path +
//! exact `--owner cc-reminder` marker + SHA-256 command fingerprint (design
//! 9.2/9.3/9.4). No brace or key is ever located via regex or substring search.

pub mod atomic;
mod jsonc;

use std::collections::BTreeMap;
use std::path::Path;

use jsonc_parser::ast::Value;
use serde_json::json;

use crate::error::{AppError, ErrorDomain};
use crate::events::catalog::catalogued_hooks;
use crate::hook_command::{HookCommand, canonical_hook_command, command_fingerprint};
use crate::model::AgentKind;

pub use atomic::atomic_replace_checked;

/// One hook entry CC Reminder owns inside an Agent's configuration.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OwnedHookEntry {
    pub source_event: String,
    pub matcher: Option<String>,
    pub command: HookCommand,
    pub timeout_seconds: u8,
}

/// Result of producing a minimal patch for an Agent config file.
///
/// `before_hooks_subtree` holds the raw source bytes of the top-level `hooks`
/// object before patching (a verbatim rollback snapshot; empty when `hooks` was
/// absent), and the two hashes bracket the change so callers can detect drift
/// between inspection and replacement.
#[derive(Clone, Debug)]
pub struct ConfigPatch {
    pub bytes: Vec<u8>,
    pub before_hooks_subtree: Vec<u8>,
    pub before_hash: String,
    pub after_hash: String,
}

/// Canonical helper invocation for `agent` / `event`. Bakes in the stable
/// `--owner cc-reminder --agent <a> --event <e>` marker and emits both the
/// posix `command` and the Windows `commandWindows` override.
pub fn owned_command(helper: &Path, agent: AgentKind, event: &str) -> HookCommand {
    canonical_hook_command(helper, agent, event)
}

/// Lowercase SHA-256 over `len || source_event || len || canonical-definition-json`.
/// Kept separate from [`command_fingerprint`]: relocating the helper binary at
/// the same path changes neither, while any emitted definition field change
/// (matcher, timeout, commandWindows presence) changes this fingerprint.
pub fn hook_definition_fingerprint(agent: AgentKind, entry: &OwnedHookEntry) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    let event = entry.source_event.as_bytes();
    hasher.update((event.len() as u32).to_be_bytes());
    hasher.update(event);
    let definition = definition_value(agent, entry);
    let definition_bytes = serde_json::to_vec(&definition).expect("definition serializable");
    hasher.update((definition_bytes.len() as u32).to_be_bytes());
    hasher.update(definition_bytes);
    hex::encode(hasher.finalize())
}

/// SHA-256 hex of an arbitrary byte buffer. Exposed so callers can hash the
/// on-disk bytes between inspection and replacement (design 9.4.2/9.4.3).
pub fn sha256_hex(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    hex::encode(Sha256::digest(bytes))
}

/// Patch Claude Code's user-level JSONC `settings.json`.
pub fn patch_claude_settings(
    source: &[u8],
    desired: &[OwnedHookEntry],
) -> Result<ConfigPatch, AppError> {
    patch_agent(AgentKind::ClaudeCode, source, desired)
}

/// Patch Codex's user-level JSON `~/.codex/hooks.json`.
pub fn patch_codex_hooks(
    source: &[u8],
    desired: &[OwnedHookEntry],
) -> Result<ConfigPatch, AppError> {
    patch_agent(AgentKind::Codex, source, desired)
}

/// Return every CC-reminder-owned entry found in `source` for `agent`.
/// Recognition is structural (triad) and self-contained: it does not need to be
/// told the helper path ahead of time.
pub fn inspect_owned_entries(
    agent: AgentKind,
    source: &[u8],
) -> Result<Vec<OwnedHookEntry>, AppError> {
    let text = std::str::from_utf8(source).map_err(|_| invalid_config("non-utf8 config".into()))?;
    let root = jsonc::parse(text)?;
    let Some(hooks_obj) = jsonc::hooks_object(&root) else {
        return Ok(Vec::new());
    };
    let mut out = Vec::new();
    for prop in &hooks_obj.properties {
        let event_name = prop.name.as_str().to_owned();
        let Some(array) = prop.value.as_array() else {
            continue;
        };
        for element in &array.elements {
            let Some(group) = element.as_object() else {
                continue;
            };
            let matcher = group
                .get("matcher")
                .and_then(|p| p.value.as_string_lit())
                .map(|lit| lit.value.to_string());
            let Some(handlers) = group.get("hooks").and_then(|p| p.value.as_array()) else {
                continue;
            };
            for handler in &handlers.elements {
                let Some(handler_obj) = handler.as_object() else {
                    continue;
                };
                let Some(command) = handler_obj
                    .get("command")
                    .and_then(|p| p.value.as_string_lit())
                    .map(|lit| lit.value.to_string())
                else {
                    continue;
                };
                let command_windows = handler_obj
                    .get("commandWindows")
                    .and_then(|p| p.value.as_string_lit())
                    .map(|lit| lit.value.to_string());
                let timeout = handler_obj
                    .get("timeout")
                    .and_then(|p| p.value.as_number_lit())
                    .and_then(|lit| lit.value.parse::<u8>().ok())
                    .unwrap_or(1);
                let Some(recognized) = recognize_owned(agent, &command, command_windows.as_deref())
                else {
                    continue;
                };
                if recognized != event_name {
                    continue;
                }
                out.push(OwnedHookEntry {
                    source_event: event_name.clone(),
                    matcher: matcher.clone(),
                    command: HookCommand {
                        command,
                        command_windows,
                    },
                    timeout_seconds: timeout,
                });
            }
        }
    }
    Ok(out)
}

// ---- core patch construction -----------------------------------------------

fn patch_agent(
    agent: AgentKind,
    source: &[u8],
    desired: &[OwnedHookEntry],
) -> Result<ConfigPatch, AppError> {
    let text = std::str::from_utf8(source).map_err(|_| invalid_config("non-utf8 config".into()))?;
    let before_hash = sha256_hex(source);
    let before_hooks_subtree = match jsonc::parse(text) {
        Ok(root) => hooks_subtree_bytes(text, &root),
        Err(_) => Vec::new(),
    };

    let result = build(agent, text, desired)?;
    jsonc::validate(&result)?;
    let result_bytes = result.into_bytes();
    let after_hash = sha256_hex(&result_bytes);
    Ok(ConfigPatch {
        bytes: result_bytes,
        before_hooks_subtree,
        before_hash,
        after_hash,
    })
}

/// Raw source bytes of the top-level `hooks` object before patching — a
/// verbatim rollback snapshot. Empty when `hooks` is absent or unparseable.
fn hooks_subtree_bytes(text: &str, root: &Value) -> Vec<u8> {
    jsonc::hooks_object(root)
        .map(|object| text.as_bytes()[object.range.start..object.range.end].to_vec())
        .unwrap_or_default()
}

fn build(agent: AgentKind, source: &str, desired: &[OwnedHookEntry]) -> Result<String, AppError> {
    let catalogued: Vec<String> = catalogued_hooks()
        .into_iter()
        .filter(|(a, _)| *a == agent)
        .map(|(_, event)| event)
        .collect();

    // One entry per event; last wins. Validate every event is catalogued.
    let mut desired_map: BTreeMap<String, &OwnedHookEntry> = BTreeMap::new();
    for entry in desired {
        if !catalogued.contains(&entry.source_event) {
            return Err(invalid_config(format!(
                "event {} is not catalogued for agent {}",
                entry.source_event,
                agent.as_str()
            )));
        }
        desired_map.insert(entry.source_event.clone(), entry);
    }

    let root = jsonc::parse(source)?;
    let Some(root_obj) = root.as_object() else {
        // Scalar or empty root: synthesise a fresh document.
        return Ok(synthesise_fresh(agent, &desired_map, source));
    };

    let hooks_prop = root_obj.get("hooks");
    let hooks_obj = hooks_prop.and_then(|prop| prop.value.as_object());
    if hooks_prop.is_some() && hooks_obj.is_none() {
        return Err(invalid_config("top-level `hooks` is not an object".into()));
    }

    let mut edits: Vec<Edit> = Vec::new();

    if let Some(hooks_obj) = hooks_obj {
        // First pass: plan edits for events that already have an array.
        let mut handled_events: Vec<String> = Vec::new();
        for prop in &hooks_obj.properties {
            let event = prop.name.as_str().to_owned();
            let Some(array) = prop.value.as_array() else {
                continue;
            };
            let owned_indices: Vec<usize> = array
                .elements
                .iter()
                .enumerate()
                .filter(|(_, element)| element_is_owned(agent, element))
                .map(|(index, _)| index)
                .collect();

            if let Some(entry) = desired_map.get(&event) {
                let group_text = serialise_owned_group(agent, entry);
                if let Some(&first) = owned_indices.first() {
                    let range = jsonc::value_range(&array.elements[first]);
                    edits.push(Edit::replace(range.start, range.end, group_text));
                    for &index in owned_indices.iter().skip(1).rev() {
                        let (start, end) = remove_element_span(&array.elements, index, source);
                        edits.push(Edit::delete(start, end));
                    }
                } else {
                    let (pos, text) = insert_array_append(array, source, &group_text);
                    edits.push(Edit::insert(pos, text));
                }
                handled_events.push(event);
            } else {
                for &index in owned_indices.iter().rev() {
                    let (start, end) = remove_element_span(&array.elements, index, source);
                    edits.push(Edit::delete(start, end));
                }
            }
        }
        for event in handled_events {
            desired_map.remove(&event);
        }

        // Second pass: remaining desired events get a brand-new property.
        for (event, entry) in &desired_map {
            let group_text = serialise_owned_group(agent, entry);
            let prop_text = format!("\"{event}\": [{group_text}]");
            let (pos, text) = insert_object_property(hooks_obj, source, &prop_text);
            edits.push(Edit::insert(pos, text));
        }
    } else {
        // No hooks object at all: inject one onto the root.
        let hooks_body = build_hooks_body(agent, &desired_map);
        let prop_text = format!("\"hooks\": {hooks_body}");
        let (pos, text) = insert_object_property(root_obj, source, &prop_text);
        edits.push(Edit::insert(pos, text));
    }

    Ok(apply_edits(source, &edits))
}

fn synthesise_fresh(
    agent: AgentKind,
    desired: &BTreeMap<String, &OwnedHookEntry>,
    source: &str,
) -> String {
    let newline = detect_newline(source.as_bytes());
    let hooks_body = build_hooks_body(agent, desired);
    if desired.is_empty() {
        format!("{{{newline}}}")
    } else {
        format!("{{{newline}  \"hooks\": {hooks_body}{newline}}}")
    }
}

fn build_hooks_body(agent: AgentKind, desired: &BTreeMap<String, &OwnedHookEntry>) -> String {
    if desired.is_empty() {
        return "{}".to_owned();
    }
    let mut body = String::from("{\n");
    let last = desired.len();
    for (index, (event, entry)) in desired.iter().enumerate() {
        let group_text = serialise_owned_group(agent, entry);
        body.push_str(&format!("    \"{event}\": [{group_text}]"));
        if index + 1 < last {
            body.push(',');
        }
        body.push('\n');
    }
    body.push_str("  }");
    body
}

/// Serialise one owned matcher-group object as compact JSON.
fn serialise_owned_group(agent: AgentKind, entry: &OwnedHookEntry) -> String {
    let mut handler = serde_json::Map::new();
    handler.insert("type".to_owned(), json!("command"));
    handler.insert("command".to_owned(), json!(entry.command.command));
    if agent == AgentKind::Codex
        && let Some(command_windows) = &entry.command.command_windows
    {
        handler.insert("commandWindows".to_owned(), json!(command_windows));
    }
    handler.insert("timeout".to_owned(), json!(entry.timeout_seconds));
    let group = json!({
        "matcher": entry.matcher.clone().unwrap_or_default(),
        "hooks": [serde_json::Value::Object(handler)],
    });
    serde_json::to_string(&group).expect("group serialisable")
}

fn definition_value(agent: AgentKind, entry: &OwnedHookEntry) -> serde_json::Value {
    let mut map = serde_json::Map::new();
    map.insert(
        "matcher".to_owned(),
        json!(entry.matcher.clone().unwrap_or_default()),
    );
    map.insert("command".to_owned(), json!(entry.command.command));
    if agent == AgentKind::Codex
        && let Some(command_windows) = &entry.command.command_windows
    {
        map.insert("commandWindows".to_owned(), json!(command_windows));
    }
    map.insert("timeout".to_owned(), json!(entry.timeout_seconds));
    serde_json::Value::Object(map)
}

// ---- ownership recognition (triad) -----------------------------------------

/// Returns the recognised event when `command` (and optional `command_windows`)
/// exactly reproduce a canonical CC-reminder invocation. Requires the owner
/// marker AND a SHA-256 fingerprint match against the reconstructed canonical
/// command — a foreign lookalike with the marker but a different command fails
/// the fingerprint check and is left alone.
fn recognize_owned(
    agent: AgentKind,
    command: &str,
    command_windows: Option<&str>,
) -> Option<String> {
    // `canonical_hook_command` posix-quotes every argument, so we tokenise the
    // command with a small shell-word reader (handles `'/path with space/'` and
    // the `'\''` apostrophe escape) rather than searching for a literal marker.
    let tokens = shell_words(command);
    if tokens.len() != 7 {
        return None;
    }
    if tokens[1] != "--owner"
        || tokens[2] != "cc-reminder"
        || tokens[3] != "--agent"
        || tokens[4] != agent.as_str()
        || tokens[5] != "--event"
    {
        return None;
    }
    let event = &tokens[6];
    let catalogued: Vec<String> = catalogued_hooks()
        .into_iter()
        .filter(|(a, _)| *a == agent)
        .map(|(_, event)| event)
        .collect();
    if !catalogued.iter().any(|candidate| candidate == event) {
        return None;
    }
    let canonical = canonical_hook_command(Path::new(&tokens[0]), agent, event);
    let expected = match agent {
        AgentKind::ClaudeCode => HookCommand {
            command_windows: None,
            ..canonical
        },
        AgentKind::Codex => canonical,
    };
    let candidate = HookCommand {
        command: command.to_owned(),
        command_windows: command_windows.map(str::to_owned),
    };
    if command_fingerprint(&expected) == command_fingerprint(&candidate) {
        Some(event.clone())
    } else {
        None
    }
}

fn element_is_owned(agent: AgentKind, element: &Value) -> bool {
    let Some(group) = element.as_object() else {
        return false;
    };
    let Some(handlers) = group.get("hooks").and_then(|prop| prop.value.as_array()) else {
        return false;
    };
    handlers
        .elements
        .iter()
        .any(|handler| handler_event(agent, handler).is_some())
}

fn handler_event(agent: AgentKind, handler: &Value) -> Option<String> {
    let obj = handler.as_object()?;
    let command = obj.get("command")?.value.as_string_lit()?;
    let command_windows = obj
        .get("commandWindows")
        .and_then(|prop| prop.value.as_string_lit())
        .map(|lit| lit.value.as_ref());
    recognize_owned(agent, command.value.as_ref(), command_windows)
}

/// Split a canonical posix command string into its shell words, honouring
/// single-quote runs (including the `'\''` apostrophe escape) and backslash
/// escapes outside quotes. Whitespace inside a quoted run is preserved, which
/// is what makes a helper path containing spaces tokenise as a single word.
fn shell_words(command: &str) -> Vec<String> {
    let bytes = command.as_bytes();
    let mut words = Vec::new();
    let mut current: Vec<u8> = Vec::new();
    let mut in_quotes = false;
    let mut has_token = false;
    let mut index = 0;
    while index < bytes.len() {
        let byte = bytes[index];
        if in_quotes {
            if byte == b'\'' {
                in_quotes = false;
            } else {
                current.push(byte);
            }
            index += 1;
        } else if byte == b'\'' {
            in_quotes = true;
            has_token = true;
            index += 1;
        } else if byte == b'\\' && index + 1 < bytes.len() {
            current.push(bytes[index + 1]);
            has_token = true;
            index += 2;
        } else if byte.is_ascii_whitespace() {
            if has_token {
                words.push(String::from_utf8(std::mem::take(&mut current)).expect("utf8 word"));
                has_token = false;
            }
            index += 1;
        } else {
            current.push(byte);
            has_token = true;
            index += 1;
        }
    }
    if has_token {
        words.push(String::from_utf8(current).expect("utf8 word"));
    }
    words
}

// ---- text splicing helpers -------------------------------------------------

#[derive(Clone, Debug)]
struct Edit {
    start: usize,
    end: usize,
    text: String,
}

impl Edit {
    fn insert(pos: usize, text: String) -> Self {
        Self {
            start: pos,
            end: pos,
            text,
        }
    }
    fn replace(start: usize, end: usize, text: String) -> Self {
        Self { start, end, text }
    }
    fn delete(start: usize, end: usize) -> Self {
        Self {
            start,
            end,
            text: String::new(),
        }
    }
}

fn apply_edits(source: &str, edits: &[Edit]) -> String {
    let bytes = source.as_bytes();
    let mut sorted: Vec<Edit> = edits.to_vec();
    sorted.sort_by_key(|edit| edit.start);
    let mut out = Vec::with_capacity(bytes.len() + 64);
    let mut cursor = 0usize;
    for edit in &sorted {
        debug_assert!(edit.start >= cursor, "overlapping or out-of-order edit");
        if edit.start < cursor {
            // Defensive: skip an overlapping edit rather than corrupt output.
            continue;
        }
        out.extend_from_slice(&bytes[cursor..edit.start]);
        out.extend_from_slice(edit.text.as_bytes());
        cursor = edit.end;
    }
    out.extend_from_slice(&bytes[cursor..]);
    String::from_utf8(out).expect("splice boundaries preserve utf8")
}

/// Span (start, end) covering one array element plus a single adjacent comma,
/// preferring a trailing comma so deletion preserves sibling formatting.
fn remove_element_span(elements: &[Value], index: usize, source: &str) -> (usize, usize) {
    let bytes = source.as_bytes();
    let range = jsonc::value_range(&elements[index]);
    let mut end = range.end;
    while end < bytes.len() && bytes[end].is_ascii_whitespace() {
        end += 1;
    }
    if end < bytes.len() && bytes[end] == b',' {
        return (range.start, end + 1);
    }
    let mut start = range.start;
    while start > 0 && bytes[start - 1].is_ascii_whitespace() {
        start -= 1;
    }
    if start > 0 && bytes[start - 1] == b',' {
        return (start - 1, range.end);
    }
    (range.start, range.end)
}

/// Append one element to the end of an existing array, returning the splice
/// position and the text to insert (including the separating comma/indent).
fn insert_array_append(
    array: &jsonc_parser::ast::Array,
    source: &str,
    text: &str,
) -> (usize, String) {
    let bytes = source.as_bytes();
    let open = array.range.start + 1;
    if array.elements.is_empty() {
        return (open, text.to_owned());
    }
    let last = array.elements.last().expect("non-empty");
    let last_end = jsonc::value_range(last).end;
    let close = array.range.end.saturating_sub(1);
    let comma = needs_comma(bytes, last_end, close);
    let indent = line_indent(bytes, jsonc::value_range(last).start);
    let newline = detect_newline(bytes);
    (last_end, format!("{comma}{newline}{indent}{text}"))
}

/// Insert one property into an existing object, returning the splice position
/// and the text to insert (including the separating comma/indent).
fn insert_object_property(
    object: &jsonc_parser::ast::Object,
    source: &str,
    text: &str,
) -> (usize, String) {
    let bytes = source.as_bytes();
    let open = object.range.start + 1;
    if object.properties.is_empty() {
        return (open, format!(" {text}"));
    }
    let last = object.properties.last().expect("non-empty");
    let last_end = last.range.end;
    let close = object.range.end.saturating_sub(1);
    let comma = needs_comma(bytes, last_end, close);
    let indent = line_indent(bytes, last.range.start);
    let newline = detect_newline(bytes);
    (last_end, format!("{comma}{newline}{indent}{text}"))
}

fn needs_comma(bytes: &[u8], after: usize, close: usize) -> &'static str {
    let mut probe = after;
    while probe < close && bytes[probe].is_ascii_whitespace() {
        probe += 1;
    }
    if probe < close && bytes[probe] == b',' {
        ""
    } else {
        ","
    }
}

/// Leading whitespace of the source line that contains `pos`.
fn line_indent(bytes: &[u8], pos: usize) -> &str {
    let mut line_start = pos;
    while line_start > 0 && bytes[line_start - 1] != b'\n' {
        line_start -= 1;
    }
    let mut indent_end = line_start;
    while indent_end < bytes.len() && (bytes[indent_end] == b' ' || bytes[indent_end] == b'\t') {
        indent_end += 1;
    }
    // The property begins at `pos`; indent is the whitespace from line start to pos.
    let end = pos.min(indent_end).max(line_start);
    std::str::from_utf8(&bytes[line_start..end]).unwrap_or("")
}

fn detect_newline(bytes: &[u8]) -> &'static str {
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'\n' {
            return if index > 0 && bytes[index - 1] == b'\r' {
                "\r\n"
            } else {
                "\n"
            };
        }
        index += 1;
    }
    "\n"
}

fn invalid_config(message: String) -> AppError {
    AppError {
        domain: ErrorDomain::Configuration,
        code: "configuration.invalid_jsonc".to_owned(),
        message,
        suggested_action: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shell_words_unquotes_posix_runs_and_keeps_quoted_spaces() {
        let words = shell_words("'/path with space/hook' '--owner' 'cc-reminder'");
        assert_eq!(
            words,
            vec![
                "/path with space/hook".to_owned(),
                "--owner".to_owned(),
                "cc-reminder".to_owned()
            ]
        );
        let apostrophe = shell_words("'it'\\''s'");
        assert_eq!(apostrophe, vec!["it's".to_owned()]);
    }

    #[test]
    fn recognize_rejects_lookalike_with_extra_args() {
        let cmd = "'/x/hook' '--owner' 'cc-reminder' '--agent' 'codex' '--event' 'Stop' '--extra'";
        assert!(recognize_owned(AgentKind::Codex, cmd, None).is_none());
    }

    #[test]
    fn line_indent_reads_leading_whitespace() {
        assert_eq!(line_indent(b"\n    \"x\"", 6), "    ");
        assert_eq!(line_indent(b"{\n  \"a\"", 4), "  ");
    }
}
