//! Shared health projection, tray state, and notification-pause arithmetic.
//!
//! Task 15 §B/§C: a single `HealthSnapshot` drives the overview page, the tray
//! menu and page-level badges. Pause durations are computed deterministically
//! from a local-aware clock so "Today" always means the next local midnight.
//!
//! This module is pure: it depends only on typed inputs and chrono. The
//! command layer feeds it real repository state; the frontend re-fetches typed
//! state after a `core://health-changed` / `core://queue-changed` /
//! `core://history-changed` event rather than trusting event payloads.

use chrono::{DateTime, FixedOffset, NaiveTime, TimeZone, Utc};
use serde::{Deserialize, Serialize};

use crate::model::AgentKind;

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

/// Coarse overall health level. Ordered worst-first for `max`-based rollup.
/// ` ponytail: three levels cover the tray/overview needs; add `Degraded` if a
/// future page needs to distinguish "stale agent" from "failed delivery".
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HealthLevel {
    Ok,
    Warning,
    Error,
}

impl HealthLevel {
    /// Worst of two levels. Used by the rollup.
    pub fn worst(self, other: Self) -> Self {
        match (self, other) {
            (Self::Error, _) | (_, Self::Error) => Self::Error,
            (Self::Warning, _) | (_, Self::Warning) => Self::Warning,
            (Self::Ok, Self::Ok) => Self::Ok,
        }
    }
}

/// Per-Agent integration state shown on the overview and Agent page.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AgentIntegrationHealth {
    pub agent: AgentKind,
    pub installed: bool,
    pub version: Option<String>,
    pub health: HealthLevel,
    pub summary: String,
}

/// Per-channel state shown on the overview and Channels page.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ChannelHealthState {
    pub channel_id: String,
    pub name: String,
    pub kind: String,
    pub health: HealthLevel,
    pub paused: bool,
    pub summary: String,
}

/// A stable, suggested remediation for a health issue. `command_id` and
/// `action_id` are closed identifiers the frontend can route to a page/action
/// without trusting free text.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct HealthIssue {
    pub issue_code: String,
    pub level: HealthLevel,
    pub message: String,
    pub suggested_command: Option<String>,
    pub suggested_action: Option<String>,
}

/// Aggregated counts and stable issues. The same projection feeds overview,
/// tray and page badges.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct HealthSnapshot {
    pub overall: HealthLevel,
    pub agents: Vec<AgentIntegrationHealth>,
    pub channels: Vec<ChannelHealthState>,
    pub pending_jobs: u64,
    pub retry_jobs: u64,
    pub failed_jobs: u64,
    pub expired_jobs: u64,
    pub succeeded_jobs: u64,
    pub spool_count: u64,
    pub rejected_count: u64,
    pub last_success_at: Option<DateTime<Utc>>,
    pub issues: Vec<HealthIssue>,
}

impl HealthSnapshot {
    /// Tray string: app name plus failed-job count when nonzero. Native tray
    /// items render this verbatim; the frontend never parses it.
    pub fn tray_label(&self) -> String {
        if self.failed_jobs == 0 {
            "CC Reminder".to_owned()
        } else {
            format!("CC Reminder - {} 个失败任务", self.failed_jobs)
        }
    }
}

/// Inputs to [`project_health`]. Built by the command layer from repository
/// reads; tests build it directly via the `with_*` helpers.
#[derive(Clone, Debug, Default)]
pub struct HealthInputs {
    pub agents: Vec<AgentIntegrationHealth>,
    pub channels: Vec<ChannelHealthState>,
    pub pending_jobs: u64,
    pub retry_jobs: u64,
    pub failed_jobs: u64,
    pub expired_jobs: u64,
    pub succeeded_jobs: u64,
    pub spool_count: u64,
    pub rejected_count: u64,
    pub last_success_at: Option<DateTime<Utc>>,
    pub issues: Vec<HealthIssue>,
}

impl HealthInputs {
    /// Test helper: a snapshot that only differs in failed-job count.
    pub fn with_failed_jobs(failed_jobs: u64) -> Self {
        Self {
            failed_jobs,
            ..Self::default()
        }
    }
}

/// Pure projection: derive the single shared snapshot from typed inputs. No
/// I/O, no clock. The rollup takes the worst level across agents, channels and
/// queue counts; explicit issues can only raise (never lower) the result.
pub fn project_health(inputs: &HealthInputs) -> HealthSnapshot {
    let mut overall = HealthLevel::Ok;
    for agent in &inputs.agents {
        overall = overall.worst(agent.health);
    }
    for channel in &inputs.channels {
        overall = overall.worst(channel.health);
    }
    if inputs.failed_jobs > 0 {
        overall = overall.worst(HealthLevel::Error);
    }
    if inputs.retry_jobs > 0 || inputs.rejected_count > 0 {
        overall = overall.worst(HealthLevel::Warning);
    }
    for issue in &inputs.issues {
        overall = overall.worst(issue.level);
    }
    HealthSnapshot {
        overall,
        agents: inputs.agents.clone(),
        channels: inputs.channels.clone(),
        pending_jobs: inputs.pending_jobs,
        retry_jobs: inputs.retry_jobs,
        failed_jobs: inputs.failed_jobs,
        expired_jobs: inputs.expired_jobs,
        succeeded_jobs: inputs.succeeded_jobs,
        spool_count: inputs.spool_count,
        rejected_count: inputs.rejected_count,
        last_success_at: inputs.last_success_at,
        issues: inputs.issues.clone(),
    }
}

/// Tray pause durations. Closed so the frontend cannot invent new ones.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PauseDuration {
    FifteenMinutes,
    OneHour,
    Today,
}

/// Compute the (local-aware) instant a pause ends. Pure. "Today" means the
/// next local midnight strictly after `now` (so a 23:59 pause still waits for
/// tomorrow's midnight, never the same calendar day). Returns a
/// `DateTime<FixedOffset>` so `to_rfc3339` carries the local offset.
pub fn pause_until(duration: PauseDuration, now: DateTime<FixedOffset>) -> DateTime<FixedOffset> {
    match duration {
        PauseDuration::FifteenMinutes => now + chrono::Duration::minutes(15),
        PauseDuration::OneHour => now + chrono::Duration::hours(1),
        PauseDuration::Today => {
            let midnight = NaiveTime::from_hms_opt(0, 0, 0).expect("midnight is valid");
            let today_midnight = now
                .timezone()
                .from_local_datetime(&now.date_naive().and_time(midnight))
                .single()
                .unwrap_or(now);
            if today_midnight > now {
                today_midnight
            } else {
                today_midnight + chrono::Duration::days(1)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn one_health_projection_drives_overview_tray_and_pages() {
        let snapshot = project_health(&HealthInputs::with_failed_jobs(2));
        assert_eq!(snapshot.overall, HealthLevel::Error);
        assert_eq!(snapshot.failed_jobs, 2);
        assert_eq!(snapshot.tray_label(), "CC Reminder - 2 个失败任务");
    }

    #[test]
    fn pause_today_uses_local_midnight_and_does_not_change_rules() {
        let now = local_time("2026-07-29T14:00:00+08:00");
        let result = pause_until(PauseDuration::Today, now);
        assert_eq!(result.to_rfc3339(), "2026-07-30T00:00:00+08:00");
    }

    #[test]
    fn pause_fifteen_and_one_hour_are_offset_based() {
        let now = local_time("2026-07-29T09:00:00+00:00");
        assert_eq!(
            pause_until(PauseDuration::FifteenMinutes, now).to_rfc3339(),
            "2026-07-29T09:15:00+00:00"
        );
        assert_eq!(
            pause_until(PauseDuration::OneHour, now).to_rfc3339(),
            "2026-07-29T10:00:00+00:00"
        );
    }

    #[test]
    fn pause_today_at_one_minute_past_midnight_waits_for_next_day() {
        let now = local_time("2026-07-29T00:01:00+08:00");
        let result = pause_until(PauseDuration::Today, now);
        assert_eq!(result.to_rfc3339(), "2026-07-30T00:00:00+08:00");
    }

    #[test]
    fn healthy_snapshot_has_clean_tray_label() {
        let snapshot = project_health(&HealthInputs::default());
        assert_eq!(snapshot.overall, HealthLevel::Ok);
        assert_eq!(snapshot.tray_label(), "CC Reminder");
    }

    #[test]
    fn retry_jobs_and_rejected_count_raise_to_warning_without_failed() {
        let inputs = HealthInputs {
            retry_jobs: 1,
            rejected_count: 2,
            ..HealthInputs::default()
        };
        assert_eq!(project_health(&inputs).overall, HealthLevel::Warning);
    }

    #[test]
    fn explicit_issue_can_raise_overall_but_never_lower_it() {
        let inputs = HealthInputs {
            failed_jobs: 1,
            issues: vec![HealthIssue {
                issue_code: "agent.not_detected".into(),
                level: HealthLevel::Warning,
                message: "agent missing".into(),
                suggested_command: Some("detect_agents".into()),
                suggested_action: None,
            }],
            ..HealthInputs::default()
        };
        // failed_jobs already Error; the Warning issue cannot lower it.
        assert_eq!(project_health(&inputs).overall, HealthLevel::Error);
    }

    #[test]
    fn agent_and_channel_health_roll_up_to_overall() {
        let inputs = HealthInputs {
            agents: vec![AgentIntegrationHealth {
                agent: AgentKind::Codex,
                installed: false,
                version: None,
                health: HealthLevel::Warning,
                summary: "not detected".into(),
            }],
            channels: vec![ChannelHealthState {
                channel_id: "ab".into(),
                name: "eng".into(),
                kind: "ding_talk".into(),
                health: HealthLevel::Error,
                paused: false,
                summary: "auth paused".into(),
            }],
            ..HealthInputs::default()
        };
        assert_eq!(project_health(&inputs).overall, HealthLevel::Error);
    }

    fn local_time(rfc3339: &str) -> DateTime<FixedOffset> {
        DateTime::parse_from_rfc3339(rfc3339).unwrap()
    }
}
