mod claude;
mod codex;
mod detect;
pub mod selfheal;

use std::collections::BTreeSet;
use std::path::PathBuf;

use semver::Version;
use serde::{Deserialize, Serialize};

use crate::error::AppError;
use crate::events::catalog::CapabilityResolution;
use crate::installer::lifecycle::HookEnvironment;
use crate::model::{AgentInstallationRecord, AgentKind, InstallationHealth, TrustStatus};

pub use claude::ClaudeIntegration;
pub use codex::CodexIntegration;
pub use detect::{AgentVersionCache, Detection, DetectionState, detect_agent};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AgentHealthUpdate {
    pub agent: AgentKind,
    pub health: InstallationHealth,
}

pub fn persist_detection(
    repository: &crate::storage::integrations::IntegrationRepository,
    detection: &Detection,
) -> Result<AgentHealthUpdate, AppError> {
    let health = match detection.state {
        DetectionState::Detected => InstallationHealth::Healthy,
        DetectionState::Missing => InstallationHealth::NeedsRepair,
        DetectionState::InvalidVersion
        | DetectionState::ProcessFailed
        | DetectionState::TimedOut => InstallationHealth::Error,
    };
    repository.upsert_agent(&AgentInstallationRecord {
        agent: detection.agent,
        executable_path: detection.executable_path.clone(),
        version: detection.version.clone(),
        capability_verification: detection
            .capability_verification
            .unwrap_or(crate::events::catalog::CatalogVerification::UpgradeRequired),
        health_status: health,
        last_checked_at: detection.checked_at,
    })?;
    Ok(AgentHealthUpdate {
        agent: detection.agent,
        health,
    })
}

pub trait AgentIntegration {
    fn detect(&self) -> Detection;
    fn capabilities(&self, version: &Version) -> CapabilityResolution;
    /// Install/refresh the owned Hook entries for `selection`. No implementation
    /// accepts a caller-supplied Agent config path — the integration's fixed
    /// user-level path is always used.
    fn install_hooks(
        &self,
        env: &HookEnvironment,
        selection: &HookSelection,
    ) -> Result<Installation, AppError>;
    fn inspect_hooks(
        &self,
        env: &HookEnvironment,
        selection: &HookSelection,
    ) -> Result<HookHealth, AppError>;
}

/// Desired Hook installation: the events to own, plus the signed helper path
/// and version those entries must point at. The helper path/version come from
/// [`crate::installer::helper::HelperInstaller`]; rule-save commands never
/// construct a `HookSelection` implicitly (design 8.4, 9.4).
#[derive(Clone, Debug)]
pub struct HookSelection {
    pub events: BTreeSet<String>,
    pub helper_path: PathBuf,
    pub helper_version: Version,
}

impl HookSelection {
    pub fn events(&self) -> &BTreeSet<String> {
        &self.events
    }
}

/// Result of an Install/Repair/UpgradeHelper/Uninstall transaction.
#[derive(Clone, Debug)]
pub struct Installation {
    pub agent: AgentKind,
    pub records: Vec<crate::model::HookInstallationRecord>,
}

/// Per-entry + aggregate health of the owned Hook installation. `inspect` is
/// read-only (design 9.4).
#[derive(Clone, Debug)]
pub struct HookHealth {
    pub agent: AgentKind,
    pub entries: Vec<HookEntryHealth>,
    pub aggregate: HealthAggregate,
    pub selection_out_of_date: bool,
}

#[derive(Clone, Debug)]
pub struct HookEntryHealth {
    pub source_event: String,
    pub command_fingerprint: String,
    pub definition_fingerprint: String,
    pub trust_status: TrustStatus,
    pub health: EntryHealth,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EntryHealth {
    #[default]
    Healthy,
    Missing,
    Drifted,
    HelperMismatch,
    NeedsTrust,
    AgentUpgradeRequired,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HealthAggregate {
    #[default]
    Unknown,
    Healthy,
    NeedsRepair,
    Error,
}

/// v2-issues(计划行 2482):后台周期性重检 Agent(默认 6 小时)。升级/
/// 卸载等外部变化由此反映进健康状态:结果落库并广播 board-wide
/// health-changed。形态复用 retention ticker——固定间隔 + 取消令牌,
/// 失败仅记日志不打断循环;首个立即 tick 被跳过(启动检测由前端触发)。
pub async fn redetect_loop(
    integrations: crate::storage::integrations::IntegrationRepository,
    diagnostics: std::sync::Arc<crate::diagnostics::Diagnostics>,
    events: crate::worker::CoreEventSink,
    period: std::time::Duration,
    cancel: crate::worker::CancellationToken,
) {
    let mut interval = tokio::time::interval(period);
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    interval.tick().await;
    loop {
        tokio::select! {
            _ = interval.tick() => {
                // detect_agent 阻塞式 spawn CLI 并等待——放 blocking 池,
                // 不占 tokio worker。
                let task_integrations = integrations.clone();
                let task_diagnostics = diagnostics.clone();
                let _ = tokio::task::spawn_blocking(move || {
                    for agent in [crate::model::AgentKind::ClaudeCode, crate::model::AgentKind::Codex] {
                        let detection = crate::agents::detect::detect_agent(agent, None);
                        if let Err(err) = persist_detection(&task_integrations, &detection) {
                            task_diagnostics.info(
                                "agents",
                                &format!("periodic re-detection persist failed: {err}"),
                            );
                        }
                    }
                })
                .await;
                crate::worker::emit(
                    &events,
                    crate::worker::CoreEvent::HealthChanged { channel_id: None },
                );
                diagnostics.info("agents", "periodic re-detection pass completed");
            }
            _ = cancel.cancelled() => return,
        }
    }
}
