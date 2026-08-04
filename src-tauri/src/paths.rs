use std::path::PathBuf;

use directories::BaseDirs;
use serde::{Deserialize, Serialize};
#[cfg(windows)]
use sha2::{Digest, Sha256};

const APP_ID: &str = "com.ccreminder.app";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AppPaths {
    pub data_dir: PathBuf,
    pub database: PathBuf,
    pub spool: PathBuf,
    pub logs: PathBuf,
    pub bin: PathBuf,
    pub agent_versions: PathBuf,
    pub project_paths: PathBuf,
    pub correlation_key: PathBuf,
    pub ipc: PathBuf,
}

impl AppPaths {
    pub fn discover() -> std::io::Result<Self> {
        #[cfg(feature = "test-support")]
        let root = std::env::var_os("CC_REMINDER_TEST_DATA_DIR").map(PathBuf::from);
        #[cfg(not(feature = "test-support"))]
        let root: Option<PathBuf> = None;
        let root = root
            .or_else(|| BaseDirs::new().map(|dirs| dirs.data_dir().to_path_buf()))
            .ok_or_else(|| {
                std::io::Error::new(std::io::ErrorKind::NotFound, "data directory unavailable")
            })?
            .join(APP_ID);
        #[cfg(unix)]
        let ipc = root.join("ipc").join("hook.sock");
        #[cfg(windows)]
        let ipc = {
            let sid = crate::security::permissions::current_user_sid_string().map_err(|_| {
                std::io::Error::new(
                    std::io::ErrorKind::PermissionDenied,
                    "current SID unavailable",
                )
            })?;
            let digest = Sha256::digest(sid.as_bytes());
            PathBuf::from(format!(
                r"\\.\pipe\cc-reminder-{}",
                &hex::encode(digest)[..16]
            ))
        };
        let paths = Self {
            database: root.join("cc-reminder.sqlite3"),
            spool: root.join("spool"),
            logs: root.join("logs"),
            bin: root.join("bin"),
            agent_versions: root.join("agent-versions.json"),
            project_paths: root.join("project-paths.json"),
            correlation_key: root.join("correlation.key"),
            ipc,
            data_dir: root,
        };
        Ok(paths)
    }

    pub fn ensure(&self) -> std::io::Result<()> {
        for path in [&self.data_dir, &self.spool, &self.logs, &self.bin] {
            if path.extension().is_none() {
                std::fs::create_dir_all(path)?;
            }
        }
        #[cfg(unix)]
        if let Some(parent) = self.ipc.parent() {
            std::fs::create_dir_all(parent)?;
        }
        Ok(())
    }

    pub fn endpoint(&self) -> crate::ipc::server::Endpoint {
        #[cfg(unix)]
        {
            crate::ipc::server::Endpoint::Unix(self.ipc.clone())
        }
        #[cfg(windows)]
        {
            crate::ipc::server::Endpoint::Windows(self.ipc.to_string_lossy().into_owned())
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentVersionCacheFile {
    pub schema_version: u16,
    pub agents: std::collections::BTreeMap<crate::model::AgentKind, CachedAgentVersion>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CachedAgentVersion {
    pub version: semver::Version,
    pub detected_at: chrono::DateTime<chrono::Utc>,
}
