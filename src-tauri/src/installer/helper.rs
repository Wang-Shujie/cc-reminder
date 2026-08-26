//! Signed helper installation (Task 11, design 9.1).
//!
//! The packaged helper binary is verified against a bundled manifest BEFORE it
//! is copied to its stable per-user `bin` path. Verification is length +
//! SHA-256 over the packaged bytes; the manifest is never accepted from runtime
//! configuration. Installation writes a same-directory temp, applies owner-only
//! permissions (0700 dir / 0700 exec on Unix, current-SID DACL on Windows),
//! `sync_all`, atomically renames, then re-hashes the installed file. A lower
//! semantic helper version is rejected unless the caller passes an explicit
//! rollback confirmation.
//!
//! Production wiring: release packaging stages the signed helper under
//! `resources/bin/` and regenerates `resources/helper-manifest.json` from the
//! final signed bytes (see `.github/workflows/release.yml`); Tauri bundles both
//! files relative to the resource directory. [`load_bundled_installer`] is the
//! only production bridge between those files and [`HelperInstaller`]: it joins
//! FIXED relative paths under the shell-resolved resource directory — never a
//! caller-supplied path — selects the current target's entry, reads the bundled
//! bytes, and hands entry + bytes to the same verify-then-copy logic the unit
//! tests exercise. Development builds ship the committed PLACEHOLDER manifest
//! whose triple matches no compile target; in that case (and whenever the
//! bundled bytes are absent) the loader fails with the typed
//! `configuration.helper_unavailable` error instead of panicking or installing
//! placeholder bytes.

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

/// Manifest location RELATIVE to Tauri's resource directory
/// (`app.path().resource_dir()`; dev builds mirror the layout under
/// `target/<profile>` via tauri-build). Fixed by construction — never accepted
/// from runtime configuration or command input.
pub const MANIFEST_RESOURCE_PATH: &str = "resources/helper-manifest.json";

/// Directory holding the staged helper binaries, RELATIVE to the resource
/// directory. Release CI overwrites its contents with signed per-target bytes.
pub const BIN_RESOURCE_DIR: &str = "resources/bin";

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

// ---------------------------------------------------------------------------
// Production loading bridge: bundled resources → HelperInstaller
// ---------------------------------------------------------------------------

/// Typed failure for "this installation cannot deploy a signed helper".
/// Development builds carry the committed PLACEHOLDER manifest whose target
/// triple matches no compile target, and may lack the staged helper bytes
/// entirely — that state must surface as an actionable error, NEVER as a panic
/// or a placeholder install.
pub(crate) fn helper_unavailable_error(detail: &str) -> AppError {
    AppError {
        domain: ErrorDomain::Configuration,
        code: "configuration.helper_unavailable".to_owned(),
        message: format!("signed hook helper is unavailable in this installation: {detail}"),
        suggested_action: Some(
            "install an official release build; development builds do not bundle the signed helper"
                .to_owned(),
        ),
    }
}

/// Reject manifest filenames that could escape the fixed `resources/bin`
/// directory (defense in depth: the manifest ships inside the signed bundle,
/// but nothing here should ever trust it with path structure).
fn sanitize_bundled_filename(filename: &str) -> Result<(), AppError> {
    let ok = !filename.is_empty()
        && filename
            == Path::new(filename)
                .file_name()
                .unwrap_or_default()
                .to_string_lossy();
    if ok {
        Ok(())
    } else {
        Err(helper_unavailable_error(&format!(
            "manifest entry filename {filename:?} is not a plain file name"
        )))
    }
}

/// Build a [`HelperInstaller`] from the packaged resources: parse
/// `<resources_dir>/helper-manifest.json`, select the entry for the CURRENT
/// compile target, and read the bundled bytes from `resources/bin/<filename>`.
///
/// `resources_dir` is resolved by the app shell from Tauri's resource-dir API
/// (never from command input); both joined subpaths are the constants above.
/// Any failure — missing/malformed manifest, no entry for this target
/// (placeholder manifests), missing bytes — returns
/// `configuration.helper_unavailable`.
pub fn load_bundled_installer(
    resources_dir: &Path,
    bin_dir: &Path,
) -> Result<HelperInstaller, AppError> {
    let manifest_path = resources_dir.join(MANIFEST_RESOURCE_PATH);
    let text = fs::read_to_string(&manifest_path).map_err(|_| {
        helper_unavailable_error(&format!(
            "{} is missing or unreadable",
            MANIFEST_RESOURCE_PATH
        ))
    })?;
    let manifest: HelperManifestFile = serde_json::from_str(&text)
        .map_err(|_| helper_unavailable_error("helper manifest is malformed"))?;
    let triple = current_target_triple();
    let entry = select_target_entry(&manifest, triple)
        .map_err(|_| {
            helper_unavailable_error(&format!(
                "manifest has no helper entry for the current target ({triple}); \
                 this is expected only for development builds with the placeholder manifest"
            ))
        })?
        .clone();
    sanitize_bundled_filename(&entry.filename)?;
    let bytes_path = resources_dir.join(BIN_RESOURCE_DIR).join(&entry.filename);
    let bytes = fs::read(&bytes_path).map_err(|_| {
        helper_unavailable_error(&format!(
            "bundled helper {} is missing",
            bytes_path.display()
        ))
    })?;
    Ok(HelperInstaller::new(bin_dir.to_path_buf(), entry, bytes))
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

    /// Deployment bridge for the desktop shell: make sure the stable-path
    /// binary exists and matches this manifest entry. Idempotent — when the
    /// stable file already carries byte-identical content (same length +
    /// SHA-256), the copy is skipped entirely; otherwise a full verified
    /// install runs. Every Install/Repair pass calls this BEFORE mutating
    /// Agent config so `HookInstaller::apply` always finds its helper.
    pub fn ensure_installed(&self) -> Result<InstalledHelper, AppError> {
        if let Ok(existing) = fs::read(self.stable_path())
            && existing.len() as u64 == self.entry.length
            && sha256_hex(&existing) == self.entry.sha256
        {
            return Ok(InstalledHelper {
                path: self.stable_path(),
                version: self.entry.helper_version.clone(),
            });
        }
        self.install()
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

    // ---- production loading bridge ----------------------------------------

    /// Lay out a directory exactly like the packaged resources: a manifest at
    /// `resources/helper-manifest.json` and bytes under `resources/bin/`.
    fn bundled_resources(root: &Path, entry_triple: &str, bytes: &[u8]) -> std::path::PathBuf {
        let resources = root.join("resources");
        fs::create_dir_all(resources.join("bin")).unwrap();
        fs::write(resources.join("bin").join(stable_filename()), bytes).unwrap();
        let manifest = HelperManifestFile {
            helpers: vec![HelperManifestEntry {
                target_triple: entry_triple.to_owned(),
                helper_version: Version::parse("0.9.0").unwrap(),
                filename: stable_filename(),
                length: bytes.len() as u64,
                sha256: sha256_hex(bytes),
            }],
        };
        fs::write(
            resources.join("helper-manifest.json"),
            serde_json::to_vec_pretty(&manifest).unwrap(),
        )
        .unwrap();
        resources
    }

    #[test]
    fn bundled_resources_load_and_install_through_the_bridge() {
        let root = tempfile::tempdir().unwrap();
        let payload = b"signed dev-loop helper bytes";
        // The synthetic manifest carries THIS compile target's triple, exactly
        // like release packaging does per platform.
        bundled_resources(root.path(), current_target_triple(), payload);
        let bin_dir = root.path().join("data").join("bin");

        let installer = load_bundled_installer(root.path(), &bin_dir).unwrap();
        assert_eq!(
            installer.manifest_version(),
            &Version::parse("0.9.0").unwrap()
        );
        let installed = installer.ensure_installed().unwrap();
        assert_eq!(fs::read(&installed.path).unwrap(), payload);

        // Idempotent re-run skips the copy (stable bytes already match).
        let before = fs::read(&installed.path).unwrap();
        installer.ensure_installed().unwrap();
        assert_eq!(fs::read(&installed.path).unwrap(), before);
    }

    #[test]
    fn placeholder_manifest_without_matching_target_is_rejected_as_unavailable() {
        let root = tempfile::tempdir().unwrap();
        // The committed development manifest: a placeholder triple that can
        // never match a real compile target.
        bundled_resources(root.path(), "REPLACE_WITH_TARGET_TRIPLE", b"x");
        let error =
            load_bundled_installer(root.path(), root.path().join("bin").as_path()).unwrap_err();
        assert_eq!(error.domain, ErrorDomain::Configuration);
        assert_eq!(error.code, "configuration.helper_unavailable");
        assert!(error.suggested_action.is_some());
        // Nothing was installed.
        assert!(!root.path().join("bin").join(stable_filename()).exists());
    }

    #[test]
    fn missing_bundled_bytes_are_rejected_as_unavailable_without_panicking() {
        let root = tempfile::tempdir().unwrap();
        let resources = bundled_resources(root.path(), current_target_triple(), b"bytes");
        // Remove the staged binary: release parity broken / absent bundle.
        fs::remove_file(resources.join("bin").join(stable_filename())).unwrap();
        let error =
            load_bundled_installer(root.path(), root.path().join("bin").as_path()).unwrap_err();
        assert_eq!(error.code, "configuration.helper_unavailable");
    }

    #[test]
    fn malformed_manifest_is_rejected_as_unavailable() {
        let root = tempfile::tempdir().unwrap();
        let resources = root.path().join("resources");
        fs::create_dir_all(&resources).unwrap();
        fs::write(resources.join("helper-manifest.json"), b"{ not json").unwrap();
        let error =
            load_bundled_installer(root.path(), root.path().join("bin").as_path()).unwrap_err();
        assert_eq!(error.code, "configuration.helper_unavailable");
    }

    #[test]
    fn manifest_filename_that_escapes_the_bin_dir_is_rejected() {
        let root = tempfile::tempdir().unwrap();
        let resources = bundled_resources(root.path(), current_target_triple(), b"bytes");
        // Rewrite the manifest with a traversal filename; even though this file
        // lives inside the signed bundle, the loader refuses path structure.
        let manifest = HelperManifestFile {
            helpers: vec![HelperManifestEntry {
                target_triple: current_target_triple().to_owned(),
                helper_version: Version::parse("0.9.0").unwrap(),
                filename: "../escape".to_owned(),
                length: 5,
                sha256: sha256_hex(b"bytes"),
            }],
        };
        fs::write(
            resources.join("helper-manifest.json"),
            serde_json::to_vec_pretty(&manifest).unwrap(),
        )
        .unwrap();
        let error =
            load_bundled_installer(root.path(), root.path().join("bin").as_path()).unwrap_err();
        assert_eq!(error.code, "configuration.helper_unavailable");
    }
}
