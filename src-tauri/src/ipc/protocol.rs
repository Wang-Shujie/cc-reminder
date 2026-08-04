use crate::events::normalize::CapturedHookEvent;
use serde::{Deserialize, Serialize};
use std::time::Duration;
use uuid::Uuid;

pub const IPC_PROTOCOL_VERSION: u16 = 1;
pub const MAX_HOOK_BYTES: usize = 1_048_576;
pub const MAX_JSON_DEPTH: usize = 32;
pub const MAX_JSON_FIELDS: usize = 256;
pub const MAX_JSON_NODES: usize = 4_096;
pub const MAX_SAFE_ENVELOPE_BYTES: usize = 65_536;
pub const MAX_SPOOL_FILES: usize = 4_096;
pub const IPC_CONNECT_TIMEOUT: Duration = Duration::from_millis(35);
pub const IPC_TOTAL_TIMEOUT: Duration = Duration::from_millis(75);

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct IngressRequest {
    pub protocol_version: u16,
    pub helper_version: String,
    pub command_fingerprint: String,
    pub event: CapturedHookEvent,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum IngressResponse {
    Accepted { event_id: Uuid },
    Rejected { error_code: String },
}
