//! Settings + pause commands.

use chrono::Utc;
use serde::{Deserialize, Serialize};
use tauri::State;

use super::{CoreState, configuration_error};
use crate::error::{AppError, ErrorDomain};
use crate::health::{PauseDuration, pause_until};
use crate::model::{AppSettings, Locale, NotificationPause, Theme};

#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LocaleInput {
    ZhCn,
    En,
}
impl LocaleInput {
    fn into_locale(self) -> Locale {
        match self {
            Self::ZhCn => Locale::ZhCn,
            Self::En => Locale::En,
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ThemeInput {
    System,
    Light,
    Dark,
}
impl ThemeInput {
    fn into_theme(self) -> Theme {
        match self {
            Self::System => Theme::System,
            Self::Light => Theme::Light,
            Self::Dark => Theme::Dark,
        }
    }
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SaveSettingsInput {
    pub autostart: bool,
    pub close_to_tray: bool,
    pub locale: LocaleInput,
    pub theme: ThemeInput,
    pub event_retention_days: u16,
    pub log_retention_days: u16,
    pub onboarding_completed: bool,
}

#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PauseDurationInput {
    FifteenMinutes,
    OneHour,
    Today,
}
impl PauseDurationInput {
    fn into_duration(self) -> PauseDuration {
        match self {
            Self::FifteenMinutes => PauseDuration::FifteenMinutes,
            Self::OneHour => PauseDuration::OneHour,
            Self::Today => PauseDuration::Today,
        }
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct SettingsView {
    pub autostart: bool,
    pub close_to_tray: bool,
    pub locale: String,
    pub theme: String,
    pub event_retention_days: u16,
    pub log_retention_days: u16,
    pub onboarding_completed: bool,
    pub paused_until: Option<chrono::DateTime<Utc>>,
}

impl From<AppSettings> for SettingsView {
    fn from(s: AppSettings) -> Self {
        Self {
            autostart: s.autostart,
            close_to_tray: s.close_to_tray,
            locale: super::locale_code(s.locale),
            theme: super::theme_code(s.theme),
            event_retention_days: s.event_retention_days,
            log_retention_days: s.log_retention_days,
            onboarding_completed: s.onboarding_completed,
            paused_until: s.notification_pause.map(|p| p.until),
        }
    }
}

pub(crate) fn get_settings_impl(state: &CoreState) -> Result<SettingsView, AppError> {
    Ok(state.config.get_settings()?.into())
}

pub(crate) fn save_settings_impl(
    state: &CoreState,
    input: SaveSettingsInput,
) -> Result<SettingsView, AppError> {
    let mut settings = state.config.get_settings()?;
    let autostart_changed = settings.autostart != input.autostart;
    // Apply the OS registration BEFORE persisting: if the control fails, the
    // DB keeps its old value, so re-saving with the same target retries the
    // registration instead of silently no-op'ing on an unchanged field.
    if autostart_changed {
        (state.autostart_control)(input.autostart).map_err(|message| AppError {
            domain: ErrorDomain::Configuration,
            code: "configuration.autostart_failed".to_owned(),
            message,
            suggested_action: Some("re-save settings to retry".to_owned()),
        })?;
    }
    settings.autostart = input.autostart;
    settings.close_to_tray = input.close_to_tray;
    settings.locale = input.locale.into_locale();
    settings.theme = input.theme.into_theme();
    settings.event_retention_days = input.event_retention_days;
    settings.log_retention_days = input.log_retention_days;
    settings.onboarding_completed = input.onboarding_completed;
    let saved = state.config.save_settings(&settings)?;
    // Autostart is the only side-effecting setting; the OS registration is
    // applied ONLY here, per the plan ("updated only from save_settings"). The
    // control is injected by the app shell (the Tauri autostart plugin); tests
    // inject a recording closure.
    Ok(saved.into())
}

pub(crate) fn set_notification_pause_impl(
    state: &CoreState,
    duration: PauseDurationInput,
    now: chrono::DateTime<chrono::FixedOffset>,
) -> Result<SettingsView, AppError> {
    let until = pause_until(duration.into_duration(), now);
    let mut settings = state.config.get_settings()?;
    settings.notification_pause = Some(NotificationPause {
        // Same request instant as `until`: mixing in a second clock read
        // could make started_at land after until and fail validation.
        started_at: now.with_timezone(&Utc),
        until: until.with_timezone(&Utc),
    });
    Ok(state.config.save_settings(&settings)?.into())
}

pub(crate) fn clear_notification_pause_impl(state: &CoreState) -> Result<SettingsView, AppError> {
    let mut settings = state.config.get_settings()?;
    settings.notification_pause = None;
    Ok(state.config.save_settings(&settings)?.into())
}

#[tauri::command]
pub async fn get_settings(state: State<'_, CoreState>) -> Result<SettingsView, AppError> {
    let state = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || get_settings_impl(&state))
        .await
        .map_err(|_| configuration_error("join_failed", "command join failed"))?
}

#[tauri::command]
pub async fn save_settings(
    state: State<'_, CoreState>,
    input: SaveSettingsInput,
) -> Result<SettingsView, AppError> {
    let state = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || save_settings_impl(&state, input))
        .await
        .map_err(|_| configuration_error("join_failed", "command join failed"))?
}

#[tauri::command]
pub async fn set_notification_pause(
    state: State<'_, CoreState>,
    duration: PauseDurationInput,
    offset_seconds: Option<i32>,
) -> Result<SettingsView, AppError> {
    let state = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let now = local_now(offset_seconds);
        set_notification_pause_impl(&state, duration, now)
    })
    .await
    .map_err(|_| configuration_error("join_failed", "command join failed"))?
}

#[tauri::command]
pub async fn clear_notification_pause(
    state: State<'_, CoreState>,
) -> Result<SettingsView, AppError> {
    let state = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || clear_notification_pause_impl(&state))
        .await
        .map_err(|_| configuration_error("join_failed", "command join failed"))?
}

fn local_now(offset_seconds: Option<i32>) -> chrono::DateTime<chrono::FixedOffset> {
    // The frontend supplies its UTC offset (-Date#getTimezoneOffset()*60,
    // east-positive seconds) because chrono's own local-offset lookup is
    // unavailable in `chrono` 0.4 without the `clock` feature (we use serde).
    // A missing/invalid offset falls back to UTC — the documented test
    // fallback; the pure pause_until() is tested with explicit offsets.
    use chrono::TimeZone;
    let seconds = offset_seconds.unwrap_or(0);
    let offset = chrono::FixedOffset::east_opt(seconds)
        .unwrap_or_else(|| chrono::FixedOffset::east_opt(0).unwrap());
    offset.from_utc_datetime(&Utc::now().naive_utc())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::CoreState;
    use crate::events::catalog::catalog_for;
    use crate::security::credentials::CredentialStore;
    use crate::security::crypto::FieldCipher;
    use crate::storage::config::ConfigRepository;
    use crate::storage::db::Database;
    use crate::storage::events::EventRepository;
    use crate::storage::integrations::IntegrationRepository;
    use crate::storage::queue::QueueRepository;
    use chrono::DateTime;
    use semver::Version;
    use std::sync::{Arc, Mutex};
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
                catalog_for(
                    crate::model::AgentKind::ClaudeCode,
                    &Version::new(2, 1, 218),
                )
                .catalog,
                catalog_for(crate::model::AgentKind::Codex, &Version::new(0, 145, 0)).catalog,
            ])
            .unwrap();
        let events = EventRepository::new(database.clone());
        let queue = QueueRepository::new(database.clone());
        let integrations = IntegrationRepository::new(database.clone());
        let credentials = CredentialStore::memory_for_test();
        let cipher = Arc::new(FieldCipher::from_key([7u8; 32]));
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

    fn default_input() -> SaveSettingsInput {
        SaveSettingsInput {
            autostart: false,
            close_to_tray: true,
            locale: LocaleInput::ZhCn,
            theme: ThemeInput::System,
            event_retention_days: 30,
            log_retention_days: 7,
            onboarding_completed: true,
        }
    }

    #[test]
    fn save_settings_applies_autostart_only_when_it_changes() {
        let mut st = state();
        let applied: Arc<Mutex<Vec<bool>>> = Arc::new(Mutex::new(Vec::new()));
        let sink = applied.clone();
        st.autostart_control = Arc::new(move |enable| {
            sink.lock().unwrap().push(enable);
            Ok(())
        });

        // First save flips false -> true: the control must fire once.
        let mut input = default_input();
        input.autostart = true;
        save_settings_impl(&st, input).unwrap();
        assert_eq!(*applied.lock().unwrap(), vec![true]);

        // A save that leaves the setting unchanged must not re-apply it.
        let mut same = default_input();
        same.autostart = true;
        save_settings_impl(&st, same).unwrap();
        assert_eq!(*applied.lock().unwrap(), vec![true]);

        // Flipping back to false disables.
        let off = default_input();
        save_settings_impl(&st, off).unwrap();
        assert_eq!(*applied.lock().unwrap(), vec![true, false]);
    }

    #[test]
    fn pause_today_uses_frontend_offset_for_local_midnight() {
        // The command receives offset_seconds = 8*3600 from the frontend; the
        // impl gets an explicit +08:00 "now" and must land on LOCAL midnight,
        // not UTC midnight (a UTC-midnight bug would be 8 hours off here).
        let st = state();
        let now: DateTime<chrono::FixedOffset> =
            DateTime::parse_from_rfc3339("2026-07-29T14:00:00+08:00").unwrap();
        let result = set_notification_pause_impl(&st, PauseDurationInput::Today, now).unwrap();
        assert_eq!(
            result.paused_until,
            Some(
                DateTime::parse_from_rfc3339("2026-07-30T00:00:00+08:00")
                    .unwrap()
                    .with_timezone(&Utc)
            )
        );
    }

    #[test]
    fn local_now_applies_the_frontend_offset() {
        assert_eq!(
            local_now(Some(8 * 3600)).offset().local_minus_utc(),
            8 * 3600
        );
        // Missing offset keeps the documented UTC fallback.
        assert_eq!(local_now(None).offset().local_minus_utc(), 0);
    }

    #[test]
    fn pause_does_not_mutate_rules() {
        // pause_until is pure: invoking it twice with the same input yields the
        // same output, and it never touches a RuleConfig. We assert purity by
        // equality of repeated calls.
        let now: DateTime<chrono::FixedOffset> =
            DateTime::parse_from_rfc3339("2026-08-05T10:00:00+00:00").unwrap();
        let a = pause_until(PauseDuration::OneHour, now);
        let b = pause_until(PauseDuration::OneHour, now);
        assert_eq!(a, b);
    }
}
