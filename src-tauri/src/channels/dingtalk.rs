//! DingTalk custom group-robot sender.
//!
//! Sends `markdown` first; on an *explicit* keyword/format rejection only,
//! sends exactly one `text` fallback within the same worker attempt. Never
//! `at.isAtAll: true`, no phone lists in v1.
//!
//! Signing (for "signed robot" mode):
//! `sign = urlencode(base64(hmac_sha256(secret, timestamp + "\n" + secret)))`,
//! sent as the `timestamp` + `sign` query parameters alongside `access_token`.

use std::time::{SystemTime, UNIX_EPOCH};

use base64::Engine as _;
use chrono::Utc;
use hmac::{Hmac, Mac};
use secrecy::{ExposeSecret, SecretString};
use serde_json::{Value, json};
use sha2::Sha256;
use url::Url;

use crate::channels::http::{
    build_client, format_error, http_status_error, map_reqwest_error, read_capped,
};
use crate::channels::{
    ChannelConfig, render_markdown, render_text, truncate_chars, validate_official_webhook,
};
use crate::error::{ChannelSender, DeliveryError, DeliveryErrorKind, DeliveryReceipt};
use crate::model::{ChannelKind, NotificationDocument};

type HmacSha256 = Hmac<Sha256>;

/// DingTalk enforces 20,000 chars on both text and markdown content.
const DINGTALK_CONTENT_CHAR_LIMIT: usize = 20_000;

/// DingTalk errcode returned for both keyword-not-in-content (plain robots)
/// and sign-mismatch (signed robots). The signing-secret presence and the
/// errmsg string disambiguate them at runtime.
const ERRCODE_KEYWORD_OR_FORMAT: i64 = 310_000;

/// Compute the DingTalk signed-robot signature for the given millisecond
/// timestamp and secret, returning a value ready to be used verbatim as the
/// `sign` query parameter (already percent-encoded).
///
/// Fixed vector (matches the DingTalk docs):
/// `timestamp = 1_609_459_200_000`, `secret = "SECtest"` -> the value asserted
/// in the contract test.
pub fn dingtalk_signature(timestamp_millis: i64, secret: &str) -> String {
    let string_to_sign = format!("{timestamp_millis}\n{secret}");
    let mut mac =
        HmacSha256::new_from_slice(secret.as_bytes()).expect("HMAC accepts arbitrary key length");
    mac.update(string_to_sign.as_bytes());
    let signature = mac.finalize().into_bytes();
    let b64 = base64::engine::general_purpose::STANDARD.encode(signature);
    url_encode_query_value(&b64)
}

/// Percent-encode a query value the way DingTalk's reference implementation
/// does: unreserved (`-._~` + alnum) passes through, every other byte is
/// percent-encoded. This makes `/` and `=` -> `%2F` / `%3D`, which is exactly
/// what the fixed vector asserts.
fn url_encode_query_value(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for &c in value.as_bytes() {
        match c {
            b'-' | b'.' | b'_' | b'~' => out.push(c as char),
            b'0'..=b'9' | b'A'..=b'Z' | b'a'..=b'z' => out.push(c as char),
            _ => out.push_str(&format!("%{c:02X}")),
        }
    }
    out
}

pub struct DingTalkSender {
    /// The fully-built webhook URL (with access_token; timestamp+sign are
    /// appended per-attempt for signed robots). Kept inside a SecretString so
    /// it never appears in logs or Debug output.
    endpoint: SecretString,
    signing_secret: Option<SecretString>,
    keyword_prefix: Option<String>,
    client: reqwest::Client,
    config: ChannelConfig,
}

impl DingTalkSender {
    /// Production constructor. Always validates that `webhook` is the official
    /// DingTalk endpoint; rejects anything else before any network call.
    pub fn new(
        webhook: SecretString,
        signing_secret: Option<SecretString>,
        keyword_prefix: Option<String>,
    ) -> Result<Self, DeliveryError> {
        Self::with_config(
            webhook,
            signing_secret,
            keyword_prefix,
            ChannelConfig::default(),
        )
    }

    pub fn with_config(
        webhook: SecretString,
        signing_secret: Option<SecretString>,
        keyword_prefix: Option<String>,
        config: ChannelConfig,
    ) -> Result<Self, DeliveryError> {
        validate_official_webhook(ChannelKind::DingTalk, webhook.expose_secret())
            .map_err(|_| config_error("dingtalk.webhook.invalid"))?;
        Ok(Self {
            endpoint: webhook,
            signing_secret,
            keyword_prefix,
            client: build_client(config),
            config,
        })
    }

    /// Test-only constructor that bypasses host validation so the adapter can
    /// be pointed at the in-test mock server.
    #[cfg(any(test, feature = "test-support"))]
    pub fn for_contract_test(
        endpoint: String,
        keyword_prefix: Option<String>,
        signing_secret: Option<String>,
    ) -> Self {
        let config = ChannelConfig::default();
        // The mock server speaks plain http on 127.0.0.1; we cannot use the
        // production https_only client against it, so build a test client that
        // keeps redirect policy `none` but allows http. Timeouts and the cap
        // still apply.
        let client = reqwest::Client::builder()
            .user_agent(crate::channels::http::USER_AGENT)
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(config.connect_timeout())
            .timeout(config.request_timeout())
            .build()
            .expect("test client builds");
        Self {
            endpoint: SecretString::from(endpoint),
            signing_secret: signing_secret.map(SecretString::from),
            keyword_prefix,
            client,
            config,
        }
    }

    /// Send the fixed localized test-connection message. This is a real send,
    /// not a read-only probe: neither platform exposes a usable one.
    pub async fn test_connection(&self) -> Result<DeliveryReceipt, DeliveryError> {
        let doc = NotificationDocument {
            title: "CC Reminder test".to_owned(),
            severity: crate::model::Severity::Info,
            facts: Vec::new(),
            body: "CC Reminder 测试消息 / CC Reminder test message".to_owned(),
            footer: None,
        };
        self.send(&doc).await
    }

    async fn send_one(
        &self,
        body: Value,
        attempt_markdown: bool,
    ) -> Result<DeliveryOutcome, DeliveryError> {
        let url = self.signed_request_url()?;
        let response = self
            .client
            .post(url)
            .json(&body)
            .send()
            .await
            .map_err(map_reqwest_error)?;
        let (status, payload) = read_capped(response, self.config.max_body_bytes()).await?;

        if !status.is_success() {
            return Err(http_status_error(status, None));
        }

        let parsed: DingResponse = serde_json::from_slice(&payload)
            .map_err(|_| format_error("malformed DingTalk response", Some(status.as_u16())))?;

        if parsed.errcode == 0 {
            return Ok(DeliveryOutcome::Success(DeliveryReceipt {
                http_status: status.as_u16(),
                platform_code: Some(parsed.errcode.to_string()),
                sent_at: Utc::now(),
            }));
        }

        // 310000 is overloaded: "keyword not in content" (plain robot) and
        // "sign not match" (signed robot). If signing is enabled classify it
        // as Signature; otherwise permit one text fallback on the first
        // (markdown) attempt.
        if parsed.errcode == ERRCODE_KEYWORD_OR_FORMAT {
            if self.signing_secret.is_some() {
                return Err(DeliveryError {
                    kind: DeliveryErrorKind::Signature,
                    code: "dingtalk.signature".to_owned(),
                    redacted_message: redact(&parsed.errmsg),
                    http_status: Some(status.as_u16()),
                    platform_code: Some(parsed.errcode.to_string()),
                    retry_after_seconds: None,
                });
            }
            if attempt_markdown {
                return Ok(DeliveryOutcome::TryTextFallback);
            }
        }

        Err(map_platform_error(
            parsed.errcode,
            &parsed.errmsg,
            status.as_u16(),
        ))
    }

    /// Build the request URL for this attempt: the configured endpoint, plus a
    /// fresh `timestamp` + `sign` pair for signed robots. The signing secret is
    /// read and dropped inside this function.
    fn signed_request_url(&self) -> Result<Url, DeliveryError> {
        let mut url = Url::parse(self.endpoint.expose_secret())
            .map_err(|_| config_error("dingtalk.webhook.malformed"))?;
        if let Some(secret) = &self.signing_secret {
            // Preserve all existing pairs (typically just access_token), drop
            // any stale timestamp/sign from a previous attempt, then append a
            // fresh signature.
            let preserved: Vec<(String, String)> = url
                .query_pairs()
                .filter(|(k, _)| k != "timestamp" && k != "sign")
                .map(|(k, v)| (k.into_owned(), v.into_owned()))
                .collect();
            let (timestamp, sign) = current_signature(secret.expose_secret());
            url.query_pairs_mut().clear();
            for (k, v) in &preserved {
                url.query_pairs_mut().append_pair(k, v);
            }
            url.query_pairs_mut()
                .append_pair("timestamp", &timestamp.to_string())
                .append_pair("sign", &sign);
        }
        Ok(url)
    }

    fn prefixed_markdown(&self, document: &NotificationDocument) -> Value {
        let mut md = render_markdown(document);
        if let Some(prefix) = &self.keyword_prefix {
            md = format!("{prefix} {md}");
        }
        let text = truncate_chars(&md, DINGTALK_CONTENT_CHAR_LIMIT);
        json!({
            "msgtype": "markdown",
            "markdown": { "title": &document.title, "text": text },
        })
    }

    fn prefixed_text(&self, document: &NotificationDocument) -> Value {
        let mut txt = render_text(document);
        if let Some(prefix) = &self.keyword_prefix {
            txt = format!("{prefix} {txt}");
        }
        let content = truncate_chars(&txt, DINGTALK_CONTENT_CHAR_LIMIT);
        json!({
            "msgtype": "text",
            "text": { "content": content },
        })
    }
}

#[async_trait::async_trait]
impl ChannelSender for DingTalkSender {
    async fn send(
        &self,
        document: &NotificationDocument,
    ) -> Result<DeliveryReceipt, DeliveryError> {
        let markdown_body = self.prefixed_markdown(document);
        match self.send_one(markdown_body, true).await? {
            DeliveryOutcome::Success(receipt) => Ok(receipt),
            DeliveryOutcome::TryTextFallback => {
                let text_body = self.prefixed_text(document);
                match self.send_one(text_body, false).await? {
                    DeliveryOutcome::Success(receipt) => Ok(receipt),
                    // Text also rejected -> permanent Format error to the worker.
                    DeliveryOutcome::TryTextFallback => Err(DeliveryError {
                        kind: DeliveryErrorKind::Format,
                        code: "dingtalk.format".to_owned(),
                        redacted_message: "platform rejected both markdown and text".to_owned(),
                        http_status: None,
                        platform_code: Some(ERRCODE_KEYWORD_OR_FORMAT.to_string()),
                        retry_after_seconds: None,
                    }),
                }
            }
        }
    }
}

enum DeliveryOutcome {
    Success(DeliveryReceipt),
    TryTextFallback,
}

#[derive(serde::Deserialize)]
struct DingResponse {
    #[serde(default)]
    errcode: i64,
    #[serde(default)]
    errmsg: String,
}

fn current_signature(secret: &str) -> (i64, String) {
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0);
    let sign = dingtalk_signature(timestamp, secret);
    (timestamp, sign)
}

/// Map a non-zero DingTalk errcode onto the worker-facing `DeliveryErrorKind`.
///
/// Reference errcodes (DingTalk group robot):
/// - 300000 / 300001 / 300002: access_token invalid / expired -> Authentication
/// - 310000: keyword / sign -- handled by the caller (Signature or fallback)
/// - 400101: keyword not in content (also format) -> Format
/// - otherwise -> TemporaryPlatform
fn map_platform_error(errcode: i64, errmsg: &str, http_status: u16) -> DeliveryError {
    let redacted = redact(errmsg);
    let platform_code = Some(errcode.to_string());
    match errcode {
        300_000..=300_002 => DeliveryError {
            kind: DeliveryErrorKind::Authentication,
            code: "dingtalk.authentication".to_owned(),
            redacted_message: redacted,
            http_status: Some(http_status),
            platform_code,
            retry_after_seconds: None,
        },
        400_101 => DeliveryError {
            kind: DeliveryErrorKind::Format,
            code: "dingtalk.format".to_owned(),
            redacted_message: redacted,
            http_status: Some(http_status),
            platform_code,
            retry_after_seconds: None,
        },
        // ponytail: unknown codes default to TemporaryPlatform. Tighten to a
        // denylist of permanent codes if DingTalk ever publishes more.
        _ => DeliveryError {
            kind: DeliveryErrorKind::TemporaryPlatform,
            code: "dingtalk.temporary".to_owned(),
            redacted_message: redacted,
            http_status: Some(http_status),
            platform_code,
            retry_after_seconds: None,
        },
    }
}

/// Strip the platform free-text message down to a short, safe summary so the
/// redacted_message field is safe to persist and log.
fn redact(errmsg: &str) -> String {
    let trimmed = errmsg.trim();
    if trimmed.len() > 120 {
        trimmed[..120].to_owned()
    } else {
        trimmed.to_owned()
    }
}

fn config_error(code: &str) -> DeliveryError {
    DeliveryError {
        kind: DeliveryErrorKind::Format,
        code: code.to_owned(),
        redacted_message: "channel configuration is invalid".to_owned(),
        http_status: None,
        platform_code: None,
        retry_after_seconds: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fixed_signature_vector() {
        assert_eq!(
            dingtalk_signature(1_609_459_200_000, "SECtest"),
            "p5mXVLdX%2FBTrc2KtuhTs6ZcGOXtsKU5g1oE3WtfH4hY%3D"
        );
    }
}
