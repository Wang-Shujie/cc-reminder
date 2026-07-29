use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::model::NotificationDocument;

#[derive(Clone, Debug, Serialize, Deserialize, thiserror::Error)]
#[error("{domain:?}/{code}: {message}")]
pub struct AppError {
    pub domain: ErrorDomain,
    pub code: String,
    pub message: String,
    pub suggested_action: Option<String>,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ErrorDomain {
    Integration,
    Configuration,
    SecretStore,
    Delivery,
    Storage,
    Update,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct DeliveryReceipt {
    pub http_status: u16,
    pub platform_code: Option<String>,
    pub sent_at: DateTime<Utc>,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DeliveryErrorKind {
    Network,
    Timeout,
    HttpStatus,
    TemporaryPlatform,
    Authentication,
    Signature,
    Permission,
    Format,
}

#[derive(Clone, Debug, Serialize, Deserialize, thiserror::Error)]
#[error("{kind:?}/{code}: {redacted_message}")]
pub struct DeliveryError {
    pub kind: DeliveryErrorKind,
    pub code: String,
    pub redacted_message: String,
    pub http_status: Option<u16>,
    pub platform_code: Option<String>,
    pub retry_after_seconds: Option<u64>,
}

#[async_trait::async_trait]
pub trait ChannelSender: Send + Sync {
    async fn send(&self, document: &NotificationDocument)
    -> Result<DeliveryReceipt, DeliveryError>;
}
