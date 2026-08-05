#[cfg(test)]
mod tests {
    use chrono::Utc;
    use semver::Version;

    use super::ClaudeIntegration;
    use crate::agents::{Detection, DetectionState};
    use crate::events::catalog::CatalogVerification;
    use crate::model::AgentKind;

    #[test]
    fn user_settings_path_is_read_only_and_has_no_agent_scope_override() {
        let integration = ClaudeIntegration::with_detection(detected("2.1.218"));
        assert_eq!(
            integration.user_settings_path(std::path::Path::new("/home/test")),
            std::path::PathBuf::from("/home/test/.claude/settings.json")
        );
    }

    #[test]
    fn exact_version_is_ready_to_install() {
        let integration = ClaudeIntegration::with_detection(detected("2.1.218"));
        assert_eq!(
            integration
                .capabilities(&Version::new(2, 1, 218))
                .verification,
            CatalogVerification::Exact
        );
        assert!(integration.validate_install_version(false).is_ok());
    }

    #[test]
    fn compatible_unverified_requires_confirmation() {
        let integration = ClaudeIntegration::with_detection(detected("2.1.219"));
        assert_eq!(
            integration
                .capabilities(&Version::new(2, 1, 219))
                .verification,
            CatalogVerification::CompatibleUnverified
        );
        assert_eq!(
            integration
                .validate_install_version(false)
                .unwrap_err()
                .code,
            "integration.agent_confirmation_required"
        );
        assert!(integration.validate_install_version(true).is_ok());
    }

    #[test]
    fn missing_executable_cannot_be_installed() {
        let integration = ClaudeIntegration::with_detection(Detection {
            agent: AgentKind::ClaudeCode,
            executable_path: None,
            version: None,
            capability_verification: None,
            state: DetectionState::Missing,
            checked_at: Utc::now(),
        });
        assert_eq!(
            integration
                .validate_install_version(false)
                .unwrap_err()
                .code,
            "integration.agent_not_detected"
        );
    }

    #[test]
    fn redetection_updates_claude_installation_without_creating_a_config_snapshot() {
        let root = tempfile::tempdir().unwrap();
        let database = crate::storage::db::Database::open(
            &root.path().join("com.ccreminder.app/cc-reminder.sqlite3"),
        )
        .unwrap();
        let repository = crate::storage::integrations::IntegrationRepository::new(database);
        let integration = ClaudeIntegration::with_detection(detected("2.1.218"));

        let update = integration.redetect(&repository).unwrap();
        let stored = repository.agent(AgentKind::ClaudeCode).unwrap();

        assert_eq!(update.health, crate::model::InstallationHealth::Healthy);
        assert_eq!(stored.version, Some(Version::new(2, 1, 218)));
        assert_eq!(repository.snapshot_count(AgentKind::ClaudeCode).unwrap(), 0);
    }

    fn detected(version: &str) -> Detection {
        let version = Version::parse(version).unwrap();
        Detection {
            agent: AgentKind::ClaudeCode,
            executable_path: Some("/bin/claude".into()),
            capability_verification: Some(
                crate::events::catalog::catalog_for(AgentKind::ClaudeCode, &version).verification,
            ),
            version: Some(version),
            state: DetectionState::Detected,
            checked_at: Utc::now(),
        }
    }
}
use std::path::{Path, PathBuf};

use semver::Version;

use crate::agents::{AgentHealthUpdate, Detection, persist_detection};
use crate::error::{AppError, ErrorDomain};
use crate::events::catalog::{CapabilityResolution, CatalogVerification, catalog_for};
use crate::model::AgentKind;
use crate::storage::integrations::IntegrationRepository;

#[derive(Clone, Debug)]
pub struct ClaudeIntegration {
    configured_path: Option<PathBuf>,
    detection: Option<Detection>,
}

impl ClaudeIntegration {
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
            crate::agents::detect::detect_agent(
                AgentKind::ClaudeCode,
                self.configured_path.as_deref(),
            )
        })
    }

    pub fn capabilities(&self, version: &Version) -> CapabilityResolution {
        catalog_for(AgentKind::ClaudeCode, version)
    }

    pub fn user_settings_path(&self, home: &Path) -> PathBuf {
        home.join(".claude/settings.json")
    }

    pub fn validate_install_version(&self, confirmed_unverified: bool) -> Result<(), AppError> {
        validate_detection(&self.detect(), confirmed_unverified)
    }

    pub fn redetect(
        &self,
        repository: &IntegrationRepository,
    ) -> Result<AgentHealthUpdate, AppError> {
        persist_detection(repository, &self.detect())
    }
}

pub(crate) fn validate_detection(
    detection: &Detection,
    confirmed_unverified: bool,
) -> Result<(), AppError> {
    let Some(verification) = detection.capability_verification else {
        return Err(integration_error(
            "integration.agent_not_detected",
            "agent executable was not detected",
        ));
    };
    match verification {
        CatalogVerification::Exact => Ok(()),
        CatalogVerification::CompatibleUnverified if confirmed_unverified => Ok(()),
        CatalogVerification::CompatibleUnverified => Err(integration_error(
            "integration.agent_confirmation_required",
            "agent version requires user confirmation",
        )),
        CatalogVerification::UpgradeRequired => Err(integration_error(
            "integration.agent_upgrade_required",
            "agent version requires an upgrade",
        )),
    }
}

fn integration_error(code: &str, message: &str) -> AppError {
    AppError {
        domain: ErrorDomain::Integration,
        code: code.to_owned(),
        message: message.to_owned(),
        suggested_action: None,
    }
}
