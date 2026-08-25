//! Retention: bounded deletion of expired history (Task 20).
//!
//! Deletes expired delivery attempts/jobs/events and processed ingress in
//! bounded batches inside one transaction per pass, checkpoints WAL, vacuums
//! only when >20% of pages are free AND no worker lease is live, and deletes
//! log files older than the configured log retention. Configuration (rules,
//! channels, projects, settings) is never touched.

use chrono::{DateTime, Utc};
use rusqlite::params;

use crate::error::{AppError, ErrorDomain};
use crate::storage::db::Database;

/// Delete in batches of this size so a single pass holds bounded row locks.
const DELETE_BATCH: i64 = 500;

fn retention_error(code: &str, message: &str) -> AppError {
    AppError {
        domain: ErrorDomain::Storage,
        code: code.to_owned(),
        message: message.to_owned(),
        suggested_action: None,
    }
}

#[derive(Clone, Debug)]
pub struct RetentionService {
    database: Database,
    logs: std::path::PathBuf,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RetentionStats {
    pub deleted_events: u64,
    pub deleted_jobs: u64,
    pub deleted_attempts: u64,
    pub deleted_ingress: u64,
    pub vacuumed: bool,
}

impl RetentionService {
    pub fn new(database: Database, logs: impl Into<std::path::PathBuf>) -> Self {
        Self {
            database,
            logs: logs.into(),
        }
    }

    /// One retention pass. Reads the configured retention days from settings
    /// (falling back to 30-day events / 7-day logs), then performs the
    /// bounded transactional deletes and the gated vacuum.
    pub fn run_once(&self, now: DateTime<Utc>) -> Result<RetentionStats, AppError> {
        let connection = self.database.connect()?;
        let (event_days, log_days) = retention_settings(&self.database);
        let event_cutoff = (now - chrono::Duration::days(i64::from(event_days))).to_rfc3339();
        let log_cutoff = now - chrono::Duration::days(i64::from(log_days));

        let mut stats = RetentionStats::default();

        // 1. Delivery attempts for jobs that will be deleted, then the jobs,
        //    then events with no remaining jobs, then processed ingress — all
        //    in bounded batches inside one immediate transaction.
        connection.execute_batch("BEGIN IMMEDIATE").map_err(|_| {
            retention_error("storage.retention_failed", "retention could not start")
        })?;
        let result = (|| -> Result<(), AppError> {
            // Attempts whose job is past TTL.
            loop {
                let deleted = connection
                    .execute(
                        "DELETE FROM delivery_attempts WHERE id IN (
                             SELECT a.id FROM delivery_attempts a
                             JOIN delivery_jobs j ON j.id = a.job_id
                             WHERE j.created_at < ?1 AND a.id IN (
                                 SELECT id FROM delivery_attempts LIMIT ?2
                             )
                         )",
                        params![event_cutoff, DELETE_BATCH],
                    )
                    .map_err(|_| {
                        retention_error("storage.retention_failed", "attempt deletion failed")
                    })? as u64;
                stats.deleted_attempts += deleted;
                if deleted < DELETE_BATCH as u64 {
                    break;
                }
            }
            // Terminal/past-TTL jobs.
            loop {
                let deleted = connection
                    .execute(
                        "DELETE FROM delivery_jobs WHERE id IN (
                             SELECT id FROM delivery_jobs
                             WHERE created_at < ?1 LIMIT ?2
                         )",
                        params![event_cutoff, DELETE_BATCH],
                    )
                    .map_err(|_| {
                        retention_error("storage.retention_failed", "job deletion failed")
                    })? as u64;
                stats.deleted_jobs += deleted;
                if deleted < DELETE_BATCH as u64 {
                    break;
                }
            }
            // Events with no remaining delivery jobs and past the cutoff.
            loop {
                let deleted = connection
                    .execute(
                        "DELETE FROM events WHERE id IN (
                             SELECT e.id FROM events e
                             LEFT JOIN delivery_jobs j ON j.event_id = e.id
                             WHERE j.id IS NULL AND e.occurred_at < ?1 LIMIT ?2
                         )",
                        params![event_cutoff, DELETE_BATCH],
                    )
                    .map_err(|_| {
                        retention_error("storage.retention_failed", "event deletion failed")
                    })? as u64;
                stats.deleted_events += deleted;
                if deleted < DELETE_BATCH as u64 {
                    break;
                }
            }
            // Processed ingress older than the cutoff.
            loop {
                let deleted = connection
                    .execute(
                        "DELETE FROM ingress_events WHERE id IN (
                             SELECT id FROM ingress_events
                             WHERE state <> 'pending' AND received_at < ?1 LIMIT ?2
                         )",
                        params![event_cutoff, DELETE_BATCH],
                    )
                    .map_err(|_| {
                        retention_error("storage.retention_failed", "ingress deletion failed")
                    })? as u64;
                stats.deleted_ingress += deleted;
                if deleted < DELETE_BATCH as u64 {
                    break;
                }
            }
            Ok(())
        })();
        match result {
            Ok(()) => connection.execute_batch("COMMIT").map_err(|_| {
                retention_error("storage.retention_failed", "retention commit failed")
            })?,
            Err(error) => {
                let _ = connection.execute_batch("ROLLBACK");
                return Err(error);
            }
        }

        // 2. Checkpoint WAL so the deleted pages return to the main file.
        let _ = connection.execute_batch("PRAGMA wal_checkpoint(TRUNCATE)");

        // 3. Vacuum only when >20% of pages are free AND no live lease.
        if should_vacuum(&connection, now)? {
            let _ = connection.execute_batch("VACUUM");
            stats.vacuumed = true;
        }

        // 4. Log files older than the configured log retention.
        self.delete_old_logs(log_cutoff);

        Ok(stats)
    }

    fn delete_old_logs(&self, cutoff: DateTime<Utc>) {
        if let Ok(entries) = std::fs::read_dir(&self.logs) {
            for entry in entries.flatten() {
                let path = entry.path();
                let expired = std::fs::metadata(&path)
                    .and_then(|meta| meta.modified())
                    .ok()
                    .and_then(|modified| {
                        DateTime::from_timestamp(
                            modified
                                .duration_since(std::time::UNIX_EPOCH)
                                .ok()?
                                .as_secs() as i64,
                            0,
                        )
                    })
                    .is_some_and(|modified| modified < cutoff);
                if expired {
                    let _ = std::fs::remove_file(&path);
                }
            }
        }
    }
}

fn retention_settings(database: &Database) -> (u16, u16) {
    // app_settings is a key/value JSON store; read through the repository so
    // the (unwritten-row) default of 30/7 applies when settings are absent.
    match crate::storage::config::ConfigRepository::new(database.clone()).get_settings() {
        Ok(settings) => (
            settings.event_retention_days.clamp(1, 365),
            settings.log_retention_days.clamp(1, 365),
        ),
        Err(_) => (30, 7),
    }
}

fn should_vacuum(connection: &rusqlite::Connection, now: DateTime<Utc>) -> Result<bool, AppError> {
    let free: i64 = connection
        .query_row("PRAGMA freelist_count", [], |row| row.get(0))
        .map_err(|_| retention_error("storage.retention_failed", "free-page count unavailable"))?;
    let total: i64 = connection
        .query_row("PRAGMA page_count", [], |row| row.get(0))
        .map_err(|_| retention_error("storage.retention_failed", "page count unavailable"))?;
    if total <= 0 || free * 5 <= total {
        return Ok(false);
    }
    let now_rfc = now.to_rfc3339();
    let live_lease: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM delivery_jobs
             WHERE state = 'sending' AND lease_expires_at IS NOT NULL AND lease_expires_at > ?1",
            params![now_rfc],
            |row| row.get(0),
        )
        .map_err(|_| retention_error("storage.retention_failed", "lease check unavailable"))?;
    Ok(live_lease == 0)
}

/// Clear history while preserving every active job's parent event (Task 20).
/// Deletes attempts + terminal jobs, then only events with no remaining
/// pending/sending/retry job, plus processed ingress — one transaction.
pub fn clear_history(database: &Database, now: DateTime<Utc>) -> Result<u64, AppError> {
    let connection = database.connect()?;
    connection
        .execute_batch("BEGIN IMMEDIATE")
        .map_err(|_| retention_error("storage.clear_failed", "clear-history could not start"))?;
    let result = (|| -> Result<u64, AppError> {
        // Attempts for terminal jobs only.
        connection
            .execute(
                "DELETE FROM delivery_attempts WHERE job_id IN (
                     SELECT id FROM delivery_jobs
                     WHERE state IN ('succeeded','failed','expired')
                 )",
                [],
            )
            .map_err(|_| retention_error("storage.clear_failed", "attempt deletion failed"))?;
        // Terminal jobs only — pending/sending/retry jobs keep their rows.
        connection
            .execute(
                "DELETE FROM delivery_jobs WHERE state IN ('succeeded','failed','expired')",
                [],
            )
            .map_err(|_| retention_error("storage.clear_failed", "job deletion failed"))?;
        // Events with NO remaining job of any state: active jobs' parents
        // survive by construction.
        let deleted = connection
            .execute(
                "DELETE FROM events WHERE id NOT IN (
                     SELECT DISTINCT event_id FROM delivery_jobs
                 )",
                [],
            )
            .map_err(|_| retention_error("storage.clear_failed", "event deletion failed"))?
            as u64;
        connection
            .execute("DELETE FROM ingress_events WHERE state <> 'pending'", [])
            .map_err(|_| retention_error("storage.clear_failed", "ingress deletion failed"))?;
        let _ = now;
        Ok(deleted)
    })();
    match result {
        Ok(deleted) => {
            connection.execute_batch("COMMIT").map_err(|_| {
                retention_error("storage.clear_failed", "clear-history commit failed")
            })?;
            let _ = connection.execute_batch("PRAGMA wal_checkpoint(TRUNCATE)");
            Ok(deleted)
        }
        Err(error) => {
            let _ = connection.execute_batch("ROLLBACK");
            Err(error)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::EventEnvelope;
    use crate::storage::events::EventRepository;
    use crate::storage::queue::QueueRepository;

    struct Harness {
        _root: tempfile::TempDir,
        database: Database,
        service: RetentionService,
    }

    fn harness() -> Harness {
        let root = tempfile::tempdir().unwrap();
        let database = Database::open(
            &root
                .path()
                .join("com.ccreminder.app")
                .join("cc-reminder.sqlite3"),
        )
        .unwrap();
        let service = RetentionService::new(database.clone(), root.path().join("logs"));
        std::fs::create_dir_all(root.path().join("logs")).unwrap();
        Harness {
            _root: root,
            database,
            service,
        }
    }

    fn envelope(id: uuid::Uuid, occurred_at: chrono::DateTime<Utc>) -> EventEnvelope {
        EventEnvelope {
            id,
            source: crate::model::AgentKind::Codex,
            source_version: semver::Version::new(0, 145, 0),
            source_event: "Stop".to_owned(),
            category: crate::model::EventCategory::Completion,
            occurred_at,
            received_at: occurred_at,
            project_id: None,
            project_display_name: None,
            unmatched_cwd_fingerprint: None,
            session_ref: None,
            turn_ref: None,
            model: None,
            permission_mode: None,
            severity: crate::model::Severity::Info,
            public_fields: Default::default(),
            encrypted_sensitive_fields: None,
            correlation_id: uuid::Uuid::now_v7(),
            action_id: None,
            action_capabilities: Vec::new(),
        }
    }

    fn event_count(connection: &rusqlite::Connection) -> i64 {
        connection
            .query_row("SELECT COUNT(*) FROM events", [], |row| row.get(0))
            .unwrap()
    }

    fn rule_count(connection: &rusqlite::Connection) -> i64 {
        connection
            .query_row("SELECT COUNT(*) FROM global_rules", [], |row| row.get(0))
            .unwrap()
    }

    fn channel_count(connection: &rusqlite::Connection) -> i64 {
        connection
            .query_row("SELECT COUNT(*) FROM channels", [], |row| row.get(0))
            .unwrap()
    }

    #[test]
    fn retention_removes_expired_history_and_old_logs_but_keeps_configuration() {
        let harness = harness();
        let events = EventRepository::new(harness.database.clone());
        let old = Utc::now() - chrono::Duration::days(60);
        let recent = Utc::now() - chrono::Duration::days(1);
        events
            .insert_event(
                &envelope(uuid::Uuid::now_v7(), old),
                None,
                crate::storage::events::EventProcessingOutcome::Suppressed,
                None,
            )
            .unwrap();
        events
            .insert_event(
                &envelope(uuid::Uuid::now_v7(), recent),
                None,
                crate::storage::events::EventProcessingOutcome::Suppressed,
                None,
            )
            .unwrap();
        // Configuration: one rule + one channel must survive.
        let config = crate::storage::config::ConfigRepository::new(harness.database.clone());
        let _ = config.ensure_global_rules(&[crate::events::catalog::catalog_for(
            crate::model::AgentKind::Codex,
            &semver::Version::new(0, 145, 0),
        )
        .catalog]);
        let channel_id = uuid::Uuid::now_v7();
        config
            .save_channel(&crate::model::ChannelRecord {
                id: channel_id,
                kind: crate::model::ChannelKind::WeCom,
                name: "ops".to_owned(),
                credential_ref: format!("cc-reminder/channel/{channel_id}"),
                public_config: crate::model::ChannelPublicConfig::WeCom,
                health_status: crate::model::ChannelHealth::Unknown,
                paused_reason_code: None,
                consecutive_auth_failures: 0,
                last_succeeded_at: None,
                next_allowed_at: None,
            })
            .unwrap();

        harness.service.run_once(Utc::now()).unwrap();

        let connection = harness.database.connect().unwrap();
        assert_eq!(event_count(&connection), 1);
        assert!(rule_count(&connection) >= 1);
        assert_eq!(channel_count(&connection), 1);
    }

    #[test]
    fn clear_history_preserves_active_jobs_parent_events() {
        let harness = harness();
        let events = EventRepository::new(harness.database.clone());
        let queue = QueueRepository::new(harness.database.clone());
        let config = crate::storage::config::ConfigRepository::new(harness.database.clone());
        let channel_id = uuid::Uuid::now_v7();
        config
            .save_channel(&crate::model::ChannelRecord {
                id: channel_id,
                kind: crate::model::ChannelKind::WeCom,
                name: "ops".to_owned(),
                credential_ref: format!("cc-reminder/channel/{channel_id}"),
                public_config: crate::model::ChannelPublicConfig::WeCom,
                health_status: crate::model::ChannelHealth::Unknown,
                paused_reason_code: None,
                consecutive_auth_failures: 0,
                last_succeeded_at: None,
                next_allowed_at: None,
            })
            .unwrap();
        let now = Utc::now();
        let active_parent = uuid::Uuid::now_v7();
        let done_parent = uuid::Uuid::now_v7();
        events
            .insert_event(
                &envelope(active_parent, now),
                None,
                crate::storage::events::EventProcessingOutcome::Queued,
                None,
            )
            .unwrap();
        events
            .insert_event(
                &envelope(done_parent, now),
                None,
                crate::storage::events::EventProcessingOutcome::Queued,
                None,
            )
            .unwrap();
        for (event_id, state) in [(active_parent, "pending"), (done_parent, "succeeded")] {
            let job_id = uuid::Uuid::now_v7();
            queue
                .enqueue(&crate::storage::queue::DeliveryJob {
                    id: job_id,
                    event_id,
                    rule_id: uuid::Uuid::now_v7(),
                    rule_version: "v1".to_owned(),
                    channel_id,
                    idempotency_key: format!("k-{event_id}"),
                    document: crate::model::NotificationDocument {
                        title: "t".to_owned(),
                        severity: crate::model::Severity::Info,
                        facts: Vec::new(),
                        body: String::new(),
                        footer: None,
                    },
                    state: crate::storage::queue::DeliveryStatus::Pending,
                    attempts: 0,
                    next_attempt_at: now,
                    expires_at: now + chrono::Duration::minutes(30),
                    lease_owner: None,
                    lease_expires_at: None,
                    aggregate_key: None,
                    aggregate_release_at: None,
                })
                .unwrap();
            let connection = harness.database.connect().unwrap();
            let _ = connection.execute(
                "UPDATE delivery_jobs SET state = ?1 WHERE id = ?2",
                params![state, job_id.to_string()],
            );
        }

        clear_history(&harness.database, now).unwrap();

        let connection = harness.database.connect().unwrap();
        let remaining: i64 = connection
            .query_row("SELECT COUNT(*) FROM events", [], |row| row.get(0))
            .unwrap();
        assert_eq!(remaining, 1);
        let surviving: String = connection
            .query_row("SELECT id FROM events", [], |row| row.get(0))
            .unwrap();
        assert_eq!(surviving, active_parent.to_string());
    }

    #[test]
    fn old_log_files_are_deleted_but_recent_ones_stay() {
        let harness = harness();
        let logs = harness._root.path().join("logs");
        let old_log = logs.join("cc-reminder.log.2");
        std::fs::write(&old_log, b"old").unwrap();
        let old_time =
            std::time::SystemTime::now() - std::time::Duration::from_secs(60 * 60 * 24 * 30);
        std::fs::File::options()
            .write(true)
            .open(&old_log)
            .unwrap()
            .set_modified(old_time)
            .unwrap();
        let fresh_log = logs.join("cc-reminder.log");
        std::fs::write(&fresh_log, b"fresh").unwrap();

        harness.service.run_once(Utc::now()).unwrap();

        assert!(!old_log.exists());
        assert!(fresh_log.exists());
    }
}
