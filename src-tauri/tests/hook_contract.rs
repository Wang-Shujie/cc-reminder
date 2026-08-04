#![cfg(feature = "test-support")]

use std::io::Write;
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
use std::process::{Command, Stdio};

#[cfg(unix)]
use cc_reminder_lib::events::normalize::CapturedHookEvent;
use cc_reminder_lib::events::normalize::SafeIngressEvent;
#[cfg(unix)]
use cc_reminder_lib::ipc::protocol::{IPC_PROTOCOL_VERSION, IngressRequest, IngressResponse};
#[cfg(unix)]
use cc_reminder_lib::ipc::server::{Endpoint, IpcServer};
use cc_reminder_lib::model::{AgentKind, ProjectMatchCacheFile, ProjectMatchCacheProject};
use cc_reminder_lib::paths::{AgentVersionCacheFile, CachedAgentVersion};
use cc_reminder_lib::storage::db::Database;
use semver::Version;
use tempfile::TempDir;
use uuid::Uuid;

#[test]
fn valid_hook_invocation_is_neutral_even_when_every_sink_is_unavailable() {
    let env = HookEnvironment::new();
    std::fs::create_dir(env.app().join("cc-reminder.sqlite3")).unwrap();
    std::fs::write(env.app().join("spool"), b"unavailable").unwrap();

    let output = env.run(
        br#"{"source_version":"0.145.0","session_id":"raw-session-id","cwd":"/private/client"}"#,
    );

    assert!(output.status.success());
    assert_eq!(output.stdout, b"{}\n");
    assert!(output.stderr.is_empty());
}

struct HookEnvironment {
    root: TempDir,
}

impl HookEnvironment {
    fn new() -> Self {
        let root = tempfile::tempdir().unwrap();
        let app = root.path().join("com.ccreminder.app");
        std::fs::create_dir_all(&app).unwrap();
        Self { root }
    }

    fn app(&self) -> std::path::PathBuf {
        self.root.path().join("com.ccreminder.app")
    }

    fn private_write(&self, name: &str, bytes: &[u8]) {
        let path = self.app().join(name);
        std::fs::write(&path, bytes).unwrap();
        #[cfg(unix)]
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600)).unwrap();
    }

    fn version_cache(&self, schema_version: u16) {
        let cache = AgentVersionCacheFile {
            schema_version,
            agents: [(
                AgentKind::Codex,
                CachedAgentVersion {
                    version: Version::new(0, 145, 0),
                    detected_at: chrono::Utc::now(),
                },
            )]
            .into_iter()
            .collect(),
        };
        self.private_write("agent-versions.json", &serde_json::to_vec(&cache).unwrap());
    }

    fn run(&self, input: &[u8]) -> std::process::Output {
        self.run_args(
            [
                "--owner",
                "cc-reminder",
                "--agent",
                "codex",
                "--event",
                "Stop",
            ],
            input,
        )
    }

    fn run_args<const N: usize>(&self, args: [&str; N], input: &[u8]) -> std::process::Output {
        let mut child = Command::new(env!("CARGO_BIN_EXE_cc-reminder-hook"))
            .args(args)
            .env("CC_REMINDER_TEST_DATA_DIR", self.root.path())
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        child.stdin.take().unwrap().write_all(input).unwrap();
        child.wait_with_output().unwrap()
    }
}

#[test]
fn invalid_cli_and_bounded_json_fail_neutrally_without_persistence() {
    let env = HookEnvironment::new();
    env.version_cache(1);
    Database::open(&env.app().join("cc-reminder.sqlite3")).unwrap();

    for args in [
        ["--owner", "other", "--agent", "codex", "--event", "Stop"],
        [
            "--owner",
            "cc-reminder",
            "--agent",
            "other",
            "--event",
            "Stop",
        ],
        [
            "--owner",
            "cc-reminder",
            "--agent",
            "codex",
            "--event",
            "Other",
        ],
    ] {
        let output = env.run_args(args, br#"{}"#);
        assert!(output.status.success());
        assert_eq!(output.stdout, b"{}\n");
        assert!(output.stderr.is_empty());
    }
    for output in [
        env.run_args(
            [
                "--owner",
                "cc-reminder",
                "--agent",
                "codex",
                "--event",
                "Stop",
                "--extra",
                "value",
            ],
            br#"{}"#,
        ),
        env.run_args(
            [
                "--owner",
                "cc-reminder",
                "--owner",
                "cc-reminder",
                "--agent",
                "codex",
                "--event",
                "Stop",
            ],
            br#"{}"#,
        ),
        env.run_args(
            [
                "--owner",
                "cc-reminder",
                "--agent",
                "codex",
                "--event",
                "Stop",
                "trailing",
            ],
            br#"{}"#,
        ),
    ] {
        assert!(output.status.success());
        assert_eq!(output.stdout, b"{}\n");
        assert!(output.stderr.is_empty());
    }

    let too_many_fields = serde_json::to_vec(&serde_json::Value::Object(
        (0..257)
            .map(|index| (format!("field-{index}"), serde_json::Value::Null))
            .collect(),
    ))
    .unwrap();
    let mut too_deep = "null".to_owned();
    for _ in 0..33 {
        too_deep = format!("[{too_deep}]");
    }
    let too_many_nodes = serde_json::to_vec(&serde_json::json!({
        "nodes": vec![serde_json::Value::Null; 4_096]
    }))
    .unwrap();
    let oversized = vec![b'x'; cc_reminder_lib::ipc::protocol::MAX_HOOK_BYTES + 1];
    for input in [
        b"not-json".as_slice(),
        b"[]".as_slice(),
        b"{} {}".as_slice(),
        br#"{"hook_event_name":"SessionStart"}"#.as_slice(),
        too_many_fields.as_slice(),
        too_deep.as_bytes(),
        too_many_nodes.as_slice(),
        oversized.as_slice(),
    ] {
        let output = env.run(input);
        assert!(output.status.success());
        assert_eq!(output.stdout, b"{}\n");
        assert!(output.stderr.is_empty());
    }

    let connection = Database::open_ingress_writer(&env.app().join("cc-reminder.sqlite3")).unwrap();
    let count: i64 = connection
        .query_row("SELECT COUNT(*) FROM ingress_events", [], |row| row.get(0))
        .unwrap();
    assert_eq!(count, 0);
}

#[test]
fn unknown_and_forbidden_hook_fields_never_reach_durable_ingress() {
    let env = HookEnvironment::new();
    env.version_cache(1);
    Database::open(&env.app().join("cc-reminder.sqlite3")).unwrap();

    env.run(
        br#"{"unknown":"do-not-store","transcript_path":"/raw/transcript","stop_hook_active":true}"#,
    );

    let connection = Database::open_ingress_writer(&env.app().join("cc-reminder.sqlite3")).unwrap();
    let json: String = connection
        .query_row("SELECT safe_envelope_json FROM ingress_events", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert!(!json.contains("unknown"));
    assert!(!json.contains("do-not-store"));
    assert!(!json.contains("transcript"));
    assert!(!json.contains("/raw/transcript"));
    assert!(json.contains("stop_hook_active"));
}

#[test]
fn missing_correlation_key_still_persists_without_sensitive_references() {
    let env = HookEnvironment::new();
    env.version_cache(1);
    Database::open(&env.app().join("cc-reminder.sqlite3")).unwrap();
    std::fs::create_dir(env.app().join("correlation.key")).unwrap();

    let output =
        env.run(br#"{"session_id":"raw-session","turn_id":"raw-turn","cwd":"/private/client"}"#);

    assert!(output.status.success());
    assert_eq!(output.stdout, b"{}\n");
    assert!(output.stderr.is_empty());
    let connection = Database::open_ingress_writer(&env.app().join("cc-reminder.sqlite3")).unwrap();
    let json: String = connection
        .query_row("SELECT safe_envelope_json FROM ingress_events", [], |row| {
            row.get(0)
        })
        .unwrap();
    let safe: SafeIngressEvent = serde_json::from_str(&json).unwrap();
    assert_eq!(safe.project_display_name.as_deref(), Some("client"));
    assert!(safe.cwd_fingerprint.is_none());
    assert!(safe.session_ref.is_none());
    assert!(safe.turn_ref.is_none());
    assert!(!json.contains("raw-session"));
    assert!(!json.contains("raw-turn"));
    assert!(!json.contains("/private/client"));
}

#[test]
fn private_valid_project_cache_matches_offline_and_wrong_schema_is_ignored() {
    let env = HookEnvironment::new();
    env.version_cache(1);
    Database::open(&env.app().join("cc-reminder.sqlite3")).unwrap();
    let project_id = Uuid::now_v7();
    let cache = ProjectMatchCacheFile {
        version: 1,
        projects: vec![ProjectMatchCacheProject {
            id: project_id,
            display_name: "client-app".into(),
            canonical_paths: vec!["/workspace/client".into()],
        }],
    };
    env.private_write("project-paths.json", &serde_json::to_vec(&cache).unwrap());

    env.run(br#"{"cwd":"/workspace/client/src"}"#);

    let connection = Database::open_ingress_writer(&env.app().join("cc-reminder.sqlite3")).unwrap();
    let json: String = connection
        .query_row("SELECT safe_envelope_json FROM ingress_events", [], |row| {
            row.get(0)
        })
        .unwrap();
    let safe: SafeIngressEvent = serde_json::from_str(&json).unwrap();
    assert_eq!(safe.project_id, Some(project_id));
    assert_eq!(safe.project_display_name.as_deref(), Some("client-app"));
    assert!(safe.cwd_fingerprint.is_none());

    connection
        .execute("DELETE FROM ingress_events", [])
        .unwrap();
    let bad = ProjectMatchCacheFile {
        version: 2,
        ..cache
    };
    env.private_write("project-paths.json", &serde_json::to_vec(&bad).unwrap());
    env.run(br#"{"cwd":"/workspace/client/src"}"#);
    let json: String = connection
        .query_row("SELECT safe_envelope_json FROM ingress_events", [], |row| {
            row.get(0)
        })
        .unwrap();
    let safe: SafeIngressEvent = serde_json::from_str(&json).unwrap();
    assert_eq!(safe.project_id, None);
    assert_eq!(safe.project_display_name.as_deref(), Some("src"));
}

#[test]
fn wrong_or_public_version_cache_drops_the_event() {
    for schema in [0, 2] {
        let env = HookEnvironment::new();
        env.version_cache(schema);
        Database::open(&env.app().join("cc-reminder.sqlite3")).unwrap();
        env.run(br#"{}"#);
        let connection =
            Database::open_ingress_writer(&env.app().join("cc-reminder.sqlite3")).unwrap();
        let count: i64 = connection
            .query_row("SELECT COUNT(*) FROM ingress_events", [], |row| row.get(0))
            .unwrap();
        assert_eq!(count, 0);
    }

    #[cfg(unix)]
    {
        let env = HookEnvironment::new();
        env.version_cache(1);
        std::fs::set_permissions(
            env.app().join("agent-versions.json"),
            std::fs::Permissions::from_mode(0o644),
        )
        .unwrap();
        Database::open(&env.app().join("cc-reminder.sqlite3")).unwrap();
        env.run(br#"{}"#);
        let connection =
            Database::open_ingress_writer(&env.app().join("cc-reminder.sqlite3")).unwrap();
        let count: i64 = connection
            .query_row("SELECT COUNT(*) FROM ingress_events", [], |row| row.get(0))
            .unwrap();
        assert_eq!(count, 0);
    }
}

#[cfg(unix)]
#[test]
#[ignore = "dedicated release IPC latency smoke"]
fn local_ipc_p95_is_under_one_hundred_milliseconds() {
    let root = tempfile::tempdir().unwrap();
    let endpoint = Endpoint::Unix(root.path().join("ipc/hook.sock"));
    let mut server = IpcServer::bind(endpoint.clone()).unwrap();
    let handler = std::thread::spawn(move || {
        for _ in 0..500 {
            let (_, response) = server.receiver.blocking_recv().unwrap();
            response
                .blocking_send(IngressResponse::Accepted {
                    event_id: Uuid::now_v7(),
                })
                .unwrap();
        }
    });
    let request = IngressRequest {
        protocol_version: IPC_PROTOCOL_VERSION,
        helper_version: "0.1.0".into(),
        command_fingerprint: "latency-smoke".into(),
        event: CapturedHookEvent {
            source: AgentKind::Codex,
            source_version: Version::new(0, 145, 0),
            source_event: "Stop".into(),
            occurred_at: chrono::Utc::now(),
            cwd: None,
            session_id: None,
            turn_id: None,
            model: None,
            permission_mode: None,
            public_fields: Default::default(),
            sensitive_fields: Default::default(),
        },
    };
    let mut samples = Vec::with_capacity(500);
    for _ in 0..500 {
        let started = std::time::Instant::now();
        assert!(matches!(
            cc_reminder_lib::ipc::send_ingress(&endpoint, &request),
            Ok(IngressResponse::Accepted { .. })
        ));
        samples.push(started.elapsed());
    }
    handler.join().unwrap();
    samples.sort_unstable();
    let p95 = samples[474];
    eprintln!("local IPC p95: {:.3} ms", p95.as_secs_f64() * 1_000.0);
    assert!(p95 < std::time::Duration::from_millis(100));
}
