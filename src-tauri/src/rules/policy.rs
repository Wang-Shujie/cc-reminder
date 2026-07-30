use chrono::{
    DateTime, Datelike, Days, FixedOffset, NaiveDate, NaiveTime, TimeDelta, TimeZone, Utc,
};
use sha2::{Digest, Sha256};

use crate::events::catalog::HookCapability;
use crate::model::{
    DeliveryMode, EventEnvelope, FilterGroup, NotificationPause, QuietBehavior, RuleConfig,
    ScalarValue,
};

use super::resolve::parse_quiet_time;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PolicyDecision {
    SendNow,
    Aggregate {
        bucket_key: String,
        release_at: DateTime<Utc>,
    },
    DeferUntil(DateTime<Utc>),
    Suppress(SuppressReason),
    Expire,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SuppressReason {
    UnsupportedCapability,
    Disabled,
    FilterMismatch,
    GlobalPause,
    QuietHours,
    Cooldown,
    WindowLimit,
}

pub struct PolicyInput<'a> {
    pub event: &'a EventEnvelope,
    pub capability: &'a HookCapability,
    pub rule: &'a RuleConfig,
    pub notification_pause: Option<&'a NotificationPause>,
    pub now: DateTime<Utc>,
    pub local_offset: FixedOffset,
    pub recent_delivery_times: &'a [DateTime<Utc>],
}

pub fn matches_filters(event: &EventEnvelope, filters: &FilterGroup) -> bool {
    matches_dimension(&filters.tool_names, public_string(event, "tool_name"))
        && matches_dimension(
            &filters.event_subtypes,
            public_string(event, "event_subtype"),
        )
        && matches_dimension(&filters.permission_modes, event.permission_mode.as_deref())
        && matches_dimension(&filters.models, event.model.as_deref())
        && matches_dimension(&filters.statuses, public_string(event, "status"))
}

pub fn evaluate_policy(input: &PolicyInput<'_>) -> PolicyDecision {
    if input.capability.source_event != input.event.source_event {
        return PolicyDecision::Suppress(SuppressReason::UnsupportedCapability);
    }
    if !input.rule.enabled {
        return PolicyDecision::Suppress(SuppressReason::Disabled);
    }
    if !matches_filters(input.event, &input.rule.filters) {
        return PolicyDecision::Suppress(SuppressReason::FilterMismatch);
    }
    if input.now.signed_duration_since(input.event.occurred_at)
        >= TimeDelta::seconds(i64::from(input.rule.delivery.ttl_seconds))
    {
        return PolicyDecision::Expire;
    }
    if input.notification_pause.is_some_and(|pause| {
        pause_contains(pause, input.now) || pause_contains(pause, input.event.occurred_at)
    }) {
        return PolicyDecision::Suppress(SuppressReason::GlobalPause);
    }
    if let Some(decision) = quiet_decision(input) {
        return decision;
    }
    if input.rule.delivery.cooldown_seconds > 0 {
        let cutoff =
            input.now - TimeDelta::seconds(i64::from(input.rule.delivery.cooldown_seconds));
        if input
            .recent_delivery_times
            .iter()
            .any(|delivery| *delivery > cutoff && *delivery <= input.now)
        {
            return PolicyDecision::Suppress(SuppressReason::Cooldown);
        }
    }
    let window_start =
        input.now - TimeDelta::seconds(i64::from(input.rule.delivery.window_seconds));
    let deliveries_in_window = input
        .recent_delivery_times
        .iter()
        .filter(|delivery| **delivery > window_start && **delivery <= input.now)
        .count();
    if deliveries_in_window >= input.rule.delivery.max_per_window as usize {
        return PolicyDecision::Suppress(SuppressReason::WindowLimit);
    }

    match input.rule.delivery.mode {
        DeliveryMode::Immediate => PolicyDecision::SendNow,
        DeliveryMode::Aggregate { .. } if input.event.source_event == "PermissionRequest" => {
            PolicyDecision::SendNow
        }
        DeliveryMode::Aggregate { window_seconds } if window_seconds > 0 => {
            aggregate_decision(input, window_seconds)
        }
        DeliveryMode::Aggregate { .. } => PolicyDecision::SendNow,
    }
}

fn matches_dimension(expected: &[String], actual: Option<&str>) -> bool {
    expected.is_empty()
        || actual.is_some_and(|actual| expected.iter().any(|expected| expected == actual))
}

fn public_string<'a>(event: &'a EventEnvelope, field: &str) -> Option<&'a str> {
    match event.public_fields.get(field) {
        Some(ScalarValue::String(value)) => Some(value),
        _ => None,
    }
}

fn pause_contains(pause: &NotificationPause, instant: DateTime<Utc>) -> bool {
    instant >= pause.started_at && instant < pause.until
}

fn quiet_decision(input: &PolicyInput<'_>) -> Option<PolicyDecision> {
    let quiet = input.rule.quiet_hours.as_ref()?;
    if quiet
        .bypass_at_or_above
        .is_some_and(|severity| input.event.severity >= severity)
    {
        return None;
    }

    let start = parse_quiet_time(&quiet.start_local)?;
    let end = parse_quiet_time(&quiet.end_local)?;
    let local_now = input.now.with_timezone(&input.local_offset);
    let time = local_now.time();
    let weekday = local_now.weekday().number_from_monday() as u8;
    let (active, start_weekday) = quiet_membership(time, weekday, start, end);
    if !active || !quiet.weekdays.contains(&start_weekday) {
        return None;
    }

    Some(match input.rule.delivery.quiet_behavior {
        QuietBehavior::Suppress => PolicyDecision::Suppress(SuppressReason::QuietHours),
        QuietBehavior::Defer => PolicyDecision::DeferUntil(quiet_end(local_now, start, end)),
    })
}

fn quiet_membership(time: NaiveTime, weekday: u8, start: NaiveTime, end: NaiveTime) -> (bool, u8) {
    if start < end {
        (time >= start && time < end, weekday)
    } else if start > end {
        if time >= start {
            (true, weekday)
        } else if time < end {
            (true, if weekday == 1 { 7 } else { weekday - 1 })
        } else {
            (false, weekday)
        }
    } else {
        (true, weekday)
    }
}

fn quiet_end(now: DateTime<FixedOffset>, start: NaiveTime, end: NaiveTime) -> DateTime<Utc> {
    let end_date = if start >= end && now.time() >= start || start == end {
        next_day(now.date_naive())
    } else {
        now.date_naive()
    };
    now.offset()
        .from_local_datetime(&end_date.and_time(end))
        .single()
        .expect("a fixed offset has one local representation")
        .with_timezone(&Utc)
}

fn next_day(date: NaiveDate) -> NaiveDate {
    date.checked_add_days(Days::new(1))
        .expect("valid DateTime has a following day")
}

fn aggregate_decision(input: &PolicyInput<'_>, window_seconds: u32) -> PolicyDecision {
    let window_seconds = i64::from(window_seconds);
    let release_timestamp = (input.now.timestamp().div_euclid(window_seconds) + 1) * window_seconds;
    let release_at = DateTime::from_timestamp(release_timestamp, 0)
        .expect("aggregate release derived from a valid DateTime");
    let mut hasher = Sha256::new();
    hash_part(&mut hasher, input.event.source.as_str().as_bytes());
    hash_part(&mut hasher, input.event.source_event.as_bytes());
    hash_part(
        &mut hasher,
        input
            .event
            .project_id
            .as_ref()
            .map(uuid::Uuid::as_bytes)
            .map(<[u8; 16]>::as_slice)
            .unwrap_or_default(),
    );
    hash_part(
        &mut hasher,
        &serde_json::to_vec(input.rule).expect("RuleConfig serialization cannot fail"),
    );
    hash_part(&mut hasher, &release_timestamp.to_be_bytes());

    PolicyDecision::Aggregate {
        bucket_key: hex::encode(hasher.finalize()),
        release_at,
    }
}

fn hash_part(hasher: &mut Sha256, value: &[u8]) {
    hasher.update((value.len() as u64).to_be_bytes());
    hasher.update(value);
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use chrono::{DateTime, FixedOffset, TimeDelta, Utc};
    use semver::Version;
    use uuid::Uuid;

    use super::{PolicyDecision, PolicyInput, SuppressReason, evaluate_policy, matches_filters};
    use crate::events::catalog::{HookCapability, catalog_for};
    use crate::events::normalize::{NormalizeContext, capture_hook_json, normalize_event};
    use crate::model::{
        AgentKind, DeliveryMode, EventCategory, EventEnvelope, FilterGroup, NotificationPause,
        QuietBehavior, QuietHours, ScalarValue, Severity,
    };
    use crate::projects::PathPlatform;
    use crate::rules::resolve::default_rule;

    #[test]
    fn filter_dimensions_are_anded_and_values_within_them_are_ored() {
        let mut event = event("Stop");
        event.model = Some("gpt-5".into());
        event.permission_mode = Some("plan".into());
        event
            .public_fields
            .insert("tool_name".into(), ScalarValue::String("Read".into()));
        let filters = FilterGroup {
            tool_names: vec!["Write".into(), "Read".into()],
            permission_modes: vec!["default".into(), "plan".into()],
            models: vec!["other".into(), "gpt-5".into()],
            ..FilterGroup::default()
        };

        assert!(matches_filters(&event, &filters));

        let mismatched = FilterGroup {
            models: vec!["missing".into()],
            ..filters
        };
        assert!(!matches_filters(&event, &mismatched));
    }

    #[test]
    fn normalized_catalog_fields_drive_subtype_and_status_filters() {
        let captured = capture_hook_json(
            AgentKind::Codex,
            "SessionEnd",
            Version::new(0, 145, 0),
            serde_json::json!({ "reason": "clear" }),
        )
        .unwrap();
        let event = normalize_event(
            captured,
            &NormalizeContext {
                correlation_key: [3_u8; 32],
                projects: Vec::new(),
                platform: PathPlatform::Unix,
            },
        )
        .unwrap();
        let filters = FilterGroup {
            event_subtypes: vec!["clear".into()],
            statuses: vec!["end".into()],
            ..FilterGroup::default()
        };

        assert!(matches_filters(&event, &filters));
    }

    #[test]
    fn empty_filter_dimensions_are_wildcards() {
        assert!(matches_filters(&event("Stop"), &FilterGroup::default()));
    }

    #[test]
    fn policy_checks_run_in_the_declared_order() {
        let mut fixture = Fixture::new("Stop");
        fixture.rule.enabled = false;
        fixture.capability = capability("PermissionRequest");
        assert_eq!(
            evaluate_policy(&fixture.input()),
            PolicyDecision::Suppress(SuppressReason::UnsupportedCapability)
        );

        fixture.capability = capability("Stop");
        fixture.rule.filters.models = vec!["missing".into()];
        assert_eq!(
            evaluate_policy(&fixture.input()),
            PolicyDecision::Suppress(SuppressReason::Disabled)
        );

        fixture.rule.enabled = true;
        fixture.event.occurred_at = fixture.now - TimeDelta::seconds(1_800);
        assert_eq!(
            evaluate_policy(&fixture.input()),
            PolicyDecision::Suppress(SuppressReason::FilterMismatch)
        );

        fixture.rule.filters = FilterGroup::default();
        fixture.pause = Some(NotificationPause {
            started_at: fixture.now - TimeDelta::hours(1),
            until: fixture.now + TimeDelta::hours(1),
        });
        assert_eq!(evaluate_policy(&fixture.input()), PolicyDecision::Expire);

        fixture.event.occurred_at = fixture.now;
        fixture.rule.quiet_hours = Some(all_day_quiet());
        assert_eq!(
            evaluate_policy(&fixture.input()),
            PolicyDecision::Suppress(SuppressReason::GlobalPause)
        );

        fixture.pause = None;
        fixture.rule.delivery.cooldown_seconds = 60;
        fixture.recent_delivery_times = vec![fixture.now - TimeDelta::seconds(1)];
        assert_eq!(
            evaluate_policy(&fixture.input()),
            PolicyDecision::Suppress(SuppressReason::QuietHours)
        );

        fixture.rule.quiet_hours = None;
        fixture.rule.delivery.max_per_window = 1;
        fixture.rule.delivery.window_seconds = 60;
        fixture.rule.delivery.mode = DeliveryMode::Aggregate { window_seconds: 60 };
        assert_eq!(
            evaluate_policy(&fixture.input()),
            PolicyDecision::Suppress(SuppressReason::Cooldown)
        );

        fixture.rule.delivery.cooldown_seconds = 0;
        assert_eq!(
            evaluate_policy(&fixture.input()),
            PolicyDecision::Suppress(SuppressReason::WindowLimit)
        );
    }

    #[test]
    fn expired_offline_event_is_not_sent_under_new_rules() {
        let mut fixture = Fixture::new("Stop");
        fixture.event.occurred_at = fixture.now - TimeDelta::minutes(31);
        fixture.rule.delivery.ttl_seconds = 1_800;

        assert_eq!(evaluate_policy(&fixture.input()), PolicyDecision::Expire);
    }

    #[test]
    fn event_expires_exactly_at_its_ttl_boundary() {
        let mut fixture = Fixture::new("Stop");
        fixture.event.occurred_at = fixture.now - TimeDelta::seconds(1_800);

        assert_eq!(evaluate_policy(&fixture.input()), PolicyDecision::Expire);
    }

    #[test]
    fn offline_event_that_occurred_during_global_pause_is_suppressed() {
        let mut fixture = Fixture::new("Stop");
        fixture.now = at("2026-07-29T16:00:00+08:00");
        fixture.event.occurred_at = at("2026-07-29T14:30:00+08:00");
        fixture.event.received_at = fixture.now;
        fixture.rule.delivery.ttl_seconds = 10_800;
        fixture.pause = Some(NotificationPause {
            started_at: at("2026-07-29T14:00:00+08:00"),
            until: at("2026-07-29T15:00:00+08:00"),
        });

        assert_eq!(
            evaluate_policy(&fixture.input()),
            PolicyDecision::Suppress(SuppressReason::GlobalPause)
        );
    }

    #[test]
    fn pause_intervals_include_start_and_exclude_end() {
        let mut fixture = Fixture::new("Stop");
        fixture.pause = Some(NotificationPause {
            started_at: fixture.now,
            until: fixture.now + TimeDelta::hours(1),
        });
        assert_eq!(
            evaluate_policy(&fixture.input()),
            PolicyDecision::Suppress(SuppressReason::GlobalPause)
        );

        fixture.now += TimeDelta::hours(1);
        fixture.event.occurred_at = fixture.now;
        assert_eq!(evaluate_policy(&fixture.input()), PolicyDecision::SendNow);
    }

    #[test]
    fn overnight_quiet_hours_use_the_start_days_weekday() {
        let mut fixture = Fixture::new("Stop");
        fixture.rule.quiet_hours = Some(QuietHours {
            start_local: "22:00".into(),
            end_local: "08:00".into(),
            weekdays: vec![1],
            bypass_at_or_above: None,
        });
        fixture.now = at("2026-07-27T22:00:00Z");
        fixture.event.occurred_at = fixture.now;
        assert_eq!(
            evaluate_policy(&fixture.input()),
            PolicyDecision::Suppress(SuppressReason::QuietHours)
        );

        fixture.now = at("2026-07-28T07:59:59Z");
        fixture.event.occurred_at = fixture.now;
        assert_eq!(
            evaluate_policy(&fixture.input()),
            PolicyDecision::Suppress(SuppressReason::QuietHours)
        );

        fixture.now = at("2026-07-28T08:00:00Z");
        fixture.event.occurred_at = fixture.now;
        assert_eq!(evaluate_policy(&fixture.input()), PolicyDecision::SendNow);
    }

    #[test]
    fn quiet_hours_respect_weekday_and_severity_bypass() {
        let mut fixture = Fixture::new("Stop");
        fixture.now = at("2026-07-27T10:00:00Z");
        fixture.event.occurred_at = fixture.now;
        fixture.rule.quiet_hours = Some(QuietHours {
            start_local: "09:00".into(),
            end_local: "17:00".into(),
            weekdays: vec![2],
            bypass_at_or_above: Some(Severity::Error),
        });
        assert_eq!(evaluate_policy(&fixture.input()), PolicyDecision::SendNow);

        fixture.rule.quiet_hours.as_mut().unwrap().weekdays = vec![1];
        fixture.event.severity = Severity::Critical;
        assert_eq!(evaluate_policy(&fixture.input()), PolicyDecision::SendNow);

        fixture.event.severity = Severity::Warning;
        assert_eq!(
            evaluate_policy(&fixture.input()),
            PolicyDecision::Suppress(SuppressReason::QuietHours)
        );
    }

    #[test]
    fn defer_quiet_behavior_returns_the_exclusive_end_boundary() {
        let mut fixture = Fixture::new("Stop");
        fixture.now = at("2026-07-27T10:00:00Z");
        fixture.event.occurred_at = fixture.now;
        fixture.rule.delivery.quiet_behavior = QuietBehavior::Defer;
        fixture.rule.quiet_hours = Some(QuietHours {
            start_local: "09:00".into(),
            end_local: "17:00".into(),
            weekdays: vec![1],
            bypass_at_or_above: None,
        });

        assert_eq!(
            evaluate_policy(&fixture.input()),
            PolicyDecision::DeferUntil(at("2026-07-27T17:00:00Z"))
        );
    }

    #[test]
    fn non_utc_quiet_hours_use_local_weekday_and_return_a_utc_deadline() {
        let mut fixture = Fixture::new("Stop");
        fixture.now = at("2026-07-27T14:30:00Z");
        fixture.event.occurred_at = fixture.now;
        fixture.local_offset = FixedOffset::east_opt(8 * 60 * 60).unwrap();
        fixture.rule.delivery.quiet_behavior = QuietBehavior::Defer;
        fixture.rule.quiet_hours = Some(QuietHours {
            start_local: "22:00".into(),
            end_local: "08:00".into(),
            weekdays: vec![1],
            bypass_at_or_above: None,
        });

        assert_eq!(
            evaluate_policy(&fixture.input()),
            PolicyDecision::DeferUntil(at("2026-07-28T00:00:00Z"))
        );
    }

    #[test]
    fn cooldown_includes_recent_deliveries_but_excludes_its_lower_boundary() {
        let mut fixture = Fixture::new("Stop");
        fixture.rule.delivery.cooldown_seconds = 60;
        fixture.recent_delivery_times = vec![fixture.now - TimeDelta::seconds(59)];
        assert_eq!(
            evaluate_policy(&fixture.input()),
            PolicyDecision::Suppress(SuppressReason::Cooldown)
        );

        fixture.recent_delivery_times = vec![fixture.now - TimeDelta::seconds(60)];
        assert_eq!(evaluate_policy(&fixture.input()), PolicyDecision::SendNow);
    }

    #[test]
    fn per_window_cap_counts_only_the_active_half_open_window() {
        let mut fixture = Fixture::new("Stop");
        fixture.rule.delivery.max_per_window = 2;
        fixture.rule.delivery.window_seconds = 60;
        fixture.recent_delivery_times = vec![
            fixture.now - TimeDelta::seconds(59),
            fixture.now - TimeDelta::seconds(1),
        ];
        assert_eq!(
            evaluate_policy(&fixture.input()),
            PolicyDecision::Suppress(SuppressReason::WindowLimit)
        );

        fixture.recent_delivery_times = vec![
            fixture.now - TimeDelta::seconds(60),
            fixture.now - TimeDelta::seconds(1),
        ];
        assert_eq!(evaluate_policy(&fixture.input()), PolicyDecision::SendNow);
    }

    #[test]
    fn permission_requests_never_aggregate() {
        let mut fixture = Fixture::new("PermissionRequest");
        fixture.rule.delivery.mode = DeliveryMode::Aggregate { window_seconds: 60 };

        assert_eq!(evaluate_policy(&fixture.input()), PolicyDecision::SendNow);
    }

    #[test]
    fn aggregate_events_share_an_epoch_aligned_window_key_without_raw_ids() {
        let mut first = Fixture::new("Stop");
        first.now = at("2026-07-27T12:00:30Z");
        first.event.occurred_at = first.now;
        first.event.project_id = Some(Uuid::from_u128(42));
        first.rule.delivery.mode = DeliveryMode::Aggregate { window_seconds: 60 };
        let PolicyDecision::Aggregate {
            bucket_key,
            release_at,
        } = evaluate_policy(&first.input())
        else {
            panic!("expected aggregate decision");
        };

        let mut second = first;
        second.now = at("2026-07-27T12:00:59Z");
        second.event.occurred_at = second.now;
        let PolicyDecision::Aggregate {
            bucket_key: second_key,
            release_at: second_release,
        } = evaluate_policy(&second.input())
        else {
            panic!("expected aggregate decision");
        };

        assert_eq!(release_at, at("2026-07-27T12:01:00Z"));
        assert_eq!(second_release, release_at);
        assert_eq!(second_key, bucket_key);
        assert_eq!(bucket_key.len(), 64);
        assert!(!bucket_key.contains(&Uuid::from_u128(42).to_string()));
    }

    #[test]
    fn aggregate_bucket_key_separates_projects_and_windows() {
        let mut fixture = Fixture::new("Stop");
        fixture.now = at("2026-07-27T12:00:30Z");
        fixture.event.occurred_at = fixture.now;
        fixture.event.project_id = Some(Uuid::from_u128(42));
        fixture.rule.delivery.mode = DeliveryMode::Aggregate { window_seconds: 60 };
        let first_key = aggregate_key(&fixture);

        fixture.event.project_id = Some(Uuid::from_u128(43));
        assert_ne!(aggregate_key(&fixture), first_key);

        fixture.event.project_id = Some(Uuid::from_u128(42));
        fixture.now = at("2026-07-27T12:01:00Z");
        fixture.event.occurred_at = fixture.now;
        assert_ne!(aggregate_key(&fixture), first_key);
    }

    fn aggregate_key(fixture: &Fixture) -> String {
        match evaluate_policy(&fixture.input()) {
            PolicyDecision::Aggregate { bucket_key, .. } => bucket_key,
            decision => panic!("expected aggregate decision, got {decision:?}"),
        }
    }

    struct Fixture {
        event: EventEnvelope,
        capability: HookCapability,
        rule: crate::model::RuleConfig,
        pause: Option<NotificationPause>,
        now: DateTime<Utc>,
        local_offset: FixedOffset,
        recent_delivery_times: Vec<DateTime<Utc>>,
    }

    impl Fixture {
        fn new(source_event: &str) -> Self {
            let now = at("2026-07-27T12:00:00Z");
            let mut event = event(source_event);
            event.occurred_at = now;
            event.received_at = now;
            Self {
                event,
                capability: capability(source_event),
                rule: default_rule(AgentKind::Codex, source_event),
                pause: None,
                now,
                local_offset: FixedOffset::east_opt(0).unwrap(),
                recent_delivery_times: Vec::new(),
            }
        }

        fn input(&self) -> PolicyInput<'_> {
            PolicyInput {
                event: &self.event,
                capability: &self.capability,
                rule: &self.rule,
                notification_pause: self.pause.as_ref(),
                now: self.now,
                local_offset: self.local_offset,
                recent_delivery_times: &self.recent_delivery_times,
            }
        }
    }

    fn event(source_event: &str) -> EventEnvelope {
        EventEnvelope {
            id: Uuid::from_u128(1),
            source: AgentKind::Codex,
            source_version: Version::new(0, 145, 0),
            source_event: source_event.into(),
            category: EventCategory::Completion,
            occurred_at: at("2026-07-27T12:00:00Z"),
            received_at: at("2026-07-27T12:00:00Z"),
            project_id: None,
            project_display_name: None,
            unmatched_cwd_fingerprint: None,
            session_ref: None,
            turn_ref: None,
            model: None,
            permission_mode: None,
            severity: Severity::Info,
            public_fields: BTreeMap::new(),
            encrypted_sensitive_fields: None,
            correlation_id: Uuid::from_u128(2),
            action_id: None,
            action_capabilities: Vec::new(),
        }
    }

    fn capability(source_event: &str) -> HookCapability {
        catalog_for(AgentKind::Codex, &Version::new(0, 145, 0))
            .catalog
            .hooks
            .into_iter()
            .find(|hook| hook.source_event == source_event)
            .unwrap()
    }

    fn all_day_quiet() -> QuietHours {
        QuietHours {
            start_local: "00:00".into(),
            end_local: "00:00".into(),
            weekdays: vec![1, 2, 3, 4, 5, 6, 7],
            bypass_at_or_above: None,
        }
    }

    fn at(value: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(value)
            .unwrap()
            .with_timezone(&Utc)
    }
}
