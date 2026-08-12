//! WeCom (企业微信) group-robot sender.
//!
//! Sends `markdown`; falls back to a single `text` only if the platform
//! explicitly rejects the markdown shape. No images, files, template cards or
//! mention lists in v1.
//!
//! Limits:
//! - markdown content: 4,096 UTF-8 bytes
//! - text content:     2,048 UTF-8 bytes

use chrono::Utc;
use secrecy::{ExposeSecret, SecretString};
use serde_json::{Value, json};

use crate::channels::http::{
    build_client, format_error, http_status_error, map_reqwest_error, read_capped,
    redact_error_message, retry_after_from_headers,
};
use crate::channels::{
    ChannelConfig, render_markdown, render_text, truncate_bytes, validate_official_webhook,
};
use crate::error::{ChannelSender, DeliveryError, DeliveryErrorKind, DeliveryReceipt};
use crate::model::{ChannelKind, NotificationDocument};

const WECOM_MARKDOWN_BYTE_LIMIT: usize = 4_096;
const WECOM_TEXT_BYTE_LIMIT: usize = 2_048;

pub struct WeComSender {
    endpoint: SecretString,
    client: reqwest::Client,
    config: ChannelConfig,
}

impl WeComSender {
    /// Production constructor. Always validates the webhook is the official
    /// WeCom endpoint; rejects anything else before any network call.
    pub fn new(webhook: SecretString) -> Result<Self, DeliveryError> {
        Self::with_config(webhook, ChannelConfig::default())
    }

    pub fn with_config(
        webhook: SecretString,
        config: ChannelConfig,
    ) -> Result<Self, DeliveryError> {
        validate_official_webhook(ChannelKind::WeCom, webhook.expose_secret())
            .map_err(|_| config_error("wecom.webhook.invalid"))?;
        Ok(Self {
            endpoint: webhook,
            client: build_client(config),
            config,
        })
    }

    /// Test-only constructor that bypasses host validation so the adapter can
    /// be pointed at the in-test mock server.
    #[cfg(any(test, feature = "test-support"))]
    pub fn for_contract_test(endpoint: String) -> Self {
        let config = ChannelConfig::default();
        let client = reqwest::Client::builder()
            .user_agent(crate::channels::http::USER_AGENT)
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(config.connect_timeout())
            .timeout(config.request_timeout())
            .build()
            .expect("test client builds");
        Self {
            endpoint: SecretString::from(endpoint),
            client,
            config,
        }
    }

    /// Send the fixed localized test-connection message. Real send, not a
    /// read-only probe: WeCom exposes no usable health check.
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

    fn markdown_body(&self, document: &NotificationDocument) -> Value {
        let content = truncate_bytes(&render_markdown(document), WECOM_MARKDOWN_BYTE_LIMIT);
        json!({ "msgtype": "markdown", "markdown": { "content": content } })
    }

    fn text_body(&self, document: &NotificationDocument) -> Value {
        let content = truncate_bytes(&render_text(document), WECOM_TEXT_BYTE_LIMIT);
        json!({ "msgtype": "text", "text": { "content": content } })
    }

    async fn send_one(
        &self,
        body: Value,
        attempt_markdown: bool,
    ) -> Result<DeliveryOutcome, DeliveryError> {
        let response = self
            .client
            .post(self.endpoint.expose_secret())
            .json(&body)
            .send()
            .await
            .map_err(map_reqwest_error)?;
        let status = response.status();
        // Capture Retry-After before the body is consumed.
        let retry_after = retry_after_from_headers(response.headers());
        let (read_status, payload) = read_capped(response, self.config.max_body_bytes()).await?;

        if !status.is_success() {
            return Err(http_status_error(read_status, retry_after));
        }

        let parsed: WeComResponse = serde_json::from_slice(&payload)
            .map_err(|_| format_error("malformed WeCom response", Some(read_status.as_u16())))?;

        if parsed.errcode == 0 {
            return Ok(DeliveryOutcome::Success(DeliveryReceipt {
                http_status: read_status.as_u16(),
                platform_code: Some(parsed.errcode.to_string()),
                sent_at: Utc::now(),
            }));
        }

        // 45033 = markdown unsupported / formatting rejected -> one text
        // fallback on the markdown attempt. Everything else maps straight to
        // a permanent or temporary code.
        if attempt_markdown && parsed.errcode == 45033 {
            return Ok(DeliveryOutcome::TryTextFallback);
        }

        Err(map_platform_error(
            parsed.errcode,
            &parsed.errmsg,
            read_status.as_u16(),
        ))
    }
}

#[async_trait::async_trait]
impl ChannelSender for WeComSender {
    async fn send(
        &self,
        document: &NotificationDocument,
    ) -> Result<DeliveryReceipt, DeliveryError> {
        match self.send_one(self.markdown_body(document), true).await? {
            DeliveryOutcome::Success(receipt) => Ok(receipt),
            DeliveryOutcome::TryTextFallback => {
                match self.send_one(self.text_body(document), false).await? {
                    DeliveryOutcome::Success(receipt) => Ok(receipt),
                    DeliveryOutcome::TryTextFallback => Err(DeliveryError {
                        kind: DeliveryErrorKind::Format,
                        code: "wecom.format".to_owned(),
                        redacted_message: "platform rejected both markdown and text".to_owned(),
                        http_status: None,
                        platform_code: Some("45033".to_owned()),
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
struct WeComResponse {
    #[serde(default)]
    errcode: i64,
    #[serde(default)]
    errmsg: String,
}

/// Map WeCom errcodes onto `DeliveryErrorKind`.
///
/// Reference (WeCom group robot):
/// - 40014: invalid key -> Authentication
/// - 48002: API forbidden (no permission) -> Permission
/// - 45009: API freq out of limit -> TemporaryPlatform
/// - 45033: markdown rejected -> Format (handled as fallback by caller)
/// - 41001 / signature class: Signature
fn map_platform_error(errcode: i64, errmsg: &str, http_status: u16) -> DeliveryError {
    let redacted = redact_error_message(errmsg);
    let platform_code = Some(errcode.to_string());
    match errcode {
        40014 | 41009 | 40001 => DeliveryError {
            kind: DeliveryErrorKind::Authentication,
            code: "wecom.authentication".to_owned(),
            redacted_message: redacted,
            http_status: Some(http_status),
            platform_code,
            retry_after_seconds: None,
        },
        48002 | 48011 => DeliveryError {
            kind: DeliveryErrorKind::Permission,
            code: "wecom.permission".to_owned(),
            redacted_message: redacted,
            http_status: Some(http_status),
            platform_code,
            retry_after_seconds: None,
        },
        41001 | 41005 | 40029 => DeliveryError {
            kind: DeliveryErrorKind::Signature,
            code: "wecom.signature".to_owned(),
            redacted_message: redacted,
            http_status: Some(http_status),
            platform_code,
            retry_after_seconds: None,
        },
        // ponytail: unknown codes default to TemporaryPlatform. Tighten if the
        // platform ever publishes a stable permanent-code list.
        _ => DeliveryError {
            kind: DeliveryErrorKind::TemporaryPlatform,
            code: "wecom.temporary".to_owned(),
            redacted_message: redacted,
            http_status: Some(http_status),
            platform_code,
            retry_after_seconds: None,
        },
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
