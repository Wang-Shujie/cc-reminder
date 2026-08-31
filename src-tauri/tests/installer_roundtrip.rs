//! Round-trip and atomic-replacement tests for the Agent hook config installer
//! (Task 10) and the full checked Hook mutation transaction (Task 11).
//!
//! These tests assert the contract documented in
//! `docs/superpowers/plans/2026-07-29-cc-reminder.md` Tasks 10–11 and design
//! 9.2/9.3/9.4: structured JSONC/JSON patching that preserves every foreign
//! byte, ownership recognition via the (helper path + `--owner cc-reminder` +
//! command fingerprint) triad, checked atomic replacement that refuses to
//! overwrite a drifted file, and a lifecycle transaction that encrypts only the
//! previous `hooks` subtree and records separate command/definition fingerprints.

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use cc_reminder_lib::agents::{EntryHealth, HealthAggregate, HookSelection};
use cc_reminder_lib::error::AppError;
use cc_reminder_lib::hook_command::command_fingerprint;
use cc_reminder_lib::installer::helper::{
    HelperInstaller, HelperManifestEntry, current_target_triple,
};
use cc_reminder_lib::installer::lifecycle::{HookAction, HookInstaller};
use cc_reminder_lib::installer::{
    ConfigPatch, OwnedHookEntry, atomic_replace_checked, hook_definition_fingerprint,
    inspect_owned_entries, owned_command, patch_claude_settings, patch_codex_hooks, sha256_hex,
};
use cc_reminder_lib::model::{AgentKind, TrustStatus};
use cc_reminder_lib::security::crypto::FieldCipher;
use cc_reminder_lib::storage::db::Database;
use cc_reminder_lib::storage::integrations::IntegrationRepository;
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

/// Escape a path for use inside a JSON string literal (tests hand-write JSON
/// fixtures or assert on raw serialized text). Windows paths contain
/// backslashes that MUST be doubled — both when seeding a fixture and when
/// asserting a helper path appears in installed (serialized) bytes.
fn json_path(helper: &Path) -> String {
    helper.to_string_lossy().replace('\\', "\\\\")
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
    // The command is serialized JSON: Windows backslash separators arrive
    // escaped (`\\`), so assert against the escaped form. Quoting follows the
    // host contract: POSIX single quotes vs the always-double-quote Windows
    // form (hook_command::canonical_hook_command).
    assert!(text.contains(&json_path(&helper)));
    #[cfg(windows)]
    // Serialized form of the always-double-quoted owner token: `"cc-reminder"`
    // becomes `\"cc-reminder\"` inside the JSON string value.
    assert!(text.contains("\\\"cc-reminder\\\""));
    #[cfg(not(windows))]
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
    // Serialized JSON escapes the path's backslashes; match that form.
    assert!(text.contains(&json_path(&helper)));
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

// 以下三个测试断言 unix 权限位语义(mode 0o600/0o640、目录写位),
// Windows 无对应语义,仅 unix 编译。
#[test]
#[cfg(unix)]
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
#[cfg(unix)]
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
#[cfg(unix)]
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
// v2.0.1 macOS 适配检查:Windows 执行形式断言只在 Windows 编译下成立
// (cfg!(windows) 随编译目标切换);Unix 版断言见 hook_command_carries_posix_form。
#[cfg(windows)]
fn hook_command_carries_both_platform_commands() {
    let dir = tempfile::tempdir().unwrap();
    let helper = helper_path(&dir);
    let cmd = owned_command(&helper, CODEX, "Stop");
    // Windows: `command` itself must survive Codex's `cmd.exe /C "..."` wrap
    // (quotes around the exe only, bare args); the all-quoted POSIX form dies
    // with exit 1 there.
    assert!(cmd.command.starts_with('"'));
    assert!(cmd.command_windows.is_some());
    let win = cmd.command_windows.as_deref().unwrap();
    assert!(win.contains("cc-reminder-hook"));
    // Both carry the agent/event markers.
    assert!(cmd.command.contains("codex"));
    assert!(cmd.command.contains("Stop"));
    assert!(win.contains("\"--agent\" \"codex\" \"--event\" \"Stop\""));
}

/// Unix 编译下的对照:command 恒为 POSIX 单引号形式,commandWindows 仍为
/// windows_quote 形式(键存在但无消费方)。
#[test]
#[cfg(not(windows))]
fn hook_command_carries_posix_form_on_unix() {
    let dir = tempfile::tempdir().unwrap();
    let helper = helper_path(&dir);
    let cmd = owned_command(&helper, CODEX, "Stop");
    assert!(
        cmd.command.starts_with('\''),
        "unix command must be POSIX-quoted"
    );
    assert!(cmd.command.contains("'codex'"));
    let win = cmd.command_windows.as_deref().unwrap();
    assert!(win.contains("cc-reminder-hook"));
    assert!(win.contains("\"--agent\" \"codex\" \"--event\" \"Stop\""));
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

/// Scoped re-review regression: when every group for a desired event is
/// all-owned but sits in a NON-matching matcher group (e.g. the user moved our
/// owned handler into a "Bash" group), cleanup empties the whole array. The
/// placement None-arm must still emit a valid single canonical group at the
/// wide matcher — not a dangling `[,group]` that fails validation.
#[test]
fn owned_handler_in_wrong_matcher_group_moves_to_canonical_group() {
    let dir = tempfile::tempdir().unwrap();
    let helper = helper_path(&dir);
    let handler = canonical_handler_json(CLAUDE, &helper, "Stop");
    let group = group_json("Bash", &[&handler]);
    let source = format!(r#"{{"hooks":{{"Stop":[{group}]}}}}"#);

    let result = patch_claude_settings(
        source.as_bytes(),
        &owned_entries(CLAUDE, &helper, &["Stop"]),
    )
    .expect("all-owned non-matching array must patch, not error");
    assert!(strict_jsonc_ok(&result.bytes), "output must be valid JSON");

    let inspected = inspect_owned_entries(CLAUDE, &result.bytes).unwrap();
    assert_eq!(inspected.len(), 1, "exactly one owned Stop entry");
    assert_eq!(inspected[0].source_event, "Stop");
    assert_eq!(
        inspected[0].matcher.as_deref(),
        Some(""),
        "canonical handler placed in the wide matcher group",
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
///
/// Uses the `test-support` rename-failure seam, so it only exists when that
/// feature is enabled; the rest of this suite compiles and runs unflagged.
#[test]
#[cfg(feature = "test-support")]
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

// ============================================================================
// Task 11: full checked Hook mutation transaction (lifecycle, snapshot, trust)
// ============================================================================

/// The hook_installations row written by apply(Install) must carry the exact
/// fingerprint a LIVE helper reports for itself: canonical_hook_command always
/// includes the Windows override component, even for agents whose config
/// schema cannot persist it (Claude drops commandWindows on round-trip, so
/// deriving the fingerprint from the parsed-back entry silently breaks the
/// process_live trust gate for every real invocation).
#[test]
fn recorded_command_fingerprint_matches_what_the_runtime_helper_reports() {
    for agent in [CLAUDE, CODEX] {
        let env = InstallerEnvironment::new(agent, true);
        let selection = env.selection(&["Stop"]);
        env.apply(HookAction::Install, &selection).unwrap();

        let row = env.repository.hook(agent, "Stop").unwrap();
        let expected = cc_reminder_lib::hook_command::command_fingerprint(
            &cc_reminder_lib::installer::owned_command(&env.helper.stable_path(), agent, "Stop"),
        );
        assert_eq!(
            row.command_fingerprint, expected,
            "{agent:?} stored fingerprint diverges from the runtime self-report"
        );
    }
}

/// Wired test environment for one agent. Owns the tempdir, database, cipher,
/// helper installer, and the HookInstaller under test. Mirrors how the desktop
/// shell wires a `HookEnvironment`, with a deterministic data key.
struct InstallerEnvironment {
    #[allow(dead_code)]
    root: TempDir,
    agent: AgentKind,
    config_path: PathBuf,
    repository: IntegrationRepository,
    helper: HelperInstaller,
    cipher: Option<FieldCipher>,
}

impl InstallerEnvironment {
    fn new(agent: AgentKind, cipher_available: bool) -> Self {
        let root = tempfile::tempdir().unwrap();
        let data_dir = root.path().join("com.ccreminder.app");
        let database_path = data_dir.join("cc-reminder.sqlite3");
        let database = Database::open(&database_path).unwrap();
        let repository = IntegrationRepository::new(database);
        let bin_dir = root.path().join("bin");
        let helper = HelperInstaller::new(
            bin_dir.clone(),
            manifest_entry(b"helper-body", "0.1.0"),
            b"helper-body".to_vec(),
        );
        helper.install().unwrap();
        let home = root.path().join("home");
        fs::create_dir_all(&home).unwrap();
        let config_path = match agent {
            AgentKind::ClaudeCode => home.join(".claude/settings.json"),
            AgentKind::Codex => home.join(".codex/hooks.json"),
        };
        let cipher = if cipher_available {
            Some(FieldCipher::from_key([3_u8; 32]))
        } else {
            None
        };
        Self {
            root,
            agent,
            config_path,
            repository,
            helper,
            cipher,
        }
    }

    fn claude_fixture() -> Self {
        Self::new(CLAUDE, true)
    }

    fn codex_fixture() -> Self {
        Self::new(CODEX, true)
    }

    fn installer(&self) -> HookInstaller {
        HookInstaller::new(
            self.agent,
            self.config_path.clone(),
            self.repository.clone(),
            self.cipher.clone(),
            self.helper.clone(),
        )
    }

    /// Build a selection whose helper path/version track the installed helper.
    fn selection(&self, events: &[&str]) -> HookSelection {
        HookSelection {
            events: events
                .iter()
                .map(|e| (*e).to_owned())
                .collect::<BTreeSet<_>>(),
            helper_path: self.helper.stable_path(),
            helper_version: self.helper.manifest_version().clone(),
        }
    }

    fn apply(&self, action: HookAction, selection: &HookSelection) -> InstallationResult {
        InstallationResult {
            inner: self.installer().apply(action, selection),
        }
    }

    fn observe_ingress(&self, event: &str, command_fingerprint: &str) -> TrustStatus {
        self.installer()
            .observe_ingress(event, command_fingerprint)
            .unwrap()
    }

    fn inspect(&self, selection: &HookSelection) -> cc_reminder_lib::agents::HookHealth {
        self.installer().inspect(selection).unwrap()
    }

    fn snapshot_ciphertext(&self) -> Vec<u8> {
        self.repository
            .latest_snapshot(self.agent)
            .map(|s| s.ciphertext)
            .unwrap_or_default()
    }

    fn snapshot_source_hash(&self) -> Option<String> {
        self.repository
            .latest_snapshot(self.agent)
            .ok()
            .map(|s| s.source_hash)
    }
}

/// Wrapper so tests can call `.unwrap()` on the Installation while still
/// asserting error codes on failure paths.
struct InstallationResult {
    inner: Result<cc_reminder_lib::agents::Installation, AppError>,
}

impl InstallationResult {
    fn unwrap(self) -> cc_reminder_lib::agents::Installation {
        self.inner.unwrap()
    }
    fn unwrap_err_code(self) -> String {
        self.inner.unwrap_err().code
    }
}

fn manifest_entry(bytes: &[u8], version: &str) -> HelperManifestEntry {
    HelperManifestEntry {
        target_triple: current_target_triple().to_owned(),
        helper_version: semver::Version::parse(version).unwrap(),
        filename: "cc-reminder-hook".to_owned(),
        length: bytes.len() as u64,
        sha256: sha256_hex(bytes),
    }
}

fn owned_events_on_disk(agent: AgentKind, path: &Path) -> Vec<String> {
    let mut events = inspect_owned_entries(agent, &fs::read(path).unwrap_or_default())
        .unwrap()
        .into_iter()
        .map(|e| e.source_event)
        .collect::<Vec<_>>();
    events.sort();
    events
}

#[test]
fn apply_snapshots_only_the_previous_hook_subtree_then_installs_selection() {
    let env = InstallerEnvironment::claude_fixture();
    // Seed a foreign hook command that must end up encrypted in the snapshot,
    // never persisted in plaintext by the app.
    fs::create_dir_all(env.config_path.parent().unwrap()).unwrap();
    fs::write(
        &env.config_path,
        b"{\"hooks\":{\"Stop\":[{\"matcher\":\"\",\"hooks\":[{\"type\":\"command\",\"command\":\"/bin/foreign command keep-out\"}]}]}}",
    )
    .unwrap();

    let installation = env
        .apply(
            HookAction::Install,
            &env.selection(&["PermissionRequest", "Stop"]),
        )
        .unwrap();

    assert_eq!(
        owned_events_on_disk(CLAUDE, &env.config_path),
        vec!["PermissionRequest".to_owned(), "Stop".to_owned()]
    );
    // Two records, separate fingerprints.
    assert_eq!(installation.records.len(), 2);
    assert!(
        installation
            .records
            .iter()
            .all(|r| { r.command_fingerprint != r.definition_fingerprint })
    );
    // Claude entries never require trust.
    assert!(
        installation
            .records
            .iter()
            .all(|r| r.trust_status == TrustStatus::NotRequired)
    );

    // A snapshot was written and is encrypted: no plaintext foreign command.
    assert!(env.snapshot_count(CLAUDE) >= 1);
    let ciphertext = env.snapshot_ciphertext();
    assert!(!ciphertext.is_empty());
    assert!(
        !String::from_utf8_lossy(&ciphertext).contains("foreign command keep-out"),
        "previous hooks subtree must be encrypted, never plaintext"
    );
    // Source hash captured for drift-free disaster recovery.
    assert!(env.snapshot_source_hash().is_some());
}

#[test]
fn snapshot_encryption_unavailable_aborts_before_writing_agent_config() {
    let env = InstallerEnvironment::new(CLAUDE, false);
    let err = env
        .apply(HookAction::Install, &env.selection(&["Stop"]))
        .unwrap_err_code();
    assert_eq!(err, "security.encryption_unavailable");
    // No config file was created and no snapshot row was written.
    assert!(!env.config_path.exists());
    assert_eq!(env.snapshot_count(CLAUDE), 0);
}

#[test]
fn reinstall_with_identical_content_leaves_config_bytes_untouched() {
    let env = InstallerEnvironment::codex_fixture();
    env.apply(HookAction::Install, &env.selection(&["Stop"]))
        .unwrap();
    let first = fs::read(&env.config_path).unwrap();

    // A duplicate install (a self-heal tick re-firing Repair, a double click)
    // must NOT rewrite the file: Codex binds its trust to a hash of the
    // hooks.json CONTENT, so even an identical-content rewrite invalidates the
    // /hooks confirmation the user already completed.
    env.apply(HookAction::Install, &env.selection(&["Stop"]))
        .unwrap();
    let second = fs::read(&env.config_path).unwrap();
    assert_eq!(first, second, "identical apply must not rewrite the file");
}

#[test]
fn codex_change_waits_for_official_trust_until_matching_hook_is_observed() {
    let env = InstallerEnvironment::codex_fixture();
    let installation = env
        .apply(HookAction::Install, &env.selection(&["Stop"]))
        .unwrap();
    let record = &installation.records[0];
    assert_eq!(record.trust_status, TrustStatus::NeedsUserConfirmation);

    // A non-matching fingerprint does NOT transition trust.
    env.observe_ingress("Stop", "wrong-fingerprint");
    let health = env.inspect(&env.selection(&["Stop"]));
    assert_eq!(
        health.entries[0].trust_status,
        TrustStatus::NeedsUserConfirmation
    );

    // The real ingress fingerprint transitions to ObservedWorking.
    env.observe_ingress("Stop", &record.command_fingerprint);
    let health = env.inspect(&env.selection(&["Stop"]));
    assert_eq!(health.entries[0].trust_status, TrustStatus::ObservedWorking);
}

#[test]
fn binary_only_helper_upgrade_preserves_codex_trust_and_both_fingerprints() {
    let mut env = InstallerEnvironment::codex_fixture();
    let installed = env
        .apply(HookAction::Install, &env.selection(&["Stop"]))
        .unwrap();
    let cmd_fp = installed.records[0].command_fingerprint.clone();
    let def_fp = installed.records[0].definition_fingerprint.clone();
    env.observe_ingress("Stop", &cmd_fp);
    assert_eq!(
        env.inspect(&env.selection(&["Stop"])).entries[0].trust_status,
        TrustStatus::ObservedWorking
    );

    // Swap the helper binary at the SAME stable path (higher version).
    let upgraded = HelperInstaller::new(
        env.helper_bin_dir(),
        manifest_entry(b"helper-body-v2", "0.2.0"),
        b"helper-body-v2".to_vec(),
    );
    upgraded.install().unwrap();
    env.helper = upgraded;

    let reinstalled = env
        .apply(HookAction::UpgradeHelper, &env.selection(&["Stop"]))
        .unwrap();
    let record = &reinstalled.records[0];
    assert_eq!(record.command_fingerprint, cmd_fp);
    assert_eq!(record.definition_fingerprint, def_fp);
    assert_eq!(record.trust_status, TrustStatus::ObservedWorking);
    assert_eq!(record.helper_version, "0.2.0");
}

#[test]
fn helper_path_change_resets_codex_trust_to_needs_user_confirmation() {
    let mut env = InstallerEnvironment::codex_fixture();
    let installed = env
        .apply(HookAction::Install, &env.selection(&["Stop"]))
        .unwrap();
    let cmd_fp = installed.records[0].command_fingerprint.clone();
    env.observe_ingress("Stop", &cmd_fp);
    assert_eq!(
        env.inspect(&env.selection(&["Stop"])).entries[0].trust_status,
        TrustStatus::ObservedWorking
    );

    // Move the helper to a different stable path (new bin dir). Both fingerprints
    // change because the command string is part of both fingerprints, so the
    // stored observed trust no longer applies and resets.
    let new_bin = env.root.path().join("bin2");
    let moved = HelperInstaller::new(
        new_bin,
        manifest_entry(b"helper-body", "0.1.0"),
        b"helper-body".to_vec(),
    );
    moved.install().unwrap();
    env.helper = moved;

    let reinstalled = env
        .apply(HookAction::Install, &env.selection(&["Stop"]))
        .unwrap();
    let record = &reinstalled.records[0];
    assert_ne!(record.command_fingerprint, cmd_fp);
    assert_eq!(record.trust_status, TrustStatus::NeedsUserConfirmation);
}

#[test]
fn uninstall_removes_only_owned_matching_entries_and_leaves_a_lookalike() {
    let env = InstallerEnvironment::claude_fixture();
    // Seed a foreign lookalike INSIDE hooks.Stop: it carries the owner marker
    // but a trailing --extra arg, so the triad recognizer (7 tokens exact)
    // leaves it alone. The real owned entry has the canonical command.
    fs::create_dir_all(env.config_path.parent().unwrap()).unwrap();
    let helper = env.helper.stable_path();
    let lookalike = format!(
        "{{\"type\":\"command\",\"command\":\"{} --owner cc-reminder --agent claude-code --event Stop --extra\",\"timeout\":1}}",
        json_path(&helper)
    );
    let seed = format!("{{\"hooks\":{{\"Stop\":[{{\"matcher\":\"\",\"hooks\":[{lookalike}]}}]}}}}");
    fs::write(&env.config_path, seed.as_bytes()).unwrap();

    env.apply(HookAction::Install, &env.selection(&["Stop"]))
        .unwrap();
    // Exactly one owned entry now coexists with the lookalike.
    assert_eq!(
        owned_events_on_disk(CLAUDE, &env.config_path),
        vec!["Stop".to_owned()]
    );

    env.apply(HookAction::Uninstall, &env.selection(&[]))
        .unwrap();
    // The owned entry is gone; the lookalike (different command) survives.
    assert!(owned_events_on_disk(CLAUDE, &env.config_path).is_empty());
    let after = fs::read_to_string(&env.config_path).unwrap();
    assert!(
        after.contains("--event Stop --extra"),
        "foreign lookalike with a different command must be preserved"
    );
}

/// Uses the `test-support` config-drift seam (see the rename-failure test
/// above); the rest of this suite compiles and runs unflagged.
#[test]
#[cfg(feature = "test-support")]
fn external_drift_between_inspection_and_replace_is_rejected_without_overwrite() {
    use cc_reminder_lib::installer::lifecycle::force_config_drift_for_test;
    let env = InstallerEnvironment::claude_fixture();
    env.apply(HookAction::Install, &env.selection(&["Stop"]))
        .unwrap();

    // The seam simulates an external editor rewriting the file between the
    // installer's inspection read and its atomic replace. The replace's
    // independent re-read must detect the drift, refuse, and leave the external
    // edit in place — it does NOT silently restore the original.
    force_config_drift_for_test(true);
    let err = env
        .apply(HookAction::Repair, &env.selection(&["Stop"]))
        .unwrap_err_code();
    force_config_drift_for_test(false);

    assert_eq!(err, "integration.config_drift");
    let after = fs::read_to_string(&env.config_path).unwrap();
    assert_eq!(after, "{\"external\":\"drift\"}");
    assert!(owned_events_on_disk(CLAUDE, &env.config_path).is_empty());
}

/// A selection carrying a helper_version that no longer matches the installed
/// helper's manifest must be rejected, so a selection cached before a helper
/// upgrade cannot record a stale version in the installation rows.
#[test]
fn stale_helper_version_in_selection_is_rejected() {
    let env = InstallerEnvironment::claude_fixture();
    let mut stale = env.selection(&["Stop"]);
    stale.helper_version = semver::Version::new(99, 0, 0);
    assert_eq!(
        env.apply(HookAction::Install, &stale).unwrap_err_code(),
        "update.helper_not_installed"
    );
}

#[test]
fn selection_out_of_date_is_reported_and_repair_converges() {
    let env = InstallerEnvironment::claude_fixture();
    env.apply(HookAction::Install, &env.selection(&["Stop"]))
        .unwrap();

    // Required selection now wants PermissionRequest too: installed owned set
    // differs from the required selection.
    let health = env.inspect(&env.selection(&["PermissionRequest", "Stop"]));
    assert!(health.selection_out_of_date);
    assert_eq!(health.aggregate, HealthAggregate::NeedsRepair);

    // An explicit Repair converges in one checked patch.
    env.apply(
        HookAction::Repair,
        &env.selection(&["PermissionRequest", "Stop"]),
    )
    .unwrap();
    let health = env.inspect(&env.selection(&["PermissionRequest", "Stop"]));
    assert!(!health.selection_out_of_date);
    assert_eq!(health.aggregate, HealthAggregate::Healthy);
}

#[test]
fn inspect_reports_healthy_entries_for_an_aligned_installation() {
    let env = InstallerEnvironment::claude_fixture();
    env.apply(HookAction::Install, &env.selection(&["Stop"]))
        .unwrap();
    let health = env.inspect(&env.selection(&["Stop"]));
    assert!(!health.selection_out_of_date);
    assert_eq!(health.entries.len(), 1);
    assert_eq!(health.entries[0].health, EntryHealth::Healthy);
    assert_eq!(health.entries[0].trust_status, TrustStatus::NotRequired);
}

// Additional accessors split into a second impl block for locality.
impl InstallerEnvironment {
    fn snapshot_count(&self, agent: AgentKind) -> usize {
        self.repository.snapshot_count(agent).unwrap()
    }
    fn helper_bin_dir(&self) -> PathBuf {
        self.helper.stable_path().parent().unwrap().to_path_buf()
    }
}

#[test]
fn agent_integration_trait_routes_install_and_inspect_through_the_fixed_user_path() {
    use cc_reminder_lib::agents::AgentIntegration;
    use cc_reminder_lib::installer::lifecycle::HookEnvironment;

    // The trait impls must NOT accept a caller-supplied config path: Claude uses
    // home/.claude/settings.json, Codex uses codex_home/hooks.json. Both delegate
    // to HookInstaller, producing records with separate fingerprints.
    for agent in [CLAUDE, CODEX] {
        let env = if agent == CLAUDE {
            InstallerEnvironment::claude_fixture()
        } else {
            InstallerEnvironment::codex_fixture()
        };
        let hook_env = HookEnvironment {
            repository: env.repository.clone(),
            cipher: env.cipher.clone(),
            helper: env.helper.clone(),
            home: env
                .config_path
                .parent()
                .unwrap()
                .parent()
                .unwrap()
                .to_path_buf(),
            codex_home: None,
        };
        let integration: Box<dyn AgentIntegration> = match agent {
            CLAUDE => Box::new(cc_reminder_lib::agents::ClaudeIntegration::with_detection(
                dummy_detection(CLAUDE),
            )),
            CODEX => Box::new(cc_reminder_lib::agents::CodexIntegration::with_detection(
                dummy_detection(CODEX),
            )),
        };
        let selection = env.selection(&["Stop"]);
        let installed = integration.install_hooks(&hook_env, &selection).unwrap();
        assert_eq!(installed.records.len(), 1);
        let health = integration.inspect_hooks(&hook_env, &selection).unwrap();
        assert_eq!(health.entries.len(), 1);
        let (expected_health, expected_trust) = if agent == CLAUDE {
            (EntryHealth::Healthy, TrustStatus::NotRequired)
        } else {
            // Codex needs official trust confirmation before it is Healthy.
            (EntryHealth::NeedsTrust, TrustStatus::NeedsUserConfirmation)
        };
        assert_eq!(health.entries[0].health, expected_health);
        assert_eq!(health.entries[0].trust_status, expected_trust);
    }
}

fn dummy_detection(agent: AgentKind) -> cc_reminder_lib::agents::Detection {
    use cc_reminder_lib::agents::DetectionState;
    use chrono::Utc;
    let version = match agent {
        CLAUDE => semver::Version::new(2, 1, 218),
        CODEX => semver::Version::new(0, 145, 0),
    };
    cc_reminder_lib::agents::Detection {
        agent,
        executable_path: Some("/bin/agent".into()),
        version: Some(version.clone()),
        capability_verification: Some(
            cc_reminder_lib::events::catalog::catalog_for(agent, &version).verification,
        ),
        state: DetectionState::Detected,
        checked_at: Utc::now(),
    }
}

// ============================================================================
// Fix wave: production bundled-helper loading bridge
//
// The desktop shell previously built its HelperInstaller from an EMPTY manifest
// with no bytes, so no Install/Repair could ever satisfy lifecycle.rs's stable-
// path requirement (`update.helper_not_installed`). The bridge lives in
// `installer::helper::load_bundled_installer` + `HelperInstaller::ensure_installed`;
// these tests drive it end-to-end against a fixture directory laid out exactly
// like the packaged resources (release CI regenerates both files from signed
// bytes at the same relative paths).
// ============================================================================

/// Lay out `root` like Tauri's resource directory: `resources/helper-manifest.json`
/// plus `resources/bin/cc-reminder-hook`, with ONE synthetic entry describing
/// THIS compile target so the loader's selection succeeds.
fn write_bundled_resources(root: &std::path::Path, bytes: &[u8], version: &str) {
    let resources = root.join("resources");
    fs::create_dir_all(resources.join("bin")).unwrap();
    fs::write(resources.join("bin").join("cc-reminder-hook"), bytes).unwrap();
    let manifest = serde_json::json!({
        "helpers": [{
            "target_triple": current_target_triple(),
            "helper_version": version,
            "filename": "cc-reminder-hook",
            "length": bytes.len(),
            "sha256": sha256_hex(bytes),
        }],
    });
    fs::write(
        resources.join("helper-manifest.json"),
        serde_json::to_vec_pretty(&manifest).unwrap(),
    )
    .unwrap();
}

/// Full production path: load from a packaged-resource fixture → deploy the
/// stable-path helper → run a real checked Install transaction whose selection
/// is derived from the WIRED installer (path + manifest version).
#[test]
fn bundled_resource_fixture_drives_a_full_install_through_the_production_loader() {
    let root = tempfile::tempdir().unwrap();
    let helper_bytes = b"synthetic signed helper body".to_vec();
    write_bundled_resources(root.path(), &helper_bytes, "0.1.0");

    // The shell passes Tauri's resource dir here; the fixture mirrors it.
    let data_dir = root.path().join("com.ccreminder.app");
    let bin_dir = data_dir.join("bin");
    let installer =
        cc_reminder_lib::installer::helper::load_bundled_installer(root.path(), &bin_dir)
            .expect("packaged-resource layout must yield an installer");
    assert_eq!(
        installer.manifest_version(),
        &semver::Version::parse("0.1.0").unwrap()
    );

    // Idempotent deployment: first call copies verified bytes, a second call
    // skips the copy because the stable file already matches the manifest.
    let deployed = installer.ensure_installed().unwrap();
    assert!(installed_bytes_are(&deployed.path, &helper_bytes));
    let redeployed = installer.ensure_installed().unwrap();
    assert_eq!(redeployed.path, deployed.path);
    assert_eq!(
        cc_reminder_lib::installer::sha256_hex(&fs::read(&redeployed.path).unwrap()),
        cc_reminder_lib::installer::sha256_hex(&helper_bytes)
    );

    // A full checked mutation now succeeds WITHOUT update.helper_not_installed,
    // using a selection derived from the wired installer itself.
    let database_path = data_dir.join("cc-reminder.sqlite3");
    let database = Database::open(&database_path).unwrap();
    let repository = IntegrationRepository::new(database);
    let home = root.path().join("home");
    fs::create_dir_all(&home).unwrap();
    let config_path = home.join(".claude/settings.json");
    let hook_installer = HookInstaller::new(
        CLAUDE,
        config_path.clone(),
        repository,
        Some(FieldCipher::from_key([5_u8; 32])),
        installer,
    );
    let selection = HookSelection {
        events: ["Stop".to_owned()].into_iter().collect::<BTreeSet<_>>(),
        helper_path: deployed.path,
        helper_version: semver::Version::parse("0.1.0").unwrap(),
    };
    hook_installer
        .apply(HookAction::Install, &selection)
        .unwrap();
    assert_eq!(
        owned_events_on_disk(CLAUDE, &config_path),
        vec!["Stop".to_owned()]
    );
}

fn installed_bytes_are(path: &Path, expected: &[u8]) -> bool {
    fs::read(path)
        .map(|bytes| bytes == expected)
        .unwrap_or(false)
}

/// The committed development layout (placeholder triple) must fail selection
/// with the typed `configuration.helper_unavailable` — never panic, never
/// install anything.
#[test]
fn placeholder_manifest_fixture_is_rejected_with_helper_unavailable() {
    let root = tempfile::tempdir().unwrap();
    let resources = root.path().join("resources");
    fs::create_dir_all(resources.join("bin")).unwrap();
    fs::write(
        resources.join("bin").join("cc-reminder-hook"),
        b"placeholder",
    )
    .unwrap();
    fs::write(
        resources.join("helper-manifest.json"),
        r#"{"helpers":[{"target_triple":"REPLACE_WITH_TARGET_TRIPLE","helper_version":"0.0.0-placeholder","filename":"cc-reminder-hook","length":0,"sha256":"REPLACE_WITH_SHA256_OF_THE_PACKAGED_HELPER_BYTES"}]}"#,
    )
    .unwrap();

    let error = cc_reminder_lib::installer::helper::load_bundled_installer(
        root.path(),
        &root.path().join("data").join("bin"),
    )
    .unwrap_err();
    assert_eq!(
        error.domain,
        cc_reminder_lib::error::ErrorDomain::Configuration
    );
    assert_eq!(error.code, "configuration.helper_unavailable");
    assert!(error.suggested_action.is_some());
    // Nothing reached any stable path.
    assert!(
        !root
            .path()
            .join("data")
            .join("bin")
            .join("cc-reminder-hook")
            .exists()
    );
}
