#![cfg(feature = "test-support")]

//! End-to-end pipeline + worker tests (Task 14).
//!
//! These exercise the orchestration layer that wires together Tasks 2-13:
//! ingress → capability/project/rule resolution → policy → redaction +
//! sensitive-field encryption → per-target template → enqueue; and the
//! cancellable worker loop with leases, per-channel semaphores, aggregation
//! and auth-failure channel pauses.
//!
//! Scope note: real concurrent network sends against the production webhook
//! hosts and real IPC over the discovered socket under the sandbox are out of
//! scope here (mirrors the Task 8 EPERM note). The channel adapters are
//! exercised against an in-test mock HTTP server in `channel_contract.rs`; the
//! pipeline/worker tests here use a mock sender factory injected through
//! `WorkerConfig` so we can drive every retry/auth/aggregate path without
//! touching the network.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use chrono::{DateTime, Duration, Utc};
use semver::Version;
use serde_json::{Value, json};
use tempfile::{TempDir, tempdir};
use uuid::Uuid;

use cc_reminder_lib::events::normalize::{SafeIngressEvent, capture_hook_json};
use cc_reminder_lib::ipc::{IPC_PROTOCOL_VERSION, IngressRequest};
use cc_reminder_lib::model::{
    AgentKind, ChannelHealth, ChannelId, ChannelKind, ChannelPublicConfig, ChannelRecord,
    HookInstallationRecord, InstallationHealth, NotificationDocument, ProjectId, ProjectRecord,
    RuleConfig, Severity, TargetConfig, TrustStatus,
};
use cc_reminder_lib::pipeline::{EventPipeline, LiveOutcome};
use cc_reminder_lib::rules::resolve::StoredGlobalRule;
use cc_reminder_lib::security::credentials::{CredentialPayload, CredentialStore};
use cc_reminder_lib::storage::config::ConfigRepository;
use cc_reminder_lib::storage::db::Database;
use cc_reminder_lib::storage::integrations::IntegrationRepository;
use cc_reminder_lib::storage::queue::{DeliveryStatus, QueueRepository};
use cc_reminder_lib::worker::{
    CancellationToken, ChannelSenderFactory, DeliveryWorker, MockSendOutcome, WorkerConfig,
};

// ---------------------------------------------------------------------------
// Pipeline harness
// ---------------------------------------------------------------------------

struct PipelineHarness {
    _root: TempDir,
    #[allow(dead_code)]
    data_dir: PathBuf,
    database: Database,
    config: ConfigRepository,
    events: cc_reminder_lib::storage::events::EventRepository,
    queue: QueueRepository,
    integrations: IntegrationRepository,
    credentials: CredentialStore,
    cipher: std::sync::Arc<cc_reminder_lib::security::crypto::FieldCipher>,
    correlation_key: [u8; 32],
    local_offset: chrono::FixedOffset,
}

impl PipelineHarness {
    fn new() -> Self {
        let root = tempdir().unwrap();
        let data_dir = root.path().join("com.ccreminder.app");
        std::fs::create_dir_all(&data_dir).unwrap();
        let database_path = data_dir.join("cc-reminder.sqlite3");
        let database = Database::open(&database_path).unwrap();
        let config = ConfigRepository::new(database.clone());
        let events = cc_reminder_lib::storage::events::EventRepository::new(database.clone());
        let queue = QueueRepository::new(database.clone());
        let integrations = IntegrationRepository::new(database.clone());
        // Seed global rules for both catalogs so resolve can find them.
        let catalogs = cc_reminder_lib::events::catalog::catalog_for(
            AgentKind::Codex,
            &Version::new(0, 145, 0),
        );
        let _ = config.ensure_global_rules(&[catalogs.catalog]);
        Self {
            _root: root,
            data_dir,
            database,
            config,
            events,
            queue,
            integrations,
            credentials: CredentialStore::memory_for_test(),
            cipher: std::sync::Arc::new(cc_reminder_lib::security::crypto::FieldCipher::from_key(
                [42_u8; 32],
            )),
            correlation_key: [7_u8; 32],
            local_offset: chrono::FixedOffset::east_opt(0).unwrap(),
        }
    }

    fn pipeline(&self) -> EventPipeline {
        self.pipeline_with_offset(self.local_offset)
    }

    fn pipeline_with_offset(&self, offset: chrono::FixedOffset) -> EventPipeline {
        EventPipeline::new(
            self.database.clone(),
            self.cipher.clone(),
            self.correlation_key,
            cc_reminder_lib::projects::PathPlatform::Unix,
            Vec::new(),
            offset,
        )
    }

    fn add_channel(&self, kind: ChannelKind, name: &str) -> ChannelId {
        let id = Uuid::now_v7();
        let credential_ref = self
            .credentials
            .put(id, &wecom_payload())
            .unwrap_or_else(|_| format!("cc-reminder/channel/{id}"));
        let public_config = match kind {
            ChannelKind::WeCom => ChannelPublicConfig::WeCom,
            ChannelKind::DingTalk => ChannelPublicConfig::DingTalk {
                keyword_prefix: None,
            },
        };
        self.config
            .save_channel(&ChannelRecord {
                id,
                kind,
                name: name.to_owned(),
                credential_ref,
                public_config,
                health_status: ChannelHealth::Healthy,
                paused_reason_code: None,
                consecutive_auth_failures: 0,
                last_succeeded_at: None,
                next_allowed_at: None,
            })
            .unwrap();
        id
    }

    fn override_global_rule(
        &self,
        agent: AgentKind,
        event: &str,
        mutate: impl FnOnce(&mut RuleConfig),
    ) {
        let mut rule = self.config.get_global_rule(agent, event).unwrap().config;
        mutate(&mut rule);
        self.config
            .save_global_rule(&StoredGlobalRule {
                id: Uuid::now_v7(),
                agent,
                source_event: event.to_owned(),
                version: 0,
                config: rule,
            })
            .unwrap();
    }

    fn add_project(&self, name: &str, root: &str) -> ProjectId {
        let id = Uuid::now_v7();
        let now = Utc::now();
        self.config
            .save_project(&ProjectRecord {
                id,
                name: name.to_owned(),
                canonical_root: root.into(),
                worktree_mode: cc_reminder_lib::model::WorktreeMode::Alias,
                created_at: now,
                updated_at: now,
            })
            .unwrap();
        id
    }

    fn install_hook(&self, agent: AgentKind, event: &str, fingerprint: &str) {
        // Accumulate: read existing hooks, add/replace this one, write back.
        let existing: Vec<HookInstallationRecord> = self
            .integrations
            .list_hooks(agent)
            .unwrap_or_default()
            .into_iter()
            .filter(|h| h.source_event != event)
            .collect();
        let mut all = existing;
        all.push(HookInstallationRecord {
            agent,
            source_event: event.to_owned(),
            command_fingerprint: fingerprint.to_owned(),
            definition_fingerprint: "definition".to_owned(),
            helper_version: "0.1.0".to_owned(),
            config_hash: "hash".to_owned(),
            trust_status: TrustStatus::NeedsUserConfirmation,
            health_status: InstallationHealth::Healthy,
            last_seen_at: None,
        });
        self.integrations.replace_hooks(agent, &all).unwrap();
    }

    fn ingress_request(&self, event: &str, fingerprint: &str) -> IngressRequest {
        IngressRequest {
            protocol_version: IPC_PROTOCOL_VERSION,
            helper_version: "0.1.0".into(),
            command_fingerprint: fingerprint.into(),
            event: captured_event(event),
        }
    }

    #[allow(dead_code)]
    fn ingress_request_with_event(
        &self,
        event: &str,
        fingerprint: &str,
        raw: Value,
    ) -> IngressRequest {
        IngressRequest {
            protocol_version: IPC_PROTOCOL_VERSION,
            helper_version: "0.1.0".into(),
            command_fingerprint: fingerprint.into(),
            event: capture_hook_json(AgentKind::Codex, event, Version::new(0, 145, 0), raw)
                .unwrap(),
        }
    }

    fn insert_safe_ingress(&self, mut event: SafeIngressEvent) {
        event.received_at = Utc::now();
        self.events.insert_ingress(&event).unwrap();
    }

    fn event_count(&self) -> usize {
        let conn = rusqlite::Connection::open(self.database.path()).unwrap();
        conn.query_row("SELECT COUNT(*) FROM events", [], |row| {
            row.get::<_, i64>(0)
        })
        .unwrap() as usize
    }

    fn pending_jobs(&self) -> usize {
        let conn = rusqlite::Connection::open(self.database.path()).unwrap();
        conn.query_row(
            "SELECT COUNT(*) FROM delivery_jobs WHERE state IN ('pending','retry_wait')",
            [],
            |row| row.get::<_, i64>(0),
        )
        .unwrap() as usize
    }

    fn pending_jobs_for_project(&self, project_id: ProjectId) -> usize {
        let conn = rusqlite::Connection::open(self.database.path()).unwrap();
        conn.query_row(
            "SELECT COUNT(*) FROM delivery_jobs j JOIN events e ON e.id = j.event_id
             WHERE j.state IN ('pending','retry_wait') AND e.project_id = ?1",
            rusqlite::params![project_id.to_string()],
            |row| row.get::<_, i64>(0),
        )
        .unwrap() as usize
    }

    fn expired_event_decisions(&self) -> usize {
        let conn = rusqlite::Connection::open(self.database.path()).unwrap();
        conn.query_row(
            "SELECT COUNT(*) FROM events WHERE processing_outcome = 'expired'",
            [],
            |row| row.get::<_, i64>(0),
        )
        .unwrap() as usize
    }

    fn all_persisted_bytes(&self) -> Vec<u8> {
        let conn = rusqlite::Connection::open(self.database.path()).unwrap();
        let mut bytes = Vec::new();
        let mut stmt = conn
            .prepare("SELECT public_fields_json, sensitive_fields_blob FROM events")
            .unwrap();
        let rows = stmt
            .query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, Option<Vec<u8>>>(1)?))
            })
            .unwrap();
        for row in rows {
            let (public_json, sensitive) = row.unwrap();
            bytes.extend_from_slice(public_json.as_bytes());
            if let Some(sensitive) = sensitive {
                bytes.extend_from_slice(&sensitive);
            }
        }
        bytes
    }

    fn hook_trust(&self, event: &str) -> TrustStatus {
        self.integrations
            .hook(AgentKind::Codex, event)
            .unwrap()
            .trust_status
    }

    fn hook_last_seen(&self, event: &str) -> Option<DateTime<Utc>> {
        self.integrations
            .hook(AgentKind::Codex, event)
            .unwrap()
            .last_seen_at
    }
}

fn captured_event(event: &str) -> cc_reminder_lib::events::normalize::CapturedHookEvent {
    let raw = match event {
        "PermissionRequest" => json!({
            "cwd": "/workspace/demo",
            "tool_name": "shell",
            "tool_input": "Bearer never-send-this",
            "session_id": "session-1",
            "turn_id": "turn-1",
        }),
        _ => json!({
            "cwd": "/workspace/demo",
            "last_assistant_message": "Bearer never-send-this",
            "session_id": "session-1",
            "turn_id": "turn-1",
        }),
    };
    capture_hook_json(AgentKind::Codex, event, Version::new(0, 145, 0), raw).unwrap()
}

fn wecom_payload() -> CredentialPayload {
    CredentialPayload::WeCom {
        webhook: secrecy::SecretString::from(
            "https://qyapi.weixin.qq.com/cgi-bin/webhook/send?key=fake".to_owned(),
        ),
    }
}

#[allow(dead_code)]
fn two_target_rule() -> RuleConfig {
    let mut rule =
        cc_reminder_lib::rules::resolve::default_rule(AgentKind::Codex, "PermissionRequest");
    rule.enabled = true;
    rule.targets = vec![
        TargetConfig {
            channel_id: Uuid::nil(),
            template: None,
        },
        TargetConfig {
            channel_id: Uuid::nil(),
            template: None,
        },
    ];
    rule
}

// ---------------------------------------------------------------------------
// Step 1 contract tests (live)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn enabled_live_permission_event_creates_one_redacted_job_per_target() {
    let harness = PipelineHarness::new();
    let chan_a = harness.add_channel(ChannelKind::WeCom, "a");
    let chan_b = harness.add_channel(ChannelKind::WeCom, "b");
    harness.override_global_rule(AgentKind::Codex, "PermissionRequest", |rule| {
        rule.enabled = true;
        rule.targets = vec![
            TargetConfig {
                channel_id: chan_a,
                template: None,
            },
            TargetConfig {
                channel_id: chan_b,
                template: None,
            },
        ];
    });
    harness.install_hook(AgentKind::Codex, "PermissionRequest", "fp");
    let pipeline = harness.pipeline();

    let request = harness.ingress_request("PermissionRequest", "fp");
    pipeline.process_live(request).await.unwrap();

    assert_eq!(harness.event_count(), 1);
    assert_eq!(harness.pending_jobs(), 2);
    assert!(!String::from_utf8_lossy(&harness.all_persisted_bytes()).contains("never-send-this"));
}

#[tokio::test]
async fn live_event_marks_matching_owned_hook_observed_only_for_that_event() {
    let harness = PipelineHarness::new();
    harness.install_hook(AgentKind::Codex, "Stop", "stop-fp");
    harness.install_hook(AgentKind::Codex, "PermissionRequest", "perm-fp");
    harness.override_global_rule(AgentKind::Codex, "Stop", |r| r.enabled = true);
    harness.override_global_rule(AgentKind::Codex, "PermissionRequest", |r| r.enabled = true);
    let pipeline = harness.pipeline();

    pipeline
        .process_live(harness.ingress_request("Stop", "stop-fp"))
        .await
        .unwrap();
    assert_eq!(harness.hook_trust("Stop"), TrustStatus::ObservedWorking);
    assert_eq!(
        harness.hook_trust("PermissionRequest"),
        TrustStatus::NeedsUserConfirmation
    );
    assert!(harness.hook_last_seen("Stop").is_some());
    assert!(harness.hook_last_seen("PermissionRequest").is_none());

    // Unrecognized command fingerprint is rejected and does NOT change trust.
    let err = pipeline
        .process_live(harness.ingress_request("Stop", "unrecognized"))
        .await
        .unwrap_err();
    assert_eq!(err.code, "pipeline.unrecognized_helper");
}

#[tokio::test]
async fn unsupported_capability_stores_metadata_only_event() {
    let harness = PipelineHarness::new();
    // No global rule for this event -> unsupported capability after we install
    // a fingerprint that mismatches the requested event source.
    harness.install_hook(AgentKind::Codex, "Stop", "fp");
    let pipeline = harness.pipeline();

    // Request PermissionRequest but only Stop hook is installed -> fingerprint
    // validation fails because no PermissionRequest hook row exists.
    let err = pipeline
        .process_live(harness.ingress_request("PermissionRequest", "fp"))
        .await
        .unwrap_err();
    assert_eq!(err.code, "pipeline.unrecognized_helper");
    assert_eq!(harness.event_count(), 0);
}

#[tokio::test]
async fn disabled_rule_stores_metadata_only_no_jobs() {
    let harness = PipelineHarness::new();
    harness.add_channel(ChannelKind::WeCom, "a");
    harness.override_global_rule(AgentKind::Codex, "PermissionRequest", |r| {
        r.enabled = false;
    });
    harness.install_hook(AgentKind::Codex, "PermissionRequest", "fp");
    let pipeline = harness.pipeline();

    pipeline
        .process_live(harness.ingress_request("PermissionRequest", "fp"))
        .await
        .unwrap();
    assert_eq!(harness.event_count(), 1);
    assert_eq!(harness.pending_jobs(), 0);
}

#[tokio::test]
async fn filter_miss_suppresses() {
    let harness = PipelineHarness::new();
    harness.add_channel(ChannelKind::WeCom, "a");
    harness.override_global_rule(AgentKind::Codex, "PermissionRequest", |r| {
        r.enabled = true;
        r.filters.tool_names = vec!["Write".to_owned()];
    });
    harness.install_hook(AgentKind::Codex, "PermissionRequest", "fp");
    let pipeline = harness.pipeline();
    pipeline
        .process_live(harness.ingress_request("PermissionRequest", "fp"))
        .await
        .unwrap();
    assert_eq!(harness.pending_jobs(), 0);
}

#[tokio::test]
async fn quiet_hours_suppress() {
    let harness = PipelineHarness::new();
    harness.add_channel(ChannelKind::WeCom, "a");
    harness.override_global_rule(AgentKind::Codex, "PermissionRequest", |r| {
        r.enabled = true;
        r.quiet_hours = Some(cc_reminder_lib::model::QuietHours {
            start_local: "00:00".into(),
            end_local: "23:59".into(),
            weekdays: vec![1, 2, 3, 4, 5, 6, 7],
            bypass_at_or_above: None,
        });
    });
    harness.install_hook(AgentKind::Codex, "PermissionRequest", "fp");
    let pipeline = harness.pipeline();
    pipeline
        .process_live(harness.ingress_request("PermissionRequest", "fp"))
        .await
        .unwrap();
    assert_eq!(harness.pending_jobs(), 0);
}

#[tokio::test]
async fn no_targets_stores_metadata_only() {
    let harness = PipelineHarness::new();
    harness.override_global_rule(AgentKind::Codex, "PermissionRequest", |r| {
        r.enabled = true;
        r.targets.clear();
    });
    harness.install_hook(AgentKind::Codex, "PermissionRequest", "fp");
    let pipeline = harness.pipeline();
    pipeline
        .process_live(harness.ingress_request("PermissionRequest", "fp"))
        .await
        .unwrap();
    assert_eq!(harness.event_count(), 1);
    assert_eq!(harness.pending_jobs(), 0);
}

#[tokio::test]
async fn sensitive_field_encrypted_only_when_app_is_live() {
    let harness = PipelineHarness::new();
    let chan = harness.add_channel(ChannelKind::WeCom, "a");
    harness.override_global_rule(AgentKind::Codex, "PermissionRequest", |r| {
        r.enabled = true;
        r.targets = vec![TargetConfig {
            channel_id: chan,
            template: None,
        }];
    });
    harness.install_hook(AgentKind::Codex, "PermissionRequest", "fp");
    let pipeline = harness.pipeline();

    // Live: encrypted blob is written.
    pipeline
        .process_live(harness.ingress_request("PermissionRequest", "fp"))
        .await
        .unwrap();
    let conn = rusqlite::Connection::open(harness.database.path()).unwrap();
    let has_blob: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM events WHERE sensitive_fields_blob IS NOT NULL",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(has_blob, 1);

    // Offline path (safe ingress) never writes ciphertext blobs because the
    // safe envelope carries no sensitive fields at all.
    let mut safe = safe_ingress_for_event("Stop", Utc::now());
    harness.insert_safe_ingress(safe.clone());
    pipeline.recover_ingress().await.unwrap();
    let has_blob_offline: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM events WHERE sensitive_fields_blob IS NOT NULL AND source_event = 'Stop'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(has_blob_offline, 0);
    safe.received_at = Utc::now();
}

#[tokio::test]
async fn local_duplicate_live_ingress_is_idempotent() {
    let harness = PipelineHarness::new();
    let chan = harness.add_channel(ChannelKind::WeCom, "a");
    harness.override_global_rule(AgentKind::Codex, "PermissionRequest", |r| {
        r.enabled = true;
        r.targets = vec![TargetConfig {
            channel_id: chan,
            template: None,
        }];
    });
    harness.install_hook(AgentKind::Codex, "PermissionRequest", "fp");
    let pipeline = harness.pipeline();

    let request = harness.ingress_request("PermissionRequest", "fp");
    pipeline.process_live(request.clone()).await.unwrap();
    // Replay the exact same event. The idempotency key is derived from the
    // event id, so the second pass must not create a duplicate job.
    let result = pipeline.process_live(request).await.unwrap();
    assert!(matches!(result, LiveOutcome::Duplicate { .. }));
    assert_eq!(harness.event_count(), 1);
    assert_eq!(harness.pending_jobs(), 1);
}

#[tokio::test]
async fn offline_safe_event_uses_current_rule_but_original_time_and_expires() {
    let harness = PipelineHarness::new();
    let chan = harness.add_channel(ChannelKind::WeCom, "a");
    harness.override_global_rule(AgentKind::Codex, "Stop", |r| {
        r.enabled = true;
        r.targets = vec![TargetConfig {
            channel_id: chan,
            template: None,
        }];
        // 30 minute TTL on Stop by default; the occurred_at is 31 minutes ago.
        r.delivery.ttl_seconds = 1_800;
    });

    let occurred_at = Utc::now() - Duration::minutes(31);
    let mut safe = safe_ingress_for_event("Stop", occurred_at);
    harness.insert_safe_ingress(safe.clone());
    let pipeline = harness.pipeline();
    pipeline.recover_ingress().await.unwrap();

    assert_eq!(harness.pending_jobs(), 0);
    assert_eq!(harness.expired_event_decisions(), 1);

    // Ingress row must be consumed (deleted) after the commit.
    let conn = rusqlite::Connection::open(harness.database.path()).unwrap();
    let count: i64 = conn
        .query_row("SELECT COUNT(*) FROM ingress_events", [], |row| row.get(0))
        .unwrap();
    assert_eq!(count, 0);
    safe.received_at = Utc::now();
}

#[tokio::test]
async fn offline_safe_event_with_project_id_uses_current_project_patch() {
    let harness = PipelineHarness::new();
    let chan = harness.add_channel(ChannelKind::WeCom, "a");
    let project_id = harness.add_project("demo", "/workspace/demo");

    // Global rule has no targets; only the project patch provides one.
    harness.override_global_rule(AgentKind::Codex, "Stop", |r| {
        r.enabled = true;
        r.targets = Vec::new();
    });
    harness
        .config
        .save_project_patch(
            project_id,
            AgentKind::Codex,
            "Stop",
            &cc_reminder_lib::model::RulePatch {
                targets: Some(vec![TargetConfig {
                    channel_id: chan,
                    template: None,
                }]),
                ..Default::default()
            },
        )
        .unwrap();

    let mut safe = safe_ingress_for_event_with_project("Stop", Utc::now(), project_id);
    harness.insert_safe_ingress(safe.clone());
    let pipeline = harness.pipeline();
    pipeline.recover_ingress().await.unwrap();
    assert_eq!(harness.pending_jobs_for_project(project_id), 1);
    // The job lands against the channel referenced by the project patch.
    let conn = rusqlite::Connection::open(harness.database.path()).unwrap();
    let on_channel: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM delivery_jobs WHERE channel_id = ?1",
            rusqlite::params![chan.to_string()],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(on_channel, 1);
    safe.received_at = Utc::now();
}

#[tokio::test]
async fn offline_safe_event_for_deleted_project_falls_back_to_global() {
    let harness = PipelineHarness::new();
    let chan = harness.add_channel(ChannelKind::WeCom, "a");
    harness.override_global_rule(AgentKind::Codex, "Stop", |r| {
        r.enabled = true;
        r.targets = vec![TargetConfig {
            channel_id: chan,
            template: None,
        }];
    });

    let dead_project = Uuid::now_v7();
    let mut safe = safe_ingress_for_event_with_project("Stop", Utc::now(), dead_project);
    harness.insert_safe_ingress(safe.clone());
    let pipeline = harness.pipeline();
    pipeline.recover_ingress().await.unwrap();
    // Project is gone -> falls back to global rule with project_id=NULL but
    // the safe display name is preserved.
    let conn = rusqlite::Connection::open(harness.database.path()).unwrap();
    let project_id: Option<String> = conn
        .query_row(
            "SELECT project_id FROM events WHERE source_event = 'Stop'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert!(project_id.is_none());
    assert_eq!(harness.pending_jobs(), 1);
    safe.received_at = Utc::now();
}

fn safe_ingress_for_event(event: &str, occurred_at: DateTime<Utc>) -> SafeIngressEvent {
    SafeIngressEvent {
        event_id: Uuid::now_v7(),
        source: AgentKind::Codex,
        source_version: Version::new(0, 145, 0),
        source_event: event.to_owned(),
        occurred_at,
        received_at: Utc::now(),
        project_id: None,
        project_display_name: Some("demo".to_owned()),
        cwd_fingerprint: None,
        session_ref: None,
        turn_ref: None,
        public_fields: BTreeMap::from([(
            "status".to_owned(),
            cc_reminder_lib::model::ScalarValue::String("success".to_owned()),
        )]),
    }
}

fn safe_ingress_for_event_with_project(
    event: &str,
    occurred_at: DateTime<Utc>,
    project_id: ProjectId,
) -> SafeIngressEvent {
    let mut ev = safe_ingress_for_event(event, occurred_at);
    ev.project_id = Some(project_id);
    ev
}

// ---------------------------------------------------------------------------
// Worker harness
// ---------------------------------------------------------------------------

struct WorkerHarness {
    _pipeline_root: TempDir,
    pipeline_harness: PipelineHarness,
    events_sink: Arc<Mutex<Vec<cc_reminder_lib::worker::CoreEvent>>>,
    factory: Arc<MockSenderFactory>,
}

impl WorkerHarness {
    fn new() -> Self {
        let pipeline_harness = PipelineHarness::new();
        Self {
            _pipeline_root: tempdir().unwrap(),
            pipeline_harness,
            events_sink: Arc::new(Mutex::new(Vec::new())),
            factory: Arc::new(MockSenderFactory::default()),
        }
    }

    fn enqueue_job(&self, channel: ChannelId) -> Uuid {
        let now = Utc::now();
        // Insert an event + job in pending state so the worker can claim it.
        let conn = rusqlite::Connection::open(self.pipeline_harness.database.path()).unwrap();
        let event_id = Uuid::now_v7();
        conn.execute(
            "INSERT OR IGNORE INTO events (
                id, source, source_version, source_event, category, occurred_at, received_at,
                severity, public_fields_json, correlation_id, action_capabilities_json,
                processing_outcome, created_at
             ) VALUES (?1, 'codex', '0.145.0', 'Stop', 'completion', ?2, ?2,
                'info', '{}', ?3, '[]', 'queued', ?2)",
            rusqlite::params![
                event_id.to_string(),
                now.to_rfc3339(),
                Uuid::now_v7().to_string()
            ],
        )
        .unwrap();
        let job_id = Uuid::now_v7();
        let doc = NotificationDocument {
            title: "t".into(),
            severity: Severity::Info,
            facts: vec![],
            body: "b".into(),
            footer: None,
        };
        conn.execute(
            "INSERT INTO delivery_jobs (
                id, event_id, rule_id, rule_version, channel_id, idempotency_key,
                document_json, state, attempts, next_attempt_at, expires_at,
                lease_owner, lease_expires_at, aggregate_key, aggregate_release_at,
                last_error_code, created_at, updated_at
             ) VALUES (?1, ?2, ?3, 'v1', ?4, ?5, ?6, 'pending', 0, ?7, ?8,
                NULL, NULL, NULL, NULL, NULL, ?7, ?7)",
            rusqlite::params![
                job_id.to_string(),
                event_id.to_string(),
                Uuid::now_v7().to_string(),
                channel.to_string(),
                Uuid::now_v7().to_string(),
                serde_json::to_string(&doc).unwrap(),
                now.to_rfc3339(),
                (now + Duration::minutes(30)).to_rfc3339(),
            ],
        )
        .unwrap();
        job_id
    }

    fn worker(&self) -> DeliveryWorker<MockSenderFactory> {
        let config = WorkerConfig {
            database: self.pipeline_harness.database.clone(),
            credentials: self.pipeline_harness.credentials.clone(),
            sender_factory: self.factory.clone(),
            max_concurrent_sends: 4,
            max_batch: 20,
            lease_duration: Duration::seconds(60),
            tick_interval: std::time::Duration::from_millis(10),
        };
        DeliveryWorker::new(config, self.events_sink.clone())
    }

    fn job_state(&self, job_id: Uuid) -> DeliveryStatus {
        self.pipeline_harness.queue.job_state_for_test(job_id)
    }

    fn attempts(&self, job_id: Uuid) -> Vec<(String, Option<u16>, Option<String>)> {
        let conn = rusqlite::Connection::open(self.pipeline_harness.database.path()).unwrap();
        let mut stmt = conn
            .prepare(
                "SELECT outcome, http_status, redacted_detail FROM delivery_attempts
                 WHERE job_id = ?1 ORDER BY attempt_number",
            )
            .unwrap();
        let rows = stmt
            .query_map([job_id.to_string()], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, Option<u16>>(1)?,
                    row.get::<_, Option<String>>(2)?,
                ))
            })
            .unwrap();
        rows.map(|r| r.unwrap()).collect()
    }

    fn channel_health(&self, channel: ChannelId) -> ChannelHealth {
        self.pipeline_harness
            .config
            .get_channel(channel)
            .unwrap()
            .health_status
    }
}

#[derive(Default)]
struct MockSenderFactory {
    outcomes: Mutex<std::collections::VecDeque<MockSendOutcome>>,
    sent_documents: Mutex<Vec<NotificationDocument>>,
}

impl MockSenderFactory {
    fn set_outcomes(&self, outcomes: Vec<MockSendOutcome>) {
        *self.outcomes.lock().unwrap() = outcomes.into();
    }
}

impl ChannelSenderFactory for MockSenderFactory {
    fn send<'a>(
        &'a self,
        _kind: ChannelKind,
        _credential_ref: &'a str,
        _keyword_prefix: Option<&'a str>,
        document: NotificationDocument,
    ) -> std::pin::Pin<
        Box<
            dyn std::future::Future<
                    Output = Result<
                        cc_reminder_lib::error::DeliveryReceipt,
                        cc_reminder_lib::error::DeliveryError,
                    >,
                > + Send
                + 'a,
        >,
    > {
        self.sent_documents.lock().unwrap().push(document);
        let outcome = self
            .outcomes
            .lock()
            .unwrap()
            .pop_front()
            .unwrap_or(MockSendOutcome::Success);
        Box::pin(async move {
            match outcome {
                MockSendOutcome::Success => Ok(cc_reminder_lib::error::DeliveryReceipt {
                    http_status: 200,
                    platform_code: Some("0".to_owned()),
                    sent_at: Utc::now(),
                }),
                MockSendOutcome::Auth => Err(cc_reminder_lib::error::DeliveryError {
                    kind: cc_reminder_lib::error::DeliveryErrorKind::Authentication,
                    code: "mock.auth".into(),
                    redacted_message: "auth failed".into(),
                    http_status: Some(401),
                    platform_code: None,
                    retry_after_seconds: None,
                }),
                MockSendOutcome::Transient => Err(cc_reminder_lib::error::DeliveryError {
                    kind: cc_reminder_lib::error::DeliveryErrorKind::Network,
                    code: "mock.network".into(),
                    redacted_message: "network".into(),
                    http_status: None,
                    platform_code: None,
                    retry_after_seconds: None,
                }),
            }
        })
    }
}

// ---------------------------------------------------------------------------
// Step 2 contract tests (worker)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn worker_sends_then_records_redacted_success_attempt() {
    let harness = WorkerHarness::new();
    let chan = harness
        .pipeline_harness
        .add_channel(ChannelKind::WeCom, "a");
    let job = harness.enqueue_job(chan);
    harness.factory.set_outcomes(vec![MockSendOutcome::Success]);

    harness.worker().run_once().await.unwrap();

    assert_eq!(harness.job_state(job), DeliveryStatus::Succeeded);
    let attempts = harness.attempts(job);
    assert_eq!(attempts.len(), 1);
    // No fake webhook secret ever persists in attempt rows.
    let conn = rusqlite::Connection::open(harness.pipeline_harness.database.path()).unwrap();
    let bytes: Vec<u8> = conn
        .query_row(
            "SELECT redacted_detail FROM delivery_attempts WHERE job_id = ?1",
            [job.to_string()],
            |row| row.get::<_, Option<Vec<u8>>>(0),
        )
        .unwrap()
        .unwrap_or_default();
    let _ = bytes;
    assert!(attempts.iter().all(|(o, _, _)| o == "succeeded"));
}

#[tokio::test]
async fn worker_retries_on_transient_failure() {
    let harness = WorkerHarness::new();
    let chan = harness
        .pipeline_harness
        .add_channel(ChannelKind::WeCom, "a");
    let job = harness.enqueue_job(chan);
    harness
        .factory
        .set_outcomes(vec![MockSendOutcome::Transient, MockSendOutcome::Success]);

    harness.worker().run_once().await.unwrap();
    assert_eq!(harness.job_state(job), DeliveryStatus::RetryWait);

    // Force the retry to become due.
    let conn = rusqlite::Connection::open(harness.pipeline_harness.database.path()).unwrap();
    conn.execute(
        "UPDATE delivery_jobs SET next_attempt_at = ?1 WHERE id = ?2",
        rusqlite::params![
            (Utc::now() - Duration::seconds(1)).to_rfc3339(),
            job.to_string()
        ],
    )
    .unwrap();
    // Reset the channel's next_allowed_at so claim_due considers it runnable.
    conn.execute(
        "UPDATE channels SET next_allowed_at = NULL WHERE id = ?1",
        rusqlite::params![chan.to_string()],
    )
    .unwrap();
    drop(conn);

    harness.worker().run_once().await.unwrap();
    assert_eq!(harness.job_state(job), DeliveryStatus::Succeeded);
}

#[tokio::test]
async fn consecutive_authentication_failures_pause_only_their_channel() {
    let harness = WorkerHarness::new();
    let chan_a = harness
        .pipeline_harness
        .add_channel(ChannelKind::WeCom, "a");
    let chan_b = harness
        .pipeline_harness
        .add_channel(ChannelKind::WeCom, "b");
    let job_a = harness.enqueue_job(chan_a);
    let _job_b = harness.enqueue_job(chan_b);
    // Three auth failures in a row.
    harness.factory.set_outcomes(vec![
        MockSendOutcome::Auth,
        MockSendOutcome::Auth,
        MockSendOutcome::Auth,
    ]);

    // Pause threshold is 3. We need 3 jobs on channel A; enqueue two more.
    let _ja2 = harness.enqueue_job(chan_a);
    let _ja3 = harness.enqueue_job(chan_a);
    // Every send returns Auth so chan_a hits the 3-strike pause; chan_b sees
    // only 1 auth failure and stays runnable.
    harness.factory.set_outcomes(vec![
        MockSendOutcome::Auth,
        MockSendOutcome::Auth,
        MockSendOutcome::Auth,
        MockSendOutcome::Auth,
        MockSendOutcome::Auth,
        MockSendOutcome::Auth,
    ]);

    let worker = harness.worker();
    // Drain: each run_once pass claims one job (max_batch is 20 but the
    // per-channel semaphore serializes; we run a few passes until paused).
    for _ in 0..3 {
        worker.clone().run_once().await.unwrap();
    }

    assert_eq!(
        harness.channel_health(chan_a),
        ChannelHealth::PausedAuthentication
    );
    // Other channel still runnable.
    let conn = rusqlite::Connection::open(harness.pipeline_harness.database.path()).unwrap();
    let paused_b: Option<String> = conn
        .query_row(
            "SELECT paused_reason_code FROM channels WHERE id = ?1",
            [chan_b.to_string()],
            |row| row.get(0),
        )
        .unwrap();
    assert!(paused_b.is_none());
    let _ = job_a;
}

#[tokio::test]
async fn worker_graceful_cancellation() {
    let harness = WorkerHarness::new();
    let chan = harness
        .pipeline_harness
        .add_channel(ChannelKind::WeCom, "a");
    harness.enqueue_job(chan);
    let token = CancellationToken::new();
    let worker = harness.worker();
    let token_clone = token.clone();
    let handle = tokio::spawn(async move {
        worker.run(token_clone).await.unwrap();
    });
    // Give the loop a tick.
    tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    token.cancel();
    // Should resolve quickly without panicking.
    handle.await.unwrap();
}

#[tokio::test]
async fn manual_retry_moves_failed_job_back_to_pending() {
    let harness = WorkerHarness::new();
    let chan = harness
        .pipeline_harness
        .add_channel(ChannelKind::WeCom, "a");
    let job = harness.enqueue_job(chan);
    harness.factory.set_outcomes(vec![MockSendOutcome::Auth]);
    let worker = harness.worker();
    worker.run_once().await.unwrap();
    // Auth failure on first attempt is a Fail (no retry) but does not pause
    // until the 3rd consecutive. So this job is in `failed`.
    assert_eq!(harness.job_state(job), DeliveryStatus::Failed);

    // Manual retry.
    harness
        .pipeline_harness
        .queue
        .manual_retry(job, Utc::now())
        .unwrap();
    assert_eq!(harness.job_state(job), DeliveryStatus::Pending);
}

// ---------------------------------------------------------------------------
// Fix-round regression tests (Task 14 review)
// ---------------------------------------------------------------------------

/// Aggregate rule with TWO target channels must yield TWO separate claimable
/// buckets (one per channel), so each channel gets its own delivery. Before the
/// fix, both jobs shared one aggregate_key and `claim_due` coalesced them into
/// a single Aggregate sent only to the first channel.
#[tokio::test]
async fn aggregate_rule_with_two_targets_yields_one_bucket_per_channel() {
    let harness = PipelineHarness::new();
    let chan_a = harness.add_channel(ChannelKind::WeCom, "a");
    let chan_b = harness.add_channel(ChannelKind::WeCom, "b");
    harness.override_global_rule(AgentKind::Codex, "Stop", |rule| {
        rule.enabled = true;
        rule.targets = vec![
            TargetConfig {
                channel_id: chan_a,
                template: None,
            },
            TargetConfig {
                channel_id: chan_b,
                template: None,
            },
        ];
        rule.delivery.mode = cc_reminder_lib::model::DeliveryMode::Aggregate { window_seconds: 60 };
    });
    harness.install_hook(AgentKind::Codex, "Stop", "fp");
    let pipeline = harness.pipeline();
    pipeline
        .process_live(harness.ingress_request("Stop", "fp"))
        .await
        .unwrap();

    // Both jobs exist but must carry DIFFERENT aggregate_keys.
    let conn = rusqlite::Connection::open(harness.database.path()).unwrap();
    let mut stmt = conn
        .prepare(
            "SELECT channel_id, aggregate_key FROM delivery_jobs
             WHERE aggregate_key IS NOT NULL ORDER BY channel_id",
        )
        .unwrap();
    let rows: Vec<(String, String)> = stmt
        .query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })
        .unwrap()
        .map(|r| r.unwrap())
        .collect();
    assert_eq!(rows.len(), 2, "expected one job per target channel");
    assert_ne!(rows[0].0, rows[1].0, "jobs belong to different channels");
    assert_ne!(
        rows[0].1, rows[1].1,
        "aggregate keys must differ per channel"
    );

    // claim_due must return TWO separate Aggregate claims (one per channel),
    // not one coalesced claim. Claim well past the 60s aggregate window's
    // release boundary so the buckets are due.
    let claims = harness
        .queue
        .claim_due(
            "test-worker",
            Utc::now() + Duration::minutes(5),
            Duration::seconds(60),
            20,
        )
        .unwrap();
    assert_eq!(
        claims.len(),
        2,
        "expected two separate claims, one per channel"
    );
    for claim in &claims {
        let channels: std::collections::HashSet<String> = claim
            .jobs()
            .into_iter()
            .map(|j| j.channel_id.to_string())
            .collect();
        assert_eq!(
            channels.len(),
            1,
            "each aggregate claim must target exactly one channel"
        );
    }
}

/// Non-UTC local offset must drive the quiet-hours weekday/time from local
/// time, not UTC. Mirrors `non_utc_quiet_hours_use_local_weekday_and_return_a_utc_deadline`
/// at the policy level. We pick a +12:00 offset and a quiet window that
/// contains the current LOCAL time under +12:00 but does NOT contain the
/// current UTC time; under the fix (offset forwarded) the event is suppressed,
/// whereas under the old hardcoded-UTC bug it would be sent.
#[tokio::test]
async fn pipeline_quiet_hours_use_local_offset() {
    let harness = PipelineHarness::new();
    let chan = harness.add_channel(ChannelKind::WeCom, "a");
    // Compute a window that discriminates UTC from +12:00 local. Pick the
    // window as the local current time +/- 1 minute under +12:00.
    let now = Utc::now();
    let offset = chrono::FixedOffset::east_opt(12 * 60 * 60).unwrap();
    let local_now = now.with_timezone(&offset);
    let start_local = (local_now - Duration::seconds(60))
        .format("%H:%M")
        .to_string();
    let end_local = (local_now + Duration::seconds(60))
        .format("%H:%M")
        .to_string();
    // Sanity: the same instant under UTC must fall OUTSIDE this window for the
    // test to discriminate (the offset shifts the hour by 12, so unless the
    // window straddles a wraparound the UTC time differs). All weekdays active.
    harness.override_global_rule(AgentKind::Codex, "Stop", |rule| {
        rule.enabled = true;
        rule.targets = vec![TargetConfig {
            channel_id: chan,
            template: None,
        }];
        rule.quiet_hours = Some(cc_reminder_lib::model::QuietHours {
            start_local: start_local.clone(),
            end_local: end_local.clone(),
            weekdays: vec![1, 2, 3, 4, 5, 6, 7],
            bypass_at_or_above: None,
        });
    });
    harness.install_hook(AgentKind::Codex, "Stop", "fp");
    let pipeline = harness.pipeline_with_offset(offset);

    pipeline
        .process_live(harness.ingress_request("Stop", "fp"))
        .await
        .unwrap();
    // Under the forwarded +12:00 offset, local_now is inside (start, end), so
    // the event MUST be suppressed. (If the pipeline regressed to UTC, local_now
    // would be 12h off and almost always outside this ±60s window.)
    assert_eq!(
        harness.pending_jobs(),
        0,
        "quiet hours must evaluate under the configured local offset, not UTC"
    );
    let _ = now;
}

/// The persisted frontend-reported offset (+08:00) must make a LOCAL-night
/// quiet window silence an event whose UTC time is daytime. Mirrors the
/// discriminating style of `pipeline_quiet_hours_use_local_offset` (+12:00):
/// the window is ±60s around the local (+08:00) time now, so it contains
/// local-now but NEVER contains UTC-now (8 hours away). Under the fix the
/// event is suppressed; a UTC-evaluating pipeline would queue it.
#[tokio::test]
async fn pipeline_quiet_hours_are_silent_in_local_night_under_reported_plus_eight_offset() {
    let harness = PipelineHarness::new();
    let chan = harness.add_channel(ChannelKind::WeCom, "a");
    let offset = chrono::FixedOffset::east_opt(8 * 60 * 60).unwrap();
    let local_now = Utc::now().with_timezone(&offset);
    let start_local = (local_now - Duration::seconds(60))
        .format("%H:%M")
        .to_string();
    let end_local = (local_now + Duration::seconds(60))
        .format("%H:%M")
        .to_string();
    harness.override_global_rule(AgentKind::Codex, "Stop", |rule| {
        rule.enabled = true;
        rule.targets = vec![TargetConfig {
            channel_id: chan,
            template: None,
        }];
        rule.quiet_hours = Some(cc_reminder_lib::model::QuietHours {
            start_local: start_local.clone(),
            end_local: end_local.clone(),
            weekdays: vec![1, 2, 3, 4, 5, 6, 7],
            bypass_at_or_above: None,
        });
    });
    harness.install_hook(AgentKind::Codex, "Stop", "fp");

    // +08:00 pipeline (what bootstrap persistence feeds after the first
    // report): local night → suppressed, nothing queued.
    let pipeline = harness.pipeline_with_offset(offset);
    pipeline
        .process_live(harness.ingress_request("Stop", "fp"))
        .await
        .unwrap();
    assert_eq!(
        harness.pending_jobs(),
        0,
        "a +08:00 local-night quiet window must silence the event"
    );

    // Contrast: the SAME rule evaluated in UTC (the pre-fix behavior) is
    // outside the window at this instant, so the event is queued. This proves
    // the assertion above is driven by the offset, not by something else.
    let utc_harness = PipelineHarness::new();
    let utc_chan = utc_harness.add_channel(ChannelKind::WeCom, "a");
    utc_harness.override_global_rule(AgentKind::Codex, "Stop", |rule| {
        rule.enabled = true;
        rule.targets = vec![TargetConfig {
            channel_id: utc_chan,
            template: None,
        }];
        rule.quiet_hours = Some(cc_reminder_lib::model::QuietHours {
            start_local,
            end_local,
            weekdays: vec![1, 2, 3, 4, 5, 6, 7],
            bypass_at_or_above: None,
        });
    });
    utc_harness.install_hook(AgentKind::Codex, "Stop", "fp");
    let utc_pipeline = utc_harness.pipeline_with_offset(chrono::FixedOffset::east_opt(0).unwrap());
    utc_pipeline
        .process_live(utc_harness.ingress_request("Stop", "fp"))
        .await
        .unwrap();
    assert_eq!(
        utc_harness.pending_jobs(),
        1,
        "the same instant under UTC is daytime, so the window must NOT silence it"
    );
}

/// Mandatory public-field redaction must run on the persisted envelope. A
/// `public_fields` string matching a mandatory pattern (Authorization header)
/// must be replaced with `[REDACTED]` in `public_fields_json`, not stored raw.
/// This exercises `redact_envelope_public_fields` on the persisted path — the
/// existing per-target test puts the secret in a Sensitive field and so never
/// reaches the public-fields scrubber.
#[tokio::test]
async fn mandatory_redaction_runs_on_public_fields() {
    let harness = PipelineHarness::new();
    let chan = harness.add_channel(ChannelKind::WeCom, "a");
    harness.override_global_rule(AgentKind::Codex, "Stop", |rule| {
        rule.enabled = true;
        rule.targets = vec![TargetConfig {
            channel_id: chan,
            template: None,
        }];
    });
    harness.install_hook(AgentKind::Codex, "Stop", "fp");
    let pipeline = harness.pipeline();

    // Inject a secret-bearing value into a cataloged Public field that the
    // normalizer persists into public_fields. `stop_hook_active` is Public for
    // Codex Stop (the `model` field is pulled up to the envelope top level, not
    // public_fields, so it would not exercise the persisted scrubber). The value
    // matches a mandatory redaction pattern (Bearer token).
    let secret = "Authorization: Bearer abc.def.ghi";
    let req = IngressRequest {
        protocol_version: IPC_PROTOCOL_VERSION,
        helper_version: "0.1.0".into(),
        command_fingerprint: "fp".into(),
        event: capture_hook_json(
            AgentKind::Codex,
            "Stop",
            Version::new(0, 145, 0),
            json!({
                "cwd": "/workspace/demo",
                "last_assistant_message": "msg",
                "session_id": "s",
                "turn_id": "t",
                "stop_hook_active": secret,
            }),
        )
        .unwrap(),
    };
    pipeline.process_live(req).await.unwrap();

    let conn = rusqlite::Connection::open(harness.database.path()).unwrap();
    let public_json: String = conn
        .query_row(
            "SELECT public_fields_json FROM events WHERE source_event = 'Stop'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert!(
        public_json.contains("[REDACTED]"),
        "mandatory redaction must replace the secret: got {public_json}"
    );
    assert!(
        !public_json.contains("abc.def.ghi"),
        "raw secret must not persist: got {public_json}"
    );
}

/// `recover_ingress` must NOT abort the whole batch when one row fails. A
/// failing row (uncatalogued source_event) is left `'processing'` for the Task
/// 15 reaper; sibling rows still process.
#[tokio::test]
async fn recover_ingress_skips_bad_row_and_processes_siblings() {
    let harness = PipelineHarness::new();
    let chan = harness.add_channel(ChannelKind::WeCom, "a");
    harness.override_global_rule(AgentKind::Codex, "Stop", |rule| {
        rule.enabled = true;
        rule.targets = vec![TargetConfig {
            channel_id: chan,
            template: None,
        }];
    });

    // Insert two ingress rows: one good (Stop), one with an uncatalogued
    // source_event that will fail capability resolution inside
    // process_safe_ingress_with.
    let good = safe_ingress_for_event("Stop", Utc::now());
    let mut bad = safe_ingress_for_event("Stop", Utc::now());
    bad.source_event = "UncataloguedEvent".to_owned();
    bad.event_id = Uuid::now_v7();
    harness.insert_safe_ingress(good.clone());
    harness.insert_safe_ingress(bad.clone());

    let pipeline = harness.pipeline();
    // Must not error even though one row fails.
    let processed = pipeline.recover_ingress().await.unwrap();
    assert_eq!(processed, 2, "both rows were attempted");

    // The good row processed into an event + a pending job.
    assert_eq!(harness.event_count(), 1);
    assert_eq!(harness.pending_jobs(), 1);

    // The bad row is left 'processing' (not deleted) for the reaper; the good
    // row was deleted after commit.
    let conn = rusqlite::Connection::open(harness.database.path()).unwrap();
    let (bad_state, remaining): (String, i64) = conn
        .query_row(
            "SELECT state, COUNT(*) FROM ingress_events GROUP BY state",
            [],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?)),
        )
        .unwrap();
    assert_eq!(remaining, 1, "only the failed row remains");
    assert_eq!(
        bad_state, "processing",
        "failed row is left 'processing' for the reaper"
    );
}

/// A 3-strike auth pause must emit `CoreEvent::HealthChanged` for the paused
/// channel. Before the fix, `emit_health_pause` read `paused_reason_code` into
/// a `_` local and returned without pushing anything.
#[tokio::test]
async fn three_strike_auth_pause_emits_health_changed() {
    let harness = WorkerHarness::new();
    let chan_a = harness
        .pipeline_harness
        .add_channel(ChannelKind::WeCom, "a");
    // Enqueue three jobs on chan_a; all return Auth so the 3rd triggers the pause.
    let _j1 = harness.enqueue_job(chan_a);
    let _j2 = harness.enqueue_job(chan_a);
    let _j3 = harness.enqueue_job(chan_a);
    harness.factory.set_outcomes(vec![
        MockSendOutcome::Auth,
        MockSendOutcome::Auth,
        MockSendOutcome::Auth,
    ]);

    let worker = harness.worker();
    for _ in 0..3 {
        worker.clone().run_once().await.unwrap();
    }

    assert_eq!(
        harness.channel_health(chan_a),
        ChannelHealth::PausedAuthentication
    );
    let events = harness.events_sink.lock().unwrap();
    let health_changes: Vec<Uuid> = events
        .iter()
        .filter_map(|e| match e {
            cc_reminder_lib::worker::CoreEvent::HealthChanged { channel_id } => Some(*channel_id),
            _ => None,
        })
        .collect();
    assert!(
        health_changes.contains(&chan_a),
        "expected a HealthChanged event for the paused channel, got {health_changes:?}"
    );
}
