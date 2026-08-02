use std::collections::{BTreeMap, BTreeSet};

use crate::error::{AppError, ErrorDomain};
use crate::events::catalog::{Sensitivity, catalog_for};
use crate::model::{EventEnvelope, NotificationDocument, ScalarValue, Severity};
use crate::security::redact::Redactor;

const MAX_TEMPLATE_BYTES: usize = 16 * 1024;
const BASE_TEMPLATE_FIELDS: [&str; 6] = [
    "agent.name",
    "agent.version",
    "project.name",
    "event.name",
    "event.severity",
    "event.occurred_at",
];

pub const DEFAULT_TEMPLATE_ZH: &str = concat!(
    "[{{agent.name}}] {{event.label}}\n",
    "项目：{{project.name}}\n",
    "状态：{{event.status}}\n",
    "摘要：{{event.summary}}\n",
    "时间：{{event.occurred_at}}",
);

#[derive(Clone, Debug)]
pub struct TemplateContext {
    values: BTreeMap<String, String>,
    authorized: BTreeSet<&'static str>,
    severity: Severity,
}

pub fn build_template_context(event: &EventEnvelope, allowed_fields: &[String]) -> TemplateContext {
    let mut values = BTreeMap::from([
        ("agent.name".into(), event.source.as_str().into()),
        ("agent.version".into(), event.source_version.to_string()),
        ("event.name".into(), event.source_event.clone()),
        (
            "event.severity".into(),
            severity_name(event.severity).into(),
        ),
        ("event.occurred_at".into(), event.occurred_at.to_rfc3339()),
    ]);
    let mut authorized = BASE_TEMPLATE_FIELDS.into_iter().collect::<BTreeSet<_>>();
    if let Some(project) = &event.project_display_name {
        values.insert("project.name".into(), project.clone());
    }

    if let Some(hook) = catalog_for(event.source, &event.source_version)
        .catalog
        .hooks
        .into_iter()
        .find(|hook| hook.source_event == event.source_event)
    {
        values.insert("event.label".into(), hook.label_zh);
        values.insert("event.status".into(), hook.phase);
        authorized.extend(["event.label", "event.status"]);

        for (field, path) in [
            ("duration", "event.duration"),
            ("tool_name", "event.tool_name"),
        ] {
            let is_public = hook
                .input_fields
                .iter()
                .any(|input| input.name == field && input.sensitivity == Sensitivity::Public);
            if is_public {
                authorized.insert(path);
                if let Some(value) = event.public_fields.get(field).and_then(scalar_text) {
                    values.insert(path.into(), value);
                }
            }
        }

        let summary_source = hook.input_fields.iter().find(|input| {
            matches!(
                input.name.as_str(),
                "summary" | "last_assistant_message" | "error"
            ) && input.sensitivity != Sensitivity::Forbidden
                && allowed_fields.iter().any(|field| {
                    field == &input.name || field == "summary" || field == "event.summary"
                })
        });
        if let Some(source) = summary_source {
            authorized.insert("event.summary");
            if source.sensitivity == Sensitivity::Public
                && let Some(value) = event.public_fields.get(&source.name).and_then(scalar_text)
            {
                values.insert("event.summary".into(), value);
            }
        }
    }

    TemplateContext {
        values,
        authorized,
        severity: event.severity,
    }
}

pub fn render_document(
    template: &str,
    context: &TemplateContext,
    redactor: &Redactor,
    max_chars: usize,
) -> Result<NotificationDocument, AppError> {
    if template.len() > MAX_TEMPLATE_BYTES {
        return Err(invalid_template());
    }

    let rendered = render_template(template, context)?;
    let body = redactor.redact(&rendered).chars().take(max_chars).collect();
    let event_name = context.value("event.name");
    let event_label = context.value("event.label");
    let title = if event_label.is_empty() {
        event_name
    } else {
        event_label
    };
    let agent = match (context.value("agent.name"), context.value("agent.version")) {
        ("", version) => version.to_owned(),
        (name, "") => name.to_owned(),
        (name, version) => format!("{name} {version}"),
    };
    let facts = [
        ("Agent", agent),
        ("Project", context.value("project.name").to_owned()),
        ("Hook", event_name.to_owned()),
        ("Status", context.value("event.status").to_owned()),
        ("Time", context.value("event.occurred_at").to_owned()),
    ]
    .into_iter()
    .map(|(name, value)| (name.into(), redactor.redact(&value)))
    .collect();

    Ok(NotificationDocument {
        title: redactor.redact(title),
        severity: context.severity,
        facts,
        body,
        footer: None,
    })
}

impl TemplateContext {
    fn value(&self, path: &str) -> &str {
        self.values.get(path).map_or("", String::as_str)
    }
}

fn render_template(template: &str, context: &TemplateContext) -> Result<String, AppError> {
    let mut output = String::with_capacity(template.len());
    let mut remaining = template;

    while let Some(index) = remaining.find(['{', '}']) {
        output.push_str(&remaining[..index]);
        remaining = &remaining[index..];
        if !remaining.starts_with("{{") {
            return Err(invalid_template());
        }

        let end = remaining[2..]
            .find("}}")
            .map(|offset| offset + 2)
            .ok_or_else(invalid_template)?;
        let path = &remaining[2..end];
        if path.contains(['{', '}']) || !valid_path(path) {
            return Err(invalid_template());
        }
        if !context.authorized.contains(path) {
            return Err(field_not_allowed());
        }

        output.push_str(context.value(path));
        remaining = &remaining[end + 2..];
    }

    output.push_str(remaining);
    Ok(output)
}

fn valid_path(path: &str) -> bool {
    let Some((root, leaf)) = path.split_once('.') else {
        return false;
    };
    !root.is_empty()
        && !leaf.is_empty()
        && !leaf.contains('.')
        && root.bytes().all(|byte| byte.is_ascii_lowercase())
        && leaf
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte == b'_')
}

fn scalar_text(value: &ScalarValue) -> Option<String> {
    match value {
        ScalarValue::String(value) => Some(value.clone()),
        ScalarValue::Number(value) => Some(value.to_string()),
        ScalarValue::Bool(value) => Some(value.to_string()),
        ScalarValue::Null => None,
    }
}

const fn severity_name(severity: Severity) -> &'static str {
    match severity {
        Severity::Info => "info",
        Severity::Warning => "warning",
        Severity::Error => "error",
        Severity::Critical => "critical",
    }
}

fn invalid_template() -> AppError {
    template_error(
        "configuration.template_invalid",
        "notification template is invalid",
    )
}

fn field_not_allowed() -> AppError {
    template_error(
        "configuration.template_field_not_allowed",
        "notification template field is not allowed",
    )
}

fn template_error(code: &str, message: &str) -> AppError {
    AppError {
        domain: ErrorDomain::Configuration,
        code: code.into(),
        message: message.into(),
        suggested_action: None,
    }
}

#[cfg(test)]
mod tests {
    use std::collections::{BTreeMap, BTreeSet};

    use chrono::{DateTime, Utc};
    use semver::Version;
    use uuid::Uuid;

    use crate::model::{AgentKind, EventCategory, EventEnvelope, ScalarValue, Severity};
    use crate::rules::{
        DEFAULT_TEMPLATE_ZH, TemplateContext, build_template_context, render_document,
    };
    use crate::security::redact::Redactor;

    #[test]
    fn template_cannot_access_field_removed_by_privacy_policy() {
        let context = context_with_only(&[("event.label", "需要授权")]);
        let error = render_document(
            "{{event.label}} {{event.full_prompt}}",
            &context,
            &redactor(),
            500,
        )
        .unwrap_err();

        assert_eq!(error.code, "configuration.template_field_not_allowed");
        assert!(!error.message.contains("event.full_prompt"));
    }

    #[test]
    fn renders_default_native_summary_then_redacts_and_truncates() {
        let context = context_with_only(&[
            ("agent.name", "Codex"),
            ("project.name", "cc-reminder"),
            ("event.label", "完成"),
            ("event.status", "success"),
            (
                "event.summary",
                "finished with token=very-secret and extra text",
            ),
            ("event.occurred_at", "2026-07-29T12:00:00Z"),
        ]);

        let document = render_document(DEFAULT_TEMPLATE_ZH, &context, &redactor(), 200).unwrap();
        assert!(!document.body.contains("very-secret"));
        assert!(document.body.contains("[REDACTED]"));

        let truncated = render_document("{{event.summary}}", &context, &redactor(), 24).unwrap();
        assert!(!truncated.body.contains("very-secret"));
        assert!(truncated.body.chars().count() <= 24);
    }

    #[test]
    fn rejects_executable_unknown_malformed_and_oversized_templates() {
        for template in [
            "{{#if event.label}}",
            "{{event.label()}}",
            "{{#each event.label}}",
            "{% for item in event %}",
            "{{event.label",
            "event.label}}",
        ] {
            let error =
                render_document(template, &context_with_only(&[]), &redactor(), 500).unwrap_err();
            assert_eq!(error.code, "configuration.template_invalid");
            assert!(!error.message.contains(template));
        }

        for template in ["{{unknown.label}}", "{{event.unknown}}"] {
            let error =
                render_document(template, &context_with_only(&[]), &redactor(), 500).unwrap_err();
            assert_eq!(error.code, "configuration.template_field_not_allowed");
            assert!(!error.message.contains(template));
        }

        let at_limit = "x".repeat(16 * 1024);
        assert_eq!(
            render_document(&at_limit, &context_with_only(&[]), &redactor(), 3)
                .unwrap()
                .body,
            "xxx"
        );

        let over_limit = "x".repeat(16 * 1024 + 1);
        let error =
            render_document(&over_limit, &context_with_only(&[]), &redactor(), 500).unwrap_err();
        assert_eq!(error.code, "configuration.template_invalid");
        assert!(!error.message.contains(&over_limit));
    }

    #[test]
    fn missing_authorized_values_are_empty_and_limits_count_unicode_scalars() {
        let mut context = context_with_only(&[]);
        context.authorized.insert("event.summary");
        assert_eq!(
            render_document("A{{event.summary}}B", &context, &redactor(), 500)
                .unwrap()
                .body,
            "AB"
        );

        let context = context_with_only(&[("event.summary", "甲乙丙丁")]);
        assert_eq!(
            render_document("{{event.summary}}", &context, &redactor(), 3)
                .unwrap()
                .body,
            "甲乙丙"
        );
    }

    #[test]
    fn production_context_authorizes_summary_only_when_selected_and_catalogued() {
        let event = stop_event();
        let unselected = build_template_context(&event, &[]);
        let error =
            render_document("{{event.summary}}", &unselected, &redactor(), 500).unwrap_err();
        assert_eq!(error.code, "configuration.template_field_not_allowed");

        let selected = build_template_context(&event, &["last_assistant_message".into()]);
        assert_eq!(
            render_document("A{{event.summary}}B", &selected, &redactor(), 500)
                .unwrap()
                .body,
            "AB"
        );

        let unsupported =
            build_template_context(&permission_event(), &["last_assistant_message".into()]);
        let error =
            render_document("{{event.summary}}", &unsupported, &redactor(), 500).unwrap_err();
        assert_eq!(error.code, "configuration.template_field_not_allowed");
    }

    #[test]
    fn context_uses_only_catalog_public_fields_and_builds_stable_facts() {
        let event = permission_event();
        let context = build_template_context(
            &event,
            &["summary".into(), "full_prompt".into(), "tool_input".into()],
        );
        let document = render_document("{{event.tool_name}}", &context, &redactor(), 500).unwrap();

        assert_eq!(document.title, "权限请求");
        assert_eq!(document.severity, Severity::Warning);
        assert_eq!(document.body, "Bash");
        assert_eq!(
            document.facts,
            vec![
                ("Agent".into(), "codex 0.145.0".into()),
                ("Project".into(), "cc-reminder".into()),
                ("Hook".into(), "PermissionRequest".into()),
                ("Status".into(), "request".into()),
                ("Time".into(), "2026-07-29T12:00:00+00:00".into()),
            ]
        );

        for template in ["{{event.summary}}", "{{event.full_prompt}}"] {
            let error = render_document(template, &context, &redactor(), 500).unwrap_err();
            assert_eq!(error.code, "configuration.template_field_not_allowed");
        }
    }

    #[test]
    fn redacts_title_and_facts_as_well_as_body() {
        let context = context_with_only(&[
            ("project.name", "token=project-secret"),
            ("event.label", "token=title-secret"),
            ("event.summary", "token=body-secret"),
        ]);
        let document = render_document("{{event.summary}}", &context, &redactor(), 500).unwrap();

        let outbound = format!("{} {:?} {}", document.title, document.facts, document.body);
        for secret in ["project-secret", "title-secret", "body-secret"] {
            assert!(!outbound.contains(secret));
        }
    }

    fn context_with_only(entries: &[(&'static str, &'static str)]) -> TemplateContext {
        TemplateContext {
            values: entries
                .iter()
                .map(|(key, value)| ((*key).to_owned(), (*value).to_owned()))
                .collect::<BTreeMap<_, _>>(),
            authorized: entries.iter().map(|(key, _)| *key).collect::<BTreeSet<_>>(),
            severity: Severity::Info,
        }
    }

    fn redactor() -> Redactor {
        Redactor::compile(&[]).unwrap()
    }

    fn permission_event() -> EventEnvelope {
        EventEnvelope {
            id: Uuid::from_u128(1),
            source: AgentKind::Codex,
            source_version: Version::parse("0.145.0").unwrap(),
            source_event: "PermissionRequest".into(),
            category: EventCategory::Permission,
            occurred_at: "2026-07-29T12:00:00Z".parse::<DateTime<Utc>>().unwrap(),
            received_at: "2026-07-29T12:00:01Z".parse::<DateTime<Utc>>().unwrap(),
            project_id: Some(Uuid::from_u128(2)),
            project_display_name: Some("cc-reminder".into()),
            unmatched_cwd_fingerprint: None,
            session_ref: None,
            turn_ref: None,
            model: Some("gpt-5".into()),
            permission_mode: Some("default".into()),
            severity: Severity::Warning,
            public_fields: BTreeMap::from([
                ("status".into(), ScalarValue::String("request".into())),
                ("tool_name".into(), ScalarValue::String("Bash".into())),
                (
                    "summary".into(),
                    ScalarValue::String("token=public-injection".into()),
                ),
                (
                    "full_prompt".into(),
                    ScalarValue::String("never expose this".into()),
                ),
            ]),
            encrypted_sensitive_fields: None,
            correlation_id: Uuid::from_u128(3),
            action_id: None,
            action_capabilities: Vec::new(),
        }
    }

    fn stop_event() -> EventEnvelope {
        let mut event = permission_event();
        event.source_event = "Stop".into();
        event.category = EventCategory::Completion;
        event.public_fields.clear();
        event
    }
}
