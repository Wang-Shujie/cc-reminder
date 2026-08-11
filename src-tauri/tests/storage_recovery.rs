//! Recovery and concurrency tests for the durable delivery queue (Task 12).
//!
//! These complement the in-crate unit tests in `storage::queue::tests` and
//! exercise behaviours that need two independent SQLite connections or a fresh
//! repository handle to simulate worker crash + restart:
//!   * concurrent two-connection claiming never double-claims a live-leased job,
//!   * application restart re-claims a job whose lease was abandoned,
//!   * pending/retry expiry sweeps jobs past their TTL,
//!   * a partial aggregate bucket (some jobs not yet due) is not claimed early,
//!   * completing or failing an aggregate claim updates every constituent job
//!     atomically.

use chrono::{DateTime, Duration, TimeZone, Utc};
use rusqlite::params;
use tempfile::{TempDir, tempdir};
use uuid::Uuid;

use cc_reminder_lib::error::{DeliveryError, DeliveryErrorKind, DeliveryReceipt};
use cc_reminder_lib::model::{ChannelId, NotificationDocument, RuleId, Severity};
use cc_reminder_lib::storage::db::Database;
use cc_reminder_lib::storage::queue::{
    ClaimedDelivery, DeliveryJob, DeliveryStatus, QueueRepository,
};

fn now() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 8, 5, 12, 0, 0).unwrap()
}

fn event_id(n: u8) -> Uuid {
    Uuid::parse_str(&format!("00000000-0000-0000-0000-{n:012x}")).unwrap()
}

fn rule_id() -> RuleId {
    Uuid::parse_str("22222222-2222-2222-2222-222222222222").unwrap()
}

fn channel_id() -> ChannelId {
    Uuid::parse_str("33333333-3333-3333-3333-333333333333").unwrap()
}

fn doc() -> NotificationDocument {
    NotificationDocument {
        title: "Title".to_owned(),
        severity: Severity::Info,
        facts: vec![("k".to_owned(), "v".to_owned())],
        body: "body".to_owned(),
        footer: None,
    }
}

fn open_repo() -> (TempDir, QueueRepository) {
    let root = tempdir().unwrap();
    let path = root
        .path()
        .join("com.ccreminder.app")
        .join("cc-reminder.sqlite3");
    let database = Database::open(&path).unwrap();
    insert_channel(&database, channel_id());
    (root, QueueRepository::new(database))
}

fn insert_channel(database: &Database, channel: ChannelId) {
    let conn = rusqlite::Connection::open(database.path()).unwrap();
    conn.execute(
        "INSERT INTO channels (id, kind, name, credential_ref, public_config_json, health_status,
            created_at, updated_at)
         VALUES (?1, 'we_com', 'test', ?2, '{}', 'healthy', ?3, ?3)",
        params![
            channel.to_string(),
            format!("cc-reminder/channel/access_token=opaque-{channel}"),
            now().to_rfc3339(),
        ],
    )
    .unwrap();
}

/// Insert a minimal event row so `delivery_jobs.event_id` FK is satisfied.
fn insert_event(database: &Database, event_id: Uuid) {
    let conn = rusqlite::Connection::open(database.path()).unwrap();
    let now = now().to_rfc3339();
    conn.execute(
        "INSERT OR IGNORE INTO events (
            id, source, source_version, source_event, category, occurred_at, received_at,
            severity, public_fields_json, correlation_id, action_capabilities_json,
            processing_outcome, created_at
         ) VALUES (?1, 'codex', '0.145.0', 'Stop', 'completion', ?2, ?2,
            'info', '{}', ?3, '[]', 'queued', ?2)",
        params![event_id.to_string(), now, Uuid::now_v7().to_string()],
    )
    .unwrap();
}

fn enqueue(
    repo: &QueueRepository,
    job: &DeliveryJob,
) -> cc_reminder_lib::storage::queue::EnqueueResult {
    insert_event(repo.database_for_test(), job.event_id);
    repo.enqueue(job).unwrap()
}

fn new_job(event_id: Uuid, channel: ChannelId) -> DeliveryJob {
    DeliveryJob {
        idempotency_key: QueueRepository::idempotency_key(event_id, "v1", channel),
        id: Uuid::now_v7(),
        event_id,
        rule_id: rule_id(),
        rule_version: "v1".to_owned(),
        channel_id: channel,
        document: doc(),
        state: DeliveryStatus::Pending,
        attempts: 0,
        next_attempt_at: now(),
        expires_at: now() + Duration::minutes(30),
        lease_owner: None,
        lease_expires_at: None,
        aggregate_key: None,
        aggregate_release_at: None,
    }
}

fn temporary_http(status: u16) -> DeliveryError {
    DeliveryError {
        kind: DeliveryErrorKind::HttpStatus,
        code: format!("http.{status}"),
        redacted_message: "temporary".to_owned(),
        http_status: Some(status),
        platform_code: None,
        retry_after_seconds: None,
    }
}

fn receipt(status: u16) -> DeliveryReceipt {
    DeliveryReceipt {
        http_status: status,
        platform_code: None,
        sent_at: now(),
    }
}

#[test]
fn concurrent_two_connection_claim_never_doubles_a_live_lease() {
    let (_root, repo_a) = open_repo();
    let job = new_job(event_id(1), channel_id());
    insert_event(repo_a.database_for_test(), job.event_id);
    repo_a.enqueue(&job).unwrap();

    // Open a second independent repository handle against the same database file.
    let repo_b = QueueRepository::new(Database::open(repo_a.database_path()).unwrap());

    let claimed_a = repo_a
        .claim_due("worker-a", now(), Duration::minutes(1), 10)
        .unwrap();
    assert_eq!(claimed_a.len(), 1);

    // Worker B must see nothing while A's lease is live, even though B holds a
    // separate connection.
    let claimed_b = repo_b
        .claim_due("worker-b", now(), Duration::minutes(1), 10)
        .unwrap();
    assert!(claimed_b.is_empty());
}

#[test]
fn application_restart_reclaims_an_abandoned_lease() {
    let (root, repo) = open_repo();
    let job = new_job(event_id(2), channel_id());
    enqueue(&repo, &job);
    let id = job.id;

    // Claim, then drop the repository to simulate a crash before completion.
    let claimed = repo
        .claim_due("worker-a", now(), Duration::minutes(1), 10)
        .unwrap();
    assert_eq!(claimed.len(), 1);
    drop(repo);

    let repo = QueueRepository::new(Database::open(&reopen(&root)).unwrap());
    let reclaimed = repo
        .claim_due(
            "worker-b",
            now() + Duration::minutes(2),
            Duration::minutes(1),
            10,
        )
        .unwrap();
    assert_eq!(reclaimed.len(), 1);
    match &reclaimed[0] {
        ClaimedDelivery::Single { job } => assert_eq!(job.id, id),
        ClaimedDelivery::Aggregate { .. } => panic!("expected single claim"),
    }
}

#[test]
fn expire_due_sweeps_pending_and_retry_wait_past_ttl() {
    let (_root, repo) = open_repo();
    let a = new_job(event_id(3), channel_id());
    let b = new_job(event_id(4), channel_id());
    enqueue(&repo, &a);
    enqueue(&repo, &b);
    // b goes through sending -> retry_wait to exercise the other expiry edge.
    repo.set_job_state_for_test(b.id, DeliveryStatus::Sending, now());
    repo.retry(b.id, now(), &temporary_http(500)).unwrap();

    let stats = repo.expire_due(now() + Duration::minutes(31)).unwrap();
    assert_eq!(stats.expired, 2);
    assert_eq!(repo.job_state_for_test(a.id), DeliveryStatus::Expired);
    assert_eq!(repo.job_state_for_test(b.id), DeliveryStatus::Expired);
}

#[test]
fn partial_aggregate_bucket_is_not_claimed_before_release() {
    let (_root, repo) = open_repo();
    let key = "rule|chan|proj|window";
    let release = now() + Duration::minutes(5);

    let mut due = new_job(event_id(5), channel_id());
    due.aggregate_key = Some(key.to_owned());
    due.aggregate_release_at = Some(now()); // already due
    let mut not_due = new_job(event_id(6), channel_id());
    not_due.aggregate_key = Some(key.to_owned());
    not_due.aggregate_release_at = Some(release); // not yet
    enqueue(&repo, &due);
    enqueue(&repo, &not_due);

    // Aggregate bucket is only released once ALL constituents are due; until
    // then the partial bucket is invisible to claim_due.
    let claims = repo
        .claim_due("worker-a", now(), Duration::minutes(1), 10)
        .unwrap();
    assert!(claims.is_empty());

    // After the window elapses, the full bucket is claimable together.
    let claims = repo
        .claim_due("worker-a", release, Duration::minutes(1), 10)
        .unwrap();
    match &claims[0] {
        ClaimedDelivery::Aggregate { jobs, .. } => assert_eq!(jobs.len(), 2),
        _ => panic!("expected aggregate claim"),
    }
}

#[test]
fn aggregate_completion_updates_every_constituent_atomically() {
    let (_root, repo) = open_repo();
    let key = "rule2|chan|proj|window2";
    for n in 7..=9 {
        let mut job = new_job(event_id(n), channel_id());
        job.aggregate_key = Some(key.to_owned());
        job.aggregate_release_at = Some(now());
        enqueue(&repo, &job);
    }

    let claims = repo
        .claim_due("worker-a", now(), Duration::minutes(1), 10)
        .unwrap();
    let claim = match &claims[0] {
        ClaimedDelivery::Aggregate { jobs, .. } => jobs.clone(),
        _ => panic!("expected aggregate"),
    };
    assert_eq!(claim.len(), 3);

    // Succeed the aggregate claim: every constituent must move to `succeeded`.
    repo.complete_aggregate(&claims[0], now(), receipt(200))
        .unwrap();
    for job in &claim {
        assert_eq!(repo.job_state_for_test(job.id), DeliveryStatus::Succeeded);
    }
}

#[test]
fn aggregate_failure_moves_every_constituent_to_retry_or_fail_atomically() {
    let (_root, repo) = open_repo();
    let key = "rule3|chan|proj|window3";
    let mut ids = Vec::new();
    for n in 10..=12 {
        let mut job = new_job(event_id(n), channel_id());
        job.aggregate_key = Some(key.to_owned());
        job.aggregate_release_at = Some(now());
        enqueue(&repo, &job);
        ids.push(job.id);
    }

    let claims = repo
        .claim_due("worker-a", now(), Duration::minutes(1), 10)
        .unwrap();
    repo.retry_aggregate(&claims[0], now(), &temporary_http(500))
        .unwrap();
    for id in &ids {
        assert_eq!(repo.job_state_for_test(*id), DeliveryStatus::RetryWait);
    }
}

fn reopen(root: &TempDir) -> std::path::PathBuf {
    root.path()
        .join("com.ccreminder.app")
        .join("cc-reminder.sqlite3")
}
