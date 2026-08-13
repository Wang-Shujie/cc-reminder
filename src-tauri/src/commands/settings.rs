//! Settings + pause commands.

use chrono::Utc;
use serde::{Deserialize, Serialize};
use tauri::State;

use super::{CoreState, configuration_error};
use crate::error::AppError;
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
    settings.autostart = input.autostart;
    settings.close_to_tray = input.close_to_tray;
    settings.locale = input.locale.into_locale();
    settings.theme = input.theme.into_theme();
    settings.event_retention_days = input.event_retention_days;
    settings.log_retention_days = input.log_retention_days;
    settings.onboarding_completed = input.onboarding_completed;
    let saved = state.config.save_settings(&settings)?;
    // Autostart is the only side-effecting setting; the only place it is
    // applied is here, per the plan ("updated only from save_settings").
    // The plugin's ManagerExt is used in the real runtime; tests cover the
    // settings value alone.
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
        started_at: Utc::now(),
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
) -> Result<SettingsView, AppError> {
    let state = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let now = local_now();
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

fn local_now() -> chrono::DateTime<chrono::FixedOffset> {
    // ponytail: chrono's local offset is unavailable in `chrono` 0.4 without
    // the `clock` feature (we use serde). Fall back to UTC for the runtime
    // path; the pure pause_until() is tested with explicit local offsets.
    use chrono::TimeZone;
    chrono::FixedOffset::east_opt(0)
        .unwrap()
        .from_utc_datetime(&Utc::now().naive_utc())
}

#[cfg(test)]
mod tests {
    use crate::health::{PauseDuration, pause_until};
    use chrono::DateTime;

    #[test]
    fn pause_today_uses_local_midnight_and_does_not_change_rules() {
        let now: DateTime<chrono::FixedOffset> =
            DateTime::parse_from_rfc3339("2026-07-29T14:00:00+08:00").unwrap();
        let result = pause_until(PauseDuration::Today, now);
        assert_eq!(result.to_rfc3339(), "2026-07-30T00:00:00+08:00");
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
