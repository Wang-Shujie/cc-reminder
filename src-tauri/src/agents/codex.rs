#[cfg(test)]
mod tests {
    use chrono::Utc;
    use semver::Version;

    use super::CodexIntegration;
    use crate::agents::{Detection, DetectionState};
    use crate::events::catalog::CatalogVerification;
    use crate::model::AgentKind;

    #[test]
    fn unknown_major_is_visible_but_install_is_blocked() {
        let integration = CodexIntegration::with_detection(detected("9.0.0"));
        let capability = integration.capabilities(&Version::new(9, 0, 0));
        assert_eq!(
            capability.verification,
            CatalogVerification::UpgradeRequired
        );
        assert_eq!(
            integration
                .validate_install_version(false)
                .unwrap_err()
                .code,
            "integration.agent_upgrade_required"
        );
    }

    #[test]
    fn codex_hooks_path_uses_only_codex_home_or_default_user_path() {
        let integration = CodexIntegration::with_detection(detected("0.145.0"));
        assert_eq!(
            integration.hooks_path(
                Some(std::path::Path::new("/configured/codex")),
                std::path::Path::new("/home/test")
            ),
            std::path::PathBuf::from("/configured/codex/hooks.json")
        );
        assert_eq!(
            integration.hooks_path(None, std::path::Path::new("/home/test")),
            std::path::PathBuf::from("/home/test/.codex/hooks.json")
        );
    }

    #[test]
    fn redetection_updates_installation_without_creating_a_config_snapshot() {
        let root = tempfile::tempdir().unwrap();
        let database = crate::storage::db::Database::open(
            &root.path().join("com.ccreminder.app/cc-reminder.sqlite3"),
        )
        .unwrap();
        let repository = crate::storage::integrations::IntegrationRepository::new(database);
        let integration = CodexIntegration::with_detection(detected("0.145.0"));

        let update = integration.redetect(&repository).unwrap();
        let stored = repository.agent(AgentKind::Codex).unwrap();

        assert_eq!(update.health, crate::model::InstallationHealth::Healthy);
        assert_eq!(stored.version, Some(Version::new(0, 145, 0)));
        assert_eq!(stored.executable_path, Some("/bin/codex".into()));
        assert_eq!(repository.snapshot_count(AgentKind::Codex).unwrap(), 0);
    }

    fn detected(version: &str) -> Detection {
        let version = Version::parse(version).unwrap();
        Detection {
            agent: AgentKind::Codex,
            executable_path: Some("/bin/codex".into()),
            capability_verification: Some(
                crate::events::catalog::catalog_for(AgentKind::Codex, &version).verification,
            ),
            version: Some(version),
            state: DetectionState::Detected,
            checked_at: Utc::now(),
        }
    }
}
use std::path::{Path, PathBuf};

use semver::Version;

use crate::agents::{
    AgentHealthUpdate, Detection, HookHealth, HookSelection, Installation, persist_detection,
};
use crate::error::AppError;
use crate::events::catalog::{CapabilityResolution, catalog_for};
use crate::installer::lifecycle::{HookAction, HookEnvironment, HookInstaller};
use crate::model::AgentKind;
use crate::storage::integrations::IntegrationRepository;

#[derive(Clone, Debug)]
pub struct CodexIntegration {
    configured_path: Option<PathBuf>,
    detection: Option<Detection>,
}

impl CodexIntegration {
    pub fn new(configured_path: Option<PathBuf>) -> Self {
        Self {
            configured_path,
            detection: None,
        }
    }

    pub fn with_detection(detection: Detection) -> Self {
        Self {
            configured_path: None,
            detection: Some(detection),
        }
    }

    pub fn detect(&self) -> Detection {
        self.detection.clone().unwrap_or_else(|| {
            crate::agents::detect::detect_agent(AgentKind::Codex, self.configured_path.as_deref())
        })
    }

    pub fn capabilities(&self, version: &Version) -> CapabilityResolution {
        catalog_for(AgentKind::Codex, version)
    }

    pub fn hooks_path(&self, codex_home: Option<&Path>, home: &Path) -> PathBuf {
        codex_home
            .map(Path::to_path_buf)
            .unwrap_or_else(|| home.join(".codex"))
            .join("hooks.json")
    }

    pub fn validate_install_version(&self, confirmed_unverified: bool) -> Result<(), AppError> {
        crate::agents::claude::validate_detection(&self.detect(), confirmed_unverified)
    }

    pub fn redetect(
        &self,
        repository: &IntegrationRepository,
    ) -> Result<AgentHealthUpdate, AppError> {
        persist_detection(repository, &self.detect())
    }

    fn installer(&self, env: &HookEnvironment) -> HookInstaller {
        HookInstaller::new(
            AgentKind::Codex,
            self.hooks_path(env.codex_home.as_deref(), &env.home),
            env.repository.clone(),
            env.cipher.clone(),
            env.helper.clone(),
        )
    }
}

impl crate::agents::AgentIntegration for CodexIntegration {
    fn detect(&self) -> Detection {
        CodexIntegration::detect(self)
    }

    fn capabilities(&self, version: &Version) -> CapabilityResolution {
        CodexIntegration::capabilities(self, version)
    }

    fn install_hooks(
        &self,
        env: &HookEnvironment,
        selection: &HookSelection,
    ) -> Result<Installation, AppError> {
        self.installer(env).apply(HookAction::Install, selection)
    }

    fn inspect_hooks(
        &self,
        env: &HookEnvironment,
        selection: &HookSelection,
    ) -> Result<HookHealth, AppError> {
        self.installer(env).inspect(selection)
    }
}
