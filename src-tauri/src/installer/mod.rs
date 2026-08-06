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

use std::collections::{BTreeMap, BTreeSet};
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
        // First pass: plan HANDLER-granularity edits for events that already
        // have a matcher-group array.
        let mut handled_events: Vec<String> = Vec::new();
        for prop in &hooks_obj.properties {
            let event = prop.name.as_str().to_owned();
            // Bug 3: a desired event whose value is not an array must error —
            // otherwise the second pass would insert a duplicate key. A
            // non-array FOREIGN event is left untouched.
            let Some(array) = prop.value.as_array() else {
                if desired_map.contains_key(&event) {
                    return Err(invalid_config(format!("hooks.{event} is not an array")));
                }
                continue;
            };
            plan_event(agent, &event, array, &desired_map, source, &mut edits);
            if desired_map.contains_key(&event) {
                handled_events.push(event);
            }
        }
        for event in handled_events {
            desired_map.remove(&event);
        }

        // Second pass: remaining desired events get a brand-new property. They
        // are batched into a single insert so the separating commas between them
        // are always emitted even when the original object ends with a trailing
        // comma (Bug 6: re-probing original bytes per property emitted none).
        if !desired_map.is_empty() {
            let prop_texts: Vec<String> = desired_map
                .iter()
                .map(|(event, entry)| {
                    let group_text = serialise_owned_group(agent, entry);
                    format!("\"{event}\": [{group_text}]")
                })
                .collect();
            let (pos, text) = insert_object_properties(hooks_obj, source, &prop_texts);
            edits.push(Edit::insert(pos, text));
        }
    } else {
        // No hooks object at all: inject one onto the root.
        let hooks_body = build_hooks_body(agent, &desired_map);
        let prop_text = format!("\"hooks\": {hooks_body}");
        let (pos, text) = insert_object_property(root_obj, source, &prop_text);
        edits.push(Edit::insert(pos, text));
    }

    apply_edits(source, &edits)
}

/// Plan the edits for one event's matcher-group `array`. The splice operates at
/// HANDLER granularity (Bug 2): every owned handler for `event` is removed from
/// every group's `hooks` array while co-located foreign handlers are preserved
/// verbatim; a group is dropped only when it BECOMES empty as a result of
/// removing owned handlers. When `event` is desired, exactly one canonical
/// handler is placed — into the first group whose matcher matches (appended to
/// its `hooks` array), or as a brand-new group when no such group exists. All
/// multi-element deletions are run-based so adjacent owned spans never overlap
/// (Bug 1).
fn plan_event(
    agent: AgentKind,
    event: &str,
    array: &jsonc_parser::ast::Array,
    desired_map: &BTreeMap<String, &OwnedHookEntry>,
    source: &str,
    edits: &mut Vec<Edit>,
) {
    let desired_entry = desired_map.get(event);
    let target_matcher = desired_entry
        .and_then(|entry| entry.matcher.as_deref())
        .unwrap_or("");

    // Per group: owned handler indices, hooks array. Identify the placement
    // target — the first group whose matcher matches AND that carries a hooks
    // array.
    let mut group_owned: Vec<Vec<usize>> = Vec::with_capacity(array.elements.len());
    let mut group_handlers: Vec<Option<&jsonc_parser::ast::Array>> =
        Vec::with_capacity(array.elements.len());
    let mut target_group_idx: Option<usize> = None;
    for (index, element) in array.elements.iter().enumerate() {
        let group = element.as_object();
        let handlers = group.and_then(|g| g.get("hooks").and_then(|p| p.value.as_array()));
        group_handlers.push(handlers);
        let matcher = group
            .and_then(|g| g.get("matcher"))
            .and_then(|p| p.value.as_string_lit())
            .map(|lit| lit.value.to_string());
        if desired_entry.is_some()
            && target_group_idx.is_none()
            && matcher.as_deref() == Some(target_matcher)
            && handlers.is_some()
        {
            target_group_idx = Some(index);
        }
        let owned = handlers
            .map(|h| owned_handler_indices_in(agent, event, h))
            .unwrap_or_default();
        group_owned.push(owned);
    }

    // Groups removed entirely: had owned handlers AND become empty AND are not
    // the placement target.
    let mut cleanup_groups: Vec<usize> = Vec::new();
    for (index, owned) in group_owned.iter().enumerate() {
        if owned.is_empty() {
            continue;
        }
        let all_owned = group_handlers[index]
            .map(|h| h.elements.len() == owned.len())
            .unwrap_or(false);
        if all_owned && target_group_idx != Some(index) {
            cleanup_groups.push(index);
        }
    }
    let cleanup_set: BTreeSet<usize> = cleanup_groups.iter().copied().collect();

    // When the placement target's handlers are ALL owned, placement replaces the
    // owned run with the canonical handler in one edit (avoids a delete+insert
    // pair whose spans would overlap inside the same hooks array).
    let target_all_owned = target_group_idx
        .map(|index| {
            !group_owned[index].is_empty()
                && group_handlers[index]
                    .map(|h| h.elements.len() == group_owned[index].len())
                    .unwrap_or(false)
        })
        .unwrap_or(false);

    // Handler-level deletes for groups that survive (not wholesale-removed, and
    // not the all-owned target which is handled by a run-replace below).
    for (index, owned) in group_owned.iter().enumerate() {
        if owned.is_empty() || cleanup_set.contains(&index) {
            continue;
        }
        if target_all_owned && target_group_idx == Some(index) {
            continue;
        }
        let Some(handlers) = group_handlers[index] else {
            continue;
        };
        edits.extend(delete_runs(&handlers.elements, owned, source));
    }

    // Group-level deletes (run-based) for groups that became empty.
    if !cleanup_groups.is_empty() {
        cleanup_groups.sort();
        edits.extend(delete_runs(&array.elements, &cleanup_groups, source));
    }

    // Placement: desired events get exactly one canonical handler.
    if let Some(entry) = desired_entry {
        let handler_text = serialise_owned_handler(agent, entry);
        match target_group_idx {
            Some(index) => {
                let handlers = group_handlers[index].expect("target carries a hooks array");
                if target_all_owned {
                    // Replace the whole owned run with the canonical handler.
                    let owned = &group_owned[index];
                    let first = *owned.first().expect("all-owned is non-empty");
                    let last = *owned.last().expect("all-owned is non-empty");
                    let start = jsonc::value_range(&handlers.elements[first]).start;
                    let end = jsonc::value_range(&handlers.elements[last]).end;
                    edits.push(Edit::replace(start, end, handler_text));
                } else {
                    place_handler_in_group(
                        handlers,
                        &group_owned[index],
                        source,
                        &handler_text,
                        edits,
                    );
                }
            }
            None => {
                let group_text = serialise_owned_group(agent, entry);
                let (pos, text) = insert_array_append(array, source, &group_text);
                edits.push(Edit::insert(pos, text));
            }
        }
    }
}

/// Append `handler_text` as a new last handler of `handlers`, positioned so it
/// never overlaps the owned-handler delete spans emitted for the same array. If
/// at least one non-owned handler survives, the new handler is appended right
/// after the last survivor with a fresh leading comma — relying on the original
/// separator is unsafe because it may be consumed as the leading comma of a
/// later owned run. If every handler is owned (or the array is empty) the new
/// handler is placed just after the opening bracket.
fn place_handler_in_group(
    handlers: &jsonc_parser::ast::Array,
    owned_in_target: &[usize],
    source: &str,
    handler_text: &str,
    edits: &mut Vec<Edit>,
) {
    let bytes = source.as_bytes();
    let owned_set: BTreeSet<usize> = owned_in_target.iter().copied().collect();
    let last_surviving_end = handlers
        .elements
        .iter()
        .enumerate()
        .rfind(|(index, _)| !owned_set.contains(index))
        .map(|(_, handler)| jsonc::value_range(handler).end);
    if let Some(last_end) = last_surviving_end {
        let indent = handlers
            .elements
            .first()
            .map(|handler| line_indent(bytes, jsonc::value_range(handler).start))
            .unwrap_or("");
        let newline = detect_newline(bytes);
        edits.push(Edit::insert(
            last_end,
            format!(",{newline}{indent}{handler_text}"),
        ));
    } else {
        // No surviving handlers: the array becomes `[handler_text]`.
        let open = handlers.range.start + 1;
        edits.push(Edit::insert(open, handler_text.to_owned()));
    }
}

/// Indices of owned handlers for `event` within a group's `hooks` array.
fn owned_handler_indices_in(
    agent: AgentKind,
    event: &str,
    handlers: &jsonc_parser::ast::Array,
) -> Vec<usize> {
    handlers
        .elements
        .iter()
        .enumerate()
        .filter(|(_, handler)| {
            handler_event(agent, handler).is_some_and(|recognised| recognised == event)
        })
        .map(|(index, _)| index)
        .collect()
}

/// Emit non-overlapping delete edits for the given array element `indices` by
/// grouping consecutive indices into runs and producing ONE span per run. Each
/// run prefers the TRAILING comma after its last element; if there is none (the
/// run reaches the array's final element with no trailing comma) it falls back
/// to the LEADING comma before its first element; if neither is available (the
/// run covers every element) it deletes the bare element range. Runs are always
/// separated by at least one kept element, so their spans never touch — fixing
/// Bug 1 where two per-element deletes both claimed the comma between adjacent
/// owned elements and `apply_edits` silently dropped one.
fn delete_runs(elements: &[Value], indices: &[usize], source: &str) -> Vec<Edit> {
    let bytes = source.as_bytes();
    let mut edits = Vec::new();
    let mut i = 0;
    while i < indices.len() {
        let mut j = i;
        while j + 1 < indices.len() && indices[j + 1] == indices[j] + 1 {
            j += 1;
        }
        let first = indices[i];
        let last = indices[j];
        let first_start = jsonc::value_range(&elements[first]).start;
        let last_end = jsonc::value_range(&elements[last]).end;
        // Prefer the trailing comma after the run's last element.
        let mut end = last_end;
        while end < bytes.len() && bytes[end].is_ascii_whitespace() {
            end += 1;
        }
        if end < bytes.len() && bytes[end] == b',' {
            edits.push(Edit::delete(first_start, end + 1));
        } else {
            // Fall back to the leading comma before the run's first element.
            let mut start = first_start;
            while start > 0 && bytes[start - 1].is_ascii_whitespace() {
                start -= 1;
            }
            if start > 0 && bytes[start - 1] == b',' {
                edits.push(Edit::delete(start - 1, last_end));
            } else {
                // Run covers the whole array content: delete the bare range.
                edits.push(Edit::delete(first_start, last_end));
            }
        }
        i = j + 1;
    }
    edits
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
    let group = json!({
        "matcher": entry.matcher.clone().unwrap_or_default(),
        "hooks": [serialise_owned_handler_value(agent, entry)],
    });
    serde_json::to_string(&group).expect("group serialisable")
}

/// Serialise just the canonical handler object (used when appending a handler
/// into an existing group's `hooks` array).
fn serialise_owned_handler(agent: AgentKind, entry: &OwnedHookEntry) -> String {
    serde_json::to_string(&serialise_owned_handler_value(agent, entry))
        .expect("handler serialisable")
}

fn serialise_owned_handler_value(agent: AgentKind, entry: &OwnedHookEntry) -> serde_json::Value {
    let mut handler = serde_json::Map::new();
    handler.insert("type".to_owned(), json!("command"));
    handler.insert("command".to_owned(), json!(entry.command.command));
    if agent == AgentKind::Codex
        && let Some(command_windows) = &entry.command.command_windows
    {
        handler.insert("commandWindows".to_owned(), json!(command_windows));
    }
    handler.insert("timeout".to_owned(), json!(entry.timeout_seconds));
    serde_json::Value::Object(handler)
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

fn apply_edits(source: &str, edits: &[Edit]) -> Result<String, AppError> {
    let bytes = source.as_bytes();
    let mut sorted: Vec<Edit> = edits.to_vec();
    // Sort by (start, end) so a zero-width insert at position P is applied
    // BEFORE a delete beginning at P (insert.end == P < delete.end). This makes
    // the boundary case where a placed handler is spliced exactly where an owned
    // run's leading comma begins behave correctly, while any genuine range
    // overlap is still caught below.
    sorted.sort_by_key(|edit| (edit.start, edit.end));
    let mut out = Vec::with_capacity(bytes.len() + 64);
    let mut cursor = 0usize;
    for edit in &sorted {
        if edit.start < cursor {
            // Overlapping splice edits are a programmer error in the splice
            // engine. Surface it loudly instead of silently masking one edit and
            // emitting corrupt output (Bug 1).
            return Err(invalid_config("overlapping splice edits".to_owned()));
        }
        out.extend_from_slice(&bytes[cursor..edit.start]);
        out.extend_from_slice(edit.text.as_bytes());
        cursor = edit.end;
    }
    out.extend_from_slice(&bytes[cursor..]);
    Ok(String::from_utf8(out).expect("splice boundaries preserve utf8"))
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
    let indent = line_indent(bytes, jsonc::value_range(last).start);
    let newline = detect_newline(bytes);
    // Always emit a leading comma: the new element is spliced at `last_end`,
    // i.e. BEFORE any trailing comma, so a trailing comma would otherwise end up
    // after the inserted element and leave it unseparated (same root cause as
    // the object-property insert in Bug 6).
    (last_end, format!(",{newline}{indent}{text}"))
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
    let indent = line_indent(bytes, last.range.start);
    let newline = detect_newline(bytes);
    // Always emit a leading comma. The new property is spliced at `last_end`,
    // i.e. BEFORE any trailing comma that follows the last property, so relying
    // on that comma would leave the new property unseparated (the comma would
    // end up after the inserted text). A leading comma is correct whether or not
    // a trailing comma is present.
    (last_end, format!(",{newline}{indent}{text}"))
}

/// Insert one or more properties at the tail of `object`, returning the splice
/// position and the combined text. Every property gets a leading comma (the
/// first separates it from the original last property; the rest separate the
/// new properties from each other). Batching into one splice avoids Bug 6,
/// where re-probing the original bytes per property emitted no separator
/// between adjacent new properties at the same offset; and always emitting a
/// comma avoids the trailing-comma twin where the comma landed after the
/// inserted block instead of before it.
fn insert_object_properties(
    object: &jsonc_parser::ast::Object,
    source: &str,
    texts: &[String],
) -> (usize, String) {
    let bytes = source.as_bytes();
    if object.properties.is_empty() {
        let open = object.range.start + 1;
        let joined = texts.join(", ");
        return (open, format!(" {joined}"));
    }
    let last = object.properties.last().expect("non-empty");
    let last_end = last.range.end;
    let indent = line_indent(bytes, last.range.start);
    let newline = detect_newline(bytes);
    let mut combined = String::new();
    for text in texts {
        combined.push(',');
        combined.push_str(newline);
        combined.push_str(indent);
        combined.push_str(text);
    }
    (last_end, combined)
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
