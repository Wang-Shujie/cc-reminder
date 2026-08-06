//! Round-trip and atomic-replacement tests for the Agent hook config installer (Task 10).
//!
//! These tests assert the contract documented in
//! `docs/superpowers/plans/2026-07-29-cc-reminder.md` Task 10 and design 9.2/9.3/9.4:
//! structured JSONC/JSON patching that preserves every foreign byte, ownership
//! recognition via the (helper path + `--owner cc-reminder` + command fingerprint)
//! triad, and checked atomic replacement that refuses to overwrite a drifted file.

use std::fs;
use std::path::{Path, PathBuf};

use cc_reminder_lib::error::AppError;
use cc_reminder_lib::hook_command::command_fingerprint;
use cc_reminder_lib::installer::{
    ConfigPatch, OwnedHookEntry, atomic_replace_checked, hook_definition_fingerprint,
    inspect_owned_entries, owned_command, patch_claude_settings, patch_codex_hooks, sha256_hex,
};
use cc_reminder_lib::model::AgentKind;
use tempfile::TempDir;

const CLAUDE: AgentKind = AgentKind::ClaudeCode;
const CODEX: AgentKind = AgentKind::Codex;

fn fixture_bytes(relative: &str) -> Vec<u8> {
    // The crate lives in `src-tauri/`, so the repo-root fixtures are ONE level up.
    let manifest = env!("CARGO_MANIFEST_DIR");
    let path: PathBuf = Path::new(manifest).join("../tests/fixtures").join(relative);
    std::fs::read(&path).unwrap_or_else(|err| {
        panic!(
            "failed to read fixture {relative} at {}: {err}",
            path.display()
        )
    })
}

fn helper_path(dir: &TempDir) -> PathBuf {
    dir.path().join("cc-reminder-hook")
}

fn owned_entries(agent: AgentKind, helper: &Path, events: &[&str]) -> Vec<OwnedHookEntry> {
    events
        .iter()
        .map(|event| OwnedHookEntry {
            source_event: (*event).to_owned(),
            matcher: Some(String::new()),
            command: owned_command(helper, agent, event),
            timeout_seconds: 1,
        })
        .collect()
}

/// Independent oracle: parse JSONC, drop every hook handler whose `command`
/// references the test's unique helper path, then emit a canonical (sorted-key)
/// JSON string. Drives no installer code, so it is a fair cross-check of foreign
/// preservation. The tempdir helper path is unique per run, so its presence
/// inside a command unambiguously marks an owned entry regardless of how the
/// canonical helper quotes its arguments.
fn foreign_projection(bytes: &[u8], helper: &Path) -> String {
    let text = std::str::from_utf8(bytes).expect("utf8");
    let mut value = jsonc_parser::parse_to_serde_value(text, &Default::default())
        .expect("parse")
        .expect("root value");
    let helper_str = helper.to_string_lossy().to_string();
    strip_owned(&mut value, &helper_str);
    canonicalize(&value)
}

fn strip_owned(value: &mut serde_json::Value, helper: &str) {
    let serde_json::Value::Object(root) = value else {
        return;
    };
    let Some(serde_json::Value::Object(hooks)) = root.get_mut("hooks") else {
        return;
    };
    // Handler-granularity oracle: drop only owned HANDLERS from each group's
    // `hooks` array (preserving co-located foreign handlers byte-for-byte in the
    // canonical comparison), then drop a group only when it BECAME empty as a
    // result of stripping owned handlers — mirroring the installer's empty-group
    // cleanup. Foreign-only and already-empty groups are left in place.
    let mut empty_events = Vec::new();
    for (event, groups) in hooks.iter_mut() {
        let serde_json::Value::Array(groups) = groups else {
            continue;
        };
        let mut kept_groups = Vec::new();
        for group in groups.iter_mut() {
            let handlers_opt = group.get_mut("hooks").and_then(|v| v.as_array_mut());
            let Some(handlers) = handlers_opt else {
                kept_groups.push(group.clone());
                continue;
            };
            let had_owned = handlers
                .iter()
                .any(|handler| handler_owns_command(handler, helper));
            handlers.retain(|handler| !handler_owns_command(handler, helper));
            if had_owned && handlers.is_empty() {
                // Group held owned handlers and is now empty: drop the whole group,
                // matching the installer's empty-group cleanup.
                continue;
            }
            kept_groups.push(group.clone());
        }
        *groups = kept_groups;
        if groups.is_empty() {
            empty_events.push(event.clone());
        }
    }
    for event in empty_events {
        hooks.remove(&event);
    }
    // Prune a hooks object left empty after stripping owned entries, so a
    // foreign-projection comparison is not skewed by structural residue from an
    // install that created the hooks object in the first place.
    if hooks.is_empty() {
        root.remove("hooks");
    }
}

fn handler_owns_command(handler: &serde_json::Value, helper: &str) -> bool {
    handler
        .get("command")
        .and_then(serde_json::Value::as_str)
        .is_some_and(|cmd| cmd.contains(helper))
}

/// Canonical CC-reminder handler object as compact JSON, matching what the
/// installer emits for `agent`/`event` (Claude omits `commandWindows`, Codex
/// includes it). Used to assemble source fixtures that the triad recognizer
/// accepts as owned.
fn canonical_handler_json(agent: AgentKind, helper: &Path, event: &str) -> String {
    let cmd = owned_command(helper, agent, event);
    let mut s = String::from(r#"{"type":"command","#);
    s.push_str(&format!(
        r#""command":{}"#,
        serde_json::to_string(&cmd.command).unwrap()
    ));
    // The recognizer requires the fingerprint to match the agent's canonical
    // shape: Claude's canonical handler carries NO commandWindows, so emitting
    // one for a Claude fixture would make the lookalike reject and the bug under
    // test would be masked.
    if agent == AgentKind::Codex
        && let Some(cw) = &cmd.command_windows
    {
        s.push_str(&format!(
            r#","commandWindows":{}"#,
            serde_json::to_string(cw).unwrap()
        ));
    }
    s.push_str(r#","timeout":1}"#);
    s
}

/// One matcher-group object as compact JSON, wrapping the given handler strings.
fn group_json(matcher: &str, handlers: &[&str]) -> String {
    let joined = handlers.join(",");
    format!(
        r#"{{"matcher":{},"hooks":[{}]}}"#,
        serde_json::to_string(matcher).unwrap(),
        joined
    )
}

fn canonicalize(value: &serde_json::Value) -> String {
    let mut s = serde_json::to_string(value).expect("serialize");
    // serde_json::Map is BTreeMap-backed without preserve_order, so keys are sorted.
    s.retain(|c| !c.is_whitespace());
    s
}

fn parse_jsonc_ok(bytes: &[u8]) -> bool {
    let text = match std::str::from_utf8(bytes) {
        Ok(s) => s,
        Err(_) => return false,
    };
    jsonc_parser::parse_to_serde_value(text, &Default::default()).is_ok()
}

/// Strict structural validator used where jsonc-parser's loose mode would mask
/// a real defect (e.g. Bug 6's missing separating comma between adjacent
/// properties, which jsonc-parser silently tolerates). Strips JSONC comments
/// and trailing commas with a string-aware scanner, then hands the result to
/// serde_json (strict JSON), so a missing comma between elements is a hard
/// error.
fn strict_jsonc_ok(bytes: &[u8]) -> bool {
    let text = match std::str::from_utf8(bytes) {
        Ok(s) => s,
        Err(_) => return false,
    };
    let stripped = strip_jsonc_to_strict_json(text);
    serde_json::from_str::<serde_json::Value>(&stripped).is_ok()
}

fn strip_jsonc_to_strict_json(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = String::with_capacity(bytes.len());
    let mut in_string = false;
    let mut esc = false;
    let mut line_comment = false;
    let mut block_comment = false;
    let mut index = 0;
    while index < bytes.len() {
        let b = bytes[index];
        if line_comment {
            if b == b'\n' {
                line_comment = false;
                out.push('\n');
            }
            index += 1;
            continue;
        }
        if block_comment {
            if b == b'*' && index + 1 < bytes.len() && bytes[index + 1] == b'/' {
                block_comment = false;
                index += 2;
                continue;
            }
            index += 1;
            continue;
        }
        if in_string {
            out.push(b as char);
            if esc {
                esc = false;
            } else if b == b'\\' {
                esc = true;
            } else if b == b'"' {
                in_string = false;
            }
            index += 1;
            continue;
        }
        if b == b'"' {
            in_string = true;
            out.push('"');
            index += 1;
            continue;
        }
        if b == b'/' && index + 1 < bytes.len() && bytes[index + 1] == b'/' {
            line_comment = true;
            index += 2;
            continue;
        }
        if b == b'/' && index + 1 < bytes.len() && bytes[index + 1] == b'*' {
            block_comment = true;
            index += 2;
            continue;
        }
        out.push(b as char);
        index += 1;
    }
    // Strip trailing commas (a `,` followed by optional whitespace then `]`/`}`).
    let compact = out;
    let mut result = String::with_capacity(compact.len());
    let cb = compact.as_bytes();
    let mut i = 0;
    while i < cb.len() {
        if cb[i] == b',' {
            let mut j = i + 1;
            while j < cb.len() && cb[j].is_ascii_whitespace() {
                j += 1;
            }
            if j < cb.len() && (cb[j] == b']' || cb[j] == b'}') {
                i += 1;
                continue;
            }
        }
        result.push(cb[i] as char);
        i += 1;
    }
    result
}

/// Walk an installed config and return the sorted set of events CC Reminder owns.
fn owned_events(agent: AgentKind, bytes: &[u8]) -> Vec<String> {
    let mut events = inspect_owned_entries(agent, bytes)
        .expect("inspect")
        .into_iter()
        .map(|entry| entry.source_event)
        .collect::<Vec<_>>();
    events.sort();
    events
}

#[test]
fn claude_install_and_uninstall_preserve_every_foreign_byte() {
    let original = fixture_bytes("configs/claude-settings.jsonc");
    let dir = tempfile::tempdir().unwrap();
    let helper = helper_path(&dir);

    let installed = patch_claude_settings(
        &original,
        &owned_entries(CLAUDE, &helper, &["PermissionRequest", "Stop"]),
    )
    .unwrap();
    assert_eq!(installed.before_hash, sha256_hex(&original));
    assert_eq!(installed.after_hash, sha256_hex(&installed.bytes));
    assert!(parse_jsonc_ok(&installed.bytes));
    assert_eq!(
        owned_events(CLAUDE, &installed.bytes),
        vec!["PermissionRequest".to_owned(), "Stop".to_owned()]
    );

    // Foreign text survives the install byte-for-byte (substring witnesses).
    let installed_text = String::from_utf8(installed.bytes.clone()).unwrap();
    assert!(installed_text.contains("Block comment spanning"));
    assert!(installed_text.contains("foreign-lint.sh"));
    assert!(installed_text.contains("foreign-notify.sh"));
    assert!(installed_text.contains("trailing comment after object"));
    assert!(installed_text.contains("\"permissions\""));
    // The fixture has no final newline; our owned insertion must not append one.
    assert_ne!(installed_text.as_bytes().last(), Some(&b'\n'));

    let uninstalled = patch_claude_settings(&installed.bytes, &[]).unwrap();
    assert_eq!(
        foreign_projection(&uninstalled.bytes, &helper),
        foreign_projection(&original, &helper)
    );
    assert!(parse_jsonc_ok(&uninstalled.bytes));
    assert!(owned_events(CLAUDE, &uninstalled.bytes).is_empty());
}

#[test]
fn uninstall_requires_both_owner_marker_and_command_fingerprint() {
    let original = fixture_bytes("configs/codex-hooks.json");
    let result = patch_codex_hooks(&original, &[]).unwrap();
    let text = String::from_utf8(result.bytes.clone()).unwrap();
    // The lookalike carries `--owner cc-reminder` but a different command, so it
    // must survive uninstall of the real owned entry.
    assert!(text.contains("foreign-owner-cc-reminder-lookalike"));
    // The real owned Stop entry (recognized by the fingerprint triad) is gone.
    assert!(!text.contains("/Users/fixtures/.cc-reminder/bin/cc-reminder-hook"));
    assert!(parse_jsonc_ok(&result.bytes));
}

#[test]
fn claude_round_trip_against_synthetic_sources() {
    let dir = tempfile::tempdir().unwrap();
    let helper = helper_path(&dir);
    let cases = [
        ("empty object", b"{}".as_slice()),
        ("empty hooks", b"{\"hooks\":{}}".as_slice()),
        ("whitespace hooks", b"{ \"hooks\" : { } }".as_slice()),
        ("unknown top-level fields", b"{\"a\":1,\"hooks\":{},\"z\":[1,2]}".as_slice()),
        (
            "foreign sibling events",
            b"{\"hooks\":{\"PreToolUse\":[{\"matcher\":\"Bash\",\"hooks\":[{\"type\":\"command\",\"command\":\"/bin/foreign.sh\"}]}]}}".as_slice(),
        ),
        (
            "duplicate foreign matchers",
            b"{\"hooks\":{\"Stop\":[{\"matcher\":\"\",\"hooks\":[{\"type\":\"command\",\"command\":\"/bin/a.sh\"}]},{\"matcher\":\"\",\"hooks\":[{\"type\":\"command\",\"command\":\"/bin/b.sh\"}]}]}}".as_slice(),
        ),
    ];
    for (label, source) in cases {
        round_trip(CLAUDE, patch_claude_settings, source, &helper, label);
    }
}

#[test]
fn codex_round_trip_against_synthetic_sources() {
    let dir = tempfile::tempdir().unwrap();
    let helper = helper_path(&dir);
    let cases = [
        ("empty object", b"{}".as_slice()),
        ("empty hooks", b"{\"hooks\":{}}".as_slice()),
        ("unknown top-level fields", b"{\"description\":\"x\",\"hooks\":{}}".as_slice()),
        (
            "foreign permission handler",
            b"{\"hooks\":{\"PermissionRequest\":[{\"matcher\":\"Bash\",\"hooks\":[{\"type\":\"command\",\"command\":\"/bin/p.sh\",\"timeout\":3}]}]}}".as_slice(),
        ),
    ];
    for (label, source) in cases {
        round_trip(CODEX, patch_codex_hooks, source, &helper, label);
    }
}

fn round_trip<F>(agent: AgentKind, patch: F, source: &[u8], helper: &Path, label: &str)
where
    F: Fn(&[u8], &[OwnedHookEntry]) -> Result<ConfigPatch, AppError>,
{
    let first = &["PermissionRequest"];
    let second = &["PermissionRequest", "Stop"];

    // install one
    let installed = patch(source, &owned_entries(agent, helper, first))
        .unwrap_or_else(|err| panic!("[{label}] install failed: {err:?}"));
    assert!(
        parse_jsonc_ok(&installed.bytes),
        "[{label}] installed must parse"
    );
    assert_eq!(
        owned_events(agent, &installed.bytes),
        vec!["PermissionRequest".to_owned()],
        "[{label}] after install one"
    );
    // foreign bytes untouched
    assert_eq!(
        foreign_projection(&installed.bytes, helper),
        foreign_projection(source, helper),
        "[{label}] foreign projection after install"
    );

    // update: add Stop (selection change)
    let updated = patch(&installed.bytes, &owned_entries(agent, helper, second)).unwrap();
    let mut want = vec!["PermissionRequest".to_owned(), "Stop".to_owned()];
    want.sort();
    assert_eq!(
        owned_events(agent, &updated.bytes),
        want,
        "[{label}] after add Stop"
    );
    assert_eq!(
        foreign_projection(&updated.bytes, helper),
        foreign_projection(source, helper),
        "[{label}] foreign projection after update"
    );

    // repair: rewrite the same selection (idempotent w.r.t. owned set)
    let repaired = patch(&updated.bytes, &owned_entries(agent, helper, second)).unwrap();
    assert_eq!(
        owned_events(agent, &repaired.bytes),
        want,
        "[{label}] after repair"
    );

    // selection change: remove Stop again
    let narrowed = patch(&repaired.bytes, &owned_entries(agent, helper, first)).unwrap();
    assert_eq!(
        owned_events(agent, &narrowed.bytes),
        vec!["PermissionRequest".to_owned()],
        "[{label}] after remove Stop"
    );

    // uninstall everything
    let uninstalled = patch(&narrowed.bytes, &[]).unwrap();
    assert!(
        owned_events(agent, &uninstalled.bytes).is_empty(),
        "[{label}] after uninstall"
    );
    assert_eq!(
        foreign_projection(&uninstalled.bytes, helper),
        foreign_projection(source, helper),
        "[{label}] foreign projection after uninstall"
    );
    assert!(parse_jsonc_ok(&uninstalled.bytes));
}

#[test]
fn claude_install_preserves_crlf_newlines() {
    let original = fixture_bytes("configs/claude-settings.jsonc");
    let crlf: Vec<u8> = original
        .iter()
        .flat_map(|b| {
            if *b == b'\n' {
                vec![b'\r', b'\n']
            } else {
                vec![*b]
            }
        })
        .collect();
    let dir = tempfile::tempdir().unwrap();
    let helper = helper_path(&dir);
    let installed =
        patch_claude_settings(&crlf, &owned_entries(CLAUDE, &helper, &["Stop"])).unwrap();
    let text = String::from_utf8(installed.bytes.clone()).unwrap();
    // Foreign CRLF regions survive; no bare LF sneaks in where CRLF was used.
    assert!(text.contains("\r\n"), "CRLF must survive install");
    assert!(
        !text.contains('\n') || text.contains("\r\n"),
        "any newline in foreign region must remain CRLF"
    );
    // Owned insertion happened and itself uses the detected CRLF style.
    assert!(text.contains(helper.to_string_lossy().as_ref()));
    assert!(text.contains("'cc-reminder'"));
}

#[test]
fn install_into_path_containing_spaces() {
    let dir = tempfile::Builder::new()
        .prefix("cc reminder dir with spaces")
        .tempdir()
        .unwrap();
    let helper = dir.path().join("cc-reminder-hook");
    let source = b"{}";
    let installed =
        patch_claude_settings(source, &owned_entries(CLAUDE, &helper, &["Stop"])).unwrap();
    let text = String::from_utf8(installed.bytes.clone()).unwrap();
    assert!(text.contains(helper.to_string_lossy().as_ref()));
    assert_eq!(
        owned_events(CLAUDE, &installed.bytes),
        vec!["Stop".to_owned()]
    );

    // The emitted posix command must round-trip verbatim through inspection.
    // Claude's schema has no `commandWindows`, so the inspected entry carries
    // none; we compare the posix command string directly.
    let inspected = inspect_owned_entries(CLAUDE, &installed.bytes).unwrap();
    assert_eq!(inspected.len(), 1);
    let canonical = owned_command(&helper, CLAUDE, "Stop");
    assert_eq!(inspected[0].command.command, canonical.command);
    assert!(inspected[0].command.command_windows.is_none());
}

#[test]
fn definition_fingerprint_is_independent_of_command_fingerprint() {
    let dir = tempfile::tempdir().unwrap();
    let helper = helper_path(&dir);
    let entry = OwnedHookEntry {
        source_event: "Stop".into(),
        matcher: Some(String::new()),
        command: owned_command(&helper, CODEX, "Stop"),
        timeout_seconds: 1,
    };
    let fp = hook_definition_fingerprint(CODEX, &entry);

    // The two fingerprints are computed over different fields: changing the
    // timeout alters the definition fingerprint but leaves command_fingerprint
    // (which covers only command + commandWindows) untouched.
    let mut tweaked = entry.clone();
    tweaked.timeout_seconds = 5;
    assert_ne!(hook_definition_fingerprint(CODEX, &tweaked), fp);
    assert_eq!(
        command_fingerprint(&tweaked.command),
        command_fingerprint(&entry.command)
    );

    // Re-computing over an unchanged entry is stable.
    assert_eq!(hook_definition_fingerprint(CODEX, &entry), fp);

    // Changing the matcher changes the definition fingerprint only.
    let mut rematch = entry.clone();
    rematch.matcher = Some("Bash".into());
    assert_ne!(hook_definition_fingerprint(CODEX, &rematch), fp);
    assert_eq!(
        command_fingerprint(&rematch.command),
        command_fingerprint(&entry.command)
    );
}

#[test]
fn codex_owned_entry_serializes_command_windows_and_claude_omits_it() {
    let dir = tempfile::tempdir().unwrap();
    let helper = helper_path(&dir);
    let installed = patch_codex_hooks(b"{}", &owned_entries(CODEX, &helper, &["Stop"])).unwrap();
    let text = String::from_utf8(installed.bytes.clone()).unwrap();
    assert!(text.contains("\"commandWindows\""));
    assert!(text.contains("\"timeout\":1"));

    let installed_c =
        patch_claude_settings(b"{}", &owned_entries(CLAUDE, &helper, &["Stop"])).unwrap();
    let text_c = String::from_utf8(installed_c.bytes.clone()).unwrap();
    assert!(
        !text_c.contains("commandWindows"),
        "Claude must not emit commandWindows"
    );
}

// ---- atomic replacement -------------------------------------------------------

#[test]
fn external_change_after_inspection_is_reported_without_overwrite() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.json");
    fs::write(&path, b"{\"hooks\":{}}").unwrap();
    let inspected_hash = sha256_hex(&fs::read(&path).unwrap());

    // An external editor changes the file between inspection and replacement.
    fs::write(&path, b"{\"hooks\":{},\"userChange\":true}").unwrap();

    let error = atomic_replace_checked(&path, &inspected_hash, b"replacement", None).unwrap_err();
    assert_eq!(error.code, "integration.config_drift");
    let after = fs::read_to_string(&path).unwrap();
    assert!(
        after.contains("userChange"),
        "drifted file must be left untouched"
    );
    assert!(!after.contains("replacement"));
}

#[test]
fn atomic_replace_writes_durally_and_restores_mode() {
    use std::os::unix::fs::PermissionsExt;

    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.json");
    fs::write(&path, b"{\"hooks\":{}}").unwrap();
    let inspected_hash = sha256_hex(&fs::read(&path).unwrap());

    atomic_replace_checked(
        &path,
        &inspected_hash,
        b"{\"hooks\":{\"Stop\":[]}}",
        Some(0o600),
    )
    .unwrap();

    let mode = fs::metadata(&path).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode, 0o600, "requested mode must be applied");
    let after = fs::read_to_string(&path).unwrap();
    assert_eq!(after, "{\"hooks\":{\"Stop\":[]}}");
    // No leftover temp files in the directory (the app-specific lock file is
    // persistent and reused across operations, so it is not counted as a temp).
    let temps = fs::read_dir(dir.path())
        .unwrap()
        .filter(|entry| {
            let name = entry.as_ref().unwrap().file_name();
            let name = name.to_string_lossy();
            !name.starts_with('.') && name.ends_with(".tmp")
        })
        .count();
    assert_eq!(
        temps, 0,
        "no temp files should remain after a successful replace"
    );
}

#[test]
fn atomic_replace_preserves_original_mode_when_none_requested() {
    use std::os::unix::fs::PermissionsExt;

    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.json");
    fs::write(&path, b"{\"hooks\":{}}").unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o640)).unwrap();
    let inspected_hash = sha256_hex(&fs::read(&path).unwrap());

    atomic_replace_checked(&path, &inspected_hash, b"{\"hooks\":{\"Stop\":[]}}", None).unwrap();
    let mode = fs::metadata(&path).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode, 0o640);
}

#[test]
fn atomic_replace_uses_a_same_directory_temp() {
    // If the temp lived in /tmp (different filesystem), rename would fall back to a
    // non-atomic copy. We assert same-directory placement by making the parent
    // directory unwritable AFTER the file exists: the temp cannot be created and
    // the operation fails without disturbing the original.
    use std::os::unix::fs::PermissionsExt;

    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.json");
    fs::write(&path, b"{\"hooks\":{}}").unwrap();
    let inspected_hash = sha256_hex(&fs::read(&path).unwrap());

    fs::set_permissions(dir.path(), fs::Permissions::from_mode(0o500)).unwrap();
    let result = atomic_replace_checked(&path, &inspected_hash, b"replacement", None);
    fs::set_permissions(dir.path(), fs::Permissions::from_mode(0o700)).unwrap();

    let err = result.expect_err("write should fail without directory write permission");
    assert!(matches!(
        err.domain,
        cc_reminder_lib::error::ErrorDomain::Integration
    ));
    // Original intact, no temp lingering.
    assert_eq!(fs::read_to_string(&path).unwrap(), "{\"hooks\":{}}");
    let temps = fs::read_dir(dir.path())
        .unwrap()
        .filter(|entry| {
            let name = entry.as_ref().unwrap().file_name();
            let name = name.to_string_lossy();
            name.ends_with(".tmp")
        })
        .count();
    assert_eq!(
        temps, 0,
        "no partial temp should remain after a failed replace"
    );
}

#[test]
fn atomic_replace_round_trips_through_a_real_install() {
    let dir = tempfile::tempdir().unwrap();
    let helper = helper_path(&dir);
    let path = dir.path().join("settings.json");
    fs::write(&path, b"{}").unwrap();

    // Inspect, patch, then atomically replace.
    let inspected_hash = sha256_hex(&fs::read(&path).unwrap());
    let patch = patch_claude_settings(
        &fs::read(&path).unwrap(),
        &owned_entries(CLAUDE, &helper, &["Stop", "PermissionRequest"]),
    )
    .unwrap();
    atomic_replace_checked(&path, &inspected_hash, &patch.bytes, Some(0o600)).unwrap();

    let final_bytes = fs::read(&path).unwrap();
    assert_eq!(
        owned_events(CLAUDE, &final_bytes),
        vec!["PermissionRequest".to_owned(), "Stop".to_owned()]
    );
}

#[test]
fn hook_command_carries_both_platform_commands() {
    let dir = tempfile::tempdir().unwrap();
    let helper = helper_path(&dir);
    let cmd = owned_command(&helper, CODEX, "Stop");
    assert!(cmd.command.starts_with('\'') || cmd.command.starts_with('/'));
    assert!(cmd.command_windows.is_some());
    let win = cmd.command_windows.as_deref().unwrap();
    assert!(win.contains("cc-reminder-hook"));
    // The canonical posix form quotes every argument; the windows form leaves
    // no-special-char tokens unquoted. Both carry the agent/event markers.
    assert!(cmd.command.contains("'codex'"));
    assert!(cmd.command.contains("'Stop'"));
    assert!(win.contains("--agent codex --event Stop"));
}

// ---- fix-round: handler-granularity splice, non-array guards, multi-insert ----

/// Bug 1 (uninstall): two owned matcher-groups for the same event must both be
/// removed; the old per-element delete spans overlapped at the shared comma and
/// `apply_edits` silently dropped one, leaving an owned hook alive.
#[test]
fn multi_owned_uninstall_leaves_no_owned_hook() {
    let dir = tempfile::tempdir().unwrap();
    let helper = helper_path(&dir);
    let handler = canonical_handler_json(CLAUDE, &helper, "Stop");
    let group = group_json("", &[&handler]);
    let source = format!(r#"{{"hooks":{{"Stop":[{},{}]}}}}"#, group, group);

    let result = patch_claude_settings(source.as_bytes(), &[]).unwrap();
    assert!(parse_jsonc_ok(&result.bytes));
    assert!(
        owned_events(CLAUDE, &result.bytes).is_empty(),
        "no owned handler for Stop should remain after uninstall"
    );
    let text = std::str::from_utf8(&result.bytes).unwrap();
    assert!(
        !text.contains("--owner cc-reminder"),
        "no owned marker may survive uninstall"
    );
    assert_eq!(
        foreign_projection(&result.bytes, &helper),
        foreign_projection(source.as_bytes(), &helper),
        "foreign projection must be unchanged"
    );
}

/// Bug 1 (replace): three owned groups for Stop, desired [Stop] must consolidate
/// to exactly one owned handler. The replace path replaces the first group and
/// deletes the rest; with the old per-element delete spans the two trailing
/// deletes overlapped at the shared comma and one was silently dropped, leaving
/// a second owned group alive. Three groups is the minimum that issues two
/// deletes in the replace path and so exercises the overlap.
#[test]
fn multi_owned_replace_consolidates_to_one() {
    let dir = tempfile::tempdir().unwrap();
    let helper = helper_path(&dir);
    let handler = canonical_handler_json(CLAUDE, &helper, "Stop");
    let group = group_json("", &[&handler]);
    let source = format!(r#"{{"hooks":{{"Stop":[{},{},{}]}}}}"#, group, group, group);

    let result = patch_claude_settings(
        source.as_bytes(),
        &owned_entries(CLAUDE, &helper, &["Stop"]),
    )
    .unwrap();
    assert!(parse_jsonc_ok(&result.bytes));
    let inspected = inspect_owned_entries(CLAUDE, &result.bytes).unwrap();
    assert_eq!(
        inspected.len(),
        1,
        "exactly one owned handler must remain after consolidate"
    );
    assert_eq!(
        foreign_projection(&result.bytes, &helper),
        foreign_projection(source.as_bytes(), &helper),
        "foreign projection must be unchanged"
    );
}

/// Bug 2: a foreign handler co-located in the same group as an owned handler
/// must survive an update byte-for-byte. The old group-level replace destroyed
/// every co-located foreign handler.
#[test]
fn mixed_group_preserves_foreign_handlers() {
    let dir = tempfile::tempdir().unwrap();
    let helper = helper_path(&dir);
    let owned = canonical_handler_json(CLAUDE, &helper, "Stop");
    let foreign_command = "/usr/local/bin/foreign-logger.sh";
    let foreign = format!(
        r#"{{"type":"command","command":{},"timeout":2}}"#,
        serde_json::to_string(foreign_command).unwrap()
    );
    let group = group_json("", &[&owned, &foreign]);
    let source = format!(r#"{{"hooks":{{"Stop":[{}]}}}}"#, group);

    let result = patch_claude_settings(
        source.as_bytes(),
        &owned_entries(CLAUDE, &helper, &["Stop"]),
    )
    .unwrap();
    assert!(parse_jsonc_ok(&result.bytes));
    let text = std::str::from_utf8(&result.bytes).unwrap();
    assert!(
        text.contains(foreign_command),
        "foreign handler command must survive verbatim"
    );
    let inspected = inspect_owned_entries(CLAUDE, &result.bytes).unwrap();
    assert_eq!(inspected.len(), 1, "exactly one owned handler");
    assert_eq!(
        foreign_projection(&result.bytes, &helper),
        foreign_projection(source.as_bytes(), &helper),
        "foreign projection must retain the co-located foreign handler"
    );
}

/// Bug 3: when a desired event's value is not an array, the patcher must refuse
/// rather than insert a duplicate key.
#[test]
fn non_array_desired_event_errors() {
    let dir = tempfile::tempdir().unwrap();
    let helper = helper_path(&dir);
    let source = br#"{"hooks":{"Stop":"oops"}}"#;
    let err = patch_claude_settings(source, &owned_entries(CLAUDE, &helper, &["Stop"]))
        .expect_err("non-array desired event must error");
    assert_eq!(err.code, "configuration.invalid_jsonc");
}

/// Bug 3 (negative): a non-array value for an event that is NOT desired must be
/// left untouched (no error), preserving a foreign malformed entry.
#[test]
fn non_array_foreign_event_is_left_untouched() {
    let dir = tempfile::tempdir().unwrap();
    let helper = helper_path(&dir);
    let source = br#"{"hooks":{"Stop":"oops"}}"#;
    let result = patch_claude_settings(
        source,
        &owned_entries(CLAUDE, &helper, &["PermissionRequest"]),
    )
    .unwrap();
    let text = std::str::from_utf8(&result.bytes).unwrap();
    assert!(
        text.contains(r#""Stop":"oops"#),
        "foreign non-array entry must be preserved verbatim"
    );
}

/// Bug 6: inserting two or more new event properties after a trailing comma in
/// the hooks object must still emit a separating comma between the new
/// properties (the old code re-probed original bytes and emitted none).
#[test]
fn trailing_comma_multi_insert_keeps_separating_commas() {
    let dir = tempfile::tempdir().unwrap();
    let helper = helper_path(&dir);
    // Trailing comma after the lone foreign event in the hooks object.
    let source = br#"{"hooks":{"PreToolUse":[{"matcher":"Bash","hooks":[{"type":"command","command":"/bin/x.sh"}]}],}}"#;
    let result = patch_claude_settings(
        source,
        &owned_entries(CLAUDE, &helper, &["PermissionRequest", "Stop"]),
    )
    .unwrap();
    assert!(
        strict_jsonc_ok(&result.bytes),
        "output must separate newly inserted properties with commas even after \
         a trailing comma (jsonc-parser loose mode would otherwise mask this)"
    );
    let events = owned_events(CLAUDE, &result.bytes);
    let mut want = vec!["PermissionRequest".to_owned(), "Stop".to_owned()];
    want.sort();
    let mut got = events.clone();
    got.sort();
    assert_eq!(got, want, "both newly inserted events must be present");
}

/// Bug 4 / atomic cleanup: a failure at the rename step (after the temp was
/// created and written) must leave the original target bytes unchanged and
/// remove the partial temp file. The pre-existing test only covered
/// temp-creation failure.
#[test]
fn atomic_rename_failure_leaves_original_unchanged() {
    use cc_reminder_lib::installer::atomic::force_rename_failure_for_test;

    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.json");
    fs::write(&path, b"{\"hooks\":{}}").unwrap();
    let inspected_hash = sha256_hex(&fs::read(&path).unwrap());

    force_rename_failure_for_test(true);
    let result = atomic_replace_checked(&path, &inspected_hash, b"replacement", None);
    force_rename_failure_for_test(false);

    let err = result.expect_err("rename should fail under the forced seam");
    assert!(matches!(
        err.domain,
        cc_reminder_lib::error::ErrorDomain::Integration
    ));
    assert_eq!(fs::read_to_string(&path).unwrap(), "{\"hooks\":{}}");
    let temps = fs::read_dir(dir.path())
        .unwrap()
        .filter(|entry| {
            let name = entry.as_ref().unwrap().file_name();
            let name = name.to_string_lossy();
            name.ends_with(".tmp")
        })
        .count();
    assert_eq!(
        temps, 0,
        "no partial temp should remain after a rename failure"
    );
}
