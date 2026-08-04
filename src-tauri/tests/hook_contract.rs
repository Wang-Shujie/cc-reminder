#![cfg(feature = "test-support")]

use std::io::Write;
use std::process::{Command, Stdio};

#[test]
fn valid_hook_invocation_is_neutral_even_when_every_sink_is_unavailable() {
    let binary = env!("CARGO_BIN_EXE_cc-reminder-hook");
    let mut child = Command::new(binary)
        .args([
            "--owner",
            "cc-reminder",
            "--agent",
            "codex",
            "--event",
            "Stop",
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("hook helper should spawn");
    child
        .stdin
        .take()
        .expect("stdin available")
        .write_all(br#"{"session_id":"raw-session-id","cwd":"/private/client"}"#)
        .expect("write hook payload");
    let output = child.wait_with_output().expect("helper should exit");
    assert!(output.status.success());
    assert_eq!(output.stdout, b"{}\n");
    assert!(output.stderr.is_empty());
}
