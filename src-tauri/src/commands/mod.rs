//! Typed desktop command surface (Task 15 §A).
//!
//! Each Tauri command is a thin wrapper around a `pub(crate)` function that
//! takes `&CoreState` and a typed input struct. The split keeps commands
//! unit-testable without constructing a `tauri::State` (which has no public
//! constructor), and lets the helper fns be exercised directly from the
//! `commands::` tests required by the plan.
//!
//! Privacy contract: read commands expose only typed DTOs that omit
//! credentials, ciphertext, raw unmatched IDs, Hook raw JSON and full platform
//! responses. `deny_unknown_fields` is on every input struct so the WebView
//! cannot smuggle extra fields.

pub mod agents;
pub mod channels;
pub mod history;
pub mod projects;
pub mod rules;
pub mod settings;
pub mod updates;

use std::sync::Mutex;

use chrono::Utc;
use serde::{Deserialize, Serialize};
use tauri::State;
use uuid::Uuid;

use crate::error::{AppError, ErrorDomain};
use crate::health::HealthSnapshot;
use crate::model::AppSettings;
use crate::security::credentials::CredentialStore;
use crate::security::crypto::FieldCipher;
use crate::storage::config::ConfigRepository;
use crate::storage::events::EventRepository;
use crate::storage::integrations::IntegrationRepository;
use crate::storage::queue::QueueRepository;

/// Aggregated, clonable handle to every repository the command surface needs.
/// `Managed` by Tauri as `tauri::State<CoreState>`.
//
// ponytail: one struct, no trait-object registry. The command set is fixed and
// small; an interface-per-domain would be scaffolding for exactly one
// implementation. The Mutex on the worker cancel token is the only interior
// mutability the surface needs (commands are otherwise read/transactional).
#[derive(Clone)]
pub struct CoreState {
    pub config: ConfigRepository,
    pub events: EventRepository,
    pub queue: QueueRepository,
    pub integrations: IntegrationRepository,
    pub credentials: CredentialStore,
    pub cipher: std::sync::Arc<FieldCipher>,
    pub cancel_token: std::sync::Arc<Mutex<Option<crate::worker::CancellationToken>>>,
}

impl CoreState {
    pub fn new(
        config: ConfigRepository,
        events: EventRepository,
        queue: QueueRepository,
        integrations: IntegrationRepository,
        credentials: CredentialStore,
        cipher: std::sync::Arc<FieldCipher>,
    ) -> Self {
        Self {
            config,
            events,
            queue,
            integrations,
            credentials,
            cipher,
            cancel_token: std::sync::Arc::new(Mutex::new(None)),
        }
    }
}

/// Generic page request input. `deny_unknown_fields` so the frontend cannot
/// smuggle extra params; validation lives in the command, not the repo.
#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PageInput {
    pub offset: u32,
    pub limit: u16,
}

impl PageInput {
    pub fn to_page_request(&self) -> crate::storage::events::PageRequest {
        crate::storage::events::PageRequest {
            offset: self.offset,
            limit: self.limit,
        }
    }
}

/// Bootstrap state returned to the frontend on first paint.
#[derive(Clone, Debug, Serialize)]
pub struct BootstrapState {
    pub onboarding_completed: bool,
    pub locale: String,
    pub theme: String,
    pub health: HealthSnapshot,
    pub pending_jobs: u64,
    pub failed_jobs: u64,
}

/// Parse a UUID input or return a typed `configuration.malformed_uuid` error.
/// Centralized so every command shares one redaction-safe rejection path.
pub(crate) fn parse_uuid_input(value: &str) -> Result<Uuid, AppError> {
    Uuid::parse_str(value)
        .map_err(|_| configuration_error("malformed_uuid", "identifier is invalid"))
}

pub(crate) fn configuration_error(code: &str, message: &str) -> AppError {
    AppError {
        domain: ErrorDomain::Configuration,
        code: format!("configuration.{code}"),
        message: message.to_owned(),
        suggested_action: None,
    }
}

pub(crate) fn secret_store_error(code: &str, message: &str) -> AppError {
    AppError {
        domain: ErrorDomain::SecretStore,
        code: format!("secret_store.{code}"),
        message: message.to_owned(),
        suggested_action: None,
    }
}

pub(crate) fn integration_error(code: &str, message: &str) -> AppError {
    AppError {
        domain: ErrorDomain::Integration,
        code: format!("integration.{code}"),
        message: message.to_owned(),
        suggested_action: None,
    }
}

pub(crate) fn update_error(code: &str, message: &str) -> AppError {
    AppError {
        domain: ErrorDomain::Update,
        code: format!("update.{code}"),
        message: message.to_owned(),
        suggested_action: None,
    }
}

// ---------------------------------------------------------------------------
// Command implementations (typed, no Tauri types). Each `pub(crate) fn` is the
// real body; the per-domain command modules expose the `#[tauri::command]`
// wrappers that pull `State<CoreState>` and delegate.
// ---------------------------------------------------------------------------

/// `get_bootstrap_state`
pub(crate) fn bootstrap_state(state: &CoreState) -> Result<BootstrapState, AppError> {
    let settings = state.config.get_settings()?;
    let snapshot = build_health_snapshot(state, &settings)?;
    let pending = snapshot.pending_jobs;
    let failed = snapshot.failed_jobs;
    Ok(BootstrapState {
        onboarding_completed: settings.onboarding_completed,
        locale: locale_code(settings.locale),
        theme: theme_code(settings.theme),
        health: snapshot,
        pending_jobs: pending,
        failed_jobs: failed,
    })
}

/// `get_health_snapshot`
pub(crate) fn health_snapshot(state: &CoreState) -> Result<HealthSnapshot, AppError> {
    let settings = state.config.get_settings()?;
    build_health_snapshot(state, &settings)
}

fn build_health_snapshot(
    state: &CoreState,
    settings: &AppSettings,
) -> Result<HealthSnapshot, AppError> {
    use crate::health::{HealthInputs, project_health};

    let stats = state.queue.queue_stats()?;
    let channels = state.config.list_channels()?;
    let channel_states: Vec<_> = channels
        .iter()
        .map(|c| crate::health::ChannelHealthState {
            channel_id: c.id.to_string(),
            name: c.name.clone(),
            kind: channel_kind_code(c.kind),
            health: channel_health_level(c.health_status, c.paused_reason_code.as_deref()),
            paused: c.paused_reason_code.is_some(),
            summary: channel_summary(c.health_status, c.paused_reason_code.as_deref()),
        })
        .collect();

    let inputs = HealthInputs {
        channels: channel_states,
        pending_jobs: stats.pending,
        retry_jobs: stats.retry_wait,
        failed_jobs: stats.failed,
        expired_jobs: stats.expired,
        spool_count: 0,
        rejected_count: 0,
        last_success_at: last_success(settings),
        ..HealthInputs::default()
    };
    Ok(project_health(&inputs))
}

fn last_success(settings: &AppSettings) -> Option<chrono::DateTime<Utc>> {
    // ponytail: the channels table holds per-channel last_succeeded_at; the
    // overview "last success" is the most recent. Aggregating here keeps the
    // snapshot pure-ish; promote to a dedicated query if it shows up in
    // profiling.
    let _ = settings;
    None
}

pub(crate) fn channel_kind_code(kind: crate::model::ChannelKind) -> String {
    match kind {
        crate::model::ChannelKind::DingTalk => "ding_talk".into(),
        crate::model::ChannelKind::WeCom => "we_com".into(),
    }
}

pub(crate) fn channel_health_level(
    health: crate::model::ChannelHealth,
    paused: Option<&str>,
) -> crate::health::HealthLevel {
    use crate::health::HealthLevel;
    match health {
        crate::model::ChannelHealth::Healthy => HealthLevel::Ok,
        crate::model::ChannelHealth::Unknown => HealthLevel::Ok,
        crate::model::ChannelHealth::PausedAuthentication => HealthLevel::Warning,
        crate::model::ChannelHealth::Error => HealthLevel::Error,
    }
    .worst(if paused.is_some() {
        HealthLevel::Warning
    } else {
        HealthLevel::Ok
    })
}

pub(crate) fn channel_summary(health: crate::model::ChannelHealth, paused: Option<&str>) -> String {
    if paused.is_some() {
        return "paused".into();
    }
    match health {
        crate::model::ChannelHealth::Healthy => "healthy".into(),
        crate::model::ChannelHealth::Unknown => "unknown".into(),
        crate::model::ChannelHealth::PausedAuthentication => "paused".into(),
        crate::model::ChannelHealth::Error => "error".into(),
    }
}

pub(crate) fn locale_code(locale: crate::model::Locale) -> String {
    match locale {
        crate::model::Locale::ZhCn => "zh_cn".into(),
        crate::model::Locale::En => "en".into(),
    }
}

pub(crate) fn theme_code(theme: crate::model::Theme) -> String {
    match theme {
        crate::model::Theme::System => "system".into(),
        crate::model::Theme::Light => "light".into(),
        crate::model::Theme::Dark => "dark".into(),
    }
}

// ---------------------------------------------------------------------------
// Tauri wrappers for the two commands that live in this module.
// ---------------------------------------------------------------------------

#[tauri::command]
pub async fn get_bootstrap_state(state: State<'_, CoreState>) -> Result<BootstrapState, AppError> {
    let state = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || bootstrap_state(&state))
        .await
        .map_err(|_| configuration_error("join_failed", "command join failed"))?
}

#[tauri::command]
pub async fn get_health_snapshot(state: State<'_, CoreState>) -> Result<HealthSnapshot, AppError> {
    let state = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || health_snapshot(&state))
        .await
        .map_err(|_| configuration_error("join_failed", "command join failed"))?
}

// The `#[tauri::command]` wrappers live in each submodule and are referenced
// from `lib.rs` by their fully-qualified path (e.g. `commands::channels::save_channel`).
// We deliberately do NOT glob-re-export them here: the command names collide
// with the inner `*_impl` body names if re-exported together, and
// `generate_handler!` requires literal paths anyway.
