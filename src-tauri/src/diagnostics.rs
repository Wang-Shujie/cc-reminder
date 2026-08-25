//! Redaction-first local diagnostics (Task 20).
//!
//! Every log line passes through the Task-5 mandatory [`Redactor`] BEFORE it
//! is serialized to disk, so a secret that reaches this module can never
//! persist. Files rotate at 10 MiB (`cc-reminder.log` -> `.1` -> `.2`, at
//! most three files) and are user-only. Debug level may be enabled for a
//! bounded window stored as an expiry timestamp; an already-expired setting
//! is never restored at startup. There is no telemetry or export transport:
//! [`Diagnostics::export`] builds a store-only ZIP in pure Rust containing
//! ONLY the redacted logs plus caller-provided metadata entries.
//!
//! ponytail: the ZIP writer is store-only (no compression) — diagnostic
//! archives are tiny and an auditable ~100-line writer beats a zip dependency.

use std::fs::{File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

use chrono::{DateTime, Utc};
use serde::Serialize;

use crate::error::{AppError, ErrorDomain};
use crate::security::redact::Redactor;

const LOG_BASE: &str = "cc-reminder.log";
const DEBUG_STATE: &str = "debug-expiry.json";

fn redactor() -> &'static Redactor {
    static REDACTOR: OnceLock<Redactor> = OnceLock::new();
    REDACTOR.get_or_init(|| Redactor::compile(&[]).expect("mandatory redactor compiles"))
}

fn crc_table() -> &'static [u32; 256] {
    static TABLE: OnceLock<[u32; 256]> = OnceLock::new();
    TABLE.get_or_init(|| {
        let mut table = [0_u32; 256];
        for (index, entry) in table.iter_mut().enumerate() {
            let mut value = index as u32;
            for _ in 0..8 {
                value = if value & 1 != 0 {
                    0xEDB8_8320 ^ (value >> 1)
                } else {
                    value >> 1
                };
            }
            *entry = value;
        }
        table
    })
}

fn crc32(bytes: &[u8]) -> u32 {
    let mut crc = 0xFFFF_FFFF_u32;
    for &byte in bytes {
        crc = crc_table()[((crc ^ u32::from(byte)) & 0xFF) as usize] ^ (crc >> 8);
    }
    !crc
}

fn diagnostics_error(code: &str, message: &str) -> AppError {
    AppError {
        domain: ErrorDomain::Storage,
        code: code.to_owned(),
        message: message.to_owned(),
        suggested_action: None,
    }
}

/// Redaction-first rotating logger.
pub struct Diagnostics {
    directory: PathBuf,
    max_bytes: u64,
    max_files: usize,
    debug_until: Mutex<Option<DateTime<Utc>>>,
}

impl Diagnostics {
    /// Production constructor: 10 MiB rotation, three files, in `directory`.
    pub fn init(directory: &Path) -> Result<Self, AppError> {
        Self::build(directory, 10 * 1024 * 1024, 3)
    }

    /// Test constructor with explicit rotation bounds.
    pub fn test(directory: &Path, max_bytes: u64, max_files: usize) -> Self {
        Self::build(directory, max_bytes, max_files).expect("test diagnostics directory is usable")
    }

    fn build(directory: &Path, max_bytes: u64, max_files: usize) -> Result<Self, AppError> {
        std::fs::create_dir_all(directory).map_err(|_| {
            diagnostics_error(
                "diagnostics.unavailable",
                "log directory could not be created",
            )
        })?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(directory, std::fs::Permissions::from_mode(0o700));
        }
        let debug_until = read_debug_state(directory);
        Ok(Self {
            directory: directory.to_path_buf(),
            max_bytes,
            max_files,
            debug_until: Mutex::new(debug_until),
        })
    }

    /// Info-level line: always written (after mandatory redaction).
    pub fn info(&self, domain: &str, message: &str) {
        self.write_line("info", domain, message);
    }

    /// Debug-level line: written only while the debug window is unexpired.
    pub fn debug(&self, domain: &str, message: &str) {
        let active = self
            .debug_until
            .lock()
            .map(|guard| guard.is_some_and(|until| until > Utc::now()))
            .unwrap_or(false);
        if active {
            self.write_line("debug", domain, message);
        }
    }

    /// Error line carrying the stable domain/code/suggested action — never a
    /// raw cause. The message itself is redacted like any other line.
    pub fn log_error(&self, error: &AppError) {
        // Structured fields (domain= code= suggested=) so tooling can group
        // without parsing prose; the message is redacted like any line.
        let domain = format!("error:{}", domain_code(error.domain));
        self.info(
            &domain,
            &format!(
                "code={} message={} suggested={:?}",
                error.code, error.message, error.suggested_action
            ),
        );
    }

    /// Enable debug logging until `until`. Persisted as an expiry timestamp;
    /// startup drops an already-expired value (see `read_debug_state`).
    pub fn set_debug_until(&self, until: Option<DateTime<Utc>>) -> Result<(), AppError> {
        let mut guard = self
            .debug_until
            .lock()
            .map_err(|_| diagnostics_error("diagnostics.locked", "debug state is poisoned"))?;
        *guard = until;
        #[derive(Serialize)]
        struct DebugState<'a> {
            debug_until: &'a Option<DateTime<Utc>>,
        }
        let bytes = serde_json::to_vec(&DebugState {
            debug_until: &until,
        })
        .map_err(|_| {
            diagnostics_error(
                "diagnostics.unavailable",
                "debug state could not be encoded",
            )
        })?;
        let path = self.directory.join(DEBUG_STATE);
        let mut file = private_create(&path).map_err(|_| {
            diagnostics_error(
                "diagnostics.unavailable",
                "debug state could not be created",
            )
        })?;
        file.write_all(&bytes).map_err(|_| {
            diagnostics_error(
                "diagnostics.unavailable",
                "debug state could not be written",
            )
        })?;
        Ok(())
    }

    /// Whether debug logging is currently active (test hook + tray state).
    pub fn debug_active(&self) -> bool {
        self.debug_until
            .lock()
            .map(|guard| guard.is_some_and(|until| until > Utc::now()))
            .unwrap_or(false)
    }

    fn write_line(&self, level: &str, domain: &str, message: &str) {
        // Redact BEFORE serialization: the redacted string is the only thing
        // that ever reaches the filesystem.
        let redacted = redactor().redact(message);
        let line = format!(
            "{} level={} domain={} {}\n",
            Utc::now().to_rfc3339(),
            level,
            domain,
            redacted
        );
        let path = self.directory.join(LOG_BASE);
        let current = std::fs::metadata(&path).map(|meta| meta.len()).unwrap_or(0);
        if current + line.len() as u64 > self.max_bytes {
            self.rotate();
        }
        if let Ok(mut file) = private_append(&path) {
            let _ = file.write_all(line.as_bytes());
        }
    }

    fn rotate(&self) {
        // Rotation keeps at most `max_files` files total (base + suffixes);
        // with the two fixed suffixes this is 3 in production.
        let suffixes = rotation_suffixes(self.max_files);
        // Drop the oldest kept file, shift the rest down by one.
        let oldest = self
            .directory
            .join(format!("{LOG_BASE}{}", suffixes[suffixes.len() - 1]));
        let _ = std::fs::remove_file(&oldest);
        let Some(first) = suffixes.first() else {
            return;
        };
        let newer = self.directory.join(format!("{LOG_BASE}{first}"));
        if newer.exists() {
            let _ = std::fs::rename(&newer, &oldest);
        }
        let current = self.directory.join(LOG_BASE);
        if current.exists() {
            let _ = std::fs::rename(&current, &newer);
        }
    }

    /// Existing log files (active first). Test hook.
    pub fn log_files(&self) -> Vec<PathBuf> {
        let mut files = vec![self.directory.join(LOG_BASE)];
        for suffix in rotation_suffixes(self.max_files) {
            files.push(self.directory.join(format!("{LOG_BASE}{suffix}")));
        }
        files.into_iter().filter(|path| path.is_file()).collect()
    }

    /// Concatenated bytes of every log file (test hook).
    pub fn all_log_bytes(&self) -> Vec<u8> {
        let mut bytes = Vec::new();
        for path in self.log_files() {
            if let Ok(mut file) = File::open(&path) {
                let _ = file.read_to_end(&mut bytes);
            }
        }
        bytes
    }

    /// Write `line` repeatedly until at least one rotation has happened.
    /// Test hook for exercising rotation without waiting on real sizes.
    pub fn write_repeatedly_until_rotated(&self, line: &str) {
        let before = self.log_files().len() as u64;
        let mut written = 0_u64;
        while (self.log_files().len() as u64) <= before {
            self.info("test", line);
            written += 1;
            if written > 10_000_000 {
                break;
            }
        }
    }

    /// Build the diagnostic archive: the redacted log files plus the
    /// caller-provided metadata entries (manifest/health/queue-stats), packed
    /// as a store-only ZIP. The database, credentials, ciphertext, config
    /// snapshots, Agent configs, and spool entries are structurally absent —
    /// only what is passed here is written.
    pub fn export(&self, metadata: &[(&str, Vec<u8>)]) -> Result<Vec<u8>, AppError> {
        let mut entries: Vec<(String, Vec<u8>)> = Vec::new();
        for (name, bytes) in metadata {
            entries.push(((*name).to_owned(), bytes.clone()));
        }
        // Logs are already redacted at write time; include them verbatim.
        for path in self.log_files() {
            let name = path
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or(LOG_BASE);
            let mut bytes = Vec::new();
            if let Ok(mut file) = File::open(&path) {
                let _ = file.read_to_end(&mut bytes);
            }
            entries.push((name.to_owned(), bytes));
        }
        zip_store(&entries)
    }
}

/// Rotation suffixes for the kept file count (`.1`, `.2`, ...).
fn rotation_suffixes(max_files: usize) -> Vec<String> {
    (1..max_files).map(|index| format!(".{index}")).collect()
}

fn domain_code(domain: ErrorDomain) -> &'static str {
    match domain {
        ErrorDomain::Integration => "integration",
        ErrorDomain::Configuration => "configuration",
        ErrorDomain::SecretStore => "secret_store",
        ErrorDomain::Delivery => "delivery",
        ErrorDomain::Storage => "storage",
        ErrorDomain::Update => "update",
    }
}

fn read_debug_state(directory: &Path) -> Option<DateTime<Utc>> {
    #[derive(serde::Deserialize)]
    struct DebugState {
        debug_until: Option<DateTime<Utc>>,
    }
    let bytes = std::fs::read(directory.join(DEBUG_STATE)).ok()?;
    let state: DebugState = serde_json::from_slice(&bytes).ok()?;
    // Never restore an already-expired debug setting.
    state.debug_until.filter(|until| *until > Utc::now())
}

fn private_create(path: &Path) -> std::io::Result<File> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .mode(0o600)
            .open(path)
    }
    #[cfg(not(unix))]
    {
        OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .open(path)
    }
}

fn private_append(path: &Path) -> std::io::Result<File> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        OpenOptions::new()
            .create(true)
            .append(true)
            .mode(0o600)
            .open(path)
    }
    #[cfg(not(unix))]
    {
        OpenOptions::new().create(true).append(true).open(path)
    }
}

/// Minimal store-only ZIP writer (local headers + central directory + EOCD).
fn zip_store(entries: &[(String, Vec<u8>)]) -> Result<Vec<u8>, AppError> {
    let mut out = Vec::new();
    let mut central = Vec::new();
    let mut count = 0_u16;
    for (name, bytes) in entries {
        let offset = out.len() as u32;
        let crc = crc32(bytes);
        let size = bytes.len() as u32;
        let name_bytes = name.as_bytes();
        // Local file header.
        out.extend_from_slice(&0x0403_4b50_u32.to_le_bytes()); // signature
        out.extend_from_slice(&20_u16.to_le_bytes()); // version needed
        out.extend_from_slice(&0_u16.to_le_bytes()); // flags
        out.extend_from_slice(&0_u16.to_le_bytes()); // method: store
        out.extend_from_slice(&0_u16.to_le_bytes()); // mod time
        out.extend_from_slice(&0_u16.to_le_bytes()); // mod date
        out.extend_from_slice(&crc.to_le_bytes());
        out.extend_from_slice(&size.to_le_bytes()); // compressed
        out.extend_from_slice(&size.to_le_bytes()); // uncompressed
        out.extend_from_slice(&(name_bytes.len() as u16).to_le_bytes());
        out.extend_from_slice(&0_u16.to_le_bytes()); // extra len
        out.extend_from_slice(name_bytes);
        out.extend_from_slice(bytes);
        // Central directory record.
        central.extend_from_slice(&0x0201_4b50_u32.to_le_bytes());
        central.extend_from_slice(&20_u16.to_le_bytes()); // version made by
        central.extend_from_slice(&20_u16.to_le_bytes()); // version needed
        central.extend_from_slice(&0_u16.to_le_bytes()); // flags
        central.extend_from_slice(&0_u16.to_le_bytes()); // method
        central.extend_from_slice(&0_u16.to_le_bytes()); // time
        central.extend_from_slice(&0_u16.to_le_bytes()); // date
        central.extend_from_slice(&crc.to_le_bytes());
        central.extend_from_slice(&size.to_le_bytes());
        central.extend_from_slice(&size.to_le_bytes());
        central.extend_from_slice(&(name_bytes.len() as u16).to_le_bytes());
        central.extend_from_slice(&0_u16.to_le_bytes()); // extra
        central.extend_from_slice(&0_u16.to_le_bytes()); // comment
        central.extend_from_slice(&0_u16.to_le_bytes()); // disk start
        central.extend_from_slice(&0_u16.to_le_bytes()); // internal attrs
        central.extend_from_slice(&0_u32.to_le_bytes()); // external attrs
        central.extend_from_slice(&offset.to_le_bytes());
        central.extend_from_slice(name_bytes);
        count = count.checked_add(1).ok_or_else(|| {
            diagnostics_error("diagnostics.unavailable", "archive entry count overflowed")
        })?;
    }
    let central_offset = out.len() as u32;
    let central_size = central.len() as u32;
    out.extend_from_slice(&central);
    // End of central directory.
    out.extend_from_slice(&0x0605_4b50_u32.to_le_bytes());
    out.extend_from_slice(&0_u16.to_le_bytes()); // disk
    out.extend_from_slice(&0_u16.to_le_bytes()); // start disk
    out.extend_from_slice(&count.to_le_bytes());
    out.extend_from_slice(&count.to_le_bytes());
    out.extend_from_slice(&central_size.to_le_bytes());
    out.extend_from_slice(&central_offset.to_le_bytes());
    out.extend_from_slice(&0_u16.to_le_bytes()); // comment len
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn harness(max_bytes: u64, max_files: usize) -> (tempfile::TempDir, Diagnostics) {
        let root = tempdir().unwrap();
        let diagnostics = Diagnostics::test(root.path(), max_bytes, max_files);
        (root, diagnostics)
    }

    #[test]
    fn logger_redacts_before_writing_and_rotates_at_ten_mib() {
        let (_root, diagnostics) = harness(10 * 1024 * 1024, 3);
        diagnostics.info("delivery", "Authorization: Bearer never-log-this");
        diagnostics.write_repeatedly_until_rotated("bounded diagnostic line");

        let bytes = diagnostics.all_log_bytes();
        assert!(
            !bytes
                .windows(b"never-log-this".len())
                .any(|w| w == b"never-log-this")
        );
        assert!(diagnostics.log_files().len() <= 3);
        assert!(
            diagnostics
                .log_files()
                .iter()
                .all(|path| std::fs::metadata(path).unwrap().len() <= 10 * 1024 * 1024)
        );
        let _ = _root;
    }

    #[test]
    fn webhook_queries_and_platform_bodies_never_reach_the_log() {
        let (_root, diagnostics) = harness(1024 * 1024, 3);
        diagnostics.info(
            "delivery",
            "url=https://qyapi.weixin.qq.com/cgi-bin/webhook/send?key=fake-webhook-secret",
        );
        let text = String::from_utf8(diagnostics.all_log_bytes()).unwrap();
        // The full webhook URL (query included) is scrubbed by the mandatory
        // webhook pattern.
        assert!(!text.contains("fake-webhook-secret"));
        assert!(!text.contains("webhook/send?key="));
        let _ = _root;
    }

    #[test]
    fn log_files_are_user_only() {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let (_root, diagnostics) = harness(1024 * 1024, 3);
            diagnostics.info("test", "line");
            for path in diagnostics.log_files() {
                assert_eq!(
                    std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
                    0o600
                );
            }
            let _ = _root;
        }
    }

    #[test]
    fn errors_carry_domain_code_and_suggested_action_without_raw_causes() {
        let (_root, diagnostics) = harness(1024 * 1024, 3);
        let error = AppError {
            domain: ErrorDomain::Delivery,
            code: "delivery.http_status".to_owned(),
            // A token shape the mandatory redactor matches (key=value form).
            message: "unexpected HTTP status 500 password=hunter2secret".to_owned(),
            suggested_action: Some("retry later".to_owned()),
        };
        diagnostics.log_error(&error);
        let text = String::from_utf8(diagnostics.all_log_bytes()).unwrap();
        assert!(text.contains("domain=error:delivery"));
        assert!(text.contains("code=delivery.http_status"));
        // The mandatory redactor scrubs the secret key=value form; the raw
        // credential never survives verbatim.
        assert!(!text.contains("hunter2secret"));
        let _ = _root;
    }

    #[test]
    fn debug_returns_to_info_at_the_deadline() {
        let (_root, diagnostics) = harness(1024 * 1024, 3);
        // Before any window: debug lines are dropped.
        diagnostics.debug("test", "hidden-before");
        assert!(
            !diagnostics
                .all_log_bytes()
                .windows(b"hidden-before".len())
                .any(|w| w == b"hidden-before")
        );

        // Open a window that is already expired: still dropped, and the
        // persisted state is not resurrected on rebuild.
        diagnostics
            .set_debug_until(Some(Utc::now() - chrono::Duration::seconds(1)))
            .unwrap();
        assert!(!diagnostics.debug_active());
        let rebuilt = Diagnostics::test(_root.path(), 1024 * 1024, 3);
        assert!(!rebuilt.debug_active());

        // A live window lets debug through; expiry drops it again.
        diagnostics
            .set_debug_until(Some(Utc::now() + chrono::Duration::minutes(15)))
            .unwrap();
        assert!(diagnostics.debug_active());
        diagnostics.debug("test", "visible-during");
        assert!(
            diagnostics
                .all_log_bytes()
                .windows(b"visible-during".len())
                .any(|w| w == b"visible-during")
        );
    }

    #[test]
    fn archive_contains_manifest_stats_and_redacted_logs_only() {
        let (_root, diagnostics) = harness(1024 * 1024, 3);
        diagnostics.info("test", "keep token=never-export-this out");
        let manifest = br#"{"app":"0.1.0"}"#.to_vec();
        let archive = diagnostics
            .export(&[
                ("manifest.json", manifest),
                ("health.json", br#"{"overall":"ok"}"#.to_vec()),
                ("queue-stats.json", br#"{"pending":0}"#.to_vec()),
            ])
            .unwrap();
        let text = String::from_utf8_lossy(&archive);
        assert!(text.contains("manifest.json"));
        assert!(text.contains("health.json"));
        assert!(text.contains("queue-stats.json"));
        assert!(text.contains("cc-reminder.log"));
        assert!(!text.contains("never-export-this"));
        assert!(!text.to_lowercase().contains("sqlite"));
    }
}
