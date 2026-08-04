use crate::events::normalize::SafeIngressEvent;
use crate::ipc::protocol::{MAX_SAFE_ENVELOPE_BYTES, MAX_SPOOL_FILES};
use crate::security::permissions::{
    ensure_current_user_dacl, ensure_private_directory, ensure_private_file,
};
use sha2::{Digest, Sha256};
use std::fs::{self, OpenOptions};
use std::io::{Read, Write};
use std::path::PathBuf;

#[derive(Clone, Debug)]
pub struct Spool {
    pub root: PathBuf,
}

impl Spool {
    pub fn new(root: impl Into<PathBuf>) -> Result<Self, crate::error::AppError> {
        let root = root.into();
        ensure_private_directory(&root)?;
        ensure_current_user_dacl(&root)?;
        ensure_private_directory(&root.join("rejected"))?;
        ensure_current_user_dacl(&root.join("rejected"))?;
        Ok(Self { root })
    }
    pub fn entries(&self) -> Result<Vec<PathBuf>, crate::error::AppError> {
        self.files(false).map_err(|_| invalid("spool read"))
    }
    pub fn only_entry(&self) -> PathBuf {
        self.files(false)
            .unwrap_or_default()
            .into_iter()
            .next()
            .unwrap()
    }
    pub fn write_exclusive(&self, event: &SafeIngressEvent) -> Result<(), crate::error::AppError> {
        let bytes = serde_json::to_vec(event).map_err(|_| invalid("serialization"))?;
        if bytes.len() > MAX_SAFE_ENVELOPE_BYTES {
            return Err(invalid("safe envelope too large"));
        }
        if self.is_full().map_err(|_| invalid("spool read"))? {
            return Err(invalid("spool full"));
        }
        let id = event.event_id;
        let tmp = self.root.join(format!("{id}.json.tmp"));
        let final_path = self.root.join(format!("{id}.json"));
        let mut file = private_new_file(&tmp).map_err(|_| invalid("spool write"))?;
        ensure_current_user_dacl(&tmp)?;
        file.write_all(&bytes).map_err(|_| invalid("spool write"))?;
        file.sync_all().map_err(|_| invalid("spool write"))?;
        ensure_private_file(&tmp)?;
        let published = fs::hard_link(&tmp, &final_path).map_err(|_| invalid("spool publish"));
        let _ = fs::remove_file(&tmp);
        published
    }
    pub fn drain(&self, limit: usize) -> Result<Vec<SafeIngressEvent>, crate::error::AppError> {
        let mut out = Vec::new();
        let database = self
            .root
            .parent()
            .ok_or_else(|| invalid("spool database unavailable"))?
            .join("cc-reminder.sqlite3");
        for path in self
            .candidates()
            .map_err(|_| invalid("spool read"))?
            .into_iter()
            .take(limit.min(MAX_SPOOL_FILES))
        {
            let processing =
                if path.extension().and_then(|value| value.to_str()) == Some("processing") {
                    path
                } else {
                    let processing = path.with_extension("processing");
                    if fs::rename(&path, &processing).is_err() {
                        continue;
                    }
                    processing
                };
            let parsed = read_bounded(&processing)
                .and_then(|bytes| serde_json::from_slice::<SafeIngressEvent>(&bytes).ok())
                .filter(|event| {
                    serde_json::to_vec(event)
                        .is_ok_and(|bytes| bytes.len() <= MAX_SAFE_ENVELOPE_BYTES)
                });
            if let Some(event) = parsed {
                crate::hook_command::insert_ingress(&database, &event)
                    .map_err(|_| invalid("spool ingress write"))?;
                out.push(event);
                fs::remove_file(processing).map_err(|_| invalid("spool delete"))?;
            } else {
                let name = processing
                    .file_name()
                    .map(|name| name.to_string_lossy())
                    .unwrap_or_default();
                let hash = hex::encode(Sha256::digest(name.as_bytes()));
                let rejected = self.root.join("rejected").join(format!("{hash}.json"));
                fs::rename(processing, &rejected).map_err(|_| invalid("spool reject"))?;
                ensure_private_file(&rejected)?;
            }
        }
        Ok(out)
    }

    fn candidates(&self) -> Result<Vec<PathBuf>, std::io::Error> {
        let mut candidates = Vec::new();
        for entry in fs::read_dir(&self.root)?.take(MAX_SPOOL_FILES + 1) {
            let path = entry?.path();
            if matches!(
                path.extension().and_then(|value| value.to_str()),
                Some("processing" | "json")
            ) {
                candidates.push(path);
            }
        }
        candidates.sort_by_key(|path| {
            (
                path.extension().and_then(|value| value.to_str()) != Some("processing"),
                path.clone(),
            )
        });
        Ok(candidates)
    }

    fn is_full(&self) -> Result<bool, std::io::Error> {
        let mut seen = 0;
        let mut spool_files = 0;
        for entry in fs::read_dir(&self.root)?.take(MAX_SPOOL_FILES + 1) {
            seen += 1;
            let path = entry?.path();
            if matches!(
                path.extension().and_then(|value| value.to_str()),
                Some("processing" | "json")
            ) {
                spool_files += 1;
            }
        }
        Ok(seen > MAX_SPOOL_FILES || spool_files >= MAX_SPOOL_FILES)
    }
    fn files(&self, processing: bool) -> Result<Vec<PathBuf>, std::io::Error> {
        let suffix = if processing { ".processing" } else { ".json" };
        Ok(fs::read_dir(&self.root)?
            .take(MAX_SPOOL_FILES + 1)
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.extension().and_then(|e| e.to_str()) == Some(&suffix[1..]))
            .collect())
    }
}

fn read_bounded(path: &std::path::Path) -> Option<Vec<u8>> {
    let mut bytes = Vec::new();
    std::fs::File::open(path)
        .ok()?
        .take((MAX_SAFE_ENVELOPE_BYTES + 1) as u64)
        .read_to_end(&mut bytes)
        .ok()?;
    (bytes.len() <= MAX_SAFE_ENVELOPE_BYTES).then_some(bytes)
}

#[cfg(unix)]
fn private_new_file(path: &std::path::Path) -> std::io::Result<std::fs::File> {
    use std::os::unix::fs::OpenOptionsExt;

    OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)
}

#[cfg(not(unix))]
fn private_new_file(path: &std::path::Path) -> std::io::Result<std::fs::File> {
    OpenOptions::new().write(true).create_new(true).open(path)
}

fn invalid(message: &str) -> crate::error::AppError {
    crate::error::AppError {
        domain: crate::error::ErrorDomain::Storage,
        code: "storage.spool_failed".into(),
        message: message.into(),
        suggested_action: None,
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use chrono::Utc;
    use semver::Version;
    use tempfile::tempdir;
    use uuid::Uuid;

    use super::Spool;
    use crate::events::normalize::SafeIngressEvent;
    use crate::model::AgentKind;
    use crate::storage::db::Database;

    fn event(id: Uuid) -> SafeIngressEvent {
        SafeIngressEvent {
            event_id: id,
            source: AgentKind::Codex,
            source_version: Version::new(0, 145, 0),
            source_event: "Stop".into(),
            occurred_at: Utc::now(),
            received_at: Utc::now(),
            project_id: None,
            project_display_name: Some("app".into()),
            cwd_fingerprint: None,
            session_ref: None,
            turn_ref: None,
            public_fields: BTreeMap::new(),
        }
    }

    fn fixture() -> (tempfile::TempDir, Database, Spool) {
        let root = tempdir().unwrap();
        let app = root.path().join("com.ccreminder.app");
        let database = Database::open(&app.join("cc-reminder.sqlite3")).unwrap();
        let spool = Spool::new(app.join("spool")).unwrap();
        (root, database, spool)
    }

    #[test]
    fn drain_inserts_before_delete_and_replay_is_idempotent() {
        let (_root, database, spool) = fixture();
        let safe = event(Uuid::now_v7());
        spool.write_exclusive(&safe).unwrap();

        let drained = spool.drain(100).unwrap();
        assert_eq!(drained.len(), 1);
        assert_eq!(drained[0].event_id, safe.event_id);
        assert!(spool.entries().unwrap().is_empty());
        assert_eq!(
            database
                .connect()
                .unwrap()
                .query_row("SELECT COUNT(*) FROM ingress_events", [], |row| row
                    .get::<_, i64>(0))
                .unwrap(),
            1
        );

        let processing = spool.root.join(format!("{}.processing", safe.event_id));
        std::fs::write(&processing, serde_json::to_vec(&safe).unwrap()).unwrap();
        let replayed = spool.drain(100).unwrap();
        assert_eq!(replayed.len(), 1);
        assert_eq!(replayed[0].event_id, safe.event_id);
        assert!(!processing.exists());
        assert_eq!(
            database
                .connect()
                .unwrap()
                .query_row("SELECT COUNT(*) FROM ingress_events", [], |row| row
                    .get::<_, i64>(0))
                .unwrap(),
            1
        );
    }

    #[test]
    fn invalid_claim_is_rejected_under_a_hashed_name() {
        let (_root, _database, spool) = fixture();
        let source = spool.root.join("secret-source.processing");
        std::fs::write(&source, b"not-json").unwrap();

        assert!(spool.drain(100).unwrap().is_empty());

        let rejected = std::fs::read_dir(spool.root.join("rejected"))
            .unwrap()
            .next()
            .unwrap()
            .unwrap()
            .file_name()
            .to_string_lossy()
            .into_owned();
        assert!(!rejected.contains("secret-source"));
        assert_eq!(rejected.len(), 69);
    }

    #[test]
    fn write_exclusive_refuses_an_existing_event_file() {
        let (_root, _database, spool) = fixture();
        let safe = event(Uuid::now_v7());
        spool.write_exclusive(&safe).unwrap();
        let original = std::fs::read(spool.only_entry()).unwrap();

        let error = spool.write_exclusive(&safe).unwrap_err();

        assert_eq!(error.code, "storage.spool_failed");
        assert_eq!(std::fs::read(spool.only_entry()).unwrap(), original);
        assert_eq!(spool.entries().unwrap().len(), 1);
    }
}
