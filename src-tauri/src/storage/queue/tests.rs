//! Failing tests for Task 12 — idempotency, state machine, leases, aggregation,
//! expiry, rate limiting, and retry classification. The plan's test code blocks
//! are the contract.

use chrono::{DateTime, Duration, TimeZone, Utc};
use rusqlite::params;
use tempfile::{TempDir, tempdir};
use uuid::Uuid;

use super::{
    AttemptInput, ClaimedDelivery, DeliveryJob, DeliveryStatus, EnqueueResult, QueueRepository,
    RetryDecision, RetryOutcome, RetryPolicy,
};
use crate::error::{DeliveryError, DeliveryErrorKind};
use crate::model::{ChannelId, NotificationDocument, RuleId, Severity};
use crate::storage::db::Database;

// ---------------------------------------------------------------------------
// Fixtures
// ---------------------------------------------------------------------------

fn now() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 8, 5, 12, 0, 0).unwrap()
}

fn event_id() -> Uuid {
    Uuid::parse_str("11111111-1111-1111-1111-111111111111").unwrap()
}

fn rule_id() -> RuleId {
    Uuid::parse_str("22222222-2222-2222-2222-222222222222").unwrap()
}

fn channel_id() -> ChannelId {
    Uuid::parse_str("33333333-3333-3333-3333-333333333333").unwrap()
}

fn channel_a() -> ChannelId {
    Uuid::parse_str("44444444-4444-4444-4444-444444444444").unwrap()
}

fn channel_b() -> ChannelId {
    Uuid::parse_str("55555555-5555-5555-5555-555555555555").unwrap()
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

fn test_queue() -> (TempDir, TestQueue) {
    let root = tempdir().unwrap();
    let path = root
        .path()
        .join("com.ccreminder.app")
        .join("cc-reminder.sqlite3");
    let database = Database::open(&path).unwrap();
    insert_channel(&database, channel_id());
    let queue = QueueRepository::new(database);
    (root, TestQueue { queue })
}

struct TestQueue {
    queue: QueueRepository,
}

impl TestQueue {
    fn enqueue(&self, job: &DeliveryJob) -> Result<EnqueueResult, crate::error::AppError> {
        // Ensure the event row exists so the delivery_jobs FK is satisfied.
        insert_event(self.queue.database_for_test(), job.event_id);
        self.queue.enqueue(job)
    }
    fn count_jobs(&self) -> usize {
        self.queue.count_jobs_for_test()
    }
    fn only_job_id(&self) -> Uuid {
        self.queue.only_job_id_for_test()
    }
    fn retry(
        &self,
        id: Uuid,
        when: DateTime<Utc>,
        error: DeliveryError,
    ) -> Result<RetryOutcome, crate::error::AppError> {
        self.queue.retry(id, when, &error)
    }
    fn claim_due(
        &self,
        worker: &str,
        now: DateTime<Utc>,
        lease: Duration,
        max: usize,
    ) -> Result<Vec<ClaimedDelivery>, crate::error::AppError> {
        self.queue.claim_due(worker, now, lease, max)
    }
    fn set_channel_next_allowed(
        &self,
        channel: ChannelId,
        when: DateTime<Utc>,
    ) -> Result<(), crate::error::AppError> {
        self.queue.set_channel_next_allowed(channel, when)
    }
}

fn insert_channel(database: &Database, channel: ChannelId) {
    let conn = database.connect().unwrap();
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
    let conn = database.connect().unwrap();
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

/// Build a new pending `DeliveryJob` descriptor with sensible defaults.
/// `queue.enqueue(&job)` is what persists it.
fn new_job(
    event_id: Uuid,
    rule_id: RuleId,
    rule_version: &str,
    channel_id: ChannelId,
) -> DeliveryJob {
    DeliveryJob {
        idempotency_key: QueueRepository::idempotency_key(event_id, rule_version, channel_id),
        id: Uuid::now_v7(),
        event_id,
        rule_id,
        rule_version: rule_version.to_owned(),
        channel_id,
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

fn new_aggregate_job(
    event_id: Uuid,
    rule_id: RuleId,
    rule_version: &str,
    channel_id: ChannelId,
    aggregate_key: &str,
    release_at: DateTime<Utc>,
) -> DeliveryJob {
    let mut job = new_job(event_id, rule_id, rule_version, channel_id);
    job.aggregate_key = Some(aggregate_key.to_owned());
    job.aggregate_release_at = Some(release_at);
    job
}

fn queue_with_succeeded_job() -> (TempDir, TestQueue) {
    let (root, tq) = test_queue();
    let job = new_job(event_id(), rule_id(), "effective-v3", channel_id());
    tq.enqueue(&job).unwrap();
    tq.queue
        .set_job_state_for_test(job.id, DeliveryStatus::Sending, now());
    tq.queue
        .set_job_state_for_test(job.id, DeliveryStatus::Succeeded, now());
    (root, tq)
}

fn queue_with_due_job() -> (TempDir, TestQueue) {
    let (root, tq) = test_queue();
    let job = new_job(event_id(), rule_id(), "v1", channel_id());
    tq.enqueue(&job).unwrap();
    (root, tq)
}

fn queue_with_three_jobs_in_one_due_bucket() -> (TempDir, TestQueue) {
    let (root, tq) = test_queue();
    let key = "rule-2|chan-3|proj-|window-1";
    let release = now();
    for _ in 0..3 {
        // Each job gets a distinct event id so they don't collide on the
        // idempotency key, but share the same aggregate bucket.
        let job = new_aggregate_job(Uuid::now_v7(), rule_id(), "v1", channel_id(), key, release);
        tq.enqueue(&job).unwrap();
    }
    (root, tq)
}

fn queue_with_due_jobs_on_two_channels() -> (TempDir, TestQueue) {
    let root = tempdir().unwrap();
    let path = root
        .path()
        .join("com.ccreminder.app")
        .join("cc-reminder.sqlite3");
    let database = Database::open(&path).unwrap();
    insert_channel(&database, channel_a());
    insert_channel(&database, channel_b());
    let queue = QueueRepository::new(database);
    let tq = TestQueue { queue };
    for chan in [channel_a(), channel_b()] {
        let job = new_job(Uuid::now_v7(), rule_id(), "v1", chan);
        tq.enqueue(&job).unwrap();
    }
    (root, tq)
}

fn redacted_error() -> DeliveryError {
    DeliveryError {
        kind: DeliveryErrorKind::Network,
        code: "network.unreachable".to_owned(),
        redacted_message: "unreachable".to_owned(),
        http_status: None,
        platform_code: None,
        retry_after_seconds: None,
    }
}

fn temporary_http(status: u16, retry_after: Option<Duration>) -> DeliveryError {
    DeliveryError {
        kind: DeliveryErrorKind::HttpStatus,
        code: format!("http.{status}"),
        redacted_message: "temporary".to_owned(),
        http_status: Some(status),
        platform_code: None,
        retry_after_seconds: retry_after.map(|d| d.num_seconds().max(0) as u64),
    }
}

fn invalid_credential() -> DeliveryError {
    DeliveryError {
        kind: DeliveryErrorKind::Authentication,
        code: "auth.invalid_credential".to_owned(),
        redacted_message: "bad token".to_owned(),
        http_status: Some(401),
        platform_code: None,
        retry_after_seconds: None,
    }
}

fn invalid_format() -> DeliveryError {
    DeliveryError {
        kind: DeliveryErrorKind::Format,
        code: "format.invalid".to_owned(),
        redacted_message: "bad body".to_owned(),
        http_status: None,
        platform_code: None,
        retry_after_seconds: None,
    }
}

// ---------------------------------------------------------------------------
// Step 1: idempotency and state machine
// ---------------------------------------------------------------------------

#[test]
fn one_event_rule_version_and_target_enqueues_once() {
    let (_root, queue) = test_queue();
    let job = new_job(event_id(), rule_id(), "effective-v3", channel_id());
    assert_eq!(
        queue.enqueue(&job).unwrap(),
        EnqueueResult::Inserted(job.id)
    );
    assert_eq!(
        queue.enqueue(&job).unwrap(),
        EnqueueResult::AlreadyExists(job.id)
    );
    assert_eq!(queue.count_jobs(), 1);
}

#[test]
fn different_rule_version_or_target_enqueues_distinct_jobs() {
    let (_root, queue) = test_queue();
    let base = new_job(event_id(), rule_id(), "v1", channel_id());
    let mut other_version = base.clone();
    other_version.id = Uuid::now_v7();
    other_version.rule_version = "v2".to_owned();
    let mut other_channel = base.clone();
    other_channel.id = Uuid::now_v7();
    other_channel.channel_id = channel_a();
    insert_channel(queue.queue.database_for_test(), channel_a());

    queue.enqueue(&base).unwrap();
    queue.enqueue(&other_version).unwrap();
    queue.enqueue(&other_channel).unwrap();
    assert_eq!(queue.count_jobs(), 3);
}

#[test]
fn idempotency_key_ignores_rule_id() {
    // Same event/version/channel but different rule_id must dedupe.
    let (_root, queue) = test_queue();
    let mut a = new_job(event_id(), rule_id(), "v1", channel_id());
    let mut b = a.clone();
    b.id = Uuid::now_v7();
    b.rule_id = Uuid::now_v7();
    a.rule_id = Uuid::now_v7();
    assert_eq!(queue.enqueue(&a).unwrap(), EnqueueResult::Inserted(a.id));
    assert_eq!(
        queue.enqueue(&b).unwrap(),
        EnqueueResult::AlreadyExists(a.id)
    );
}

#[test]
fn invalid_state_transition_is_rejected() {
    let (_root, queue) = queue_with_succeeded_job();
    let error = queue
        .retry(queue.only_job_id(), Utc::now(), redacted_error())
        .unwrap_err();
    assert_eq!(error.code, "storage.invalid_delivery_transition");
}

#[test]
fn allowed_state_transitions_succeed() {
    let (_root, queue) = queue_with_due_job();
    let id = queue.only_job_id();
    // pending -> sending happens via claim; simulate completion and retry paths.
    queue
        .queue
        .set_job_state_for_test(id, DeliveryStatus::Sending, now());
    // A retry on a sending job is allowed (sending -> retry_wait).
    let outcome = queue.retry(id, now(), temporary_http(500, None)).unwrap();
    assert!(matches!(outcome, RetryOutcome::RetryAt(_)));
    // succeeded terminal.
    queue
        .queue
        .set_job_state_for_test(id, DeliveryStatus::Sending, now());
    queue.queue.complete(id, now(), receipt(200)).unwrap();
}

fn receipt(status: u16) -> crate::error::DeliveryReceipt {
    crate::error::DeliveryReceipt {
        http_status: status,
        platform_code: None,
        sent_at: now(),
    }
}

// ---------------------------------------------------------------------------
// Step 2: leases, crash recovery, aggregates, expiry, rate limit
// ---------------------------------------------------------------------------

#[test]
fn expired_lease_can_be_reclaimed_but_live_lease_cannot() {
    let (_root, queue) = queue_with_due_job();
    let first = queue
        .claim_due("worker-a", now(), Duration::minutes(1), 10)
        .unwrap();
    assert_eq!(first.len(), 1);
    // Live lease — second worker cannot claim.
    assert!(
        queue
            .claim_due(
                "worker-b",
                now() + Duration::seconds(30),
                Duration::minutes(1),
                10
            )
            .unwrap()
            .is_empty()
    );
    // Lease expired — reclaimable.
    assert_eq!(
        queue
            .claim_due(
                "worker-b",
                now() + Duration::seconds(61),
                Duration::minutes(1),
                10
            )
            .unwrap()
            .len(),
        1
    );
}

#[test]
fn due_aggregate_claim_contains_all_jobs_in_the_bucket() {
    let (_root, queue) = queue_with_three_jobs_in_one_due_bucket();
    let claims = queue
        .claim_due("worker-a", now(), Duration::minutes(1), 10)
        .unwrap();
    assert!(matches!(&claims[0], ClaimedDelivery::Aggregate { jobs, .. } if jobs.len() == 3));
}

#[test]
fn channel_rate_limit_delays_only_that_channel() {
    let (_root, queue) = queue_with_due_jobs_on_two_channels();
    queue
        .set_channel_next_allowed(channel_a(), now() + Duration::seconds(30))
        .unwrap();
    let claims = queue
        .claim_due("worker-a", now(), Duration::minutes(1), 10)
        .unwrap();
    assert_eq!(
        claims
            .iter()
            .flat_map(ClaimedDelivery::jobs)
            .map(|job| job.channel_id)
            .collect::<Vec<_>>(),
        vec![channel_b()]
    );
}

#[test]
fn pending_job_expires_when_past_ttl() {
    let (_root, queue) = queue_with_due_job();
    let expired_at = now() + Duration::minutes(31);
    let stats = queue.queue.expire_due(expired_at).unwrap();
    assert_eq!(stats.expired, 1);
    assert_eq!(
        queue.queue.job_state_for_test(queue.only_job_id()),
        DeliveryStatus::Expired
    );
}

#[test]
fn retry_wait_job_expires_when_past_ttl() {
    let (_root, queue) = queue_with_due_job();
    let id = queue.only_job_id();
    queue
        .queue
        .set_job_state_for_test(id, DeliveryStatus::Sending, now());
    queue.retry(id, now(), temporary_http(500, None)).unwrap();
    let expired_at = now() + Duration::minutes(31);
    let stats = queue.queue.expire_due(expired_at).unwrap();
    assert_eq!(stats.expired, 1);
    assert_eq!(queue.queue.job_state_for_test(id), DeliveryStatus::Expired);
}

#[test]
fn manual_retry_revives_a_failed_job() {
    let (_root, queue) = queue_with_due_job();
    let id = queue.only_job_id();
    queue
        .queue
        .set_job_state_for_test(id, DeliveryStatus::Sending, now());
    queue.queue.fail(id, now(), &invalid_credential()).unwrap();
    queue.queue.manual_retry(id, now()).unwrap();
    assert_eq!(queue.queue.job_state_for_test(id), DeliveryStatus::Pending);
    assert_eq!(queue.queue.job_attempts_for_test(id), 0);
}

#[test]
fn manual_retry_refuses_expired_or_unavailable_channel() {
    let (_root, queue) = queue_with_due_job();
    let id = queue.only_job_id();
    queue
        .queue
        .expire_due(now() + Duration::minutes(31))
        .unwrap();
    let err = queue.queue.manual_retry(id, now()).unwrap_err();
    assert_eq!(err.code, "delivery.expired");

    // Paused channel: the job must be `failed` first (manual_retry only revives
    // failed jobs), then a paused channel refuses the retry.
    let (root2, q2) = queue_with_due_job();
    let id2 = q2.only_job_id();
    q2.queue
        .set_job_state_for_test(id2, DeliveryStatus::Sending, now());
    q2.queue
        .fail(id2, now(), &temporary_http(500, None))
        .unwrap();
    q2.queue
        .set_channel_state_for_test(channel_id(), "paused_authentication", now());
    let err = q2.queue.manual_retry(id2, now()).unwrap_err();
    assert_eq!(err.code, "delivery.channel_unavailable");
    drop(root2);
}

// ---------------------------------------------------------------------------
// Step 3: retry classification
// ---------------------------------------------------------------------------

fn attempt(n: u8) -> AttemptInput {
    AttemptInput {
        attempts: n,
        expires_at: now() + Duration::minutes(30),
        classified_at: now(),
    }
}

#[test]
fn retry_after_wins_over_jittered_backoff() {
    let policy = RetryPolicy::with_deterministic_jitter([0.25, 0.5]);
    let decision = policy.classify(
        attempt(2),
        &temporary_http(429, Some(Duration::seconds(90))),
    );
    assert_eq!(
        decision,
        RetryDecision::RetryAt(now() + Duration::seconds(90))
    );
}

#[test]
fn credentials_and_format_errors_fail_without_retry() {
    let policy = RetryPolicy::default();
    assert_eq!(
        policy.classify(attempt(1), &invalid_credential()),
        RetryDecision::Fail
    );
    assert_eq!(
        policy.classify(attempt(1), &invalid_format()),
        RetryDecision::Fail
    );
}

#[test]
fn network_timeout_and_temporary_platform_retry_with_jittered_backoff() {
    let policy = RetryPolicy::with_deterministic_jitter([0.5]);
    // attempt 1 -> base 2s, jitter 0.5 -> 1s.
    let d = policy.classify(attempt(1), &redacted_error());
    assert_eq!(d, RetryDecision::RetryAt(now() + Duration::seconds(1)));
}

#[test]
fn backoff_is_exponential_not_linear() {
    // Pins base*2^(attempts-1) with full jitter (1.0 so the cap dominates): the
    // delays must be 2s, 4s, 8s, 16s for attempts 1..4. A linear (base*attempts)
    // implementation would yield 2/4/6/8s and fail at attempt 3.
    let policy = RetryPolicy::with_deterministic_jitter([1.0, 1.0, 1.0, 1.0]);
    assert_eq!(
        policy.classify(attempt(1), &redacted_error()),
        RetryDecision::RetryAt(now() + Duration::seconds(2))
    );
    assert_eq!(
        policy.classify(attempt(2), &redacted_error()),
        RetryDecision::RetryAt(now() + Duration::seconds(4))
    );
    assert_eq!(
        policy.classify(attempt(3), &redacted_error()),
        RetryDecision::RetryAt(now() + Duration::seconds(8))
    );
    assert_eq!(
        policy.classify(attempt(4), &redacted_error()),
        RetryDecision::RetryAt(now() + Duration::seconds(16))
    );
}

#[test]
fn fifth_attempt_expires_or_fails_instead_of_retrying() {
    let policy = RetryPolicy::default();
    // 5 attempts already used; next is attempt 6 -> no retry.
    let d = policy.classify(attempt(5), &temporary_http(500, None));
    assert!(matches!(d, RetryDecision::Fail | RetryDecision::Expire));
}

#[test]
fn past_ttl_classifies_as_expire() {
    let policy = RetryPolicy::default();
    let past = AttemptInput {
        attempts: 1,
        expires_at: now() - Duration::seconds(1),
        classified_at: now(),
    };
    let d = policy.classify(past, &temporary_http(500, None));
    assert_eq!(d, RetryDecision::Expire);
}
