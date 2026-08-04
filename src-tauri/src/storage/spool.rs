use crate::events::normalize::SafeIngressEvent;
use crate::ipc::protocol::{MAX_SAFE_ENVELOPE_BYTES, MAX_SPOOL_FILES};
use crate::security::permissions::{ensure_private_directory, ensure_private_file};
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::PathBuf;
use uuid::Uuid;

#[derive(Clone, Debug)]
pub struct Spool {
    pub root: PathBuf,
}

impl Spool {
    pub fn new(root: impl Into<PathBuf>) -> Result<Self, crate::error::AppError> {
        let root = root.into();
        ensure_private_directory(&root)?;
        ensure_private_directory(&root.join("rejected"))?;
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
        if self.files(false).map_err(|_| invalid("spool read"))?.len() >= MAX_SPOOL_FILES {
            return Err(invalid("spool full"));
        }
        let id = event.event_id;
        let tmp = self.root.join(format!("{id}.json.tmp"));
        let final_path = self.root.join(format!("{id}.json"));
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&tmp)
            .map_err(|_| invalid("spool write"))?;
        file.write_all(&bytes).map_err(|_| invalid("spool write"))?;
        file.sync_all().map_err(|_| invalid("spool write"))?;
        ensure_private_file(&tmp)?;
        fs::rename(tmp, final_path).map_err(|_| invalid("spool rename"))
    }
    pub fn drain(&self, limit: usize) -> Result<Vec<SafeIngressEvent>, crate::error::AppError> {
        let mut out = Vec::new();
        for path in self
            .files(false)
            .map_err(|_| invalid("spool read"))?
            .into_iter()
            .take(limit)
        {
            let processing = path.with_extension("processing");
            if fs::rename(&path, &processing).is_err() {
                continue;
            }
            let parsed = fs::read(&processing)
                .ok()
                .and_then(|bytes| serde_json::from_slice::<SafeIngressEvent>(&bytes).ok());
            if let Some(event) = parsed {
                out.push(event);
                let _ = fs::remove_file(processing);
            } else {
                let _ = fs::rename(
                    processing,
                    self.root
                        .join("rejected")
                        .join(format!("{}.json", Uuid::now_v7())),
                );
            }
        }
        Ok(out)
    }
    fn files(&self, processing: bool) -> Result<Vec<PathBuf>, std::io::Error> {
        let suffix = if processing { ".processing" } else { ".json" };
        Ok(fs::read_dir(&self.root)?
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.extension().and_then(|e| e.to_str()) == Some(&suffix[1..]))
            .collect())
    }
}

fn invalid(message: &str) -> crate::error::AppError {
    crate::error::AppError {
        domain: crate::error::ErrorDomain::Storage,
        code: "storage.spool_failed".into(),
        message: message.into(),
        suggested_action: None,
    }
}
