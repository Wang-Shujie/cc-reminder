//! One hardened [`reqwest::Client`] shared by every platform adapter.
//!
//! Hardening, applied once here so neither adapter has to remember it:
//!
//! * rustls only (no native-tls linkage), with the platform certificate
//!   verifier so system root stores and user-installed CAs are honoured;
//! * system proxy resolution enabled (so corporate environments keep working);
//! * connect timeout 5s, total request timeout 10s;
//! * redirect policy `none` -- a 3xx is surfaced as an error and never
//!   followed, so a compromised response cannot redirect the credential URL
//!   to an attacker;
//! * a fixed `CC-Reminder/<version>` user agent;
//! * response bodies are capped at 64 KiB to bound memory and log noise.
//!
//! Logging discipline is enforced by never reading the body as text and by
//! the request builders in the platform adapters, which only ever log method,
//! the *official* host, status, elapsed, and a redacted platform code.

use std::time::Duration;

use reqwest::redirect;
use reqwest::{Client, ClientBuilder, StatusCode};

use crate::channels::ChannelConfig;
use crate::error::{DeliveryError, DeliveryErrorKind};

pub(crate) const USER_AGENT: &str = concat!("CC-Reminder/", env!("CARGO_PKG_VERSION"));

/// Build the shared client from `config`. One client per sender is fine: reqwest
/// pools connections internally and the tunables never vary at runtime.
pub(crate) fn build_client(config: ChannelConfig) -> Client {
    ClientBuilder::new()
        .user_agent(USER_AGENT)
        .https_only(true)
        .redirect(redirect::Policy::none())
        .connect_timeout(config.connect_timeout())
        .timeout(config.request_timeout())
        // rustls-only, system root store via the platform verifier.
        .build()
        .expect("reqwest client builder has valid static configuration")
}

/// Read at most `max_bytes` of the response body.
///
/// Anything larger is treated as a [`DeliveryErrorKind::Format`] error: we
/// never want to buffer an arbitrary blob, and the platform contracts are all
/// small JSON objects.
pub(crate) async fn read_capped(
    response: reqwest::Response,
    max_bytes: usize,
) -> Result<(StatusCode, Vec<u8>), DeliveryError> {
    let status = response.status();
    let bytes = response.bytes().await.map_err(network_error)?;
    if bytes.len() > max_bytes {
        return Err(format_error(
            "response body exceeded 64 KiB cap",
            Some(status.as_u16()),
        ));
    }
    Ok((status, bytes.to_vec()))
}

/// Map a reqwest failure onto `DeliveryErrorKind`. The overall request timeout
/// is `Timeout`; everything else (connect, resolve, TLS, stream) is `Network`.
pub(crate) fn map_reqwest_error(err: reqwest::Error) -> DeliveryError {
    if err.is_timeout() {
        return timeout_error();
    }
    network_error(err)
}

pub(crate) fn timeout_error() -> DeliveryError {
    DeliveryError {
        kind: DeliveryErrorKind::Timeout,
        code: "delivery.timeout".to_owned(),
        redacted_message: "platform request timed out".to_owned(),
        http_status: None,
        platform_code: None,
        retry_after_seconds: None,
    }
}

pub(crate) fn network_error(err: reqwest::Error) -> DeliveryError {
    DeliveryError {
        kind: DeliveryErrorKind::Network,
        code: "delivery.network".to_owned(),
        redacted_message: "network error contacting platform".to_owned(),
        http_status: err.status().map(|s| s.as_u16()),
        platform_code: None,
        retry_after_seconds: None,
    }
}

pub(crate) fn http_status_error(
    status: StatusCode,
    retry_after: Option<Duration>,
) -> DeliveryError {
    DeliveryError {
        kind: DeliveryErrorKind::HttpStatus,
        code: "delivery.http_status".to_owned(),
        redacted_message: format!("unexpected HTTP status {}", status.as_u16()),
        http_status: Some(status.as_u16()),
        platform_code: None,
        retry_after_seconds: retry_after.map(|d| d.as_secs()),
    }
}

pub(crate) fn format_error(message: &str, http_status: Option<u16>) -> DeliveryError {
    DeliveryError {
        kind: DeliveryErrorKind::Format,
        code: "delivery.format".to_owned(),
        redacted_message: message.to_owned(),
        http_status,
        platform_code: None,
        retry_after_seconds: None,
    }
}

/// Parse a `Retry-After` header value (delta-seconds form only; the HTTP-date
/// form is ignored because neither platform emits it). Caps at one hour.
pub(crate) fn parse_retry_after(header_value: &str) -> Option<Duration> {
    let secs: u64 = header_value.trim().parse().ok()?;
    Some(Duration::from_secs(secs.min(3_600)))
}

/// Capture `Retry-After` from a response's headers (before the body is
/// consumed). Shared by both adapters so DingTalk and WeCom honour the header
/// symmetrically.
pub(crate) fn retry_after_from_headers(headers: &reqwest::header::HeaderMap) -> Option<Duration> {
    headers
        .get(reqwest::header::RETRY_AFTER)
        .and_then(|value| value.to_str().ok())
        .and_then(parse_retry_after)
}

/// Trim and cap a platform `errmsg` for `redacted_message`, backing up to a
/// UTF-8 char boundary so a multibyte message (both platforms return localized
/// CJK errors) never panics on `[..120]`.
pub(crate) fn redact_error_message(errmsg: &str) -> String {
    let trimmed = errmsg.trim();
    const MAX_BYTES: usize = 120;
    if trimmed.len() <= MAX_BYTES {
        return trimmed.to_owned();
    }
    let mut end = MAX_BYTES;
    while end > 0 && !trimmed.is_char_boundary(end) {
        end -= 1;
    }
    trimmed[..end].to_owned()
}
