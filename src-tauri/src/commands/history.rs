//! History commands: list + detail + manual retry.

use chrono::Utc;
use serde::Deserialize;
use tauri::State;

use super::{CoreState, PageInput, configuration_error, parse_uuid_input};
use crate::error::AppError;
use crate::storage::events::{DeliveryStatus, EventProcessingOutcome, HistoryFilter, HistoryPage};

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HistoryFilterInput {
    pub occurred_from: Option<chrono::DateTime<Utc>>,
    pub occurred_until: Option<chrono::DateTime<Utc>>,
    pub project_id: Option<String>,
    pub source: Option<AgentKindInput>,
    pub source_event: Option<String>,
    pub channel_id: Option<String>,
    pub delivery_status: Option<DeliveryStatusInput>,
    pub processing_outcome: Option<ProcessingOutcomeInput>,
}

#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentKindInput {
    ClaudeCode,
    Codex,
}
impl AgentKindInput {
    fn into_kind(self) -> crate::model::AgentKind {
        match self {
            Self::ClaudeCode => crate::model::AgentKind::ClaudeCode,
            Self::Codex => crate::model::AgentKind::Codex,
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DeliveryStatusInput {
    NotQueued,
    Pending,
    Sending,
    RetryWait,
    Succeeded,
    Failed,
    Expired,
}
impl DeliveryStatusInput {
    fn into_status(self) -> DeliveryStatus {
        match self {
            Self::NotQueued => DeliveryStatus::NotQueued,
            Self::Pending => DeliveryStatus::Pending,
            Self::Sending => DeliveryStatus::Sending,
            Self::RetryWait => DeliveryStatus::RetryWait,
            Self::Succeeded => DeliveryStatus::Succeeded,
            Self::Failed => DeliveryStatus::Failed,
            Self::Expired => DeliveryStatus::Expired,
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProcessingOutcomeInput {
    Queued,
    Suppressed,
    Expired,
    NoTargets,
}
impl ProcessingOutcomeInput {
    fn into_outcome(self) -> EventProcessingOutcome {
        match self {
            Self::Queued => EventProcessingOutcome::Queued,
            Self::Suppressed => EventProcessingOutcome::Suppressed,
            Self::Expired => EventProcessingOutcome::Expired,
            Self::NoTargets => EventProcessingOutcome::NoTargets,
        }
    }
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GetHistoryDetailInput {
    pub event_id: String,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ManualRetryInput {
    pub job_id: String,
}

pub(crate) fn list_history_impl(
    state: &CoreState,
    filter: HistoryFilterInput,
    page: PageInput,
) -> Result<HistoryPage, AppError> {
    let parsed = HistoryFilter {
        occurred_from: filter.occurred_from,
        occurred_until: filter.occurred_until,
        project_id: filter
            .project_id
            .as_deref()
            .map(parse_uuid_input)
            .transpose()?,
        source: filter.source.map(|s| s.into_kind()),
        source_event: filter.source_event,
        channel_id: filter
            .channel_id
            .as_deref()
            .map(parse_uuid_input)
            .transpose()?,
        delivery_status: filter.delivery_status.map(|s| s.into_status()),
        processing_outcome: filter.processing_outcome.map(|o| o.into_outcome()),
        event_id: None,
    };
    state
        .storage
        .events
        .list_history(&parsed, page.to_page_request())
}

pub(crate) fn get_history_detail_impl(
    state: &CoreState,
    input: GetHistoryDetailInput,
) -> Result<HistoryPage, AppError> {
    let event_id = parse_uuid_input(&input.event_id)?;
    // By-id filter: one SQL query returning that event with its delivery jobs
    // regardless of how far back it sits in history. The history read path
    // already redacts ciphertext/credentials.
    let filter = HistoryFilter {
        event_id: Some(event_id),
        ..HistoryFilter::default()
    };
    state
        .storage
        .events
        .list_history(&filter, crate::storage::events::PageRequest::first(200))
}

pub(crate) fn manual_retry_delivery_impl(
    state: &CoreState,
    input: ManualRetryInput,
) -> Result<(), AppError> {
    let job_id = parse_uuid_input(&input.job_id)?;
    state.storage.queue.manual_retry(job_id, Utc::now())
}

#[tauri::command]
pub async fn list_history(
    state: State<'_, CoreState>,
    filter: HistoryFilterInput,
    page: PageInput,
) -> Result<HistoryPage, AppError> {
    let state = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || list_history_impl(&state, filter, page))
        .await
        .map_err(|_| configuration_error("join_failed", "command join failed"))?
}

#[tauri::command]
pub async fn get_history_detail(
    state: State<'_, CoreState>,
    input: GetHistoryDetailInput,
) -> Result<HistoryPage, AppError> {
    let state = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || get_history_detail_impl(&state, input))
        .await
        .map_err(|_| configuration_error("join_failed", "command join failed"))?
}

#[tauri::command]
pub async fn manual_retry_delivery(
    state: State<'_, CoreState>,
    input: ManualRetryInput,
) -> Result<(), AppError> {
    let state = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || manual_retry_delivery_impl(&state, input))
        .await
        .map_err(|_| configuration_error("join_failed", "command join failed"))?
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::CoreState;
    use crate::events::catalog::catalog_for;
    use crate::model::AgentKind;
    use crate::security::credentials::CredentialStore;
    use crate::security::crypto::FieldCipher;
    use crate::storage::config::ConfigRepository;
    use crate::storage::db::Database;
    use crate::storage::events::EventRepository;
    use crate::storage::integrations::IntegrationRepository;
    use crate::storage::queue::QueueRepository;
    use semver::Version;
    use tempfile::tempdir;

    fn state() -> CoreState {
        let root = tempdir().unwrap();
        let database_path = root.path().join("com.ccreminder.app/cc-reminder.sqlite3");
        std::fs::create_dir_all(database_path.parent().unwrap()).unwrap();
        std::mem::forget(root);
        let database = Database::open(&database_path).unwrap();
        let config = ConfigRepository::new(database.clone());
        config
            .ensure_global_rules(&[
                catalog_for(AgentKind::ClaudeCode, &Version::new(2, 1, 218)).catalog,
                catalog_for(AgentKind::Codex, &Version::new(0, 145, 0)).catalog,
            ])
            .unwrap();
        let events = EventRepository::new(database.clone());
        let queue = QueueRepository::new(database.clone());
        let integrations = IntegrationRepository::new(database.clone());
        let credentials = CredentialStore::memory_for_test();
        let cipher = crate::security::crypto::LazyFieldCipher::ready(std::sync::Arc::new(FieldCipher::from_key([5u8; 32])));
        let diagnostics = std::sync::Arc::new(crate::diagnostics::Diagnostics::test(
            &database_path.parent().unwrap().join("logs"),
            1024 * 1024,
            3,
        ));
        CoreState::new(
            config,
            events,
            queue,
            integrations,
            credentials,
            cipher,
            diagnostics,
        )
    }

    #[test]
    fn invalid_pagination_is_rejected() {
        let st = state();
        let err = list_history_impl(
            &st,
            HistoryFilterInput::default(),
            PageInput {
                offset: 0,
                limit: 0,
            },
        )
        .unwrap_err();
        assert_eq!(err.code, "storage.invalid_pagination");

        let err2 = list_history_impl(
            &st,
            HistoryFilterInput::default(),
            PageInput {
                offset: 0,
                limit: 201,
            },
        )
        .unwrap_err();
        assert_eq!(err2.code, "storage.invalid_pagination");
    }

    #[test]
    fn malformed_event_uuid_is_rejected() {
        let st = state();
        let err = get_history_detail_impl(
            &st,
            GetHistoryDetailInput {
                event_id: "nope".into(),
            },
        )
        .unwrap_err();
        assert_eq!(err.code, "configuration.malformed_uuid");
    }

    #[test]
    fn manual_retry_rejects_unknown_job() {
        let st = state();
        let err = manual_retry_delivery_impl(
            &st,
            ManualRetryInput {
                job_id: "not-a-uuid".into(),
            },
        )
        .unwrap_err();
        assert_eq!(err.code, "configuration.malformed_uuid");
    }
}
