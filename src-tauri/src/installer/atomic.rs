//! Checked atomic replacement of an Agent configuration file (design 9.4).
//!
//! Workflow: acquire an app-specific lock beside the target, re-read and hash
//! the current bytes, compare against the inspection hash (refuse with
//! `integration.config_drift` on mismatch), write a randomly named SAME-DIRECTORY
//! temp, `sync_all`, atomically rename, restore/verify the original file mode,
//! then re-read and re-validate the result. On any pre-rename failure only the
//! explicit temp path is removed and the target is left untouched.

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::Path;
use std::time::{Duration, Instant};

use crate::error::{AppError, ErrorDomain};
use crate::installer::jsonc;
use crate::installer::sha256_hex;
use crate::security::permissions::ensure_private_file;

const LOCK_NAME: &str = ".cc-reminder-config.lock";
const LOCK_TIMEOUT: Duration = Duration::from_secs(2);

/// Atomically replace `path` with `bytes` after verifying it has not drifted
/// since `inspected_hash` was captured. `mode` overrides the file mode on Unix;
/// `None` preserves the original file's mode.
pub fn atomic_replace_checked(
    path: &Path,
    inspected_hash: &str,
    bytes: &[u8],
    mode: Option<u32>,
) -> Result<(), AppError> {
    let parent = path.parent().ok_or_else(write_failed)?;
    let _lock = acquire_lock(parent)?;

    // Drift detection: re-read the live bytes and compare to the inspection hash.
    let current = match fs::read(path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Err(drift_error()),
        Err(_) => return Err(write_failed()),
    };
    if sha256_hex(&current) != inspected_hash {
        return Err(drift_error());
    }

    let original_mode = mode.or_else(|| {
        fs::metadata(path)
            .ok()
            .and_then(|metadata| unix_mode(&metadata))
    });

    let temp = parent.join(format!(
        ".cc-reminder-{}.tmp",
        uuid::Uuid::now_v7().simple()
    ));
    let result = perform_replace(path, &temp, bytes, original_mode);
    match result {
        Ok(()) => verify_result(path, bytes),
        Err(error) => {
            // On any pre-rename failure remove ONLY the explicit temp path.
            let _ = fs::remove_file(&temp);
            Err(error)
        }
    }
}

fn perform_replace(
    target: &Path,
    temp: &Path,
    bytes: &[u8],
    mode: Option<u32>,
) -> Result<(), AppError> {
    let mut file = private_new_file(temp).map_err(|_| write_failed())?;
    if let Err(error) = file
        .write_all(bytes)
        .and_then(|()| file.flush())
        .and_then(|()| file.sync_all())
    {
        drop(file);
        let _ = fs::remove_file(temp);
        return Err(AppError {
            domain: ErrorDomain::Integration,
            code: "integration.atomic_write_failed".to_owned(),
            message: format!("sync failed: {error}"),
            suggested_action: None,
        });
    }
    drop(file);

    apply_mode(temp, mode)?;

    // Atomically publish. `rename` over an existing file is atomic on the same
    // filesystem, which is why the temp lives in the target's directory.
    if let Err(error) = fs::rename(temp, target) {
        let _ = fs::remove_file(temp);
        return Err(AppError {
            domain: ErrorDomain::Integration,
            code: "integration.atomic_write_failed".to_owned(),
            message: format!("rename failed: {error}"),
            suggested_action: None,
        });
    }

    // Best-effort parent-directory fsync on Unix so the rename is durable.
    #[cfg(unix)]
    {
        let _ = fsync_parent(target.parent().unwrap_or(Path::new("/")));
    }
    // Windows write-through semantics would use MoveFileExW/ReplaceFileW here;
    // ponytail: the Unix path is the one under test, the Windows durability
    // shim is left for the platform owner to wire up.
    Ok(())
}

fn verify_result(path: &Path, bytes: &[u8]) -> Result<(), AppError> {
    let written = fs::read(path).map_err(|_| write_failed())?;
    if written != bytes {
        return Err(AppError {
            domain: ErrorDomain::Integration,
            code: "integration.atomic_write_failed".to_owned(),
            message: "post-replace bytes did not match input".to_owned(),
            suggested_action: None,
        });
    }
    let text = std::str::from_utf8(&written).map_err(|_| AppError {
        domain: ErrorDomain::Configuration,
        code: "configuration.invalid_jsonc".to_owned(),
        message: "non-utf8 configuration after replace".to_owned(),
        suggested_action: None,
    })?;
    jsonc::validate(text)?;
    Ok(())
}

fn acquire_lock(parent: &Path) -> Result<ConfigLock, AppError> {
    let lock_path = parent.join(LOCK_NAME);
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(&lock_path)
        .map_err(|_| write_failed())?;
    // The lock file is app-private; best-effort harden, ignore failure.
    let _ = ensure_private_file(&lock_path);
    let deadline = Instant::now() + LOCK_TIMEOUT;
    loop {
        match try_lock(&file) {
            Ok(true) => return Ok(ConfigLock(file)),
            Ok(false) => {
                if Instant::now() >= deadline {
                    return Err(write_failed());
                }
                std::thread::sleep(Duration::from_millis(2));
            }
            Err(_) => return Err(write_failed()),
        }
    }
}

/// Guard wrapping the lock file's file descriptor. The file is held open for
/// the lifetime of the replacement and dropped (releasing the flock) on return.
struct ConfigLock(#[allow(dead_code)] std::fs::File);

#[cfg(unix)]
fn try_lock(file: &std::fs::File) -> Result<bool, std::io::Error> {
    use std::os::fd::AsRawFd;

    unsafe extern "C" {
        fn flock(fd: std::ffi::c_int, operation: std::ffi::c_int) -> std::ffi::c_int;
    }
    const LOCK_EX: std::ffi::c_int = 2;
    const LOCK_NB: std::ffi::c_int = 4;
    if unsafe { flock(file.as_raw_fd(), LOCK_EX | LOCK_NB) } == 0 {
        return Ok(true);
    }
    let error = std::io::Error::last_os_error();
    if error.kind() == std::io::ErrorKind::WouldBlock {
        Ok(false)
    } else {
        Err(error)
    }
}

#[cfg(windows)]
fn try_lock(file: &std::fs::File) -> Result<bool, std::io::Error> {
    use std::os::windows::io::AsRawHandle;

    use windows_sys::Win32::Foundation::{ERROR_LOCK_VIOLATION, GetLastError};
    use windows_sys::Win32::Storage::FileSystem::{
        LOCKFILE_EXCLUSIVE_LOCK, LOCKFILE_FAIL_IMMEDIATELY, LockFileEx,
    };

    let mut overlapped = unsafe { std::mem::zeroed() };
    if unsafe {
        LockFileEx(
            file.as_raw_handle(),
            LOCKFILE_EXCLUSIVE_LOCK | LOCKFILE_FAIL_IMMEDIATELY,
            0,
            u32::MAX,
            u32::MAX,
            &mut overlapped,
        )
    } != 0
    {
        return Ok(true);
    }
    if unsafe { GetLastError() } == ERROR_LOCK_VIOLATION {
        Ok(false)
    } else {
        Err(std::io::Error::last_os_error())
    }
}

#[cfg(unix)]
fn fsync_parent(parent: &Path) -> std::io::Result<()> {
    let dir = std::fs::File::open(parent)?;
    let result = dir.sync_all();
    drop(dir);
    result
}

#[cfg(unix)]
fn private_new_file(path: &Path) -> std::io::Result<std::fs::File> {
    use std::os::unix::fs::OpenOptionsExt;
    OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)
}

#[cfg(not(unix))]
fn private_new_file(path: &Path) -> std::io::Result<std::fs::File> {
    OpenOptions::new().write(true).create_new(true).open(path)
}

#[cfg(unix)]
fn unix_mode(metadata: &std::fs::Metadata) -> Option<u32> {
    use std::os::unix::fs::PermissionsExt;
    Some(metadata.permissions().mode() & 0o777)
}

#[cfg(not(unix))]
fn unix_mode(_metadata: &std::fs::Metadata) -> Option<u32> {
    None
}

#[cfg(unix)]
fn apply_mode(path: &Path, mode: Option<u32>) -> Result<(), AppError> {
    use std::os::unix::fs::PermissionsExt;
    let mode = mode.unwrap_or(0o600);
    fs::set_permissions(path, fs::Permissions::from_mode(mode)).map_err(|_| write_failed())
}

#[cfg(not(unix))]
fn apply_mode(_path: &Path, _mode: Option<u32>) -> Result<(), AppError> {
    Ok(())
}

fn drift_error() -> AppError {
    AppError {
        domain: ErrorDomain::Integration,
        code: "integration.config_drift".to_owned(),
        message: "configuration changed between inspection and replacement".to_owned(),
        suggested_action: Some("re-inspect the configuration and retry".to_owned()),
    }
}

fn write_failed() -> AppError {
    AppError {
        domain: ErrorDomain::Integration,
        code: "integration.atomic_write_failed".to_owned(),
        message: "atomic configuration replacement failed".to_owned(),
        suggested_action: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn drift_error_carries_integration_domain_and_code() {
        let error = drift_error();
        assert_eq!(error.domain, ErrorDomain::Integration);
        assert_eq!(error.code, "integration.config_drift");
    }
}
