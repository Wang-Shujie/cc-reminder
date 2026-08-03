use std::path::Path;

use chrono::{DateTime, Utc};
use rusqlite::{OptionalExtension, TransactionBehavior, params};
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::Value;
use uuid::Uuid;

use crate::error::AppError;
use crate::model::{
    AgentInstallationRecord, AgentKind, ConfigSnapshotRecord, HookInstallationRecord,
};

use super::db::{Database, storage_error};

const MAX_SNAPSHOTS_PER_AGENT: usize = 5;
const MAX_HOOKS_PER_AGENT: usize = 128;
const MAX_HOOK_TEXT_BYTES: usize = 512;
const MAX_SNAPSHOT_CIPHERTEXT_BYTES: usize = 1024 * 1024;
const MAX_SNAPSHOT_NONCE_BYTES: usize = 64;
const MAX_SNAPSHOT_AAD_BYTES: usize = 512;
const MAX_SNAPSHOT_SOURCE_HASH_BYTES: usize = 128;

#[derive(Clone, Debug)]
pub struct IntegrationRepository {
    database: Database,
}

impl IntegrationRepository {
    pub const fn new(database: Database) -> Self {
        Self { database }
    }

    pub fn database_path(&self) -> &Path {
        self.database.path()
    }

    pub fn upsert_agent(&self, agent: &AgentInstallationRecord) -> Result<(), AppError> {
        let mut connection = self.database.connect()?;
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| write_error())?;
        transaction
            .execute(
                "INSERT INTO agent_installations (
                    agent, executable_path, version, capability_verification, health_status, last_checked_at
                 ) VALUES (?1, NULL, ?2, ?3, ?4, ?5)
                 ON CONFLICT(agent) DO UPDATE SET version = excluded.version,
                    capability_verification = excluded.capability_verification,
                    health_status = excluded.health_status, last_checked_at = excluded.last_checked_at",
                params![
                    agent.agent.as_str(),
                    agent.version.as_ref().map(ToString::to_string),
                    db_text(&agent.capability_verification)?,
                    db_text(&agent.health_status)?,
                    agent.last_checked_at.to_rfc3339(),
                ],
            )
            .map_err(|_| write_error())?;
        transaction.commit().map_err(|_| write_error())?;
        Ok(())
    }

    pub fn agent(&self, agent: AgentKind) -> Result<AgentInstallationRecord, AppError> {
        let connection = self.database.connect()?;
        connection
            .query_row(
                "SELECT agent, version, capability_verification, health_status, last_checked_at
                 FROM agent_installations WHERE agent = ?1",
                [agent.as_str()],
                agent_row,
            )
            .optional()
            .map_err(|_| query_error())?
            .ok_or_else(not_found)
    }

    pub fn replace_hooks(
        &self,
        agent: AgentKind,
        hooks: &[HookInstallationRecord],
    ) -> Result<(), AppError> {
        if hooks.len() > MAX_HOOKS_PER_AGENT
            || hooks.iter().any(|hook| {
                hook.agent != agent
                    || !valid_hook_text(&hook.source_event)
                    || !valid_hook_text(&hook.command_fingerprint)
                    || !valid_hook_text(&hook.definition_fingerprint)
                    || !valid_hook_text(&hook.helper_version)
                    || !valid_hook_text(&hook.config_hash)
            })
        {
            return Err(storage_error(
                "storage.invalid_hook",
                "hook installation is invalid",
            ));
        }
        let mut connection = self.database.connect()?;
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| write_error())?;
        transaction
            .execute(
                "DELETE FROM hook_installations WHERE agent = ?1",
                [agent.as_str()],
            )
            .map_err(|_| write_error())?;
        for hook in hooks {
            insert_hook(&transaction, hook)?;
        }
        transaction.commit().map_err(|_| write_error())?;
        Ok(())
    }

    pub fn hook(
        &self,
        agent: AgentKind,
        source_event: &str,
    ) -> Result<HookInstallationRecord, AppError> {
        let connection = self.database.connect()?;
        connection
            .query_row(
                "SELECT agent, source_event, command_fingerprint, definition_fingerprint,
                    helper_version, config_hash, trust_status, health_status, last_seen_at
                 FROM hook_installations WHERE agent = ?1 AND source_event = ?2",
                params![agent.as_str(), source_event],
                hook_row,
            )
            .optional()
            .map_err(|_| query_error())?
            .ok_or_else(not_found)
    }

    pub fn list_hooks(&self, agent: AgentKind) -> Result<Vec<HookInstallationRecord>, AppError> {
        let connection = self.database.connect()?;
        let mut statement = connection
            .prepare(
                "SELECT agent, source_event, command_fingerprint, definition_fingerprint,
                    helper_version, config_hash, trust_status, health_status, last_seen_at
                 FROM hook_installations WHERE agent = ?1 ORDER BY source_event LIMIT 129",
            )
            .map_err(|_| query_error())?;
        let hooks = statement
            .query_map([agent.as_str()], hook_row)
            .map_err(|_| query_error())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|_| stored_data_error())?;
        if hooks.len() > MAX_HOOKS_PER_AGENT {
            return Err(storage_error(
                "storage.list_limit_exceeded",
                "hook list is too large",
            ));
        }
        Ok(hooks)
    }

    pub fn mark_hook_seen(
        &self,
        agent: AgentKind,
        source_event: &str,
        command_fingerprint: &str,
        seen_at: DateTime<Utc>,
    ) -> Result<(), AppError> {
        let mut connection = self.database.connect()?;
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| write_error())?;
        transaction
            .execute(
                "UPDATE hook_installations SET
                    last_seen_at = ?1,
                    observed_command_fingerprint = ?2,
                    trust_status = CASE trust_status
                        WHEN 'needs_user_confirmation' THEN 'observed_working'
                        ELSE trust_status
                    END,
                    updated_at = ?1
                 WHERE agent = ?3 AND source_event = ?4 AND command_fingerprint = ?2",
                params![
                    seen_at.to_rfc3339(),
                    command_fingerprint,
                    agent.as_str(),
                    source_event,
                ],
            )
            .map_err(|_| write_error())?;
        transaction.commit().map_err(|_| write_error())?;
        Ok(())
    }

    pub fn save_snapshot(&self, snapshot: &ConfigSnapshotRecord) -> Result<(), AppError> {
        if snapshot.ciphertext.is_empty()
            || snapshot.ciphertext.len() > MAX_SNAPSHOT_CIPHERTEXT_BYTES
            || snapshot.nonce.is_empty()
            || snapshot.nonce.len() > MAX_SNAPSHOT_NONCE_BYTES
            || snapshot.aad.is_empty()
            || snapshot.aad.len() > MAX_SNAPSHOT_AAD_BYTES
            || snapshot.source_hash.is_empty()
            || snapshot.source_hash.len() > MAX_SNAPSHOT_SOURCE_HASH_BYTES
        {
            return Err(storage_error(
                "storage.invalid_snapshot",
                "encrypted snapshot is invalid",
            ));
        }
        let mut connection = self.database.connect()?;
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| write_error())?;
        transaction
            .execute(
                "INSERT INTO config_snapshots (
                    id, agent, config_path, hook_subtree_ciphertext, nonce, aad, source_hash,
                    file_mode, created_at
                 ) VALUES (?1, ?2, 'managed', ?3, ?4, ?5, ?6, ?7, ?8)",
                params![
                    snapshot.id.to_string(),
                    snapshot.agent.as_str(),
                    snapshot.ciphertext,
                    snapshot.nonce,
                    snapshot.aad,
                    snapshot.source_hash,
                    snapshot.file_mode,
                    snapshot.created_at.to_rfc3339(),
                ],
            )
            .map_err(|_| write_error())?;
        transaction
            .execute(
                "DELETE FROM config_snapshots
                 WHERE agent = ?1 AND id IN (
                    SELECT id FROM config_snapshots WHERE agent = ?1
                    ORDER BY created_at DESC, id DESC LIMIT -1 OFFSET ?2
                 )",
                params![snapshot.agent.as_str(), MAX_SNAPSHOTS_PER_AGENT as i64],
            )
            .map_err(|_| write_error())?;
        transaction.commit().map_err(|_| write_error())?;
        Ok(())
    }

    /// This is the explicit installer recovery read; normal status DTOs omit ciphertext.
    pub fn latest_snapshot(&self, agent: AgentKind) -> Result<ConfigSnapshotRecord, AppError> {
        let connection = self.database.connect()?;
        connection
            .query_row(
                "SELECT id, agent, hook_subtree_ciphertext, nonce, aad, source_hash, file_mode, created_at
                 FROM config_snapshots WHERE agent = ?1 ORDER BY created_at DESC, id DESC LIMIT 1",
                [agent.as_str()],
                snapshot_row,
            )
            .optional()
            .map_err(|_| query_error())?
            .ok_or_else(not_found)
    }

    pub fn snapshot_count(&self, agent: AgentKind) -> Result<usize, AppError> {
        let connection = self.database.connect()?;
        let count: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM config_snapshots WHERE agent = ?1",
                [agent.as_str()],
                |row| row.get(0),
            )
            .map_err(|_| query_error())?;
        usize::try_from(count).map_err(|_| stored_data_error())
    }
}

fn valid_hook_text(value: &str) -> bool {
    !value.is_empty() && value.len() <= MAX_HOOK_TEXT_BYTES
}

fn insert_hook(
    transaction: &rusqlite::Transaction<'_>,
    hook: &HookInstallationRecord,
) -> Result<(), AppError> {
    transaction
        .execute(
            "INSERT INTO hook_installations (
                agent, source_event, command_fingerprint, definition_fingerprint, helper_version,
                config_hash, trust_status, health_status, last_seen_at,
                observed_command_fingerprint, updated_at
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, NULL, ?10)",
            params![
                hook.agent.as_str(),
                hook.source_event,
                hook.command_fingerprint,
                hook.definition_fingerprint,
                hook.helper_version,
                hook.config_hash,
                db_text(&hook.trust_status)?,
                db_text(&hook.health_status)?,
                hook.last_seen_at.map(|time| time.to_rfc3339()),
                Utc::now().to_rfc3339(),
            ],
        )
        .map_err(|_| write_error())?;
    Ok(())
}

fn agent_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<AgentInstallationRecord> {
    let agent: String = row.get(0)?;
    let version: Option<String> = row.get(1)?;
    let verification: String = row.get(2)?;
    let health: String = row.get(3)?;
    let checked: String = row.get(4)?;
    stored_result((|| -> Result<_, AppError> {
        Ok(AgentInstallationRecord {
            agent: db_parse(&agent)?,
            version: version
                .as_deref()
                .map(semver::Version::parse)
                .transpose()
                .map_err(|_| stored_data_error())?,
            capability_verification: db_parse(&verification)?,
            health_status: db_parse(&health)?,
            last_checked_at: parse_time(&checked)?,
        })
    })())
}

fn hook_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<HookInstallationRecord> {
    let agent: String = row.get(0)?;
    let source_event = row.get(1)?;
    let command_fingerprint = row.get(2)?;
    let definition_fingerprint = row.get(3)?;
    let helper_version = row.get(4)?;
    let config_hash = row.get(5)?;
    let trust: String = row.get(6)?;
    let health: String = row.get(7)?;
    let last_seen: Option<String> = row.get(8)?;
    stored_result((|| -> Result<_, AppError> {
        Ok(HookInstallationRecord {
            agent: db_parse(&agent)?,
            source_event,
            command_fingerprint,
            definition_fingerprint,
            helper_version,
            config_hash,
            trust_status: db_parse(&trust)?,
            health_status: db_parse(&health)?,
            last_seen_at: last_seen.as_deref().map(parse_time).transpose()?,
        })
    })())
}

fn snapshot_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<ConfigSnapshotRecord> {
    let id: String = row.get(0)?;
    let agent: String = row.get(1)?;
    let ciphertext = row.get(2)?;
    let nonce = row.get(3)?;
    let aad = row.get(4)?;
    let source_hash = row.get(5)?;
    let file_mode = row.get(6)?;
    let created: String = row.get(7)?;
    stored_result((|| -> Result<_, AppError> {
        Ok(ConfigSnapshotRecord {
            id: Uuid::parse_str(&id).map_err(|_| stored_data_error())?,
            agent: db_parse(&agent)?,
            ciphertext,
            nonce,
            aad,
            source_hash,
            file_mode,
            created_at: parse_time(&created)?,
        })
    })())
}

fn db_text<T: Serialize>(value: &T) -> Result<String, AppError> {
    match serde_json::to_value(value).map_err(|_| serialization_error())? {
        Value::String(value) => Ok(value),
        _ => Err(serialization_error()),
    }
}

fn db_parse<T: DeserializeOwned>(value: &str) -> Result<T, AppError> {
    serde_json::from_value(Value::String(value.to_owned())).map_err(|_| stored_data_error())
}

fn parse_time(value: &str) -> Result<DateTime<Utc>, AppError> {
    DateTime::parse_from_rfc3339(value)
        .map(|time| time.with_timezone(&Utc))
        .map_err(|_| stored_data_error())
}

fn stored_result<T>(value: Result<T, AppError>) -> rusqlite::Result<T> {
    value.map_err(|_| rusqlite::Error::InvalidQuery)
}

fn not_found() -> AppError {
    storage_error("storage.not_found", "integration record was not found")
}

fn serialization_error() -> AppError {
    storage_error(
        "storage.serialization_failed",
        "typed storage value could not be serialized",
    )
}

fn stored_data_error() -> AppError {
    storage_error(
        "storage.invalid_stored_data",
        "stored data could not be decoded",
    )
}

fn query_error() -> AppError {
    storage_error("storage.query_failed", "database query failed")
}

fn write_error() -> AppError {
    storage_error("storage.write_failed", "database write failed")
}

#[cfg(test)]
mod tests {
    use chrono::{DateTime, TimeZone, Utc};
    use tempfile::{TempDir, tempdir};
    use uuid::Uuid;

    use crate::events::catalog::CatalogVerification;
    use crate::model::{
        AgentInstallationRecord, AgentKind, ConfigSnapshotRecord, HookInstallationRecord,
        InstallationHealth, TrustStatus,
    };
    use crate::storage::db::Database;
    use crate::storage::integrations::IntegrationRepository;

    #[test]
    fn observed_hook_updates_only_the_matching_expected_fingerprint() {
        let (_root, repository) = integration_repository_with_two_hooks();
        repository
            .mark_hook_seen(AgentKind::Codex, "Stop", "expected-fingerprint", now())
            .unwrap();

        assert_eq!(
            repository
                .hook(AgentKind::Codex, "Stop")
                .unwrap()
                .trust_status,
            TrustStatus::ObservedWorking
        );
        assert_eq!(
            repository
                .hook(AgentKind::Codex, "PermissionRequest")
                .unwrap()
                .trust_status,
            TrustStatus::NeedsUserConfirmation
        );
    }

    #[test]
    fn command_fingerprint_mismatch_leaves_trust_and_last_seen_unchanged() {
        let (_root, repository) = integration_repository_with_two_hooks();
        repository
            .mark_hook_seen(AgentKind::Codex, "Stop", "wrong-fingerprint", now())
            .unwrap();

        let hook = repository.hook(AgentKind::Codex, "Stop").unwrap();
        assert_eq!(hook.trust_status, TrustStatus::NeedsUserConfirmation);
        assert_eq!(hook.last_seen_at, None);
    }

    #[test]
    fn agent_detection_upsert_and_hook_replacement_persist_full_typed_state() {
        let (_root, repository) = test_integration_repository();
        let agent = AgentInstallationRecord {
            agent: AgentKind::Codex,
            version: Some(semver::Version::new(0, 145, 0)),
            capability_verification: CatalogVerification::Exact,
            health_status: InstallationHealth::Healthy,
            last_checked_at: now(),
        };
        repository.upsert_agent(&agent).unwrap();
        repository
            .replace_hooks(AgentKind::Codex, &[hook("Stop")])
            .unwrap();

        assert_eq!(repository.agent(AgentKind::Codex).unwrap(), agent);
        let stored = repository.hook(AgentKind::Codex, "Stop").unwrap();
        assert_eq!(stored.helper_version, "1.0.0");
        assert_eq!(stored.config_hash, "config-hash");
        assert_eq!(stored.definition_fingerprint, "definition-fingerprint");
    }

    #[test]
    fn snapshot_repository_has_no_plaintext_hook_subtree_api() {
        let (_root, repository) = test_integration_repository();
        let snapshot = encrypted_snapshot_fixture();
        repository.save_snapshot(&snapshot).unwrap();

        let stored = repository.latest_snapshot(AgentKind::ClaudeCode).unwrap();
        assert_eq!(stored.ciphertext, snapshot.ciphertext);
        assert!(!format!("{stored:?}").contains("foreign hook command"));
    }

    #[test]
    fn snapshot_retention_keeps_only_the_latest_five_per_agent() {
        let (_root, repository) = test_integration_repository();
        for second in 0..6 {
            let mut snapshot = encrypted_snapshot_fixture();
            snapshot.id = Uuid::now_v7();
            snapshot.created_at = Utc.with_ymd_and_hms(2026, 8, 3, 12, 0, second).unwrap();
            repository.save_snapshot(&snapshot).unwrap();
        }

        assert_eq!(repository.snapshot_count(AgentKind::ClaudeCode).unwrap(), 5);
    }

    #[test]
    fn hook_replacement_and_snapshot_writes_reject_unbounded_input() {
        let (_root, repository) = test_integration_repository();
        let hooks = (0..129)
            .map(|index| hook(&format!("Event{index}")))
            .collect::<Vec<_>>();
        assert_eq!(
            repository
                .replace_hooks(AgentKind::Codex, &hooks)
                .unwrap_err()
                .code,
            "storage.invalid_hook"
        );

        let mut snapshot = encrypted_snapshot_fixture();
        snapshot.ciphertext = vec![0; 1_048_577];
        assert_eq!(
            repository.save_snapshot(&snapshot).unwrap_err().code,
            "storage.invalid_snapshot"
        );
    }

    fn test_integration_repository() -> (TempDir, IntegrationRepository) {
        let root = tempdir().unwrap();
        let database = Database::open(
            &root
                .path()
                .join("com.ccreminder.app")
                .join("cc-reminder.sqlite3"),
        )
        .unwrap();
        (root, IntegrationRepository::new(database))
    }

    fn integration_repository_with_two_hooks() -> (TempDir, IntegrationRepository) {
        let (root, repository) = test_integration_repository();
        repository
            .replace_hooks(AgentKind::Codex, &[hook("Stop"), hook("PermissionRequest")])
            .unwrap();
        (root, repository)
    }

    fn hook(source_event: &str) -> HookInstallationRecord {
        HookInstallationRecord {
            agent: AgentKind::Codex,
            source_event: source_event.to_owned(),
            command_fingerprint: "expected-fingerprint".to_owned(),
            definition_fingerprint: "definition-fingerprint".to_owned(),
            helper_version: "1.0.0".to_owned(),
            config_hash: "config-hash".to_owned(),
            trust_status: TrustStatus::NeedsUserConfirmation,
            health_status: InstallationHealth::Healthy,
            last_seen_at: None,
        }
    }

    fn encrypted_snapshot_fixture() -> ConfigSnapshotRecord {
        ConfigSnapshotRecord {
            id: Uuid::now_v7(),
            agent: AgentKind::ClaudeCode,
            ciphertext: b"ciphertext-only".to_vec(),
            nonce: b"unique-nonce".to_vec(),
            aad: "cc-reminder:snapshot:fixture:hooks".to_owned(),
            source_hash: "source-hash".to_owned(),
            file_mode: Some(0o600),
            created_at: now(),
        }
    }

    fn now() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 8, 3, 12, 0, 0).unwrap()
    }
}
