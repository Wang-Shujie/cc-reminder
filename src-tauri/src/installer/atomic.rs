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
use crate::security::permissions::{ensure_current_user_dacl, ensure_private_file};

const LOCK_NAME: &str = ".cc-reminder-config.lock";
const LOCK_TIMEOUT: Duration = Duration::from_secs(2);

/// Publish the temp by renaming it over the target. Routed through a seam so the
/// rename-failure cleanup path can be exercised under the `test-support` feature
/// (production builds always call the real rename).
///
/// v2-issues(Windows write-through):Unix 上同文件系统 rename 原子、耐久性由
/// 调用方先行的 fsync 覆盖;Windows 走 MoveFileExW + WRITE_THROUGH,让替换
/// 落盘后才返回,关闭崩溃窗口。需 Windows 实机验收(macOS 无法编译验证)。
pub(crate) fn publish_rename(temp: &Path, target: &Path) -> std::io::Result<()> {
    #[cfg(feature = "test-support")]
    if RENAME_FAIL.with(|flag| flag.get()) {
        return Err(std::io::Error::from_raw_os_error(5));
    }
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        const MOVEFILE_REPLACE_EXISTING: u32 = 0x0000_0001;
        const MOVEFILE_WRITE_THROUGH: u32 = 0x0000_0002;
        // MOVEFILE_REPLACE_EXISTING 需要对目标的 DELETE 访问;杀毒/索引器/编辑器
        // 常短暂持有文件且不共享删除,Unix rename 无此限制。对这些瞬态冲突做
        // 有界重试(ponytail: 固定 5×20ms,不区分错误来源)。
        const RETRIES: usize = 5;
        const RETRY_DELAY: std::time::Duration = std::time::Duration::from_millis(20);
        const ERROR_ACCESS_DENIED: u32 = 5;
        const ERROR_SHARING_VIOLATION: u32 = 32;
        let mut temp_w: Vec<u16> = temp.as_os_str().encode_wide().collect();
        let mut target_w: Vec<u16> = target.as_os_str().encode_wide().collect();
        temp_w.push(0);
        target_w.push(0);
        // SAFETY: 两个 NUL 结尾的 UTF-16 路径指针仅在本调用内使用。
        for attempt in 0..=RETRIES {
            let moved = unsafe {
                windows_sys::Win32::Storage::FileSystem::MoveFileExW(
                    temp_w.as_ptr(),
                    target_w.as_ptr(),
                    MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
                )
            };
            if moved != 0 {
                return Ok(());
            }
            let error = std::io::Error::last_os_error();
            let transient = matches!(
                error.raw_os_error(),
                Some(code)
                    if code as u32 == ERROR_ACCESS_DENIED
                        || code as u32 == ERROR_SHARING_VIOLATION
            );
            if !transient || attempt == RETRIES {
                return Err(error);
            }
            std::thread::sleep(RETRY_DELAY);
        }
        unreachable!("loop returns on every branch")
    }
    #[cfg(not(windows))]
    {
        fs::rename(temp, target)
    }
}

#[cfg(feature = "test-support")]
std::thread_local! {
    static RENAME_FAIL: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Test-only seam: force `publish_rename` to fail until toggled back off, so the
/// rename-failure cleanup path (original bytes untouched, temp removed) can be
/// asserted. Has no effect in production builds.
#[cfg(feature = "test-support")]
pub fn force_rename_failure_for_test(on: bool) {
    RENAME_FAIL.with(|flag| flag.set(on));
}

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
    // Harden the temp's permissions before writing any payload. On Unix this is
    // an idempotent 0o600 (the create already used that mode); on Windows it
    // applies the current-user-only DACL the security brief requires, mirroring
    // storage/spool.rs and security/crypto.rs. A failure here is treated as a
    // pre-write atomic failure so the target is never disturbed.
    ensure_current_user_dacl(temp).map_err(|_| write_failed())?;
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
    if let Err(error) = publish_rename(temp, target) {
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
    // Windows 侧耐久性已由 publish_rename 的 MOVEFILE_WRITE_THROUGH 覆盖,
    // 无需父目录 fsync(NTFS 上也无对应语义)。
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
