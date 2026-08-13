//! Durable delivery queue repository, leases, aggregation, and retry policy.
//!
//! Implements Task 12 of the CC Reminder plan: a SQLite-backed delivery queue
//! over `delivery_jobs` / `delivery_attempts` with transactional enqueue/claim,
//! bounded leases that survive worker crashes, per-channel rate limiting with
//! `Retry-After` precedence, aggregate bucket claiming, and a pure
//! [`RetryPolicy::classify`] encoding design §15 retry rules.
//!
//! State machine (design §15):
//!
//! ```text
//! pending -> sending -> succeeded
//!                  \-> retry_wait -> sending
//!                  \-> failed
//! pending/retry_wait -> expired
//! ```
//!
//! Concurrency model: [`QueueRepository::enqueue`] and
//! [`QueueRepository::claim_due`] take their transaction with
//! `BEGIN IMMEDIATE` (matching `integrations::mark_hook_seen`), so concurrent
//! workers serialize on the write lock before reading candidate rows. Claim
//! then performs an atomic conditional `UPDATE ... WHERE state/predicate`,
//! which makes a live lease invisible to other workers while an expired lease
//! is reclaimable.

use std::path::Path;

use chrono::{DateTime, Duration, Utc};
use rusqlite::{OptionalExtension, TransactionBehavior, params};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::error::{AppError, DeliveryError, DeliveryErrorKind, DeliveryReceipt, ErrorDomain};
use crate::model::{ChannelId, NotificationDocument, RuleId};

use super::db::{Database, storage_error};

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

/// Repository state machine for a delivery job. Mirrors the `delivery_jobs`
/// CHECK constraint and design §15.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DeliveryStatus {
    Pending,
    Sending,
    RetryWait,
    Succeeded,
    Failed,
    Expired,
}

impl DeliveryStatus {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Sending => "sending",
            Self::RetryWait => "retry_wait",
            Self::Succeeded => "succeeded",
            Self::Failed => "failed",
            Self::Expired => "expired",
        }
    }

    fn parse(value: &str) -> Option<Self> {
        Some(match value {
            "pending" => Self::Pending,
            "sending" => Self::Sending,
            "retry_wait" => Self::RetryWait,
            "succeeded" => Self::Succeeded,
            "failed" => Self::Failed,
            "expired" => Self::Expired,
            _ => return None,
        })
    }
}

/// A persisted delivery job. The `id` is assigned by the caller (v7 UUID);
/// `idempotency_key` is derived in the repository from `(event_id, rule_version,
/// channel_id)` so callers cannot get it wrong.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DeliveryJob {
    pub id: Uuid,
    pub event_id: Uuid,
    pub rule_id: RuleId,
    pub rule_version: String,
    pub channel_id: ChannelId,
    pub idempotency_key: String,
    pub document: NotificationDocument,
    pub state: DeliveryStatus,
    pub attempts: u8,
    pub next_attempt_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
    pub lease_owner: Option<String>,
    pub lease_expires_at: Option<DateTime<Utc>>,
    pub aggregate_key: Option<String>,
    pub aggregate_release_at: Option<DateTime<Utc>>,
}

/// A claim returned by [`QueueRepository::claim_due`]. A single job is wrapped
/// in [`ClaimedDelivery::Single`]; jobs sharing one aggregate bucket that are
/// all due now are coalesced into [`ClaimedDelivery::Aggregate`].
//
// ponytail: `DeliveryJob` is ~320 bytes, which makes `Single` larger than
// `Aggregate`. Claims are short-lived (processed and dropped by the worker on
// the same tick) and never stored long-term, so the extra enum width is cheaper
// than a Box allocation per claim. Revisit if claims are ever retained.
#[derive(Clone, Debug)]
#[allow(clippy::large_enum_variant)]
pub enum ClaimedDelivery {
    Single {
        job: DeliveryJob,
    },
    Aggregate {
        aggregate_key: String,
        jobs: Vec<DeliveryJob>,
    },
}

impl ClaimedDelivery {
    /// Every constituent job in this claim (one for `Single`, many for
    /// `Aggregate`). Used by the plan's rate-limit test.
    pub fn jobs(&self) -> Vec<&DeliveryJob> {
        match self {
            Self::Single { job } => vec![job],
            Self::Aggregate { jobs, .. } => jobs.iter().collect(),
        }
    }

    fn constituent_ids(&self) -> Vec<Uuid> {
        match self {
            Self::Single { job } => vec![job.id],
            Self::Aggregate { jobs, .. } => jobs.iter().map(|job| job.id).collect(),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EnqueueResult {
    Inserted(Uuid),
    AlreadyExists(Uuid),
}

/// Outcome of [`QueueRepository::retry`]: the job either moved to `retry_wait`
/// with a scheduled next attempt, or moved to `failed` (permanent error / max
/// attempts reached). Used by the caller to set the channel's
/// `next_allowed_at`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RetryOutcome {
    RetryAt(DateTime<Utc>),
    Fail,
}

/// Aggregate result of an [`QueueRepository::expire_due`] sweep.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ExpiryStats {
    pub expired: u64,
}

/// Per-bucket queue counters returned by [`QueueRepository::queue_stats`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct QueueStats {
    pub pending: u64,
    pub sending: u64,
    pub retry_wait: u64,
    pub succeeded: u64,
    pub failed: u64,
    pub expired: u64,
}

/// What the retry policy decided should happen to a job after a send failure.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RetryDecision {
    /// Re-queue for another attempt at the given time.
    RetryAt(DateTime<Utc>),
    /// Authentication failure: pause the whole channel. The repository turns
    /// this into a `failed` job and bumps `consecutive_auth_failures`.
    PauseChannel { reason_code: &'static str },
    /// Permanent failure: do not retry.
    Fail,
    /// Job is past its TTL: move to `expired`.
    Expire,
}

/// Input bundle for [`RetryPolicy::classify`]: the current attempt count
/// (1-based, i.e. the attempt that just finished), the job's TTL, and the
/// instant at which the classifier is evaluating (so backoff is absolute).
#[derive(Clone, Copy, Debug)]
pub struct AttemptInput {
    pub attempts: u8,
    pub expires_at: DateTime<Utc>,
    pub classified_at: DateTime<Utc>,
}

/// Pure, synchronous retry classifier for design §15.2. Backoff base 2s, cap
/// 5min, full jitter; `Retry-After` (seconds or HTTP date) wins over backoff;
/// auth/signature/permission/format errors never retry; no retry past 5
/// attempts or job TTL.
#[derive(Clone, Debug)]
pub struct RetryPolicy {
    max_attempts: u8,
    base: Duration,
    cap: Duration,
    jitter: JitterSource,
}

impl Default for RetryPolicy {
    fn default() -> Self {
        Self {
            max_attempts: MAX_ATTEMPTS,
            base: BACKOFF_BASE,
            cap: BACKOFF_CAP,
            jitter: JitterSource::Random,
        }
    }
}

impl RetryPolicy {
    /// Policy with a fixed sequence of jitter samples in `[0,1)`, consumed in
    /// order. Used by tests to pin the exact delay.
    pub fn with_deterministic_jitter(samples: impl Into<Vec<f32>>) -> Self {
        Self {
            max_attempts: MAX_ATTEMPTS,
            base: BACKOFF_BASE,
            cap: BACKOFF_CAP,
            jitter: JitterSource::Deterministic(samples.into()),
        }
    }

    /// Classify the outcome of a send attempt. Pure: no I/O, no clock mutation.
    pub fn classify(&self, attempt: AttemptInput, error: &DeliveryError) -> RetryDecision {
        // Past TTL: expire wins over everything (the job is no longer useful).
        if attempt.expires_at <= attempt.classified_at {
            return RetryDecision::Expire;
        }

        match error.kind {
            // Permanent: never retry. Authentication/signature/permission/format
            // errors all classify as `Fail`; the repository separately tracks
            // consecutive authentication failures and pauses the channel once
            // the design §15.2 threshold is hit.
            DeliveryErrorKind::Format
            | DeliveryErrorKind::Authentication
            | DeliveryErrorKind::Signature
            | DeliveryErrorKind::Permission => return RetryDecision::Fail,
            // Retryable: network, timeout, http-status (filtered below),
            // temporary-platform.
            DeliveryErrorKind::Network
            | DeliveryErrorKind::Timeout
            | DeliveryErrorKind::TemporaryPlatform => {}
            DeliveryErrorKind::HttpStatus => {
                // 408/429/5xx are retryable; everything else is permanent.
                if !is_retryable_http_status(error.http_status) {
                    return RetryDecision::Fail;
                }
            }
        }

        // Out of attempts: the next attempt would be attempt+1; if that exceeds
        // the cap, fail rather than retry.
        if attempt.attempts >= self.max_attempts {
            return RetryDecision::Fail;
        }

        let when = self.next_attempt_at(attempt.classified_at, attempt.attempts, error);
        // Clamp to TTL — never schedule a retry that lands after expiry.
        if when > attempt.expires_at {
            return RetryDecision::Expire;
        }
        RetryDecision::RetryAt(when)
    }

    fn next_attempt_at(
        &self,
        classified_at: DateTime<Utc>,
        attempts_completed: u8,
        error: &DeliveryError,
    ) -> DateTime<Utc> {
        // Retry-After precedence: a server-provided delay always wins over our
        // backoff+jitter computation.
        if let Some(retry_after) = retry_after_duration(error) {
            return classified_at + retry_after;
        }

        // Full-jitter exponential backoff: base * 2^(attempts-1), capped.
        // The exponent is the number of attempts already completed, so the
        // first retry (attempts=1) waits base*2^0 = base.
        let exponent = u32::from(attempts_completed.max(1).saturating_sub(1));
        let multiplier = 2i64.checked_pow(exponent).unwrap_or(i64::MAX);
        let raw_ms = self.base.num_milliseconds().saturating_mul(multiplier);
        let capped = Duration::milliseconds(raw_ms.min(self.cap.num_milliseconds()));
        let jitter_factor = self.jitter.sample(attempts_completed);
        let delay = Duration::milliseconds(
            (capped.num_milliseconds() as f32 * jitter_factor).round() as i64,
        );
        classified_at + delay
    }
}

#[derive(Clone, Debug)]
enum JitterSource {
    Random,
    Deterministic(Vec<f32>),
}

impl JitterSource {
    /// Pure index-based sampling so `classify` stays `&self`. Deterministic
    /// samples are addressed by `(attempts_completed - 1) % len`, giving tests
    /// repeatable delays without mutating the policy.
    fn sample(&self, attempts_completed: u8) -> f32 {
        match self {
            Self::Deterministic(samples) => {
                if samples.is_empty() {
                    return 0.0;
                }
                let index = (attempts_completed.saturating_sub(1) as usize) % samples.len();
                samples[index]
            }
            Self::Random => rand::random(),
        }
    }
}

fn retry_after_duration(error: &DeliveryError) -> Option<Duration> {
    error
        .retry_after_seconds
        .filter(|&seconds| seconds > 0)
        .map(|seconds| Duration::seconds(seconds as i64))
}

fn is_retryable_http_status(status: Option<u16>) -> bool {
    match status {
        Some(408) | Some(429) => true,
        Some(status) if (500..=599).contains(&status) => true,
        _ => false,
    }
}

// ---------------------------------------------------------------------------
// Repository
// ---------------------------------------------------------------------------

#[derive(Clone, Debug)]
pub struct QueueRepository {
    database: Database,
}

impl QueueRepository {
    pub const fn new(database: Database) -> Self {
        Self { database }
    }

    pub fn database_path(&self) -> &Path {
        self.database.path()
    }

    /// Idempotency key: lowercase hex SHA-256 of
    /// `event_id_bytes || 0x00 || rule_version_bytes || 0x00 || channel_id_bytes`.
    /// `rule_id` is deliberately excluded.
    pub fn idempotency_key(event_id: Uuid, rule_version: &str, channel_id: ChannelId) -> String {
        let mut hasher = Sha256::new();
        hasher.update(event_id.as_bytes());
        hasher.update([0x00]);
        hasher.update(rule_version.as_bytes());
        hasher.update([0x00]);
        hasher.update(channel_id.as_bytes());
        hex::encode(hasher.finalize())
    }

    /// Recent successful delivery times for a (rule, channel) pair, used by the
    /// pipeline's cooldown / per-window-cap evaluation. Returns at most the
    /// last `limit` `last_succeeded_at`-equivalents derived from
    /// `delivery_attempts.completed_at` for succeeded attempts on this rule +
    /// channel.
    pub fn recent_delivery_times(
        &self,
        rule_id: RuleId,
        channel_id: ChannelId,
        limit: usize,
    ) -> Result<Vec<DateTime<Utc>>, AppError> {
        let connection = self.database.connect()?;
        let mut stmt = connection
            .prepare(
                "SELECT a.completed_at FROM delivery_attempts a
                 JOIN delivery_jobs j ON j.id = a.job_id
                 WHERE j.rule_id = ?1 AND j.channel_id = ?2 AND a.outcome = 'succeeded'
                 ORDER BY a.completed_at DESC LIMIT ?3",
            )
            .map_err(|_| query_error())?;
        let rows = stmt
            .query_map(
                params![rule_id.to_string(), channel_id.to_string(), limit as i64],
                |row| row.get::<_, String>(0),
            )
            .map_err(|_| query_error())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|_| query_error())?;
        drop(stmt);
        rows.into_iter().map(|s| parse_time(&s)).collect()
    }

    /// Enqueue a job. Returns [`EnqueueResult::AlreadyExists`] when a job with
    /// the same idempotency key already exists (one job per
    /// event+rule_version+channel). Takes a `BEGIN IMMEDIATE` transaction so
    /// concurrent enqueues of the same logical job serialize.
    pub fn enqueue(&self, job: &DeliveryJob) -> Result<EnqueueResult, AppError> {
        let key = Self::idempotency_key(job.event_id, &job.rule_version, job.channel_id);
        let now = Utc::now().to_rfc3339();
        let document = serde_json::to_string(&job.document).map_err(|_| serialization_error())?;
        let aggregate_release_at = job.aggregate_release_at.map(|time| time.to_rfc3339());

        let mut connection = self.database.connect()?;
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| write_error())?;

        // Look for an existing job with the same idempotency key first.
        let existing: Option<String> = transaction
            .query_row(
                "SELECT id FROM delivery_jobs WHERE idempotency_key = ?1",
                params![&key],
                |row| row.get(0),
            )
            .optional()
            .map_err(|_| query_error())?;
        if let Some(existing_id) = existing {
            let id = parse_uuid(&existing_id)?;
            transaction.commit().map_err(|_| write_error())?;
            return Ok(EnqueueResult::AlreadyExists(id));
        }

        transaction
            .execute(
                "INSERT INTO delivery_jobs (
                    id, event_id, rule_id, rule_version, channel_id, idempotency_key,
                    document_json, state, attempts, next_attempt_at, expires_at,
                    lease_owner, lease_expires_at, aggregate_key, aggregate_release_at,
                    last_error_code, created_at, updated_at
                 ) VALUES (
                    ?1, ?2, ?3, ?4, ?5, ?6, ?7, 'pending', 0, ?8, ?9,
                    NULL, NULL, ?10, ?11, NULL, ?12, ?12
                 )",
                params![
                    job.id.to_string(),
                    job.event_id.to_string(),
                    job.rule_id.to_string(),
                    &job.rule_version,
                    job.channel_id.to_string(),
                    &key,
                    &document,
                    job.next_attempt_at.to_rfc3339(),
                    job.expires_at.to_rfc3339(),
                    job.aggregate_key.as_deref(),
                    aggregate_release_at.as_deref(),
                    now,
                ],
            )
            .map_err(map_enqueue_error)?;
        transaction.commit().map_err(|_| write_error())?;
        Ok(EnqueueResult::Inserted(job.id))
    }

    /// Claim up to `max_batches` due batches atomically. Each batch is either a
    /// single job ([`ClaimedDelivery::Single`]) or all jobs in one aggregate
    /// bucket whose release time has arrived ([`ClaimedDelivery::Aggregate`]).
    ///
    /// Channels that are paused or whose `next_allowed_at` is in the future are
    /// excluded. An expired lease is reclaimable; a live lease is not.
    pub fn claim_due(
        &self,
        worker: &str,
        now: DateTime<Utc>,
        lease_duration: Duration,
        max_batches: usize,
    ) -> Result<Vec<ClaimedDelivery>, AppError> {
        if max_batches == 0 {
            return Ok(Vec::new());
        }
        let now_rfc = now.to_rfc3339();
        let lease_expires_at = (now + lease_duration).to_rfc3339();

        let mut connection = self.database.connect()?;
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| write_error())?;

        // Candidate single jobs: due, claimable, channel available, no
        // aggregate key. Order is stable so two racing workers split work
        // deterministically. A job in `sending` whose lease has expired is
        // reclaimable (worker crash recovery).
        let mut singles_stmt = transaction
            .prepare(
                "SELECT j.id FROM delivery_jobs j
                 JOIN channels c ON c.id = j.channel_id
                 WHERE (j.state IN ('pending', 'retry_wait')
                        OR (j.state = 'sending'
                            AND j.lease_expires_at IS NOT NULL
                            AND j.lease_expires_at <= ?1))
                   AND j.next_attempt_at <= ?1
                   AND (j.lease_expires_at IS NULL OR j.lease_expires_at <= ?1)
                   AND j.aggregate_key IS NULL
                   AND c.paused_reason_code IS NULL
                   AND (c.next_allowed_at IS NULL OR c.next_allowed_at <= ?1)
                 ORDER BY j.next_attempt_at, j.created_at, j.id",
            )
            .map_err(|_| query_error())?;
        let single_ids: Vec<String> = singles_stmt
            .query_map(params![&now_rfc], |row| row.get::<_, String>(0))
            .map_err(|_| query_error())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|_| query_error())?;
        drop(singles_stmt);

        // Candidate aggregate buckets: only buckets whose every constituent's
        // release time has arrived are claimable as a whole (a partial bucket
        // waits). A `sending` constituent whose lease expired is reclaimable.
        let mut bucket_stmt = transaction
            .prepare(
                "SELECT j.aggregate_key,
                        MAX(j.aggregate_release_at) AS bucket_release,
                        SUM(CASE WHEN j.aggregate_release_at <= ?1
                                  AND (j.lease_expires_at IS NULL OR j.lease_expires_at <= ?1)
                             THEN 0 ELSE 1 END) AS blocked
                 FROM delivery_jobs j
                 JOIN channels c ON c.id = j.channel_id
                 WHERE (j.state IN ('pending', 'retry_wait')
                        OR (j.state = 'sending'
                            AND j.lease_expires_at IS NOT NULL
                            AND j.lease_expires_at <= ?1))
                   AND j.aggregate_key IS NOT NULL
                   AND c.paused_reason_code IS NULL
                   AND (c.next_allowed_at IS NULL OR c.next_allowed_at <= ?1)
                 GROUP BY j.aggregate_key
                 HAVING bucket_release <= ?1 AND blocked = 0
                 ORDER BY bucket_release, j.aggregate_key",
            )
            .map_err(|_| query_error())?;
        let buckets: Vec<String> = bucket_stmt
            .query_map(params![&now_rfc], |row| row.get::<_, String>(0))
            .map_err(|_| query_error())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|_| query_error())?;
        drop(bucket_stmt);

        let mut claims = Vec::new();
        let mut lease_owner = worker.to_owned();

        for id in &single_ids {
            if claims.len() >= max_batches {
                break;
            }
            // Atomic conditional claim: only flip the row if its state/lease
            // predicate still holds under our write lock.
            let updated = transaction
                .execute(
                    "UPDATE delivery_jobs
                     SET state = 'sending',
                         lease_owner = ?1,
                         lease_expires_at = ?2,
                         updated_at = ?2
                     WHERE id = ?3
                       AND (state IN ('pending', 'retry_wait')
                            OR (state = 'sending'
                                AND lease_expires_at IS NOT NULL
                                AND lease_expires_at <= ?2))
                       AND (lease_expires_at IS NULL OR lease_expires_at <= ?2)",
                    params![&lease_owner, &lease_expires_at, id],
                )
                .map_err(|_| write_error())?;
            if updated == 0 {
                continue;
            }
            lease_owner = worker.to_owned();
            if let Some(job) = load_job(&transaction, id)? {
                claims.push(ClaimedDelivery::Single { job });
            }
        }

        for key in &buckets {
            if claims.len() >= max_batches {
                break;
            }
            let job_ids = bucket_job_ids(&transaction, key, &now_rfc)?;
            if job_ids.is_empty() {
                continue;
            }
            let mut claimed_jobs: Vec<DeliveryJob> = Vec::new();
            for id in &job_ids {
                let updated = transaction
                    .execute(
                        "UPDATE delivery_jobs
                         SET state = 'sending',
                             lease_owner = ?1,
                             lease_expires_at = ?2,
                             updated_at = ?2
                         WHERE id = ?3
                           AND aggregate_key = ?4
                           AND (state IN ('pending', 'retry_wait')
                                OR (state = 'sending'
                                    AND lease_expires_at IS NOT NULL
                                    AND lease_expires_at <= ?2))
                           AND (lease_expires_at IS NULL OR lease_expires_at <= ?2)",
                        params![&lease_owner, &lease_expires_at, id, key],
                    )
                    .map_err(|_| write_error())?;
                if updated == 0 {
                    // Race: a constituent transitioned under us. Roll the
                    // partial claim back so we never deliver a partial bucket.
                    for earlier in &claimed_jobs {
                        let _ = transaction.execute(
                            "UPDATE delivery_jobs SET state = 'pending', lease_owner = NULL,
                                lease_expires_at = NULL, updated_at = ?1 WHERE id = ?2",
                            params![&now_rfc, earlier.id.to_string()],
                        );
                    }
                    claimed_jobs.clear();
                    break;
                }
                if let Some(job) = load_job(&transaction, id)? {
                    claimed_jobs.push(job);
                }
            }
            if claimed_jobs.is_empty() {
                continue;
            }
            claims.push(ClaimedDelivery::Aggregate {
                aggregate_key: key.clone(),
                jobs: claimed_jobs,
            });
        }

        transaction.commit().map_err(|_| write_error())?;
        Ok(claims)
    }

    /// Mark a single claimed job as succeeded and record a redacted attempt.
    pub fn complete(
        &self,
        job_id: Uuid,
        completed_at: DateTime<Utc>,
        receipt: DeliveryReceipt,
    ) -> Result<(), AppError> {
        let mut connection = self.database.connect()?;
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| write_error())?;
        complete_one(
            &transaction,
            job_id,
            &receipt,
            "succeeded",
            completed_at,
            DeliveryStatus::Sending,
        )?;
        transaction.commit().map_err(|_| write_error())?;
        Ok(())
    }

    /// Mark every constituent of an aggregate claim as succeeded.
    pub fn complete_aggregate(
        &self,
        claim: &ClaimedDelivery,
        completed_at: DateTime<Utc>,
        receipt: DeliveryReceipt,
    ) -> Result<(), AppError> {
        let mut connection = self.database.connect()?;
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| write_error())?;
        for id in claim.constituent_ids() {
            complete_one(
                &transaction,
                id,
                &receipt,
                "succeeded",
                completed_at,
                DeliveryStatus::Sending,
            )?;
        }
        transaction.commit().map_err(|_| write_error())?;
        Ok(())
    }

    /// Apply a retryable/failed outcome to a single claimed job. On a retryable
    /// outcome the job moves to `retry_wait` with `next_attempt_at` set to
    /// `retry_at`, attempts is incremented, a redacted attempt row is written,
    /// and the channel's `next_allowed_at` is bumped to one second after now.
    /// On a permanent failure the job moves to `failed`.
    pub fn retry(
        &self,
        job_id: Uuid,
        now: DateTime<Utc>,
        error: &DeliveryError,
    ) -> Result<RetryOutcome, AppError> {
        let mut connection = self.database.connect()?;
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| write_error())?;
        let outcome = apply_retry(&transaction, job_id, now, error)?;
        transaction.commit().map_err(|_| write_error())?;
        Ok(outcome)
    }

    /// Apply a retryable/failed outcome to every constituent of an aggregate
    /// claim. All jobs share the same retry decision.
    pub fn retry_aggregate(
        &self,
        claim: &ClaimedDelivery,
        now: DateTime<Utc>,
        error: &DeliveryError,
    ) -> Result<RetryOutcome, AppError> {
        let mut connection = self.database.connect()?;
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| write_error())?;
        let mut last = RetryOutcome::Fail;
        for id in claim.constituent_ids() {
            last = apply_retry(&transaction, id, now, error)?;
        }
        transaction.commit().map_err(|_| write_error())?;
        Ok(last)
    }

    /// Move a job to `failed` (permanent). Used for non-retrying failures that
    /// still warrant a terminal `failed` state rather than a channel pause.
    pub fn fail(
        &self,
        job_id: Uuid,
        now: DateTime<Utc>,
        error: &DeliveryError,
    ) -> Result<(), AppError> {
        let mut connection = self.database.connect()?;
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| write_error())?;
        fail_one(&transaction, job_id, now, error)?;
        if matches!(error.kind, DeliveryErrorKind::Authentication) {
            pause_channel_for_auth(&transaction, job_id, PAUSE_AUTH_REASON, now)?;
        }
        transaction.commit().map_err(|_| write_error())?;
        Ok(())
    }

    /// Sweep `pending` / `retry_wait` jobs whose TTL has elapsed into
    /// `expired`.
    pub fn expire_due(&self, now: DateTime<Utc>) -> Result<ExpiryStats, AppError> {
        let now_rfc = now.to_rfc3339();
        let mut connection = self.database.connect()?;
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| write_error())?;
        let expired = transaction
            .execute(
                "UPDATE delivery_jobs
                 SET state = 'expired',
                     lease_owner = NULL,
                     lease_expires_at = NULL,
                     updated_at = ?1
                 WHERE state IN ('pending', 'retry_wait')
                   AND expires_at <= ?1",
                params![&now_rfc],
            )
            .map_err(|_| write_error())?;
        transaction.commit().map_err(|_| write_error())?;
        Ok(ExpiryStats {
            expired: expired as u64,
        })
    }

    /// Move a `failed` job back to `pending`, reset attempts to 0, and keep the
    /// idempotency key. Refuses expired jobs (`delivery.expired`) and jobs
    /// whose channel is paused or rate-limited (`delivery.channel_unavailable`).
    pub fn manual_retry(&self, job_id: Uuid, now: DateTime<Utc>) -> Result<(), AppError> {
        let now_rfc = now.to_rfc3339();
        let mut connection = self.database.connect()?;
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| write_error())?;

        let row = transaction
            .query_row(
                "SELECT j.state, c.paused_reason_code, c.next_allowed_at
                 FROM delivery_jobs j
                 JOIN channels c ON c.id = j.channel_id
                 WHERE j.id = ?1",
                params![job_id.to_string()],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, Option<String>>(1)?,
                        row.get::<_, Option<String>>(2)?,
                    ))
                },
            )
            .optional()
            .map_err(|_| query_error())?
            .ok_or_else(not_found)?;

        let (state, paused, next_allowed) = row;
        if state == "expired" || state == "succeeded" {
            let _ = next_allowed;
            let _ = paused;
            return Err(delivery_error(
                "delivery.expired",
                "job is no longer retryable",
            ));
        }
        if state != "failed" {
            let _ = next_allowed;
            let _ = paused;
            return Err(invalid_transition());
        }
        if paused.is_some() {
            return Err(delivery_error(
                "delivery.channel_unavailable",
                "channel is paused",
            ));
        }
        if let Some(allowed) = next_allowed
            && let Ok(allowed_at) = parse_time(&allowed)
            && allowed_at > now
        {
            return Err(delivery_error(
                "delivery.channel_unavailable",
                "channel is rate-limited",
            ));
        }

        let updated = transaction
            .execute(
                "UPDATE delivery_jobs
                 SET state = 'pending',
                     attempts = 0,
                     next_attempt_at = ?1,
                     lease_owner = NULL,
                     lease_expires_at = NULL,
                     last_error_code = NULL,
                     updated_at = ?1
                 WHERE id = ?2 AND state = 'failed'",
                params![&now_rfc, job_id.to_string()],
            )
            .map_err(|_| write_error())?;
        if updated == 0 {
            return Err(invalid_transition());
        }
        transaction.commit().map_err(|_| write_error())?;
        Ok(())
    }

    /// Atomically set a channel's `next_allowed_at`, used for per-channel rate
    /// limiting. A server `Retry-After` may move it further into the future
    /// than the default 1-second cool-down.
    pub fn set_channel_next_allowed(
        &self,
        channel_id: ChannelId,
        next_allowed_at: DateTime<Utc>,
    ) -> Result<(), AppError> {
        let connection = self.database.connect()?;
        connection
            .execute(
                "UPDATE channels SET next_allowed_at = ?1, updated_at = ?1 WHERE id = ?2",
                params![next_allowed_at.to_rfc3339(), channel_id.to_string()],
            )
            .map_err(|_| write_error())?;
        Ok(())
    }

    /// Bump a channel's `consecutive_auth_failures` by one and, when the
    /// design §15.2 threshold (3) is reached, flip `health_status` to
    /// `paused_authentication` with `paused_reason_code = 'authentication_failed'`.
    /// Used by the worker when a `Signature`/`Permission` failure bypasses
    /// the queue's own auth-bump path (which only fires for `Authentication`).
    /// Mirrors the SQL in [`pause_channel_for_auth`] but is keyed directly on
    /// `channel_id` rather than on a job row.
    pub fn bump_auth_failure(
        &self,
        channel_id: ChannelId,
        now: DateTime<Utc>,
    ) -> Result<(), AppError> {
        let connection = self.database.connect()?;
        bump_auth_failure_on(&connection, channel_id, now)?;
        Ok(())
    }

    /// Reset a channel's auth-failure state to healthy after a successful
    /// send: `consecutive_auth_failures = 0`, `health_status = 'healthy'`,
    /// `paused_reason_code = NULL`, and stamp `last_succeeded_at`.
    pub fn reset_auth_failures(
        &self,
        channel_id: ChannelId,
        now: DateTime<Utc>,
    ) -> Result<(), AppError> {
        let connection = self.database.connect()?;
        reset_auth_failures_on(&connection, channel_id, now)?;
        Ok(())
    }

    /// Read a channel's `health_status` text. Returns `None` if the row is
    /// absent or the column is unreadable. Used by the worker to detect the
    /// `paused_authentication` transition and emit `CoreEvent::HealthChanged`.
    pub fn channel_health_status_for(&self, channel_id: ChannelId) -> Option<String> {
        let connection = self.database.connect().ok()?;
        connection
            .query_row(
                "SELECT health_status FROM channels WHERE id = ?1",
                params![channel_id.to_string()],
                |row| row.get::<_, String>(0),
            )
            .ok()
    }

    /// Per-state queue counters.
    pub fn queue_stats(&self) -> Result<QueueStats, AppError> {
        let connection = self.database.connect()?;
        let counts: Vec<(String, i64)> = {
            let mut stmt = connection
                .prepare(
                    "SELECT state, COUNT(*) FROM delivery_jobs
                     WHERE state IN ('pending','sending','retry_wait','succeeded','failed','expired')
                     GROUP BY state",
                )
                .map_err(|_| query_error())?;
            let rows = stmt
                .query_map([], |row| {
                    Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
                })
                .map_err(|_| query_error())?;
            let mut out = Vec::new();
            for row in rows {
                let (state, count) = row.map_err(|_| query_error())?;
                out.push((state, count));
            }
            out
        };
        let mut stats = QueueStats::default();
        for (state, count) in counts {
            match state.as_str() {
                "pending" => stats.pending = count as u64,
                "sending" => stats.sending = count as u64,
                "retry_wait" => stats.retry_wait = count as u64,
                "succeeded" => stats.succeeded = count as u64,
                "failed" => stats.failed = count as u64,
                "expired" => stats.expired = count as u64,
                _ => {}
            }
        }
        Ok(stats)
    }
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

fn complete_one(
    transaction: &rusqlite::Transaction<'_>,
    job_id: Uuid,
    receipt: &DeliveryReceipt,
    outcome: &str,
    completed_at: DateTime<Utc>,
    expected_from: DeliveryStatus,
) -> Result<(), AppError> {
    let now_rfc = completed_at.to_rfc3339();
    let updated = transaction
        .execute(
            "UPDATE delivery_jobs
             SET state = 'succeeded',
                 attempts = attempts + 1,
                 lease_owner = NULL,
                 lease_expires_at = NULL,
                 last_error_code = NULL,
                 updated_at = ?1
             WHERE id = ?2 AND state = ?3",
            params![&now_rfc, job_id.to_string(), expected_from.as_str()],
        )
        .map_err(|_| write_error())?;
    if updated == 0 {
        return Err(invalid_transition());
    }
    write_attempt(
        transaction,
        job_id,
        AttemptRecord {
            outcome,
            http_status: Some(receipt.http_status),
            platform_code: receipt.platform_code.as_deref(),
            error_code: None,
            retry_at: None,
        },
        completed_at,
    )?;
    Ok(())
}

fn apply_retry(
    transaction: &rusqlite::Transaction<'_>,
    job_id: Uuid,
    now: DateTime<Utc>,
    error: &DeliveryError,
) -> Result<RetryOutcome, AppError> {
    // Load current attempts + expires_at for the classifier.
    let row = transaction
        .query_row(
            "SELECT attempts, expires_at FROM delivery_jobs WHERE id = ?1 AND state = 'sending'",
            params![job_id.to_string()],
            |row| Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?)),
        )
        .optional()
        .map_err(|_| query_error())?
        .ok_or_else(invalid_transition)?;
    let (attempts, expires_rfc) = row;
    let expires_at = parse_time(&expires_rfc)?;

    let policy = RetryPolicy::default();
    let attempt = AttemptInput {
        attempts: attempts as u8,
        expires_at,
        classified_at: now,
    };
    let decision = policy.classify(attempt, error);

    let outcome_label = outcome_label_for(error);
    let now_rfc = now.to_rfc3339();

    match decision {
        RetryDecision::RetryAt(retry_at) => {
            let retry_rfc = retry_at.to_rfc3339();
            let updated = transaction
                .execute(
                    "UPDATE delivery_jobs
                     SET state = 'retry_wait',
                         attempts = attempts + 1,
                         next_attempt_at = ?1,
                         lease_owner = NULL,
                         lease_expires_at = NULL,
                         last_error_code = ?2,
                         updated_at = ?1
                     WHERE id = ?3 AND state = 'sending'",
                    params![&retry_rfc, error.code, job_id.to_string()],
                )
                .map_err(|_| write_error())?;
            if updated == 0 {
                return Err(invalid_transition());
            }
            write_attempt(
                transaction,
                job_id,
                AttemptRecord {
                    outcome: outcome_label,
                    http_status: error.http_status,
                    platform_code: error.platform_code.as_deref(),
                    error_code: Some(error.code.as_str()),
                    retry_at: Some(&retry_rfc),
                },
                now,
            )?;
            bump_channel_rate_limit(transaction, job_id, Some(retry_at))?;
            // ponytail: we return the next-attempt time; the worker may also
            // observe Retry-After via the channel row's next_allowed_at.
            Ok(RetryOutcome::RetryAt(retry_at))
        }
        RetryDecision::PauseChannel { reason_code } => {
            // classify never emits this today (auth errors are `Fail` and the
            // pause bookkeeping runs in the Fail arm), but the variant is kept
            // so the repository can honour an explicit pause decision if a
            // future caller supplies one.
            fail_one(transaction, job_id, now, error)?;
            pause_channel_for_auth(transaction, job_id, reason_code, now)?;
            Ok(RetryOutcome::Fail)
        }
        RetryDecision::Fail => {
            fail_one(transaction, job_id, now, error)?;
            // Authentication failures don't retry, but they do count toward the
            // 3-strike channel pause (design §15.2).
            if matches!(error.kind, DeliveryErrorKind::Authentication) {
                pause_channel_for_auth(transaction, job_id, PAUSE_AUTH_REASON, now)?;
            }
            Ok(RetryOutcome::Fail)
        }
        RetryDecision::Expire => {
            let updated = transaction
                .execute(
                    "UPDATE delivery_jobs
                     SET state = 'expired',
                         attempts = attempts + 1,
                         lease_owner = NULL,
                         lease_expires_at = NULL,
                         last_error_code = ?1,
                         updated_at = ?2
                     WHERE id = ?3 AND state = 'sending'",
                    params![error.code, &now_rfc, job_id.to_string()],
                )
                .map_err(|_| write_error())?;
            if updated == 0 {
                return Err(invalid_transition());
            }
            write_attempt(
                transaction,
                job_id,
                AttemptRecord {
                    outcome: outcome_label,
                    http_status: error.http_status,
                    platform_code: error.platform_code.as_deref(),
                    error_code: Some(error.code.as_str()),
                    retry_at: None,
                },
                now,
            )?;
            Ok(RetryOutcome::Fail)
        }
    }
}

fn fail_one(
    transaction: &rusqlite::Transaction<'_>,
    job_id: Uuid,
    now: DateTime<Utc>,
    error: &DeliveryError,
) -> Result<(), AppError> {
    let now_rfc = now.to_rfc3339();
    let updated = transaction
        .execute(
            "UPDATE delivery_jobs
             SET state = 'failed',
                 attempts = attempts + 1,
                 lease_owner = NULL,
                 lease_expires_at = NULL,
                 last_error_code = ?1,
                 updated_at = ?2
             WHERE id = ?3 AND state = 'sending'",
            params![error.code, &now_rfc, job_id.to_string()],
        )
        .map_err(|_| write_error())?;
    if updated == 0 {
        return Err(invalid_transition());
    }
    write_attempt(
        transaction,
        job_id,
        AttemptRecord {
            outcome: outcome_label_for(error),
            http_status: error.http_status,
            platform_code: error.platform_code.as_deref(),
            error_code: Some(error.code.as_str()),
            retry_at: None,
        },
        now,
    )?;
    Ok(())
}

/// Structured columns written to `delivery_attempts`. Grouped so the
/// `write_attempt` helper stays under clippy's argument limit and callers
/// don't have to remember a 6-arg order.
struct AttemptRecord<'a> {
    outcome: &'a str,
    http_status: Option<u16>,
    platform_code: Option<&'a str>,
    error_code: Option<&'a str>,
    retry_at: Option<&'a str>,
}

fn write_attempt(
    transaction: &rusqlite::Transaction<'_>,
    job_id: Uuid,
    record: AttemptRecord<'_>,
    completed_at: DateTime<Utc>,
) -> Result<(), AppError> {
    let next_attempt_number = next_attempt_number(transaction, job_id)?;
    let id = Uuid::now_v7().to_string();
    let now_rfc = completed_at.to_rfc3339();
    transaction
        .execute(
            "INSERT INTO delivery_attempts (
                id, job_id, attempt_number, started_at, completed_at, outcome,
                http_status, platform_code, error_code, retry_at, redacted_detail
             ) VALUES (?1, ?2, ?3, ?4, ?4, ?5, ?6, ?7, ?8, ?9, NULL)",
            params![
                id,
                job_id.to_string(),
                next_attempt_number,
                &now_rfc,
                record.outcome,
                record.http_status,
                record.platform_code,
                record.error_code,
                record.retry_at,
            ],
        )
        .map_err(|_| write_error())?;
    Ok(())
}

fn next_attempt_number(
    transaction: &rusqlite::Transaction<'_>,
    job_id: Uuid,
) -> Result<i64, AppError> {
    let max: i64 = transaction
        .query_row(
            "SELECT COALESCE(MAX(attempt_number), 0) FROM delivery_attempts WHERE job_id = ?1",
            params![job_id.to_string()],
            |row| row.get(0),
        )
        .map_err(|_| query_error())?;
    Ok(max + 1)
}

fn bump_channel_rate_limit(
    transaction: &rusqlite::Transaction<'_>,
    job_id: Uuid,
    retry_at: Option<DateTime<Utc>>,
) -> Result<(), AppError> {
    // At least one second of cool-down; a Retry-After may push it further.
    let one_sec = Utc::now() + Duration::seconds(1);
    let target = retry_at.map(|t| t.max(one_sec)).unwrap_or(one_sec);
    let target_rfc = target.to_rfc3339();
    transaction
        .execute(
            "UPDATE channels SET next_allowed_at = ?1, updated_at = ?1
             WHERE id = (SELECT channel_id FROM delivery_jobs WHERE id = ?2)",
            params![&target_rfc, job_id.to_string()],
        )
        .map_err(|_| write_error())?;
    Ok(())
}

fn pause_channel_for_auth(
    transaction: &rusqlite::Transaction<'_>,
    job_id: Uuid,
    reason_code: &str,
    now: DateTime<Utc>,
) -> Result<(), AppError> {
    let now_rfc = now.to_rfc3339();
    // Bump consecutive_auth_failures and flip health to paused_authentication
    // once the threshold is hit. The 3-strike rule (design §15.2) lives here.
    transaction
        .execute(
            "UPDATE channels
             SET consecutive_auth_failures = consecutive_auth_failures + 1,
                 health_status = CASE
                     WHEN consecutive_auth_failures + 1 >= ?1 THEN 'paused_authentication'
                     ELSE health_status
                 END,
                 paused_reason_code = CASE
                     WHEN consecutive_auth_failures + 1 >= ?1 THEN ?2
                     ELSE paused_reason_code
                 END,
                 updated_at = ?3
             WHERE id = (SELECT channel_id FROM delivery_jobs WHERE id = ?4)",
            params![
                AUTH_PAUSE_THRESHOLD,
                reason_code,
                &now_rfc,
                job_id.to_string()
            ],
        )
        .map_err(|_| write_error())?;
    Ok(())
}

fn bump_auth_failure_on(
    connection: &rusqlite::Connection,
    channel_id: ChannelId,
    now: DateTime<Utc>,
) -> Result<(), AppError> {
    let now_rfc = now.to_rfc3339();
    connection
        .execute(
            "UPDATE channels
             SET consecutive_auth_failures = consecutive_auth_failures + 1,
                 health_status = CASE
                     WHEN consecutive_auth_failures + 1 >= ?1 THEN 'paused_authentication'
                     ELSE health_status
                 END,
                 paused_reason_code = CASE
                     WHEN consecutive_auth_failures + 1 >= ?1 THEN ?2
                     ELSE paused_reason_code
                 END,
                 updated_at = ?3
             WHERE id = ?4",
            params![
                AUTH_PAUSE_THRESHOLD,
                PAUSE_AUTH_REASON,
                &now_rfc,
                channel_id.to_string()
            ],
        )
        .map_err(|_| write_error())?;
    Ok(())
}

fn reset_auth_failures_on(
    connection: &rusqlite::Connection,
    channel_id: ChannelId,
    now: DateTime<Utc>,
) -> Result<(), AppError> {
    let now_rfc = now.to_rfc3339();
    connection
        .execute(
            "UPDATE channels
             SET consecutive_auth_failures = 0,
                 health_status = 'healthy',
                 paused_reason_code = NULL,
                 last_succeeded_at = ?1,
                 updated_at = ?1
             WHERE id = ?2",
            params![&now_rfc, channel_id.to_string()],
        )
        .map_err(|_| write_error())?;
    Ok(())
}

fn bucket_job_ids(
    transaction: &rusqlite::Transaction<'_>,
    aggregate_key: &str,
    now_rfc: &str,
) -> Result<Vec<String>, AppError> {
    let mut stmt = transaction
        .prepare(
            "SELECT id FROM delivery_jobs
             WHERE aggregate_key = ?1
               AND (state IN ('pending', 'retry_wait')
                    OR (state = 'sending'
                        AND lease_expires_at IS NOT NULL
                        AND lease_expires_at <= ?2))
               AND aggregate_release_at <= ?2
               AND (lease_expires_at IS NULL OR lease_expires_at <= ?2)
             ORDER BY created_at, id",
        )
        .map_err(|_| query_error())?;
    let ids = stmt
        .query_map(params![aggregate_key, now_rfc], |row| {
            row.get::<_, String>(0)
        })
        .map_err(|_| query_error())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| query_error())?;
    Ok(ids)
}

fn load_job(
    transaction: &rusqlite::Transaction<'_>,
    id: &str,
) -> Result<Option<DeliveryJob>, AppError> {
    let row = transaction
        .query_row(
            "SELECT id, event_id, rule_id, rule_version, channel_id, idempotency_key,
                    document_json, state, attempts, next_attempt_at, expires_at,
                    lease_owner, lease_expires_at, aggregate_key, aggregate_release_at
             FROM delivery_jobs WHERE id = ?1",
            params![id],
            job_row,
        )
        .optional()
        .map_err(|_| query_error())?;
    Ok(row)
}

fn job_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<DeliveryJob> {
    let id: String = row.get(0)?;
    let event_id: String = row.get(1)?;
    let rule_id: String = row.get(2)?;
    let rule_version: String = row.get(3)?;
    let channel_id: String = row.get(4)?;
    let idempotency_key: String = row.get(5)?;
    let document_json: String = row.get(6)?;
    let state: String = row.get(7)?;
    let attempts: i64 = row.get(8)?;
    let next_attempt_at: String = row.get(9)?;
    let expires_at: String = row.get(10)?;
    let lease_owner: Option<String> = row.get(11)?;
    let lease_expires_at: Option<String> = row.get(12)?;
    let aggregate_key: Option<String> = row.get(13)?;
    let aggregate_release_at: Option<String> = row.get(14)?;
    stored_result((|| -> Result<_, AppError> {
        let state = DeliveryStatus::parse(&state).ok_or_else(stored_data_error)?;
        let document: NotificationDocument =
            serde_json::from_str(&document_json).map_err(|_| stored_data_error())?;
        Ok(DeliveryJob {
            id: parse_uuid(&id)?,
            event_id: parse_uuid(&event_id)?,
            rule_id: parse_uuid(&rule_id)?,
            rule_version,
            channel_id: parse_uuid(&channel_id)?,
            idempotency_key,
            document,
            state,
            attempts: attempts.max(0) as u8,
            next_attempt_at: parse_time(&next_attempt_at)?,
            expires_at: parse_time(&expires_at)?,
            lease_owner,
            lease_expires_at: lease_release(lease_expires_at)?,
            aggregate_key,
            aggregate_release_at: lease_release(aggregate_release_at)?,
        })
    })())
}

fn lease_release(value: Option<String>) -> Result<Option<DateTime<Utc>>, AppError> {
    match value {
        None => Ok(None),
        Some(s) => parse_time(&s).map(Some),
    }
}

fn outcome_label_for(error: &DeliveryError) -> &'static str {
    match error.kind {
        DeliveryErrorKind::Authentication => "authentication_failed",
        DeliveryErrorKind::Signature => "signature_failed",
        DeliveryErrorKind::Permission => "permission_denied",
        DeliveryErrorKind::Format => "format_invalid",
        DeliveryErrorKind::Network => "transient_error",
        DeliveryErrorKind::Timeout => "timeout",
        DeliveryErrorKind::HttpStatus => "http_error",
        DeliveryErrorKind::TemporaryPlatform => "transient_error",
    }
}

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

const MAX_ATTEMPTS: u8 = 5;
const AUTH_PAUSE_THRESHOLD: i64 = 3;
const BACKOFF_BASE: Duration = Duration::seconds(2);
const BACKOFF_CAP: Duration = Duration::minutes(5);
const PAUSE_AUTH_REASON: &str = "authentication_failed";

// ---------------------------------------------------------------------------
// Error helpers
// ---------------------------------------------------------------------------

fn map_enqueue_error(error: rusqlite::Error) -> AppError {
    // UNIQUE violation on idempotency_key -> already exists (race-resolved).
    if let rusqlite::Error::SqliteFailure(failure, _) = &error
        && failure.code == rusqlite::ErrorCode::ConstraintViolation
    {
        return storage_error("storage.duplicate_delivery", "delivery job already exists");
    }
    write_error()
}

fn invalid_transition() -> AppError {
    storage_error(
        "storage.invalid_delivery_transition",
        "delivery job cannot transition from its current state",
    )
}

fn not_found() -> AppError {
    storage_error("storage.not_found", "delivery job was not found")
}

fn delivery_error(code: &str, message: &str) -> AppError {
    AppError {
        domain: ErrorDomain::Delivery,
        code: code.to_owned(),
        message: message.to_owned(),
        suggested_action: None,
    }
}

fn parse_uuid(value: &str) -> Result<Uuid, AppError> {
    Uuid::parse_str(value).map_err(|_| stored_data_error())
}

fn parse_time(value: &str) -> Result<DateTime<Utc>, AppError> {
    DateTime::parse_from_rfc3339(value)
        .map(|value| value.with_timezone(&Utc))
        .map_err(|_| stored_data_error())
}

fn stored_result<T>(value: Result<T, AppError>) -> rusqlite::Result<T> {
    value.map_err(|_| rusqlite::Error::InvalidQuery)
}

/// Enqueue a job into an existing transaction. Used by the live ingestion
/// path so the job is committed atomically with its event, outcome and the
/// hook-seen transition. Same idempotency contract as
/// [`QueueRepository::enqueue`].
pub fn enqueue_in_tx(
    transaction: &rusqlite::Transaction<'_>,
    job: &DeliveryJob,
) -> Result<EnqueueResult, AppError> {
    let key = QueueRepository::idempotency_key(job.event_id, &job.rule_version, job.channel_id);
    let now = Utc::now().to_rfc3339();
    let document = serde_json::to_string(&job.document).map_err(|_| serialization_error())?;
    let aggregate_release_at = job.aggregate_release_at.map(|time| time.to_rfc3339());

    let existing: Option<String> = transaction
        .query_row(
            "SELECT id FROM delivery_jobs WHERE idempotency_key = ?1",
            params![&key],
            |row| row.get(0),
        )
        .optional()
        .map_err(|_| query_error())?;
    if let Some(existing_id) = existing {
        let id = parse_uuid(&existing_id)?;
        return Ok(EnqueueResult::AlreadyExists(id));
    }
    transaction
        .execute(
            "INSERT INTO delivery_jobs (
                id, event_id, rule_id, rule_version, channel_id, idempotency_key,
                document_json, state, attempts, next_attempt_at, expires_at,
                lease_owner, lease_expires_at, aggregate_key, aggregate_release_at,
                last_error_code, created_at, updated_at
             ) VALUES (
                ?1, ?2, ?3, ?4, ?5, ?6, ?7, 'pending', 0, ?8, ?9,
                NULL, NULL, ?10, ?11, NULL, ?12, ?12
             )",
            params![
                job.id.to_string(),
                job.event_id.to_string(),
                job.rule_id.to_string(),
                &job.rule_version,
                job.channel_id.to_string(),
                &key,
                &document,
                job.next_attempt_at.to_rfc3339(),
                job.expires_at.to_rfc3339(),
                job.aggregate_key.as_deref(),
                aggregate_release_at.as_deref(),
                now,
            ],
        )
        .map_err(map_enqueue_error)?;
    Ok(EnqueueResult::Inserted(job.id))
}

fn serialization_error() -> AppError {
    storage_error(
        "storage.serialization_failed",
        "typed storage value could not be serialized",
    )
}

fn stored_data_error() -> AppError {
    storage_error(
        "storage.invalid_stored_data",
        "stored data could not be decoded",
    )
}

fn query_error() -> AppError {
    storage_error("storage.query_failed", "database query failed")
}

fn write_error() -> AppError {
    storage_error("storage.write_failed", "database write failed")
}

// ---------------------------------------------------------------------------
// Test-only helpers.
//
// Helpers used by the in-crate test module are `#[cfg(test)]`; helpers also
// used by the integration test in tests/storage_recovery.rs are exposed under
// the `test-support` feature so the external test crate can call them.
// ---------------------------------------------------------------------------

#[cfg(test)]
impl QueueRepository {
    pub fn count_jobs_for_test(&self) -> usize {
        let connection = self.database.connect().unwrap();
        let count: i64 = connection
            .query_row("SELECT COUNT(*) FROM delivery_jobs", [], |row| row.get(0))
            .unwrap_or(0);
        count as usize
    }

    pub fn only_job_id_for_test(&self) -> Uuid {
        let connection = self.database.connect().unwrap();
        let id: String = connection
            .query_row(
                "SELECT id FROM delivery_jobs ORDER BY created_at, id LIMIT 1",
                [],
                |row| row.get(0),
            )
            .unwrap();
        Uuid::parse_str(&id).unwrap()
    }

    pub fn job_attempts_for_test(&self, job_id: Uuid) -> u8 {
        let connection = self.database.connect().unwrap();
        let attempts: i64 = connection
            .query_row(
                "SELECT attempts FROM delivery_jobs WHERE id = ?1",
                params![job_id.to_string()],
                |row| row.get(0),
            )
            .unwrap();
        attempts as u8
    }

    pub fn set_channel_state_for_test(
        &self,
        channel_id: ChannelId,
        reason_code: &str,
        _at: DateTime<Utc>,
    ) {
        let connection = self.database.connect().unwrap();
        connection
            .execute(
                "UPDATE channels SET paused_reason_code = ?1, health_status = 'paused_authentication',
                    updated_at = ?2 WHERE id = ?3",
                params![reason_code, Utc::now().to_rfc3339(), channel_id.to_string()],
            )
            .unwrap();
    }
}

#[cfg(any(test, feature = "test-support"))]
impl QueueRepository {
    #[cfg(any(test, feature = "test-support"))]
    pub fn database_for_test(&self) -> &Database {
        &self.database
    }

    pub fn set_job_state_for_test(&self, job_id: Uuid, state: DeliveryStatus, _at: DateTime<Utc>) {
        let connection = self.database.connect().unwrap();
        connection
            .execute(
                "UPDATE delivery_jobs SET state = ?1, lease_owner = NULL, lease_expires_at = NULL,
                    updated_at = ?2 WHERE id = ?3",
                params![state.as_str(), Utc::now().to_rfc3339(), job_id.to_string()],
            )
            .unwrap();
        if state == DeliveryStatus::Sending {
            // Reflect a lease so subsequent state-machine transitions see the
            // expected precondition (sending with a lease owner).
            connection
                .execute(
                    "UPDATE delivery_jobs SET lease_owner = 'test', lease_expires_at = ?1
                     WHERE id = ?2",
                    params![
                        (Utc::now() + Duration::minutes(1)).to_rfc3339(),
                        job_id.to_string()
                    ],
                )
                .unwrap();
        }
    }

    pub fn job_state_for_test(&self, job_id: Uuid) -> DeliveryStatus {
        let connection = self.database.connect().unwrap();
        let state: String = connection
            .query_row(
                "SELECT state FROM delivery_jobs WHERE id = ?1",
                params![job_id.to_string()],
                |row| row.get(0),
            )
            .unwrap();
        DeliveryStatus::parse(&state).unwrap()
    }
}

#[cfg(test)]
mod tests;
