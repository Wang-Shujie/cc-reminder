use std::collections::BTreeMap;
use std::path::Path;

use chrono::{DateTime, Utc};
use rusqlite::{TransactionBehavior, params};
use semver::Version;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

use crate::actions::ActionCapability;
use crate::error::AppError;
use crate::events::normalize::SafeIngressEvent;
use crate::model::{
    AgentKind, ChannelId, EventCategory, EventEnvelope, NotificationDocument, ProjectId,
    ScalarValue, Severity,
};

use super::db::{Database, storage_error};

const MAX_HISTORY_PAGE_SIZE: u16 = 200;
const MAX_INGRESS_BATCH_SIZE: usize = 200;

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum EventProcessingOutcome {
    Queued,
    Suppressed,
    Expired,
    NoTargets,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum EventOutcomeReasonCode {
    UnsupportedCapability,
    Disabled,
    FilterMismatch,
    GlobalPause,
    QuietHours,
    Cooldown,
    WindowLimit,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct EncryptedField {
    pub nonce: Vec<u8>,
    pub ciphertext: Vec<u8>,
}

pub type EncryptedFieldMap = BTreeMap<String, EncryptedField>;

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DeliveryStatus {
    NotQueued,
    Pending,
    Sending,
    RetryWait,
    Succeeded,
    Failed,
    Expired,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct HistoryFilter {
    pub occurred_from: Option<DateTime<Utc>>,
    pub occurred_until: Option<DateTime<Utc>>,
    pub project_id: Option<ProjectId>,
    pub source: Option<AgentKind>,
    pub source_event: Option<String>,
    pub channel_id: Option<ChannelId>,
    pub delivery_status: Option<DeliveryStatus>,
    pub processing_outcome: Option<EventProcessingOutcome>,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct PageRequest {
    pub offset: u32,
    pub limit: u16,
}

impl PageRequest {
    pub const fn first(limit: u16) -> Self {
        Self { offset: 0, limit }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct HistoryPage {
    pub items: Vec<HistoryItem>,
    pub next_offset: Option<u32>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct HistoryItem {
    pub event_id: Uuid,
    pub source: AgentKind,
    pub source_version: Version,
    pub source_event: String,
    pub category: EventCategory,
    pub occurred_at: DateTime<Utc>,
    pub received_at: DateTime<Utc>,
    pub project_id: Option<ProjectId>,
    pub project_display_name: Option<String>,
    pub unmatched_cwd_fingerprint: Option<String>,
    pub session_ref: Option<String>,
    pub turn_ref: Option<String>,
    pub model: Option<String>,
    pub permission_mode: Option<String>,
    pub severity: Severity,
    pub public_fields: BTreeMap<String, ScalarValue>,
    pub correlation_id: Uuid,
    pub action_id: Option<String>,
    pub action_capabilities: Vec<ActionCapability>,
    pub processing_outcome: EventProcessingOutcome,
    pub outcome_reason_code: Option<EventOutcomeReasonCode>,
    pub delivery_job_id: Option<Uuid>,
    pub channel_id: Option<ChannelId>,
    pub document: Option<NotificationDocument>,
    pub delivery_status: DeliveryStatus,
    pub attempts: Vec<DeliveryAttemptDto>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct DeliveryAttemptDto {
    pub attempt_number: u32,
    pub started_at: DateTime<Utc>,
    pub completed_at: DateTime<Utc>,
    pub outcome: String,
    pub http_status: Option<u16>,
    pub platform_code: Option<String>,
    pub error_code: Option<String>,
    pub retry_at: Option<DateTime<Utc>>,
    pub redacted_detail: Option<String>,
}

#[derive(Clone, Debug)]
pub struct EventRepository {
    database: Database,
}

impl EventRepository {
    pub const fn new(database: Database) -> Self {
        Self { database }
    }

    pub fn database_path(&self) -> &Path {
        self.database.path()
    }

    pub fn insert_ingress(&self, event: &SafeIngressEvent) -> Result<(), AppError> {
        let envelope = serde_json::to_string(event).map_err(|_| serialization_error())?;
        let connection = self.database.connect()?;
        connection
            .execute(
                "INSERT OR IGNORE INTO ingress_events(id, safe_envelope_json, received_at, state)
                 VALUES (?1, ?2, ?3, 'pending')",
                params![
                    event.event_id.to_string(),
                    envelope,
                    event.received_at.to_rfc3339()
                ],
            )
            .map_err(|_| write_error())?;
        Ok(())
    }

    pub fn take_ingress_batch(&self, limit: usize) -> Result<Vec<SafeIngressEvent>, AppError> {
        if !(1..=MAX_INGRESS_BATCH_SIZE).contains(&limit) {
            return Err(invalid_pagination());
        }
        let mut connection = self.database.connect()?;
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| write_error())?;
        let rows = {
            let mut statement = transaction
                .prepare(
                    "SELECT id, safe_envelope_json FROM ingress_events
                     WHERE state = 'pending'
                     ORDER BY received_at, id
                     LIMIT ?1",
                )
                .map_err(|_| query_error())?;
            statement
                .query_map([limit as i64], |row| {
                    Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
                })
                .map_err(|_| query_error())?
                .collect::<Result<Vec<_>, _>>()
                .map_err(|_| query_error())?
        };
        let mut events = Vec::with_capacity(rows.len());
        for (id, envelope) in rows {
            let event = serde_json::from_str(&envelope).map_err(|_| serialization_error())?;
            transaction
                .execute(
                    "UPDATE ingress_events SET state = 'processing'
                     WHERE id = ?1 AND state = 'pending'",
                    [id],
                )
                .map_err(|_| write_error())?;
            events.push(event);
        }
        transaction.commit().map_err(|_| write_error())?;
        Ok(events)
    }

    pub fn insert_event(
        &self,
        event: &EventEnvelope,
        encrypted_fields: Option<&EncryptedFieldMap>,
        outcome: EventProcessingOutcome,
        outcome_reason_code: Option<EventOutcomeReasonCode>,
    ) -> Result<(), AppError> {
        let (sensitive_blob_id, sensitive_fields_blob) =
            match (event.encrypted_sensitive_fields.as_ref(), encrypted_fields) {
                (None, None) => (None, None),
                (Some(reference), Some(fields)) => (
                    Some(reference.blob_id.to_string()),
                    Some(serde_json::to_vec(fields).map_err(|_| serialization_error())?),
                ),
                _ => {
                    return Err(storage_error(
                        "storage.invalid_encrypted_fields",
                        "encrypted field reference and ciphertext must be stored together",
                    ));
                }
            };
        let connection = self.database.connect()?;
        let outcome_reason_code = outcome_reason_code.as_ref().map(db_text).transpose()?;
        connection
            .execute(
                "INSERT OR IGNORE INTO events (
                    id, source, source_version, source_event, category, occurred_at, received_at,
                    project_id, project_display_name, unmatched_cwd_fingerprint, session_ref,
                    turn_ref, model, permission_mode, severity, public_fields_json,
                    sensitive_blob_id, sensitive_fields_blob, correlation_id, action_id,
                    action_capabilities_json, processing_outcome, outcome_reason_code, created_at
                 ) VALUES (
                    ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15,
                    ?16, ?17, ?18, ?19, ?20, ?21, ?22, ?23, ?24
                 )",
                params![
                    event.id.to_string(),
                    event.source.as_str(),
                    event.source_version.to_string(),
                    event.source_event,
                    db_text(&event.category)?,
                    event.occurred_at.to_rfc3339(),
                    event.received_at.to_rfc3339(),
                    event.project_id.map(|id| id.to_string()),
                    event.project_display_name,
                    event.unmatched_cwd_fingerprint,
                    event.session_ref,
                    event.turn_ref,
                    event.model,
                    event.permission_mode,
                    db_text(&event.severity)?,
                    serde_json::to_string(&event.public_fields)
                        .map_err(|_| serialization_error())?,
                    sensitive_blob_id,
                    sensitive_fields_blob,
                    event.correlation_id.to_string(),
                    event.action_id,
                    serde_json::to_string(&event.action_capabilities)
                        .map_err(|_| serialization_error())?,
                    db_text(&outcome)?,
                    outcome_reason_code,
                    Utc::now().to_rfc3339(),
                ],
            )
            .map_err(|_| write_error())?;
        Ok(())
    }

    pub fn list_history(
        &self,
        filter: &HistoryFilter,
        page: PageRequest,
    ) -> Result<HistoryPage, AppError> {
        if !(1..=MAX_HISTORY_PAGE_SIZE).contains(&page.limit) {
            return Err(invalid_pagination());
        }
        let connection = self.database.connect()?;
        let source = filter.source.map(AgentKind::as_str);
        let project_id = filter.project_id.map(|id| id.to_string());
        let channel_id = filter.channel_id.map(|id| id.to_string());
        let processing_outcome = filter
            .processing_outcome
            .as_ref()
            .map(db_text)
            .transpose()?;
        let (delivery_filter_kind, delivery_state) = match filter.delivery_status {
            None => (0_u8, None),
            Some(DeliveryStatus::NotQueued) => (1, None),
            Some(status) => (2, Some(db_text(&status)?)),
        };
        let mut statement = connection
            .prepare(
                "SELECT
                    e.id, e.source, e.source_version, e.source_event, e.category,
                    e.occurred_at, e.received_at, e.project_id, e.project_display_name,
                    e.unmatched_cwd_fingerprint, e.session_ref, e.turn_ref, e.model,
                    e.permission_mode, e.severity, e.public_fields_json, e.correlation_id,
                    e.action_id, e.action_capabilities_json, e.processing_outcome,
                    e.outcome_reason_code, j.id, j.channel_id, j.document_json, j.state
                 FROM events e
                 LEFT JOIN delivery_jobs j ON j.event_id = e.id
                 WHERE (?1 IS NULL OR e.occurred_at >= ?1)
                   AND (?2 IS NULL OR e.occurred_at < ?2)
                   AND (?3 IS NULL OR e.project_id = ?3)
                   AND (?4 IS NULL OR e.source = ?4)
                   AND (?5 IS NULL OR e.source_event = ?5)
                   AND (?6 IS NULL OR j.channel_id = ?6)
                   AND (?7 IS NULL OR e.processing_outcome = ?7)
                   AND (
                     ?8 = 0 OR (?8 = 1 AND j.id IS NULL) OR (?8 = 2 AND j.state = ?9)
                   )
                 ORDER BY e.occurred_at DESC, e.id DESC, j.created_at DESC, j.id DESC
                 LIMIT ?10 OFFSET ?11",
            )
            .map_err(|_| query_error())?;
        let mut raw_rows = statement
            .query_map(
                params![
                    filter.occurred_from.map(|value| value.to_rfc3339()),
                    filter.occurred_until.map(|value| value.to_rfc3339()),
                    project_id,
                    source,
                    filter.source_event,
                    channel_id,
                    processing_outcome,
                    delivery_filter_kind,
                    delivery_state,
                    i64::from(page.limit) + 1,
                    i64::from(page.offset),
                ],
                |row| {
                    Ok(RawHistoryRow {
                        event_id: row.get(0)?,
                        source: row.get(1)?,
                        source_version: row.get(2)?,
                        source_event: row.get(3)?,
                        category: row.get(4)?,
                        occurred_at: row.get(5)?,
                        received_at: row.get(6)?,
                        project_id: row.get(7)?,
                        project_display_name: row.get(8)?,
                        unmatched_cwd_fingerprint: row.get(9)?,
                        session_ref: row.get(10)?,
                        turn_ref: row.get(11)?,
                        model: row.get(12)?,
                        permission_mode: row.get(13)?,
                        severity: row.get(14)?,
                        public_fields_json: row.get(15)?,
                        correlation_id: row.get(16)?,
                        action_id: row.get(17)?,
                        action_capabilities_json: row.get(18)?,
                        processing_outcome: row.get(19)?,
                        outcome_reason_code: row.get(20)?,
                        delivery_job_id: row.get(21)?,
                        channel_id: row.get(22)?,
                        document_json: row.get(23)?,
                        delivery_status: row.get(24)?,
                    })
                },
            )
            .map_err(|_| query_error())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|_| query_error())?;
        let has_more = raw_rows.len() > usize::from(page.limit);
        raw_rows.truncate(usize::from(page.limit));
        let mut items = Vec::with_capacity(raw_rows.len());
        for row in raw_rows {
            items.push(history_item(&connection, row)?);
        }
        let next_offset = has_more.then_some(page.offset.saturating_add(u32::from(page.limit)));
        Ok(HistoryPage { items, next_offset })
    }
}

struct RawHistoryRow {
    event_id: String,
    source: String,
    source_version: String,
    source_event: String,
    category: String,
    occurred_at: String,
    received_at: String,
    project_id: Option<String>,
    project_display_name: Option<String>,
    unmatched_cwd_fingerprint: Option<String>,
    session_ref: Option<String>,
    turn_ref: Option<String>,
    model: Option<String>,
    permission_mode: Option<String>,
    severity: String,
    public_fields_json: String,
    correlation_id: String,
    action_id: Option<String>,
    action_capabilities_json: String,
    processing_outcome: String,
    outcome_reason_code: Option<String>,
    delivery_job_id: Option<String>,
    channel_id: Option<String>,
    document_json: Option<String>,
    delivery_status: Option<String>,
}

fn history_item(
    connection: &rusqlite::Connection,
    row: RawHistoryRow,
) -> Result<HistoryItem, AppError> {
    let delivery_job_id = row.delivery_job_id.as_deref().map(parse_uuid).transpose()?;
    let attempts = match delivery_job_id {
        Some(job_id) => list_attempts(connection, job_id)?,
        None => Vec::new(),
    };
    Ok(HistoryItem {
        event_id: parse_uuid(&row.event_id)?,
        source: parse_db_text(&row.source)?,
        source_version: Version::parse(&row.source_version).map_err(|_| stored_data_error())?,
        source_event: row.source_event,
        category: parse_db_text(&row.category)?,
        occurred_at: parse_time(&row.occurred_at)?,
        received_at: parse_time(&row.received_at)?,
        project_id: row.project_id.as_deref().map(parse_uuid).transpose()?,
        project_display_name: row.project_display_name,
        unmatched_cwd_fingerprint: row.unmatched_cwd_fingerprint,
        session_ref: row.session_ref,
        turn_ref: row.turn_ref,
        model: row.model,
        permission_mode: row.permission_mode,
        severity: parse_db_text(&row.severity)?,
        public_fields: serde_json::from_str(&row.public_fields_json)
            .map_err(|_| stored_data_error())?,
        correlation_id: parse_uuid(&row.correlation_id)?,
        action_id: row.action_id,
        action_capabilities: serde_json::from_str(&row.action_capabilities_json)
            .map_err(|_| stored_data_error())?,
        processing_outcome: parse_db_text(&row.processing_outcome)?,
        outcome_reason_code: row
            .outcome_reason_code
            .as_deref()
            .map(parse_db_text)
            .transpose()?,
        delivery_job_id,
        channel_id: row.channel_id.as_deref().map(parse_uuid).transpose()?,
        document: row
            .document_json
            .as_deref()
            .map(serde_json::from_str)
            .transpose()
            .map_err(|_| stored_data_error())?,
        delivery_status: row
            .delivery_status
            .as_deref()
            .map(parse_db_text)
            .transpose()?
            .unwrap_or(DeliveryStatus::NotQueued),
        attempts,
    })
}

fn list_attempts(
    connection: &rusqlite::Connection,
    job_id: Uuid,
) -> Result<Vec<DeliveryAttemptDto>, AppError> {
    let mut statement = connection
        .prepare(
            "SELECT attempt_number, started_at, completed_at, outcome, http_status,
                    platform_code, error_code, retry_at, redacted_detail
             FROM delivery_attempts WHERE job_id = ?1 ORDER BY attempt_number",
        )
        .map_err(|_| query_error())?;
    let rows = statement
        .query_map([job_id.to_string()], |row| {
            Ok((
                row.get::<_, u32>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, Option<u16>>(4)?,
                row.get::<_, Option<String>>(5)?,
                row.get::<_, Option<String>>(6)?,
                row.get::<_, Option<String>>(7)?,
                row.get::<_, Option<String>>(8)?,
            ))
        })
        .map_err(|_| query_error())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| query_error())?;
    rows.into_iter()
        .map(
            |(
                attempt_number,
                started_at,
                completed_at,
                outcome,
                http_status,
                platform_code,
                error_code,
                retry_at,
                redacted_detail,
            )| {
                Ok(DeliveryAttemptDto {
                    attempt_number,
                    started_at: parse_time(&started_at)?,
                    completed_at: parse_time(&completed_at)?,
                    outcome,
                    http_status,
                    platform_code,
                    error_code,
                    retry_at: retry_at.as_deref().map(parse_time).transpose()?,
                    redacted_detail,
                })
            },
        )
        .collect()
}

fn db_text<T: Serialize>(value: &T) -> Result<String, AppError> {
    match serde_json::to_value(value).map_err(|_| serialization_error())? {
        Value::String(value) => Ok(value),
        _ => Err(serialization_error()),
    }
}

fn parse_db_text<T: DeserializeOwned>(value: &str) -> Result<T, AppError> {
    serde_json::from_value(Value::String(value.to_owned())).map_err(|_| stored_data_error())
}

fn parse_uuid(value: &str) -> Result<Uuid, AppError> {
    Uuid::parse_str(value).map_err(|_| stored_data_error())
}

fn parse_time(value: &str) -> Result<DateTime<Utc>, AppError> {
    DateTime::parse_from_rfc3339(value)
        .map(|value| value.with_timezone(&Utc))
        .map_err(|_| stored_data_error())
}

fn invalid_pagination() -> AppError {
    storage_error(
        "storage.invalid_pagination",
        "page size must be between 1 and 200",
    )
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

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use chrono::{TimeZone, Utc};
    use rusqlite::{Connection, params};
    use semver::Version;
    use serde_json::json;
    use tempfile::{TempDir, tempdir};
    use uuid::Uuid;

    use super::{
        DeliveryStatus, EncryptedField, EventOutcomeReasonCode, EventProcessingOutcome,
        EventRepository, HistoryFilter, PageRequest,
    };
    use crate::events::normalize::SafeIngressEvent;
    use crate::model::{
        AgentKind, EncryptedBlobRef, EventCategory, EventEnvelope, ScalarValue, Severity,
    };
    use crate::storage::db::Database;

    #[test]
    fn ingress_round_trip_contains_safe_envelope_only() {
        let (_file, repo) = test_repository();
        let input = safe_ingress_with_summary("metadata only");

        repo.insert_ingress(&input).unwrap();
        let stored = repo.take_ingress_batch(10).unwrap();

        assert_eq!(stored.len(), 1);
        assert_eq!(
            serde_json::to_value(&stored[0]).unwrap(),
            serde_json::to_value(&input).unwrap()
        );
        assert!(repo.take_ingress_batch(10).unwrap().is_empty());
        let bytes = std::fs::read(repo.database_path()).unwrap();
        assert!(!String::from_utf8_lossy(&bytes).contains("raw_prompt"));
    }

    #[test]
    fn ingress_batch_accepts_two_hundred() {
        let (_file, repo) = test_repository();

        assert!(repo.take_ingress_batch(200).unwrap().is_empty());
    }

    #[test]
    fn ingress_batch_rejects_two_hundred_one() {
        let (_file, repo) = test_repository();

        let error = repo.take_ingress_batch(201).unwrap_err();

        assert_eq!(error.code, "storage.invalid_pagination");
    }

    #[test]
    fn insert_event_serializes_only_typed_nonce_and_ciphertext_bytes() {
        let (_file, repo) = test_repository();
        let mut event = event_fixture();
        let blob_id = Uuid::now_v7();
        event.encrypted_sensitive_fields = Some(EncryptedBlobRef { blob_id });
        let encrypted = BTreeMap::from([(
            "prompt".to_owned(),
            EncryptedField {
                nonce: vec![1, 2, 3],
                ciphertext: vec![9, 8, 7],
            },
        )]);

        repo.insert_event(
            &event,
            Some(&encrypted),
            EventProcessingOutcome::Queued,
            None,
        )
        .unwrap();

        let connection = Connection::open(repo.database_path()).unwrap();
        let (stored_blob_id, stored_blob): (String, Vec<u8>) = connection
            .query_row(
                "SELECT sensitive_blob_id, sensitive_fields_blob FROM events WHERE id = ?1",
                [event.id.to_string()],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(stored_blob_id, blob_id.to_string());
        assert_eq!(stored_blob, serde_json::to_vec(&encrypted).unwrap());
        assert!(!String::from_utf8_lossy(&stored_blob).contains("plaintext"));
    }

    #[test]
    fn history_returns_redacted_documents_and_attempt_metadata() {
        let (_file, repo) = repository_with_succeeded_delivery();

        let page = repo
            .list_history(&HistoryFilter::default(), PageRequest::first(50))
            .unwrap();

        assert_eq!(page.items.len(), 1);
        assert_eq!(page.items[0].delivery_status, DeliveryStatus::Succeeded);
        assert_eq!(page.items[0].attempts.len(), 1);
        assert_eq!(
            page.items[0].attempts[0].redacted_detail.as_deref(),
            Some("redacted response")
        );
        assert_eq!(page.items[0].document.as_ref().unwrap().body, "safe body");
        assert!(!format!("{page:?}").contains("access_token"));
    }

    #[test]
    fn history_explains_non_delivery_without_exposing_ciphertext() {
        let (_file, repo) = test_repository();
        let event = event_fixture();
        repo.insert_event(
            &event,
            None,
            EventProcessingOutcome::Suppressed,
            Some(EventOutcomeReasonCode::QuietHours),
        )
        .unwrap();

        let page = repo
            .list_history(&HistoryFilter::default(), PageRequest::first(10))
            .unwrap();

        assert_eq!(page.items.len(), 1);
        assert_eq!(
            page.items[0].processing_outcome,
            EventProcessingOutcome::Suppressed
        );
        assert_eq!(
            page.items[0].outcome_reason_code,
            Some(EventOutcomeReasonCode::QuietHours)
        );
        assert_eq!(page.items[0].delivery_status, DeliveryStatus::NotQueued);
        assert!(!format!("{page:?}").contains("sensitive_fields_blob"));
    }

    #[test]
    fn outcome_reason_codes_are_closed_and_serialize_stably() {
        assert_eq!(
            serde_json::to_string(&EventOutcomeReasonCode::QuietHours).unwrap(),
            "\"quiet_hours\""
        );
        assert!(serde_json::from_str::<EventOutcomeReasonCode>("\"raw rule text\"").is_err());
    }

    #[test]
    fn history_rejects_pagination_outside_one_through_two_hundred() {
        let (_file, repo) = test_repository();

        let zero = repo
            .list_history(&HistoryFilter::default(), PageRequest::first(0))
            .unwrap_err();
        let too_large = repo
            .list_history(&HistoryFilter::default(), PageRequest::first(201))
            .unwrap_err();

        assert_eq!(zero.code, "storage.invalid_pagination");
        assert_eq!(too_large.code, "storage.invalid_pagination");
    }

    #[test]
    fn history_exact_final_page_has_no_next_offset() {
        let (_file, repo) = test_repository();
        repo.insert_event(
            &event_fixture(),
            None,
            EventProcessingOutcome::Suppressed,
            Some(EventOutcomeReasonCode::Disabled),
        )
        .unwrap();

        let page = repo
            .list_history(&HistoryFilter::default(), PageRequest::first(1))
            .unwrap();

        assert_eq!(page.items.len(), 1);
        assert_eq!(page.next_offset, None);
    }

    fn test_repository() -> (TempDir, EventRepository) {
        let root = tempdir().unwrap();
        let path = root
            .path()
            .join("com.ccreminder.app")
            .join("cc-reminder.sqlite3");
        let database = Database::open(&path).unwrap();
        let repository = EventRepository::new(database);
        (root, repository)
    }

    fn repository_with_succeeded_delivery() -> (TempDir, EventRepository) {
        let (root, repo) = test_repository();
        let event = event_fixture();
        repo.insert_event(&event, None, EventProcessingOutcome::Queued, None)
            .unwrap();
        let channel_id = Uuid::now_v7();
        let job_id = Uuid::now_v7();
        let attempt_id = Uuid::now_v7();
        let now = "2026-07-29T12:00:00Z";
        let connection = Connection::open(repo.database_path()).unwrap();
        connection
            .execute(
                "INSERT INTO channels (
                    id, kind, name, credential_ref, public_config_json, health_status,
                    created_at, updated_at
                 ) VALUES (?1, 'we_com', 'test', ?2, '{}', 'healthy', ?3, ?3)",
                params![
                    channel_id.to_string(),
                    "cc-reminder/channel/access_token=never-return-this",
                    now,
                ],
            )
            .unwrap();
        connection
            .execute(
                "INSERT INTO delivery_jobs (
                    id, event_id, rule_id, rule_version, channel_id, idempotency_key,
                    document_json, state, attempts, next_attempt_at, expires_at,
                    created_at, updated_at
                 ) VALUES (?1, ?2, ?3, 'v1', ?4, ?5, ?6, 'succeeded', 1, ?7, ?7, ?7, ?7)",
                params![
                    job_id.to_string(),
                    event.id.to_string(),
                    Uuid::now_v7().to_string(),
                    channel_id.to_string(),
                    Uuid::now_v7().to_string(),
                    json!({
                        "title": "safe title",
                        "severity": "info",
                        "facts": [["Status", "Succeeded"]],
                        "body": "safe body",
                        "footer": null,
                    })
                    .to_string(),
                    now,
                ],
            )
            .unwrap();
        connection
            .execute(
                "INSERT INTO delivery_attempts (
                    id, job_id, attempt_number, started_at, completed_at, outcome,
                    http_status, redacted_detail
                 ) VALUES (?1, ?2, 1, ?3, ?3, 'succeeded', 200, 'redacted response')",
                params![attempt_id.to_string(), job_id.to_string(), now],
            )
            .unwrap();
        (root, repo)
    }

    fn safe_ingress_with_summary(summary: &str) -> SafeIngressEvent {
        SafeIngressEvent {
            event_id: Uuid::now_v7(),
            source: AgentKind::Codex,
            source_version: Version::new(0, 145, 0),
            source_event: "Stop".to_owned(),
            occurred_at: Utc.with_ymd_and_hms(2026, 7, 29, 12, 0, 0).unwrap(),
            received_at: Utc.with_ymd_and_hms(2026, 7, 29, 12, 0, 1).unwrap(),
            project_id: None,
            project_display_name: Some("cc-reminder".to_owned()),
            cwd_fingerprint: Some("safe-fingerprint".to_owned()),
            session_ref: Some("safe-session-ref".to_owned()),
            turn_ref: Some("safe-turn-ref".to_owned()),
            public_fields: BTreeMap::from([(
                "summary".to_owned(),
                ScalarValue::String(summary.to_owned()),
            )]),
        }
    }

    fn event_fixture() -> EventEnvelope {
        EventEnvelope {
            id: Uuid::now_v7(),
            source: AgentKind::Codex,
            source_version: Version::new(0, 145, 0),
            source_event: "Stop".to_owned(),
            category: EventCategory::Completion,
            occurred_at: Utc.with_ymd_and_hms(2026, 7, 29, 12, 0, 0).unwrap(),
            received_at: Utc.with_ymd_and_hms(2026, 7, 29, 12, 0, 1).unwrap(),
            project_id: None,
            project_display_name: Some("cc-reminder".to_owned()),
            unmatched_cwd_fingerprint: None,
            session_ref: Some("safe-session-ref".to_owned()),
            turn_ref: Some("safe-turn-ref".to_owned()),
            model: Some("gpt-5".to_owned()),
            permission_mode: None,
            severity: Severity::Info,
            public_fields: BTreeMap::from([(
                "status".to_owned(),
                ScalarValue::String("success".to_owned()),
            )]),
            encrypted_sensitive_fields: None,
            correlation_id: Uuid::now_v7(),
            action_id: None,
            action_capabilities: Vec::new(),
        }
    }
}
