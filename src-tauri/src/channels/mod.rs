//! Platform channel adapters (Task 13).
//!
//! This module renders [`NotificationDocument`] into a conservative Markdown
//! subset and ships it to the official DingTalk and WeCom group-robot webhooks
//! through one hardened [`reqwest::Client`]. The document model stays
//! platform-independent; escaping, length limits, signing and fallback all live
//! here.
//!
//! Secret hygiene: webhook URLs and signing secrets are kept in
//! [`secrecy::SecretString`] for their whole lifetime and only exposed at the
//! moment they are read into a `reqwest` request. Logs only ever carry the
//! HTTP method, the *official* host, the status code, the elapsed time and a
//! redacted platform code -- never the URL query, signature, request body or
//! full response.

pub mod dingtalk;
pub mod http;
pub mod wecom;

pub use dingtalk::{DingTalkSender, dingtalk_signature};
pub use wecom::WeComSender;

use std::time::Duration;

use url::Url;

use crate::model::ChannelKind;

/// Tunables for the shared HTTP client. Defaults match the design spec:
/// 5s connect timeout, 10s request timeout, 64 KiB response body cap.
#[derive(Clone, Copy, Debug)]
pub struct ChannelConfig {
    connect_timeout: Duration,
    request_timeout: Duration,
    max_body_bytes: usize,
}

impl ChannelConfig {
    pub const fn default_const() -> Self {
        Self {
            connect_timeout: Duration::from_secs(5),
            request_timeout: Duration::from_secs(10),
            max_body_bytes: 64 * 1024,
        }
    }

    pub const fn connect_timeout(self) -> Duration {
        self.connect_timeout
    }

    pub const fn request_timeout(self) -> Duration {
        self.request_timeout
    }

    pub const fn max_body_bytes(self) -> usize {
        self.max_body_bytes
    }
}

impl Default for ChannelConfig {
    fn default() -> Self {
        Self::default_const()
    }
}

/// Accept only the exact official webhook endpoints.
///
/// DingTalk: `https://oapi.dingtalk.com/robot/send?access_token=<non-empty>`
/// WeCom:    `https://qyapi.weixin.qq.com/cgi-bin/webhook/send?key=<non-empty>`
///
/// Everything else -- http, wrong host, lookalike/punycode hosts, subpaths,
/// extra credential query names, userinfo, fragments, non-default ports,
/// IP literals, localhost -- is rejected. The credential value itself is not
/// inspected beyond "non-empty"; that is the platform's job.
pub fn validate_official_webhook(
    kind: ChannelKind,
    raw: &str,
) -> Result<(), WebhookValidationError> {
    // Reject an explicit port in the authority BEFORE the url crate normalizes
    // `:443` away. The canonical webhook must never carry one, even the default.
    if has_explicit_authority_port(raw) {
        return Err(WebhookValidationError::ExplicitPort);
    }

    let url = Url::parse(raw).map_err(|_| WebhookValidationError::Malformed)?;

    if url.scheme() != "https" {
        return Err(WebhookValidationError::NotHttps);
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err(WebhookValidationError::UserInfoPresent);
    }
    if url.port().is_some() {
        // The official endpoints are on the default port. Reject any explicit port.
        return Err(WebhookValidationError::ExplicitPort);
    }
    if url.fragment().is_some() {
        return Err(WebhookValidationError::FragmentPresent);
    }
    // Reject IP literals and localhost regardless of host spelling.
    let host = url.host_str().ok_or(WebhookValidationError::WrongHost)?;
    if host_is_ip_literal(host) || host == "localhost" {
        return Err(WebhookValidationError::IpLiteral);
    }

    let (expected_host, expected_path, credential_name) = match kind {
        ChannelKind::DingTalk => ("oapi.dingtalk.com", "/robot/send", "access_token"),
        ChannelKind::WeCom => ("qyapi.weixin.qq.com", "/cgi-bin/webhook/send", "key"),
    };

    // Exact host match (URL host is already ASCII / punycode-decoded for IDN).
    if host != expected_host {
        return Err(WebhookValidationError::WrongHost);
    }
    // Exact path match, no trailing slash, no subpaths.
    if url.path() != expected_path {
        return Err(WebhookValidationError::WrongPath);
    }

    // Exactly one query pair, named as expected, with a non-empty value.
    let mut query_pairs = url.query_pairs();
    let only = query_pairs.next();
    let extra = query_pairs.next();
    match (only, extra) {
        (Some((name, value)), None) if name == credential_name && !value.is_empty() => Ok(()),
        _ => Err(WebhookValidationError::CredentialMissing),
    }
}

fn host_is_ip_literal(host: &str) -> bool {
    // url crate already strips brackets for IPv6 in host_str? No -- it keeps the
    // bare IP for both v4 and v6. We detect either family defensively.
    if host.starts_with('[') && host.ends_with(']') {
        return true;
    }
    host.parse::<std::net::IpAddr>().is_ok()
}

/// Detect an explicit port in the authority component of a raw URL string.
///
/// We need this because the `url` crate normalizes `:443` away for https, so a
/// post-parse check cannot distinguish `host:443` from `host`. The canonical
/// webhook never carries an explicit port, so any `:` after the host (inside
/// the authority, before the first `/`/`?`/`#`) is rejected.
fn has_explicit_authority_port(raw: &str) -> bool {
    let after_scheme = match raw.split_once("://") {
        Some((_, rest)) => rest,
        None => raw,
    };
    let authority_end = after_scheme
        .find(['/', '?', '#'])
        .unwrap_or(after_scheme.len());
    let authority = &after_scheme[..authority_end];
    // Strip userinfo (user:pass@) before looking for the port colon.
    let host_port = authority
        .rsplit_once('@')
        .map(|(_, hp)| hp)
        .unwrap_or(authority);
    // An IPv6 literal in the authority is written as `[::1]:port`; the `:` inside
    // brackets is not a port separator.
    if host_port.starts_with('[')
        && let Some(close) = host_port.find(']')
    {
        return host_port[close + 1..].contains(':');
    }
    host_port.contains(':')
}

/// Why a webhook URL was rejected. Debug-only; never carries the URL.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum WebhookValidationError {
    #[error("webhook url is malformed")]
    Malformed,
    #[error("webhook must use https")]
    NotHttps,
    #[error("webhook host is not an official platform host")]
    WrongHost,
    #[error("webhook path does not match the official endpoint")]
    WrongPath,
    #[error("webhook is missing its credential query parameter")]
    CredentialMissing,
    #[error("webhook must not carry userinfo")]
    UserInfoPresent,
    #[error("webhook must not carry a fragment")]
    FragmentPresent,
    #[error("webhook must not specify an explicit port")]
    ExplicitPort,
    #[error("webhook host must not be an IP literal or localhost")]
    IpLiteral,
}

// -----------------------------------------------------------------------------
// Rendering
// -----------------------------------------------------------------------------

/// Render the document into a conservative Markdown subset.
///
/// The output uses headings, plain key/value facts, the body and an optional
/// footer. Platform-reserved characters are escaped so the rendered string is
/// inert text when interpreted by either platform. Truncation happens later at
/// the platform-specific byte/char cap (see [`truncate_chars`] and
/// [`truncate_bytes`]); this function does not truncate.
pub fn render_markdown(document: &crate::model::NotificationDocument) -> String {
    let mut out = String::with_capacity(document.body.len() + 64);
    let mut needs_blank = false;

    if !document.title.is_empty() {
        out.push_str("# ");
        out.push_str(&escape_markdown(&document.title));
        out.push('\n');
        needs_blank = true;
    }

    if !document.facts.is_empty() {
        if needs_blank {
            out.push('\n');
        }
        for (key, value) in &document.facts {
            out.push_str("- **");
            out.push_str(&escape_markdown(key));
            out.push_str("**: ");
            out.push_str(&escape_markdown(value));
            out.push('\n');
        }
        needs_blank = true;
    }

    if needs_blank {
        out.push('\n');
    }
    // DingTalk/WeCom markdown collapse a single "\n" into a space; body lines
    // must become paragraph breaks or a multi-line template renders as one
    // flowing line (field feedback: "raw text" look).
    out.push_str(&escape_markdown(&document.body).replace('\n', "\n\n"));

    if let Some(footer) = &document.footer
        && !footer.is_empty()
    {
        out.push_str("\n\n---\n\n");
        out.push_str(&escape_markdown(footer));
    }

    out
}

/// Render the document as a single plain-text blob (no Markdown syntax at all).
pub fn render_text(document: &crate::model::NotificationDocument) -> String {
    let mut out = String::with_capacity(document.body.len() + 64);
    out.push_str(&document.title);
    out.push('\n');
    out.push_str(severity_label(document.severity));
    if !document.facts.is_empty() {
        out.push('\n');
        for (key, value) in &document.facts {
            out.push_str(key);
            out.push_str(": ");
            out.push_str(value);
            out.push('\n');
        }
    }
    out.push('\n');
    out.push_str(&document.body);
    if let Some(footer) = &document.footer {
        out.push('\n');
        out.push_str(footer);
    }
    out
}

fn severity_label(severity: crate::model::Severity) -> &'static str {
    use crate::model::Severity::*;
    match severity {
        Info => "INFO",
        Warning => "WARNING",
        Error => "ERROR",
        Critical => "CRITICAL",
    }
}

/// Escape the characters that have meaning in Markdown or that DingTalk/WeCom
/// interpret specially. Conservative: escape the common emphasis/heading/list
/// punctuation and the backslash itself.
fn escape_markdown(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    for ch in input.chars() {
        match ch {
            '\\' | '`' | '*' | '_' | '#' | '[' | ']' | '<' | '>' | '!' | '(' | ')' | '|' => {
                out.push('\\');
                out.push(ch);
            }
            other => out.push(other),
        }
    }
    out
}

/// Truncate to at most `limit` Unicode scalar values.
pub fn truncate_chars(input: &str, limit: usize) -> String {
    if input.chars().count() <= limit {
        return input.to_owned();
    }
    input.chars().take(limit).collect()
}

/// Truncate to at most `limit` UTF-8 bytes without splitting a code point.
pub fn truncate_bytes(input: &str, limit: usize) -> String {
    if input.len() <= limit {
        return input.to_owned();
    }
    let mut end = limit;
    while end > 0 && !input.is_char_boundary(end) {
        end -= 1;
    }
    input[..end].to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{NotificationDocument, Severity};

    #[test]
    fn markdown_renders_facts_and_footer() {
        let doc = NotificationDocument {
            title: "Build".to_owned(),
            severity: Severity::Warning,
            facts: vec![("Branch".to_owned(), "main".to_owned())],
            body: "ship it".to_owned(),
            footer: Some("CC Reminder".to_owned()),
        };
        let md = render_markdown(&doc);
        assert!(md.starts_with("# Build"));
        assert!(md.contains("- **Branch**: main"));
        assert!(md.contains("ship it"));
        assert!(md.contains("---"));
    }

    #[test]
    fn truncate_bytes_keeps_char_boundary() {
        let s = "中".repeat(10); // 3 bytes each, 30 bytes
        let t = truncate_bytes(&s, 8);
        assert_eq!(t.len(), 6);
        assert_eq!(t, "中中");
    }

    #[test]
    fn truncate_chars_counts_scalars() {
        let s = "ab中";
        assert_eq!(truncate_chars(s, 2), "ab");
        assert_eq!(truncate_chars(s, 3), "ab中");
    }
}
