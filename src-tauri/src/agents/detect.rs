#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};
    #[cfg(unix)]
    use std::time::Instant;

    use semver::Version;

    use super::{AgentEnvironment, OsFamily, executable_candidates, parse_version};
    use crate::model::AgentKind;

    #[test]
    fn parses_current_agent_version_outputs() {
        assert_eq!(
            parse_version(AgentKind::ClaudeCode, "2.1.218 (Claude Code)").unwrap(),
            Version::new(2, 1, 218)
        );
        assert_eq!(
            parse_version(AgentKind::Codex, "codex-cli 0.145.0").unwrap(),
            Version::new(0, 145, 0)
        );
    }

    #[test]
    fn rejects_invalid_version_output() {
        assert!(parse_version(AgentKind::ClaudeCode, "not a version").is_err());
        assert!(parse_version(AgentKind::Codex, "1.2.3").is_err());
    }

    #[test]
    fn explicit_configured_path_precedes_path_and_known_locations() {
        let environment = test_environment(OsFamily::Unix);
        let candidates = executable_candidates(
            AgentKind::Codex,
            Some(Path::new("/chosen/codex")),
            &environment,
        );
        assert_eq!(candidates[0], PathBuf::from("/chosen/codex"));
        assert_eq!(candidates[1], PathBuf::from("/on-path/codex"));
        assert_eq!(deduplicated(&candidates), candidates);
    }

    #[test]
    fn candidate_names_include_documented_platform_extensions() {
        assert_eq!(
            executable_candidates(
                AgentKind::ClaudeCode,
                None,
                &test_environment(OsFamily::Windows)
            ),
            vec![
                PathBuf::from("C:/bin/claude.exe"),
                PathBuf::from("C:/bin/claude.cmd"),
                PathBuf::from("C:/Users/test/.local/bin/claude.exe"),
                PathBuf::from("C:/Users/test/.local/bin/claude.cmd"),
                PathBuf::from("C:/AppData/npm/claude.exe"),
                PathBuf::from("C:/AppData/npm/claude.cmd"),
                PathBuf::from("C:/Users/test/scoop/shims/claude.exe"),
                PathBuf::from("C:/Users/test/scoop/shims/claude.cmd"),
                PathBuf::from("C:/LocalAppData/Microsoft/WinGet/Links/claude.exe"),
                PathBuf::from("C:/LocalAppData/Microsoft/WinGet/Links/claude.cmd"),
            ]
        );
    }

    #[test]
    fn unix_known_locations_are_exact_and_not_scanned() {
        assert_eq!(
            executable_candidates(AgentKind::Codex, None, &test_environment(OsFamily::Macos)),
            vec![
                PathBuf::from("/on-path/codex"),
                PathBuf::from("/home/test/.local/bin/codex"),
                PathBuf::from("/home/test/.claude/local/codex"),
                PathBuf::from("/home/test/.npm-global/bin/codex"),
                PathBuf::from("/home/test/.volta/bin/codex"),
                PathBuf::from("/home/test/.asdf/shims/codex"),
                PathBuf::from("/home/test/.bun/bin/codex"),
                PathBuf::from("/opt/homebrew/bin/codex"),
                PathBuf::from("/usr/local/bin/codex"),
            ]
        );
    }

    #[test]
    fn canonicalization_deduplicates_symlink_targets() {
        let root = tempfile::tempdir().unwrap();
        let executable = root.path().join("codex");
        std::fs::write(&executable, "").unwrap();
        let alias = root.path().join("alias");
        #[cfg(unix)]
        std::os::unix::fs::symlink(&executable, &alias).unwrap();
        #[cfg(not(unix))]
        std::fs::hard_link(&executable, &alias).unwrap();

        assert_eq!(super::deduplicate_paths(vec![executable, alias]).len(), 1);
    }

    #[test]
    fn detected_version_cache_is_private_and_omits_executable_path() {
        let root = tempfile::tempdir().unwrap();
        let data_dir = root.path().join("com.ccreminder.app");
        let paths = crate::paths::AppPaths {
            database: data_dir.join("cc-reminder.sqlite3"),
            spool: data_dir.join("spool"),
            logs: data_dir.join("logs"),
            bin: data_dir.join("bin"),
            agent_versions: data_dir.join("agent-versions.json"),
            project_paths: data_dir.join("project-paths.json"),
            correlation_key: data_dir.join("correlation.key"),
            ipc: data_dir.join("ipc/hook.sock"),
            data_dir,
        };
        let detection = super::Detection {
            agent: AgentKind::Codex,
            executable_path: Some("/registered/codex".into()),
            version: Some(Version::new(0, 145, 0)),
            capability_verification: Some(crate::events::catalog::CatalogVerification::Exact),
            state: super::DetectionState::Detected,
            checked_at: chrono::Utc::now(),
        };

        super::AgentVersionCache::for_paths(&paths)
            .write_detected(&detection)
            .unwrap();

        let cache: crate::paths::AgentVersionCacheFile =
            serde_json::from_slice(&std::fs::read(&paths.agent_versions).unwrap()).unwrap();
        assert_eq!(cache.schema_version, 1);
        assert_eq!(
            cache.agents[&AgentKind::Codex].version,
            Version::new(0, 145, 0)
        );
        assert!(
            !std::fs::read_to_string(&paths.agent_versions)
                .unwrap()
                .contains("/registered/codex")
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                std::fs::metadata(&paths.agent_versions)
                    .unwrap()
                    .permissions()
                    .mode()
                    & 0o777,
                0o600
            );
        }
    }

    #[cfg(unix)]
    #[test]
    fn failed_version_process_is_not_reported_as_a_version() {
        let root = tempfile::tempdir().unwrap();
        let executable = executable_script(root.path(), "failed", "exit 1");

        // A non-zero exit yields Failed. Under heavy parallel test load the instant
        // child may not be reaped within the 2s DETECTION_TIMEOUT, surfacing as
        // TimedOut instead. Both mean "no version reported" (Ok is the only
        // version-producing outcome); the TimedOut variant is pinned separately
        // by version_process_is_killed_after_two_seconds, so we assert the outcome
        // rather than over-pin the variant on a scheduling-dependent path.
        assert!(matches!(
            super::run_version_command(&executable),
            Err(super::VersionCommandError::Failed) | Err(super::VersionCommandError::TimedOut)
        ));
    }

    #[cfg(unix)]
    #[test]
    fn version_process_is_killed_after_two_seconds() {
        let root = tempfile::tempdir().unwrap();
        let executable = executable_script(root.path(), "slow", "sleep 3");
        let started = Instant::now();

        assert!(matches!(
            super::run_version_command(&executable),
            Err(super::VersionCommandError::TimedOut)
        ));
        assert!(started.elapsed() < std::time::Duration::from_secs(3));
    }

    fn deduplicated(paths: &[PathBuf]) -> Vec<PathBuf> {
        let mut seen = std::collections::BTreeSet::new();
        paths
            .iter()
            .filter(|path| seen.insert((*path).clone()))
            .cloned()
            .collect()
    }

    fn test_environment(os: OsFamily) -> AgentEnvironment {
        AgentEnvironment {
            os,
            path: vec![match os {
                OsFamily::Windows => PathBuf::from("C:/bin"),
                OsFamily::Unix | OsFamily::Macos => PathBuf::from("/on-path"),
            }],
            home: Some(PathBuf::from("/home/test")),
            user_profile: Some(PathBuf::from("C:/Users/test")),
            app_data: Some(PathBuf::from("C:/AppData")),
            local_app_data: Some(PathBuf::from("C:/LocalAppData")),
        }
    }

    #[cfg(unix)]
    fn executable_script(root: &Path, name: &str, body: &str) -> PathBuf {
        use std::os::unix::fs::PermissionsExt;

        let path = root.join(name);
        std::fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700)).unwrap();
        path
    }
}
use std::collections::{BTreeMap, BTreeSet};
use std::fs::{File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::thread;
use std::time::Duration;

use chrono::{DateTime, Utc};
use semver::Version;
use serde::{Deserialize, Serialize};
use wait_timeout::ChildExt;

use crate::events::catalog::{CatalogVerification, catalog_for};
use crate::model::AgentKind;
use crate::paths::{AgentVersionCacheFile, AppPaths, CachedAgentVersion};
use crate::security::permissions::{
    ensure_current_user_dacl, ensure_private_directory, ensure_private_file, validate_private_file,
};

const MAX_PROCESS_OUTPUT_BYTES: u64 = 32 * 1024;
const MAX_CACHE_BYTES: usize = 16 * 1024;
const DETECTION_TIMEOUT: Duration = Duration::from_secs(2);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OsFamily {
    Unix,
    Macos,
    Windows,
}

#[derive(Clone, Debug)]
pub struct AgentEnvironment {
    pub os: OsFamily,
    pub path: Vec<PathBuf>,
    pub home: Option<PathBuf>,
    pub user_profile: Option<PathBuf>,
    pub app_data: Option<PathBuf>,
    pub local_app_data: Option<PathBuf>,
}

impl AgentEnvironment {
    pub fn current() -> Self {
        let os = if cfg!(windows) {
            OsFamily::Windows
        } else if cfg!(target_os = "macos") {
            OsFamily::Macos
        } else {
            OsFamily::Unix
        };
        Self {
            os,
            path: std::env::var_os("PATH")
                .as_deref()
                .map(std::env::split_paths)
                .map(Iterator::collect)
                .unwrap_or_default(),
            home: std::env::var_os("HOME").map(PathBuf::from),
            user_profile: std::env::var_os("USERPROFILE").map(PathBuf::from),
            app_data: std::env::var_os("APPDATA").map(PathBuf::from),
            local_app_data: std::env::var_os("LOCALAPPDATA").map(PathBuf::from),
        }
    }
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DetectionState {
    Detected,
    Missing,
    InvalidVersion,
    ProcessFailed,
    TimedOut,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Detection {
    pub agent: AgentKind,
    pub executable_path: Option<PathBuf>,
    pub version: Option<Version>,
    pub capability_verification: Option<CatalogVerification>,
    pub state: DetectionState,
    pub checked_at: DateTime<Utc>,
}

#[derive(Clone, Debug)]
pub struct AgentVersionCache {
    path: PathBuf,
}

impl AgentVersionCache {
    pub fn for_paths(paths: &AppPaths) -> Self {
        Self {
            path: paths.agent_versions.clone(),
        }
    }

    pub fn write_detected(&self, detection: &Detection) -> std::io::Result<()> {
        let Some(version) = detection.version.clone() else {
            return Ok(());
        };
        if detection.state != DetectionState::Detected {
            return Ok(());
        }
        let mut agents = read_cache(&self.path).unwrap_or_default();
        agents.insert(
            detection.agent,
            CachedAgentVersion {
                version,
                detected_at: detection.checked_at,
            },
        );
        let bytes = serde_json::to_vec(&AgentVersionCacheFile {
            schema_version: 1,
            agents,
        })
        .map_err(std::io::Error::other)?;
        if bytes.len() > MAX_CACHE_BYTES {
            return Err(std::io::Error::other("agent version cache exceeds limit"));
        }
        let parent = self
            .path
            .parent()
            .ok_or_else(|| std::io::Error::other("cache parent unavailable"))?;
        ensure_private_directory(parent).map_err(|_| std::io::Error::other("cache permissions"))?;
        let temporary = parent.join(format!(".agent-versions.{}.tmp", uuid::Uuid::now_v7()));
        let result = (|| {
            let mut file = private_new_file(&temporary)?;
            file.write_all(&bytes)?;
            file.sync_all()?;
            drop(file);
            std::fs::rename(&temporary, &self.path)?;
            ensure_private_file(&self.path)
                .map_err(|_| std::io::Error::other("cache permissions"))?;
            ensure_current_user_dacl(&self.path)
                .map_err(|_| std::io::Error::other("cache permissions"))
        })();
        if result.is_err() {
            let _ = std::fs::remove_file(&temporary);
        }
        result
    }
}

pub fn detect_agent(agent: AgentKind, configured_path: Option<&Path>) -> Detection {
    let checked_at = Utc::now();
    let mut last_state = DetectionState::Missing;
    for candidate in executable_candidates(agent, configured_path, &AgentEnvironment::current()) {
        if !candidate.is_file() {
            continue;
        }
        match run_version_command(&candidate) {
            Ok(output) => match parse_version(agent, &output) {
                Ok(version) => {
                    let detection = Detection {
                        agent,
                        executable_path: Some(candidate.canonicalize().unwrap_or(candidate)),
                        capability_verification: Some(catalog_for(agent, &version).verification),
                        version: Some(version),
                        state: DetectionState::Detected,
                        checked_at,
                    };
                    if let Ok(paths) = AppPaths::discover() {
                        let _ = AgentVersionCache::for_paths(&paths).write_detected(&detection);
                    }
                    return detection;
                }
                Err(_) => last_state = DetectionState::InvalidVersion,
            },
            Err(VersionCommandError::TimedOut) => last_state = DetectionState::TimedOut,
            Err(VersionCommandError::Failed) => last_state = DetectionState::ProcessFailed,
        }
    }
    Detection {
        agent,
        executable_path: None,
        version: None,
        capability_verification: None,
        state: last_state,
        checked_at,
    }
}

pub fn executable_candidates(
    agent: AgentKind,
    configured_path: Option<&Path>,
    environment: &AgentEnvironment,
) -> Vec<PathBuf> {
    let names = executable_names(agent, environment.os);
    let mut candidates = configured_path
        .into_iter()
        .map(Path::to_path_buf)
        .collect::<Vec<_>>();
    for directory in &environment.path {
        candidates.extend(names.iter().map(|name| directory.join(name)));
    }
    match environment.os {
        OsFamily::Unix | OsFamily::Macos => {
            for suffix in [
                ".local/bin",
                ".claude/local",
                ".npm-global/bin",
                ".volta/bin",
                ".asdf/shims",
                ".bun/bin",
            ] {
                if let Some(home) = &environment.home {
                    candidates.extend(names.iter().map(|name| home.join(suffix).join(name)));
                }
            }
            for directory in [Path::new("/opt/homebrew/bin"), Path::new("/usr/local/bin")] {
                candidates.extend(names.iter().map(|name| directory.join(name)));
            }
        }
        OsFamily::Windows => {
            let directories = [
                environment
                    .user_profile
                    .as_ref()
                    .map(|path| path.join(".local/bin")),
                environment.app_data.as_ref().map(|path| path.join("npm")),
                environment
                    .user_profile
                    .as_ref()
                    .map(|path| path.join("scoop/shims")),
                environment
                    .local_app_data
                    .as_ref()
                    .map(|path| path.join("Microsoft/WinGet/Links")),
            ];
            for directory in directories.into_iter().flatten() {
                candidates.extend(names.iter().map(|name| directory.join(name)));
            }
        }
    }
    deduplicate_paths(candidates)
}

pub fn parse_version(agent: AgentKind, output: &str) -> Result<Version, ()> {
    let prefix = match agent {
        AgentKind::ClaudeCode => None,
        AgentKind::Codex => Some("codex-cli"),
    };
    let output = output.trim();
    if let Some(prefix) = prefix {
        let Some(version) = output
            .strip_prefix(prefix)
            .and_then(|value| value.split_whitespace().next())
        else {
            return Err(());
        };
        return Version::parse(version).map_err(|_| ());
    }
    output
        .split_whitespace()
        .next()
        .ok_or(())
        .and_then(|version| Version::parse(version).map_err(|_| ()))
}

fn executable_names(agent: AgentKind, os: OsFamily) -> Vec<&'static str> {
    let base = match agent {
        AgentKind::ClaudeCode => "claude",
        AgentKind::Codex => "codex",
    };
    match os {
        OsFamily::Windows => match base {
            "claude" => vec!["claude.exe", "claude.cmd"],
            _ => vec!["codex.exe", "codex.cmd"],
        },
        OsFamily::Unix | OsFamily::Macos => vec![base],
    }
}

fn deduplicate_paths(paths: Vec<PathBuf>) -> Vec<PathBuf> {
    let mut seen = BTreeSet::new();
    paths
        .into_iter()
        .filter(|path| seen.insert(path.canonicalize().unwrap_or_else(|_| path.clone())))
        .collect()
}

enum VersionCommandError {
    Failed,
    TimedOut,
}

fn run_version_command(executable: &Path) -> Result<String, VersionCommandError> {
    let mut command = Command::new(executable);
    command
        .arg("--version")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }
    let mut child = command.spawn().map_err(|_| VersionCommandError::Failed)?;
    let stdout = child.stdout.take().ok_or(VersionCommandError::Failed)?;
    let stderr = child.stderr.take().ok_or(VersionCommandError::Failed)?;
    let stdout_reader = thread::spawn(move || read_bounded(stdout));
    let stderr_reader = thread::spawn(move || read_bounded(stderr));
    let status = child
        .wait_timeout(DETECTION_TIMEOUT)
        .map_err(|_| VersionCommandError::Failed)?;
    let status = match status {
        Some(status) => status,
        None => {
            kill_timed_out_process(&mut child);
            let _ = child.wait();
            let _ = stdout_reader.join();
            let _ = stderr_reader.join();
            return Err(VersionCommandError::TimedOut);
        }
    };
    let stdout = stdout_reader
        .join()
        .map_err(|_| VersionCommandError::Failed)?;
    let _ = stderr_reader
        .join()
        .map_err(|_| VersionCommandError::Failed)?;
    if !status.success() {
        return Err(VersionCommandError::Failed);
    }
    String::from_utf8(stdout).map_err(|_| VersionCommandError::Failed)
}

#[cfg(unix)]
fn kill_timed_out_process(child: &mut std::process::Child) {
    unsafe extern "C" {
        fn kill(pid: i32, signal: i32) -> i32;
    }
    // The child starts its own process group, so this also closes pipes inherited by descendants.
    unsafe {
        let _ = kill(-(child.id() as i32), 9);
    }
}

#[cfg(not(unix))]
fn kill_timed_out_process(child: &mut std::process::Child) {
    let _ = child.kill();
}

fn read_bounded(reader: impl Read) -> Vec<u8> {
    let mut output = Vec::new();
    let _ = reader
        .take(MAX_PROCESS_OUTPUT_BYTES)
        .read_to_end(&mut output);
    output
}

fn read_cache(path: &Path) -> std::io::Result<BTreeMap<AgentKind, CachedAgentVersion>> {
    validate_private_file(path).map_err(|_| std::io::Error::other("cache permissions"))?;
    let mut bytes = Vec::new();
    File::open(path)?
        .take((MAX_CACHE_BYTES + 1) as u64)
        .read_to_end(&mut bytes)?;
    if bytes.len() > MAX_CACHE_BYTES {
        return Err(std::io::Error::other("agent version cache exceeds limit"));
    }
    let cache: AgentVersionCacheFile =
        serde_json::from_slice(&bytes).map_err(std::io::Error::other)?;
    if cache.schema_version != 1 {
        return Err(std::io::Error::other("unsupported agent version cache"));
    }
    Ok(cache.agents)
}

fn private_new_file(path: &Path) -> std::io::Result<File> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        OpenOptions::new()
            .create_new(true)
            .write(true)
            .mode(0o600)
            .open(path)
    }
    #[cfg(not(unix))]
    {
        OpenOptions::new().create_new(true).write(true).open(path)
    }
}
