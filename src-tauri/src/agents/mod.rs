mod claude;
mod codex;
mod detect;

use semver::Version;

use crate::error::AppError;
use crate::events::catalog::CapabilityResolution;
use crate::model::{AgentInstallationRecord, InstallationHealth};
use crate::storage::integrations::IntegrationRepository;

pub use claude::ClaudeIntegration;
pub use codex::CodexIntegration;
pub use detect::{AgentVersionCache, Detection, DetectionState};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AgentHealthUpdate {
    pub agent: crate::model::AgentKind,
    pub health: InstallationHealth,
}

pub fn persist_detection(
    repository: &IntegrationRepository,
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
    fn install_hooks(&self, selection: &HookSelection) -> Result<Installation, AppError>;
    fn inspect_hooks(&self) -> Result<HookHealth, AppError>;
}

#[derive(Clone, Debug, Default)]
pub struct HookSelection;

#[derive(Clone, Debug, Default)]
pub struct Installation;

#[derive(Clone, Debug, Default)]
pub struct HookHealth;
