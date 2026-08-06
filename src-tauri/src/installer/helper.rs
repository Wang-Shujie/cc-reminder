//! Signed helper installation (Task 11, design 9.1).
//!
//! The packaged helper binary is verified against an embedded manifest BEFORE it
//! is copied to its stable per-user `bin` path. Verification is length +
//! SHA-256 over the packaged bytes; the manifest is never accepted from runtime
//! configuration. Installation writes a same-directory temp, applies owner-only
//! permissions (0700 dir / 0700 exec on Unix, current-SID DACL on Windows),
//! `sync_all`, atomically renames, then re-hashes the installed file. A lower
//! semantic helper version is rejected unless the caller passes an explicit
//! rollback confirmation.
//!
//! The install LOGIC here is fully covered by unit tests with injected fixture
//! bytes + manifest. Production wiring (compiling `cc-reminder-hook.rs` as a
//! per-target Tauri external binary, generating a real `helper-manifest.json`
//! with target triple / length / SHA-256 during release packaging, and
//! selecting the current target's entry at startup) is documented in
//! `resources/helper-manifest.json` and deferred to release tooling — the
//! runtime installer only consumes an already-selected manifest entry plus its
//! bytes, so the security-critical path is exercised exactly as shipped.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use semver::Version;
use serde::{Deserialize, Serialize};

use crate::error::{AppError, ErrorDomain};
use crate::installer::sha256_hex;
use crate::security::permissions::{ensure_current_user_dacl, ensure_private_directory};

const STABLE_NAME: &str = "cc-reminder-hook";
const VERSION_SIDECAR: &str = "cc-reminder-hook.version";

/// On-disk `helper-manifest.json`: one entry per compile target. At runtime the
/// current target's entry is selected and passed to [`HelperInstaller::new`].
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct HelperManifestFile {
    pub helpers: Vec<HelperManifestEntry>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct HelperManifestEntry {
    pub target_triple: String,
    pub helper_version: Version,
    pub filename: String,
    pub length: u64,
    pub sha256: String,
}

/// Select the manifest entry whose `target_triple` matches. Returns
/// `update.helper_integrity_failed` when no entry matches — the runtime never
/// silently falls back to a foreign target's helper.
pub fn select_target_entry<'a>(
    manifest: &'a HelperManifestFile,
    target_triple: &str,
) -> Result<&'a HelperManifestEntry, AppError> {
    manifest
        .helpers
        .iter()
        .find(|entry| entry.target_triple == target_triple)
        .ok_or_else(|| integrity_error("no helper manifest entry for the current target"))
}

/// The current compile target's triple (arch-vendor-os-abi).
pub fn current_target_triple() -> &'static str {
    // Composed from std::env::consts so it is available without a build script.
    // ponytail: a build.rs-generated env would be marginally more precise on
    // the vendor/abi tail, but this is sufficient for target selection.
    const ARCH: &str = std::env::consts::ARCH;
    const OS: &str = std::env::consts::OS;
    match (ARCH, OS) {
        ("x86_64", "linux") => "x86_64-unknown-linux-gnu",
        ("aarch64", "linux") => "aarch64-unknown-linux-gnu",
        ("x86_64", "macos") => "x86_64-apple-darwin",
        ("aarch64", "macos") => "aarch64-apple-darwin",
        ("x86_64", "windows") => "x86_64-pc-windows-msvc",
        ("aarch64", "windows") => "aarch64-pc-windows-msvc",
        _ => "unknown-target",
    }
}

/// Installed helper location and verified version.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InstalledHelper {
    pub path: PathBuf,
    pub version: Version,
}

#[derive(Clone, Debug, Default)]
pub struct InstallOptions {
    /// Allow installing a helper whose semantic version is LOWER than the
    /// currently installed one. Only set when the user explicitly confirms a
    /// rollback recovered from an encrypted snapshot.
    pub allow_rollback: bool,
}

impl InstallOptions {
    pub fn rollback() -> Self {
        Self {
            allow_rollback: true,
        }
    }
}

#[derive(Clone, Debug)]
pub struct HelperInstaller {
    bin_dir: PathBuf,
    entry: HelperManifestEntry,
    bytes: Vec<u8>,
}

impl HelperInstaller {
    /// Construct from an already target-selected manifest entry and its packaged
    /// bytes. Both are injected so the security-critical verify-then-copy path
    /// is identical in production and tests.
    pub fn new(bin_dir: PathBuf, entry: HelperManifestEntry, bytes: Vec<u8>) -> Self {
        Self {
            bin_dir,
            entry,
            bytes,
        }
    }

    pub fn stable_path(&self) -> PathBuf {
        self.bin_dir.join(stable_filename())
    }

    pub fn manifest_version(&self) -> &Version {
        &self.entry.helper_version
    }

    /// Verify the packaged bytes against the manifest, then atomically install.
    pub fn install(&self) -> Result<InstalledHelper, AppError> {
        self.install_with(InstallOptions::default())
    }

    pub fn install_with(&self, options: InstallOptions) -> Result<InstalledHelper, AppError> {
        // 1. Verify packaged bytes against the manifest BEFORE touching the
        //    stable path. Length first (cheap), then SHA-256.
        if self.bytes.len() as u64 != self.entry.length {
            return Err(integrity_error("helper length mismatch"));
        }
        let actual = sha256_hex(&self.bytes);
        if actual != self.entry.sha256 {
            return Err(integrity_error("helper sha-256 mismatch"));
        }

        // 2. Ensure the bin directory exists and is owner-only.
        ensure_private_directory(&self.bin_dir)?;
        #[cfg(windows)]
        ensure_current_user_dacl(&self.bin_dir)?;

        // 3. Reject a downgrade unless the caller passed an explicit rollback.
        let target = self.stable_path();
        if let Some(installed) = self.installed_version()
            && self.entry.helper_version < installed
            && !options.allow_rollback
        {
            return Err(AppError {
                domain: ErrorDomain::Update,
                code: "update.helper_rollback_blocked".to_owned(),
                message: format!(
                    "helper version {} is older than the installed {}",
                    self.entry.helper_version, installed
                ),
                suggested_action: Some("confirm an explicit rollback to continue".to_owned()),
            });
        }

        // 4. Write a same-directory temp, harden, sync, atomic rename.
        let temp = self.bin_dir.join(format!(
            ".cc-reminder-helper-{}.tmp",
            uuid::Uuid::now_v7().simple()
        ));
        let result = (|| -> Result<(), AppError> {
            let mut file = private_create(&temp)?;
            ensure_current_user_dacl(&temp)?;
            file.write_all(&self.bytes)
                .and_then(|()| file.flush())
                .and_then(|()| file.sync_all())
                .map_err(|_| write_failed())?;
            drop(file);
            apply_exec_mode(&temp)?;
            publish_rename(&temp, &target).map_err(|_| write_failed())?;
            Ok(())
        })();
        match result {
            Ok(()) => {}
            Err(error) => {
                let _ = fs::remove_file(&temp);
                return Err(error);
            }
        }

        // 5. Re-hash the installed file; it MUST match the manifest.
        let installed_bytes = fs::read(&target).map_err(|_| write_failed())?;
        if sha256_hex(&installed_bytes) != self.entry.sha256 {
            // The file on disk does not match what we just wrote — refuse to
            // record it as installed.
            return Err(integrity_error("installed helper failed re-hash"));
        }

        // 6. Record the installed version in a private sidecar so a later
        //    downgrade attempt can be detected without trusting the binary.
        write_version_sidecar(&self.bin_dir, &self.entry.helper_version)?;

        Ok(InstalledHelper {
            path: target,
            version: self.entry.helper_version.clone(),
        })
    }

    /// Version recorded by the most recent successful install, or `None`.
    pub fn installed_version(&self) -> Option<Version> {
        read_version_sidecar(&self.bin_dir)
    }
}

fn stable_filename() -> String {
    if cfg!(windows) {
        format!("{STABLE_NAME}.exe")
    } else {
        STABLE_NAME.to_owned()
    }
}

fn version_sidecar_path(bin_dir: &Path) -> PathBuf {
    bin_dir.join(VERSION_SIDECAR)
}

fn read_version_sidecar(bin_dir: &Path) -> Option<Version> {
    let bytes = fs::read(version_sidecar_path(bin_dir)).ok()?;
    let text = std::str::from_utf8(&bytes).ok()?.trim();
    Version::parse(text).ok()
}

fn write_version_sidecar(bin_dir: &Path, version: &Version) -> Result<(), AppError> {
    let sidecar = version_sidecar_path(bin_dir);
    let temp = bin_dir.join(format!(
        ".cc-reminder-version-{}.tmp",
        uuid::Uuid::now_v7().simple()
    ));
    {
        let mut file = private_create(&temp)?;
        ensure_current_user_dacl(&temp)?;
        file.write_all(version.to_string().as_bytes())
            .and_then(|()| file.flush())
            .and_then(|()| file.sync_all())
            .map_err(|_| write_failed())?;
        drop(file);
    }
    publish_rename(&temp, &sidecar).map_err(|_| write_failed())?;
    Ok(())
}

fn publish_rename(temp: &Path, target: &Path) -> std::io::Result<()> {
    // ponytail: Windows durability (MoveFileExW/replace) is left to the platform
    // owner; same-filesystem rename is atomic on the Unix path under test.
    fs::rename(temp, target)
}

#[cfg(unix)]
fn private_create(path: &Path) -> Result<std::fs::File, AppError> {
    use std::os::unix::fs::OpenOptionsExt;
    std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o700)
        .open(path)
        .map_err(|_| write_failed())
}

#[cfg(not(unix))]
fn private_create(path: &Path) -> Result<std::fs::File, AppError> {
    std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|_| write_failed())
}

#[cfg(unix)]
fn apply_exec_mode(path: &Path) -> Result<(), AppError> {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(0o700)).map_err(|_| write_failed())
}

#[cfg(not(unix))]
fn apply_exec_mode(_path: &Path) -> Result<(), AppError> {
    // Owner-only DACL was applied after creation; no mode bit concept on Windows.
    Ok(())
}

fn integrity_error(message: &str) -> AppError {
    AppError {
        domain: ErrorDomain::Update,
        code: "update.helper_integrity_failed".to_owned(),
        message: message.to_owned(),
        suggested_action: Some("reinstall the application from a trusted source".to_owned()),
    }
}

fn write_failed() -> AppError {
    AppError {
        domain: ErrorDomain::Update,
        code: "update.helper_write_failed".to_owned(),
        message: "helper installation failed".to_owned(),
        suggested_action: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry_for(bytes: &[u8], version: &str) -> HelperManifestEntry {
        HelperManifestEntry {
            target_triple: current_target_triple().to_owned(),
            helper_version: Version::parse(version).unwrap(),
            filename: stable_filename(),
            length: bytes.len() as u64,
            sha256: sha256_hex(bytes),
        }
    }

    fn installer(root: &Path, bytes: &[u8], version: &str) -> HelperInstaller {
        HelperInstaller::new(root.join("bin"), entry_for(bytes, version), bytes.to_vec())
    }

    #[test]
    fn helper_is_copied_only_after_manifest_hash_matches() {
        let root = tempfile::tempdir().unwrap();
        let installed = installer(root.path(), b"signed helper bytes", "0.1.0")
            .install()
            .unwrap();
        assert_eq!(
            std::fs::read(&installed.path).unwrap(),
            b"signed helper bytes"
        );
        assert_eq!(installed.version, Version::parse("0.1.0").unwrap());
    }

    #[test]
    fn hash_mismatch_keeps_existing_helper() {
        let root = tempfile::tempdir().unwrap();
        let bin = root.path().join("bin");
        std::fs::create_dir_all(&bin).unwrap();
        let existing = bin.join(stable_filename());
        std::fs::write(&existing, b"working old helper").unwrap();

        let installer = HelperInstaller::new(
            bin.clone(),
            entry_for(b"expected", "0.2.0"),
            b"tampered package".to_vec(),
        );
        let error = installer.install().unwrap_err();
        assert_eq!(error.code, "update.helper_integrity_failed");
        assert_eq!(std::fs::read(&existing).unwrap(), b"working old helper");
    }

    #[test]
    fn length_mismatch_is_rejected_before_copy() {
        let root = tempfile::tempdir().unwrap();
        let mut entry = entry_for(b"abc", "0.1.0");
        entry.length = 99;
        let installer = HelperInstaller::new(root.path().join("bin"), entry, b"abc".to_vec());
        assert_eq!(
            installer.install().unwrap_err().code,
            "update.helper_integrity_failed"
        );
    }

    #[cfg(unix)]
    #[test]
    fn installed_helper_is_owner_only_executable() {
        use std::os::unix::fs::PermissionsExt;
        let root = tempfile::tempdir().unwrap();
        let installed = installer(root.path(), b"bin", "0.1.0").install().unwrap();
        let file_mode = std::fs::metadata(&installed.path)
            .unwrap()
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(file_mode, 0o700);
        let dir_mode = std::fs::metadata(root.path().join("bin"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(dir_mode, 0o700);
    }

    #[test]
    fn install_leaves_no_temp_behind() {
        let root = tempfile::tempdir().unwrap();
        installer(root.path(), b"payload", "0.1.0")
            .install()
            .unwrap();
        let temps = std::fs::read_dir(root.path().join("bin"))
            .unwrap()
            .filter(|entry| {
                let name = entry.as_ref().unwrap().file_name();
                name.to_string_lossy().ends_with(".tmp")
            })
            .count();
        assert_eq!(temps, 0);
    }

    #[test]
    fn reinstalled_helper_matches_manifest_hash() {
        let root = tempfile::tempdir().unwrap();
        let installer = installer(root.path(), b"the helper", "0.1.0");
        let installed = installer.install().unwrap();
        assert_eq!(
            sha256_hex(&std::fs::read(&installed.path).unwrap()),
            installer.entry.sha256
        );
    }

    #[test]
    fn lower_version_is_rejected_without_explicit_rollback() {
        let root = tempfile::tempdir().unwrap();
        installer(root.path(), b"v2 body", "0.2.0")
            .install()
            .unwrap();
        let error = installer(root.path(), b"v1 body", "0.1.0")
            .install()
            .unwrap_err();
        assert_eq!(error.code, "update.helper_rollback_blocked");
        // Existing v2 bytes are untouched.
        assert_eq!(
            std::fs::read(root.path().join("bin").join(stable_filename())).unwrap(),
            b"v2 body"
        );
    }

    #[test]
    fn explicit_rollback_allows_lower_version() {
        let root = tempfile::tempdir().unwrap();
        installer(root.path(), b"v2 body", "0.2.0")
            .install()
            .unwrap();
        let installed = installer(root.path(), b"v1 body", "0.1.0")
            .install_with(InstallOptions::rollback())
            .unwrap();
        assert_eq!(std::fs::read(&installed.path).unwrap(), b"v1 body");
    }

    #[test]
    fn upgrade_to_higher_version_succeeds() {
        let root = tempfile::tempdir().unwrap();
        installer(root.path(), b"v1", "0.1.0").install().unwrap();
        installer(root.path(), b"v2", "0.2.0").install().unwrap();
        assert_eq!(
            std::fs::read(root.path().join("bin").join(stable_filename())).unwrap(),
            b"v2"
        );
    }

    #[test]
    fn select_target_entry_rejects_foreign_target() {
        let manifest = HelperManifestFile {
            helpers: vec![HelperManifestEntry {
                target_triple: "aarch64-apple-darwin".to_owned(),
                helper_version: Version::new(0, 1, 0),
                filename: stable_filename(),
                length: 1,
                sha256: "x".to_owned(),
            }],
        };
        assert!(select_target_entry(&manifest, "x86_64-pc-windows-msvc").is_err());
    }
}
