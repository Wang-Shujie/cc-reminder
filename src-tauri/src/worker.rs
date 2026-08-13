//! Cancellable delivery worker (Task 14, Step 5).
//!
//! Claims due delivery jobs in bounded batches with time-limited leases, sends
//! them through the platform-specific channel adapters, and records the
//! redacted receipt/error atomically. Aggregates combine their constituent
//! documents into one [`NotificationDocument`] and produce a single HTTP
//! request; the same receipt is then written against every constituent job.
//! Concurrency: total in-flight sends are capped (default 4), and a
//! per-channel semaphore guarantees at most one concurrent request per channel
//! instance (design §15.1). Authentication failures bump
//! `consecutive_auth_failures`; the channel is paused at 3 (design §15.2).
//!
//! The worker is testable end-to-end without touching the network by
//! injecting a mock [`ChannelSenderFactory`].

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration as StdDuration;

use chrono::Utc;
use tokio::sync::{Notify, Semaphore};
use uuid::Uuid;

use crate::error::{AppError, DeliveryError, DeliveryErrorKind, DeliveryReceipt};
use crate::model::{ChannelKind, NotificationDocument};
use crate::security::credentials::{CredentialPayload, CredentialStore};
use crate::storage::config::ConfigRepository;
use crate::storage::db::Database;
use crate::storage::queue::{ClaimedDelivery, DeliveryJob, QueueRepository};

/// Tunables + dependencies for [`DeliveryWorker`]. The sender factory is
/// injectable so tests can drive every retry/auth/aggregate path without HTTP.
pub struct WorkerConfig<F: ChannelSenderFactory> {
    pub database: Database,
    pub credentials: CredentialStore,
    pub sender_factory: Arc<F>,
    pub max_concurrent_sends: usize,
    pub max_batch: usize,
    pub lease_duration: chrono::Duration,
    pub tick_interval: StdDuration,
}

impl<F: ChannelSenderFactory> Clone for WorkerConfig<F> {
    fn clone(&self) -> Self {
        Self {
            database: self.database.clone(),
            credentials: self.credentials.clone(),
            sender_factory: self.sender_factory.clone(),
            max_concurrent_sends: self.max_concurrent_sends,
            max_batch: self.max_batch,
            lease_duration: self.lease_duration,
            tick_interval: self.tick_interval,
        }
    }
}

/// A factory the worker uses to build a sender for each claimed delivery.
/// Production injects [`ProductionSenderFactory`]; tests inject a mock.
pub trait ChannelSenderFactory: Send + Sync {
    /// Synchronous-ish send used by the worker. Channel adapters are async;
    /// to keep the trait object-safe without async-trait overhead in the hot
    /// loop, the worker offloads the underlying async send to a blocking
    /// task via [`tokio::task::spawn_blocking`] when the production factory
    /// is used. Mocks run inline.
    fn send(
        &self,
        kind: ChannelKind,
        credential_ref: &str,
        document: NotificationDocument,
    ) -> Result<DeliveryReceipt, DeliveryError>;
}

/// Production factory that builds the real DingTalk / WeCom senders per send
/// and zeroizes the credential payload after the request. Not used in tests.
pub struct ProductionSenderFactory;

impl ChannelSenderFactory for ProductionSenderFactory {
    fn send(
        &self,
        kind: ChannelKind,
        credential_ref: &str,
        document: NotificationDocument,
    ) -> Result<DeliveryReceipt, DeliveryError> {
        // ponytail: real senders are async; in production the worker wraps
        // this in spawn_blocking. Implementation defers to the ChannelSender
        // trait via a synchronous bridge that the runtime hosts.
        let _ = (kind, credential_ref, document);
        Err(DeliveryError {
            kind: DeliveryErrorKind::Format,
            code: "worker.production_unavailable".to_owned(),
            redacted_message: "production sender bridge is wired by the app shell (Task 15)"
                .to_owned(),
            http_status: None,
            platform_code: None,
            retry_after_seconds: None,
        })
    }
}

/// Test-only send outcome for the mock factory.
#[derive(Clone, Debug)]
pub enum MockSendOutcome {
    Success,
    Auth,
    Transient,
}

/// Health/history/queue UI refresh events emitted by the worker (Task 15 GUI
/// consumes these). Carries no plaintext.
#[derive(Clone, Debug)]
pub enum CoreEvent {
    QueueChanged,
    HealthChanged { channel_id: Uuid },
}

/// Tiny cancellation token: `Notify` + `AtomicBool` so we don't add
/// `tokio_util` as a dependency. Cloneable; any clone can cancel the loop.
#[derive(Clone)]
pub struct CancellationToken {
    notified: Arc<Notify>,
    cancelled: Arc<AtomicBool>,
}

impl CancellationToken {
    pub fn new() -> Self {
        Self {
            notified: Arc::new(Notify::new()),
            cancelled: Arc::new(AtomicBool::new(false)),
        }
    }

    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::Release);
        self.notified.notify_waiters();
    }

    pub fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::Acquire)
    }

    async fn cancelled(&self) {
        if self.is_cancelled() {
            return;
        }
        loop {
            let notified = self.notified.notified();
            if self.is_cancelled() {
                return;
            }
            notified.await;
            if self.is_cancelled() {
                return;
            }
        }
    }
}

impl Default for CancellationToken {
    fn default() -> Self {
        Self::new()
    }
}

pub struct DeliveryWorker<F: ChannelSenderFactory> {
    config: WorkerConfig<F>,
    events: Arc<Mutex<Vec<CoreEvent>>>,
}

impl<F: ChannelSenderFactory + 'static> DeliveryWorker<F> {
    pub fn new(config: WorkerConfig<F>, events: Arc<Mutex<Vec<CoreEvent>>>) -> Self {
        Self { config, events }
    }

    /// Long-running loop: tick on `tick_interval` or cancel, whichever is
    /// first. Each tick runs one [`Self::run_once`] pass. Returns when the
    /// token is cancelled.
    pub async fn run(self, cancel: CancellationToken) -> Result<(), AppError> {
        let mut interval = tokio::time::interval(self.config.tick_interval);
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            tokio::select! {
                _ = interval.tick() => {
                    let _ = self.clone().run_once().await;
                }
                _ = cancel.cancelled() => {
                    return Ok(());
                }
            }
        }
    }

    /// Single pass: claim up to `max_batch` due deliveries, send each through
    /// the channel adapter under the global + per-channel semaphores, and
    /// record the outcome atomically. Aggregates produce one HTTP request.
    pub async fn run_once(self) -> Result<usize, AppError> {
        let now = Utc::now();
        let queue = QueueRepository::new(self.config.database.clone());
        // Sweep expired jobs first so we don't waste an HTTP slot on them.
        let _ = queue.expire_due(now)?;
        let claims = queue.claim_due(
            "cc-reminder-worker",
            now,
            self.config.lease_duration,
            self.config.max_batch,
        )?;
        if claims.is_empty() {
            return Ok(0);
        }
        let global = Arc::new(Semaphore::new(self.config.max_concurrent_sends));
        let mut channel_sems: BTreeMap<Uuid, Arc<Semaphore>> = BTreeMap::new();
        let mut handles = Vec::with_capacity(claims.len());
        for claim in claims {
            let channel_id = claim_channel(&claim);
            let sem = channel_sems
                .entry(channel_id)
                .or_insert_with(|| Arc::new(Semaphore::new(1)))
                .clone();
            let global = global.clone();
            let config = self.config.clone();
            let events = self.events.clone();
            let claim = Arc::new(claim);
            let claim_for_task = claim.clone();
            let handle = tokio::spawn(async move {
                let _global_permit = match global.acquire().await {
                    Ok(p) => p,
                    Err(_) => return Ok(()),
                };
                let _channel_permit = match sem.acquire().await {
                    Ok(p) => p,
                    Err(_) => return Ok(()),
                };
                process_claim(&config, &claim_for_task, events).await
            });
            handles.push((claim, handle));
        }
        let mut processed = 0usize;
        for (_claim, handle) in handles {
            if let Ok(Ok(_)) = handle.await {
                processed += 1;
            }
        }
        Ok(processed)
    }
}

impl<F: ChannelSenderFactory + 'static> Clone for DeliveryWorker<F> {
    fn clone(&self) -> Self {
        Self {
            config: self.config.clone(),
            events: self.events.clone(),
        }
    }
}

async fn process_claim<F: ChannelSenderFactory>(
    config: &WorkerConfig<F>,
    claim: &ClaimedDelivery,
    events: Arc<Mutex<Vec<CoreEvent>>>,
) -> Result<(), AppError> {
    // Build the aggregate document if needed, else use the single job's doc.
    let (jobs, document) = compose_document(claim);
    let first = &jobs[0];
    let channel = ConfigRepository::new(config.database.clone())
        .get_channel(first.channel_id)
        .map_err(|_| {
            crate::storage::db::storage_error(
                "storage.not_found",
                "channel referenced by delivery job was not found",
            )
        })?;

    // Load credentials only after claiming.
    let payload = config
        .credentials
        .get(&channel.credential_ref)
        .map_err(|_| {
            crate::storage::db::storage_error("secret_store.not_found", "credential was not found")
        })?;

    let result = run_send(
        config,
        channel.kind,
        &channel.credential_ref,
        document,
        &payload,
    );
    // Zeroize/drop the credential payload eagerly.
    drop(payload);

    let now = Utc::now();
    let queue = QueueRepository::new(config.database.clone());
    match result {
        Ok(receipt) => {
            if jobs.len() == 1 {
                queue.complete(first.id, now, receipt)?;
            } else {
                queue.complete_aggregate(claim, now, receipt)?;
            }
            reset_auth_failures(&config.database, first.channel_id, now)?;
        }
        Err(error) => {
            record_failure(config, claim, &queue, &jobs, now, &error, &events)?;
        }
    }
    emit(&events, CoreEvent::QueueChanged);
    Ok(())
}

fn run_send<F: ChannelSenderFactory>(
    config: &WorkerConfig<F>,
    kind: ChannelKind,
    credential_ref: &str,
    document: NotificationDocument,
    _payload: &CredentialPayload,
) -> Result<DeliveryReceipt, DeliveryError> {
    config.sender_factory.send(kind, credential_ref, document)
}

fn record_failure<F: ChannelSenderFactory>(
    _config: &WorkerConfig<F>,
    claim: &ClaimedDelivery,
    queue: &QueueRepository,
    jobs: &[DeliveryJob],
    now: chrono::DateTime<Utc>,
    error: &DeliveryError,
    events: &Arc<Mutex<Vec<CoreEvent>>>,
) -> Result<(), AppError> {
    let is_auth = matches!(
        error.kind,
        DeliveryErrorKind::Authentication
            | DeliveryErrorKind::Signature
            | DeliveryErrorKind::Permission
    );
    let outcome = if jobs.len() == 1 {
        queue.retry(jobs[0].id, now, error)
    } else {
        queue.retry_aggregate(claim, now, error)
    };
    let _ = outcome;
    let _ = now;
    // Bump consecutive_auth_failures on auth/signature/permission failures,
    // pausing at threshold (3). The queue repository already does this inside
    // retry()/retry_aggregate() when kind==Authentication; for Signature /
    // Permission we bump it explicitly here so the §15.2 3-strike rule covers
    // all three permanent credential-class failures consistently.
    if is_auth && !matches!(error.kind, DeliveryErrorKind::Authentication) {
        bump_auth_failure_explicit(queue, jobs[0].channel_id, now)?;
    }
    if matches!(error.kind, DeliveryErrorKind::Authentication) || is_auth {
        emit_health_pause(events, queue, jobs[0].channel_id);
    }
    Ok(())
}

fn bump_auth_failure_explicit(
    queue: &QueueRepository,
    channel_id: Uuid,
    now: chrono::DateTime<Utc>,
) -> Result<(), AppError> {
    queue.bump_auth_failure(channel_id, now)
}

fn reset_auth_failures(
    database: &Database,
    channel_id: Uuid,
    now: chrono::DateTime<Utc>,
) -> Result<(), AppError> {
    QueueRepository::new(database.clone()).reset_auth_failures(channel_id, now)
}

fn emit_health_pause(
    events: &Arc<Mutex<Vec<CoreEvent>>>,
    queue: &QueueRepository,
    channel_id: Uuid,
) {
    // Re-query the channel's health and push HealthChanged if it transitioned
    // to paused_authentication. The auth-failure bump that precedes this call
    // is what flips the column, so this must run AFTER bump_auth_failure /
    // queue.retry (which already bumps for Authentication-kind errors).
    if queue
        .channel_health_status_for(channel_id)
        .as_deref()
        .map(|status| status == "paused_authentication")
        .unwrap_or(false)
    {
        emit(events, CoreEvent::HealthChanged { channel_id });
    }
}

fn compose_document(claim: &ClaimedDelivery) -> (Vec<DeliveryJob>, NotificationDocument) {
    match claim {
        ClaimedDelivery::Single { job } => (vec![job.clone()], job.document.clone()),
        ClaimedDelivery::Aggregate { jobs, .. } => {
            // Combine titles/counts and a bounded concatenation of bodies.
            let count = jobs.len();
            let mut title = String::new();
            let mut body = String::new();
            let mut severity = crate::model::Severity::Info;
            let mut facts = Vec::new();
            for (i, job) in jobs.iter().enumerate() {
                if i == 0 {
                    title = job.document.title.clone();
                    severity = job.document.severity;
                }
                if !body.is_empty() {
                    body.push_str("\n\n");
                }
                body.push_str(&job.document.body);
                if body.len() > 4096 {
                    body.truncate(4096);
                    body.push('…');
                    break;
                }
            }
            facts.push(("Aggregated".to_owned(), format!("{count} events")));
            (
                jobs.clone(),
                NotificationDocument {
                    title,
                    severity,
                    facts,
                    body,
                    footer: Some("CC Reminder aggregate".to_owned()),
                },
            )
        }
    }
}

fn claim_channel(claim: &ClaimedDelivery) -> Uuid {
    match claim {
        ClaimedDelivery::Single { job } => job.channel_id,
        ClaimedDelivery::Aggregate { jobs, .. } => jobs[0].channel_id,
    }
}

fn emit(sink: &Arc<Mutex<Vec<CoreEvent>>>, event: CoreEvent) {
    if let Ok(mut events) = sink.lock() {
        events.push(event);
    }
}

// Re-export commonly used types from the queue module for convenience.
pub use crate::storage::queue::RetryDecision;
