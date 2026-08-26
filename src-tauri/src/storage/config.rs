use std::collections::BTreeMap;
use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::Path;

use chrono::{DateTime, Utc};
use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::Value;
use uuid::Uuid;

use crate::error::{AppError, ErrorDomain};
use crate::events::catalog::{CapabilityCatalog, catalogued_hooks};
use crate::model::{
    AgentKind, AppSettings, ChannelId, ChannelKind, ChannelPublicConfig, ChannelRecord, PatchField,
    ProjectCacheHealth, ProjectId, ProjectMatchCacheFile, ProjectMatchCacheProject,
    ProjectPathKind, ProjectPathRecord, ProjectRecord, RuleConfig, RulePatch,
};
use crate::rules::{StoredGlobalRule, StoredRulePatch, default_rule, resolve_rule, validate_rule};
use crate::security::permissions::{ensure_current_user_dacl, ensure_private_file};

use super::db::{Database, storage_error};

const SETTINGS_KEY: &str = "settings";
const PROJECT_CACHE_HEALTH_KEY: &str = "project_path_cache_health";
const CACHE_FILE_NAME: &str = "project-paths.json";
const MAX_PROJECT_CACHE_BYTES: usize = 1024 * 1024;
const MAX_RETENTION_DAYS: u16 = 365;
const MAX_CATALOGS: usize = 2;
const MAX_HOOKS_PER_CATALOG: usize = 64;
const MAX_SOURCE_EVENT_BYTES: usize = 128;
const MAX_CONFIG_LIST_ITEMS: usize = 200;
const MAX_EFFECTIVE_RULE_ROWS: usize = 10_000;
const MAX_PROJECT_NAME_BYTES: usize = 256;
const MAX_PROJECT_PATH_BYTES: usize = 4_096;
const MAX_CHANNEL_NAME_BYTES: usize = 256;
const MAX_CREDENTIAL_REF_BYTES: usize = 256;
const MAX_PROJECT_CACHE_PROJECTS: usize = MAX_CONFIG_LIST_ITEMS;
const MAX_PROJECT_CACHE_PATHS_PER_PROJECT: usize = MAX_CONFIG_LIST_ITEMS;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GlobalRuleSeedReport {
    pub inserted: usize,
}

#[derive(Clone, Debug)]
pub struct ConfigRepository {
    database: Database,
}

impl ConfigRepository {
    pub const fn new(database: Database) -> Self {
        Self { database }
    }

    pub fn database_path(&self) -> &Path {
        self.database.path()
    }

    pub fn ensure_global_rules(
        &self,
        catalogs: &[CapabilityCatalog],
    ) -> Result<GlobalRuleSeedReport, AppError> {
        if catalogs.len() > MAX_CATALOGS
            || catalogs.iter().any(|catalog| {
                catalog.hooks.len() > MAX_HOOKS_PER_CATALOG
                    || catalog
                        .hooks
                        .iter()
                        .any(|hook| !valid_source_event(&hook.source_event))
            })
        {
            return Err(configuration_error(
                "catalog_invalid",
                "capability catalog is invalid",
            ));
        }
        let mut connection = self.database.connect()?;
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| write_error())?;
        let mut inserted = 0;
        for catalog in catalogs {
            for hook in &catalog.hooks {
                let config =
                    serde_json::to_string(&default_rule(catalog.agent, &hook.source_event))
                        .map_err(|_| serialization_error())?;
                inserted += transaction
                    .execute(
                        "INSERT OR IGNORE INTO global_rules (
                            id, agent, source_event, version, config_json, updated_at
                         ) VALUES (?1, ?2, ?3, 1, ?4, ?5)",
                        params![
                            Uuid::now_v7().to_string(),
                            agent_text(catalog.agent),
                            hook.source_event,
                            config,
                            now_text(),
                        ],
                    )
                    .map_err(|_| write_error())?;
            }
        }
        transaction.commit().map_err(|_| write_error())?;
        Ok(GlobalRuleSeedReport { inserted })
    }

    pub fn list_global_rules(&self) -> Result<Vec<StoredGlobalRule>, AppError> {
        let connection = self.database.connect()?;
        let mut statement = connection
            .prepare(
                "SELECT id, agent, source_event, version, config_json
                 FROM global_rules ORDER BY agent, source_event LIMIT 201",
            )
            .map_err(|_| query_error())?;
        let rules = statement
            .query_map([], global_rule_row)
            .map_err(|_| query_error())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|_| stored_data_error())?;
        bounded_list(rules)
    }

    pub fn get_global_rule(
        &self,
        agent: AgentKind,
        source_event: &str,
    ) -> Result<StoredGlobalRule, AppError> {
        let connection = self.database.connect()?;
        connection
            .query_row(
                "SELECT id, agent, source_event, version, config_json
                 FROM global_rules WHERE agent = ?1 AND source_event = ?2",
                params![agent_text(agent), source_event],
                global_rule_row,
            )
            .optional()
            .map_err(|_| query_error())?
            .ok_or_else(not_found)
    }

    pub fn save_global_rule(&self, rule: &StoredGlobalRule) -> Result<(), AppError> {
        validate_rule(&rule.config)?;
        let mut connection = self.database.connect()?;
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| write_error())?;
        validate_capability(&transaction, rule.agent, &rule.source_event)?;
        validate_targets(&transaction, &rule.config)?;
        let version = transaction
            .query_row(
                "SELECT version FROM global_rules WHERE agent = ?1 AND source_event = ?2",
                params![agent_text(rule.agent), rule.source_event],
                |row| row.get::<_, i64>(0),
            )
            .optional()
            .map_err(|_| query_error())?
            .unwrap_or(0);
        let version = u64::try_from(version).map_err(|_| stored_data_error())? + 1;
        transaction
            .execute(
                "INSERT INTO global_rules (id, agent, source_event, version, config_json, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)
                 ON CONFLICT(agent, source_event) DO UPDATE SET
                    id = excluded.id, version = excluded.version, config_json = excluded.config_json,
                    updated_at = excluded.updated_at",
                params![
                    rule.id.to_string(),
                    agent_text(rule.agent),
                    rule.source_event,
                    version as i64,
                    serde_json::to_string(&rule.config).map_err(|_| serialization_error())?,
                    now_text(),
                ],
            )
            .map_err(|_| write_error())?;
        transaction.commit().map_err(|_| write_error())?;
        Ok(())
    }

    pub fn save_project_patch(
        &self,
        project_id: ProjectId,
        agent: AgentKind,
        source_event: &str,
        patch: &RulePatch,
    ) -> Result<(), AppError> {
        validate_patch(patch)?;
        let mut connection = self.database.connect()?;
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| write_error())?;
        validate_capability(&transaction, agent, source_event)?;
        ensure_project(&transaction, project_id)?;
        validate_patch_targets(&transaction, patch)?;
        save_patch(&transaction, project_id, agent, source_event, patch)?;
        transaction.commit().map_err(|_| write_error())?;
        Ok(())
    }

    pub fn get_project_patch(
        &self,
        project_id: ProjectId,
        agent: AgentKind,
        source_event: &str,
    ) -> Result<StoredRulePatch, AppError> {
        let connection = self.database.connect()?;
        connection
            .query_row(
                "SELECT project_id, agent, source_event, version, patch_json
                 FROM project_rule_overrides
                 WHERE project_id = ?1 AND agent = ?2 AND source_event = ?3",
                params![project_id.to_string(), agent_text(agent), source_event],
                patch_row,
            )
            .optional()
            .map_err(|_| query_error())?
            .ok_or_else(not_found)
    }

    pub fn reset_project_patch_field(
        &self,
        project_id: ProjectId,
        agent: AgentKind,
        source_event: &str,
        field: PatchField,
    ) -> Result<(), AppError> {
        let mut connection = self.database.connect()?;
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| write_error())?;
        let stored = transaction
            .query_row(
                "SELECT project_id, agent, source_event, version, patch_json
                 FROM project_rule_overrides
                 WHERE project_id = ?1 AND agent = ?2 AND source_event = ?3",
                params![project_id.to_string(), agent_text(agent), source_event],
                patch_row,
            )
            .optional()
            .map_err(|_| query_error())?
            .ok_or_else(not_found)?;
        let mut patch = stored.patch;
        clear_patch_field(&mut patch, field);
        if patch_is_empty(&patch) {
            transaction
                .execute(
                    "DELETE FROM project_rule_overrides
                     WHERE project_id = ?1 AND agent = ?2 AND source_event = ?3",
                    params![project_id.to_string(), agent_text(agent), source_event],
                )
                .map_err(|_| write_error())?;
        } else {
            save_patch(&transaction, project_id, agent, source_event, &patch)?;
        }
        transaction.commit().map_err(|_| write_error())?;
        Ok(())
    }

    pub fn save_project(&self, project: &ProjectRecord) -> Result<(), AppError> {
        if project.name.trim().is_empty()
            || project.name.len() > MAX_PROJECT_NAME_BYTES
            || !valid_project_path(&project.canonical_root)
        {
            return Err(configuration_error("project_invalid", "project is invalid"));
        }
        let mut connection = self.database.connect()?;
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| write_error())?;
        transaction
            .execute(
                "INSERT INTO projects (id, name, canonical_root, worktree_mode, created_at, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)
                 ON CONFLICT(id) DO UPDATE SET name = excluded.name, canonical_root = excluded.canonical_root,
                    worktree_mode = excluded.worktree_mode, updated_at = excluded.updated_at",
                params![
                    project.id.to_string(), project.name, path_text(&project.canonical_root)?,
                    db_text(&project.worktree_mode)?, project.created_at.to_rfc3339(), project.updated_at.to_rfc3339(),
                ],
            )
            .map_err(|_| write_error())?;
        transaction
            .execute(
                "INSERT INTO project_paths (id, project_id, canonical_path, kind)
                 VALUES (?1, ?2, ?3, ?4)
                 ON CONFLICT(id) DO UPDATE SET
                   project_id = excluded.project_id, canonical_path = excluded.canonical_path,
                   kind = excluded.kind",
                params![
                    project.id.to_string(),
                    project.id.to_string(),
                    path_text(&project.canonical_root)?,
                    db_text(&ProjectPathKind::Root)?,
                ],
            )
            .map_err(map_path_error)?;
        transaction.commit().map_err(|_| write_error())?;
        self.write_project_cache()
    }

    pub fn get_project(&self, project_id: ProjectId) -> Result<ProjectRecord, AppError> {
        let connection = self.database.connect()?;
        connection
            .query_row(
                "SELECT id, name, canonical_root, worktree_mode, created_at, updated_at
                 FROM projects WHERE id = ?1",
                [project_id.to_string()],
                project_row,
            )
            .optional()
            .map_err(|_| query_error())?
            .ok_or_else(not_found)
    }

    pub fn list_projects(&self) -> Result<Vec<ProjectRecord>, AppError> {
        let connection = self.database.connect()?;
        let mut statement = connection
            .prepare("SELECT id, name, canonical_root, worktree_mode, created_at, updated_at FROM projects ORDER BY name, id LIMIT 201")
            .map_err(|_| query_error())?;
        let projects = statement
            .query_map([], project_row)
            .map_err(|_| query_error())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|_| stored_data_error())?;
        bounded_list(projects)
    }

    pub fn delete_project(&self, project_id: ProjectId) -> Result<(), AppError> {
        let mut connection = self.database.connect()?;
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| write_error())?;
        if transaction
            .execute(
                "DELETE FROM projects WHERE id = ?1",
                [project_id.to_string()],
            )
            .map_err(|_| write_error())?
            == 0
        {
            return Err(not_found());
        }
        transaction.commit().map_err(|_| write_error())?;
        self.write_project_cache()
    }

    pub fn save_project_path(&self, path: &ProjectPathRecord) -> Result<(), AppError> {
        if !valid_project_path(&path.canonical_path) {
            return Err(configuration_error(
                "path_invalid",
                "project path is invalid",
            ));
        }
        let mut connection = self.database.connect()?;
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| write_error())?;
        ensure_project(&transaction, path.project_id)?;
        transaction.execute(
            "INSERT INTO project_paths (id, project_id, canonical_path, kind) VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(id) DO UPDATE SET project_id = excluded.project_id,
                 canonical_path = excluded.canonical_path, kind = excluded.kind",
            params![path.id.to_string(), path.project_id.to_string(), path_text(&path.canonical_path)?, db_text(&path.kind)?],
        ).map_err(map_path_error)?;
        transaction.commit().map_err(|_| write_error())?;
        self.write_project_cache()
    }

    pub fn list_project_paths(
        &self,
        project_id: ProjectId,
    ) -> Result<Vec<ProjectPathRecord>, AppError> {
        let connection = self.database.connect()?;
        let mut statement = connection.prepare(
            "SELECT id, project_id, canonical_path, kind FROM project_paths WHERE project_id = ?1 ORDER BY canonical_path LIMIT 201",
        ).map_err(|_| query_error())?;
        let paths = statement
            .query_map([project_id.to_string()], project_path_row)
            .map_err(|_| query_error())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|_| stored_data_error())?;
        bounded_list(paths)
    }

    pub fn delete_project_path(&self, path_id: Uuid) -> Result<(), AppError> {
        let mut connection = self.database.connect()?;
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| write_error())?;
        let kind = transaction
            .query_row(
                "SELECT kind FROM project_paths WHERE id = ?1",
                [path_id.to_string()],
                |row| row.get::<_, String>(0),
            )
            .optional()
            .map_err(|_| query_error())?
            .ok_or_else(not_found)?;
        if db_parse::<ProjectPathKind>(&kind)? == ProjectPathKind::Root {
            return Err(configuration_error(
                "root_path_required",
                "a project root cannot be removed",
            ));
        }
        transaction
            .execute(
                "DELETE FROM project_paths WHERE id = ?1",
                [path_id.to_string()],
            )
            .map_err(|_| write_error())?;
        transaction.commit().map_err(|_| write_error())?;
        self.write_project_cache()
    }

    pub fn save_channel(&self, channel: &ChannelRecord) -> Result<(), AppError> {
        validate_channel(channel)?;
        let mut connection = self.database.connect()?;
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| write_error())?;
        transaction.execute(
            "INSERT INTO channels (id, kind, name, credential_ref, public_config_json, health_status,
                paused_reason_code, consecutive_auth_failures, last_succeeded_at, next_allowed_at, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?11)
             ON CONFLICT(id) DO UPDATE SET kind = excluded.kind, name = excluded.name,
                credential_ref = excluded.credential_ref, public_config_json = excluded.public_config_json,
                health_status = excluded.health_status, paused_reason_code = excluded.paused_reason_code,
                consecutive_auth_failures = excluded.consecutive_auth_failures,
                last_succeeded_at = excluded.last_succeeded_at, next_allowed_at = excluded.next_allowed_at,
                updated_at = excluded.updated_at",
            params![
                channel.id.to_string(), db_text(&channel.kind)?, channel.name, channel.credential_ref,
                serde_json::to_string(&channel.public_config).map_err(|_| serialization_error())?,
                db_text(&channel.health_status)?, channel.paused_reason_code,
                channel.consecutive_auth_failures, channel.last_succeeded_at.map(|time| time.to_rfc3339()),
                channel.next_allowed_at.map(|time| time.to_rfc3339()), now_text(),
            ],
        ).map_err(map_channel_error)?;
        transaction.commit().map_err(|_| write_error())?;
        Ok(())
    }

    pub fn get_channel(&self, channel_id: ChannelId) -> Result<ChannelRecord, AppError> {
        let connection = self.database.connect()?;
        connection.query_row(
            "SELECT id, kind, name, credential_ref, public_config_json, health_status, paused_reason_code,
                consecutive_auth_failures, last_succeeded_at, next_allowed_at FROM channels WHERE id = ?1",
            [channel_id.to_string()], channel_row,
        ).optional().map_err(|_| query_error())?.ok_or_else(not_found)
    }

    pub fn list_channels(&self) -> Result<Vec<ChannelRecord>, AppError> {
        let connection = self.database.connect()?;
        let mut statement = connection.prepare(
            "SELECT id, kind, name, credential_ref, public_config_json, health_status, paused_reason_code,
                consecutive_auth_failures, last_succeeded_at, next_allowed_at FROM channels ORDER BY name, id LIMIT 201",
        ).map_err(|_| query_error())?;
        let channels = statement
            .query_map([], channel_row)
            .map_err(|_| query_error())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|_| stored_data_error())?;
        bounded_list(channels)
    }

    pub fn delete_channel(&self, channel_id: ChannelId) -> Result<(), AppError> {
        let mut connection = self.database.connect()?;
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| write_error())?;
        if !channel_exists(&transaction, channel_id)? {
            return Err(not_found());
        }
        if channel_is_targeted(&transaction, channel_id)? {
            return Err(configuration_error(
                "channel_in_use",
                "channel is targeted by a rule",
            ));
        }
        transaction
            .execute(
                "DELETE FROM channels WHERE id = ?1",
                [channel_id.to_string()],
            )
            .map_err(|_| write_error())?;
        transaction.commit().map_err(|_| write_error())?;
        Ok(())
    }

    pub fn get_settings(&self) -> Result<AppSettings, AppError> {
        let connection = self.database.connect()?;
        let value = connection
            .query_row(
                "SELECT value_json FROM app_settings WHERE key = ?1",
                [SETTINGS_KEY],
                |row| row.get::<_, String>(0),
            )
            .optional()
            .map_err(|_| query_error())?;
        value
            .map(|value| serde_json::from_str(&value).map_err(|_| stored_data_error()))
            .transpose()
            .map(|settings| settings.unwrap_or_default())
    }

    pub fn save_settings(&self, settings: &AppSettings) -> Result<AppSettings, AppError> {
        validate_settings(settings)?;
        let mut connection = self.database.connect()?;
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| write_error())?;
        transaction.execute(
            "INSERT INTO app_settings (key, value_json, updated_at) VALUES (?1, ?2, ?3)
             ON CONFLICT(key) DO UPDATE SET value_json = excluded.value_json, updated_at = excluded.updated_at",
            params![SETTINGS_KEY, serde_json::to_string(settings).map_err(|_| serialization_error())?, now_text()],
        ).map_err(|_| write_error())?;
        transaction.commit().map_err(|_| write_error())?;
        Ok(settings.clone())
    }

    pub fn project_cache_health(&self) -> Result<ProjectCacheHealth, AppError> {
        let connection = self.database.connect()?;
        let value = connection
            .query_row(
                "SELECT value_json FROM app_settings WHERE key = ?1",
                [PROJECT_CACHE_HEALTH_KEY],
                |row| row.get::<_, String>(0),
            )
            .optional()
            .map_err(|_| query_error())?;
        value
            .map(|value| serde_json::from_str(&value).map_err(|_| stored_data_error()))
            .transpose()
            .map(|health| health.unwrap_or(ProjectCacheHealth::Healthy))
    }

    pub fn regenerate_project_cache(&self) -> Result<(), AppError> {
        self.write_project_cache()
    }

    fn write_project_cache(&self) -> Result<(), AppError> {
        let result = self.write_project_cache_file();
        self.set_project_cache_health(if result.is_ok() {
            ProjectCacheHealth::Healthy
        } else {
            ProjectCacheHealth::RegenerationFailed
        })?;
        result
    }

    fn write_project_cache_file(&self) -> Result<(), AppError> {
        let connection = self.database.connect()?;
        let projects = list_cache_projects(&connection)?;
        let bytes = serde_json::to_vec(&ProjectMatchCacheFile {
            version: 1,
            projects,
        })
        .map_err(|_| serialization_error())?;
        if bytes.len() > MAX_PROJECT_CACHE_BYTES {
            return Err(storage_error(
                "storage.project_cache_too_large",
                "project cache exceeds its size limit",
            ));
        }
        let parent = self.database.path().parent().ok_or_else(|| {
            storage_error(
                "storage.project_cache_failed",
                "project cache path is unavailable",
            )
        })?;
        let target = parent.join(CACHE_FILE_NAME);
        let temporary = parent.join(format!(".{CACHE_FILE_NAME}.{}.tmp", Uuid::now_v7()));
        let result = (|| {
            let mut file = private_new_file(&temporary)?;
            file.write_all(&bytes).map_err(|_| cache_error())?;
            file.sync_all().map_err(|_| cache_error())?;
            drop(file);
            std::fs::rename(&temporary, &target).map_err(|_| cache_error())?;
            ensure_private_file(&target).map_err(|_| cache_error())?;
            ensure_current_user_dacl(&target).map_err(|_| cache_error())
        })();
        if result.is_err() {
            let _ = std::fs::remove_file(&temporary);
        }
        result
    }

    fn set_project_cache_health(&self, health: ProjectCacheHealth) -> Result<(), AppError> {
        let connection = self.database.connect()?;
        connection
            .execute(
                "INSERT INTO app_settings (key, value_json, updated_at) VALUES (?1, ?2, ?3)
                 ON CONFLICT(key) DO UPDATE SET value_json = excluded.value_json, updated_at = excluded.updated_at",
                params![PROJECT_CACHE_HEALTH_KEY, serde_json::to_string(&health).map_err(|_| serialization_error())?, now_text()],
            )
            .map_err(|_| write_error())?;
        Ok(())
    }
}

fn save_patch(
    transaction: &rusqlite::Transaction<'_>,
    project_id: ProjectId,
    agent: AgentKind,
    source_event: &str,
    patch: &RulePatch,
) -> Result<(), AppError> {
    let version = transaction.query_row(
        "SELECT version FROM project_rule_overrides WHERE project_id = ?1 AND agent = ?2 AND source_event = ?3",
        params![project_id.to_string(), agent_text(agent), source_event], |row| row.get::<_, i64>(0),
    ).optional().map_err(|_| query_error())?.unwrap_or(0);
    let version = u64::try_from(version).map_err(|_| stored_data_error())? + 1;
    transaction.execute(
        "INSERT INTO project_rule_overrides (project_id, agent, source_event, version, patch_json, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)
         ON CONFLICT(project_id, agent, source_event) DO UPDATE SET version = excluded.version,
             patch_json = excluded.patch_json, updated_at = excluded.updated_at",
        params![project_id.to_string(), agent_text(agent), source_event, version as i64,
            serde_json::to_string(patch).map_err(|_| serialization_error())?, now_text()],
    ).map_err(|_| write_error())?;
    Ok(())
}

fn validate_capability(
    connection: &Connection,
    agent: AgentKind,
    event: &str,
) -> Result<(), AppError> {
    if catalogued_hooks().contains(&(agent, event.to_owned()))
        || connection
            .query_row(
                "SELECT 1 FROM global_rules WHERE agent = ?1 AND source_event = ?2",
                params![agent_text(agent), event],
                |_| Ok(()),
            )
            .optional()
            .map_err(|_| query_error())?
            .is_some()
    {
        Ok(())
    } else {
        Err(configuration_error(
            "capability_unknown",
            "rule capability is not catalogued",
        ))
    }
}

fn valid_source_event(value: &str) -> bool {
    !value.is_empty() && value.len() <= MAX_SOURCE_EVENT_BYTES
}

fn validate_patch(patch: &RulePatch) -> Result<(), AppError> {
    if let Some(rule) = patch_to_rule(patch) {
        validate_rule(&rule)?;
    }
    Ok(())
}

fn patch_to_rule(patch: &RulePatch) -> Option<RuleConfig> {
    (patch.targets.is_some()
        || patch.filters.is_some()
        || patch.privacy.is_some()
        || patch.delivery.is_some()
        || patch.quiet_hours.is_some())
    .then(|| RuleConfig {
        enabled: patch.enabled.unwrap_or(false),
        targets: patch.targets.clone().unwrap_or_default(),
        filters: patch.filters.clone().unwrap_or_default(),
        privacy: patch
            .privacy
            .clone()
            .unwrap_or_else(|| default_rule(AgentKind::Codex, "Stop").privacy),
        delivery: patch
            .delivery
            .clone()
            .unwrap_or_else(|| default_rule(AgentKind::Codex, "Stop").delivery),
        quiet_hours: patch.quiet_hours.clone().unwrap_or(None),
    })
}

fn validate_targets(connection: &Connection, rule: &RuleConfig) -> Result<(), AppError> {
    for target in &rule.targets {
        if !channel_exists(connection, target.channel_id)? {
            return Err(configuration_error(
                "channel_not_found",
                "rule target channel does not exist",
            ));
        }
    }
    Ok(())
}

fn validate_patch_targets(connection: &Connection, patch: &RulePatch) -> Result<(), AppError> {
    if let Some(targets) = &patch.targets {
        for target in targets {
            if !channel_exists(connection, target.channel_id)? {
                return Err(configuration_error(
                    "channel_not_found",
                    "rule target channel does not exist",
                ));
            }
        }
    }
    Ok(())
}

fn ensure_project(connection: &Connection, project_id: ProjectId) -> Result<(), AppError> {
    connection
        .query_row(
            "SELECT 1 FROM projects WHERE id = ?1",
            [project_id.to_string()],
            |_| Ok(()),
        )
        .optional()
        .map_err(|_| query_error())?
        .ok_or_else(not_found)
        .map(|_| ())
}

fn channel_exists(connection: &Connection, channel_id: ChannelId) -> Result<bool, AppError> {
    connection
        .query_row(
            "SELECT 1 FROM channels WHERE id = ?1",
            [channel_id.to_string()],
            |_| Ok(()),
        )
        .optional()
        .map(|value| value.is_some())
        .map_err(|_| query_error())
}

fn channel_is_targeted(connection: &Connection, channel_id: ChannelId) -> Result<bool, AppError> {
    let id = channel_id.to_string();
    let globals = connection
        .prepare("SELECT agent, source_event, config_json FROM global_rules ORDER BY agent, source_event LIMIT 10001")
        .map_err(|_| query_error())?
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
            ))
        })
        .map_err(|_| query_error())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| query_error())?;
    if globals.len() > MAX_EFFECTIVE_RULE_ROWS {
        return Err(configuration_error(
            "list_limit_exceeded",
            "configuration list is too large",
        ));
    }
    let mut rules = BTreeMap::new();
    for (agent, source_event, config) in globals {
        let config: RuleConfig = serde_json::from_str(&config).map_err(|_| stored_data_error())?;
        let agent: AgentKind = db_parse(&agent)?;
        if config.enabled
            && config
                .targets
                .iter()
                .any(|target| target.channel_id.to_string() == id)
        {
            return Ok(true);
        }
        rules.insert((agent, source_event), config);
    }
    let patches = connection
        .prepare("SELECT agent, source_event, patch_json FROM project_rule_overrides ORDER BY agent, source_event LIMIT 10001")
        .map_err(|_| query_error())?
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
            ))
        })
        .map_err(|_| query_error())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| query_error())?;
    if patches.len() > MAX_EFFECTIVE_RULE_ROWS {
        return Err(configuration_error(
            "list_limit_exceeded",
            "configuration list is too large",
        ));
    }
    for (agent, source_event, patch) in patches {
        let patch: RulePatch = serde_json::from_str(&patch).map_err(|_| stored_data_error())?;
        let agent: AgentKind = db_parse(&agent)?;
        let global = rules
            .get(&(agent, source_event))
            .ok_or_else(stored_data_error)?;
        let effective = resolve_rule(global, Some(&patch));
        if effective.enabled
            && effective
                .targets
                .iter()
                .any(|target| target.channel_id.to_string() == id)
        {
            return Ok(true);
        }
    }
    Ok(false)
}

fn clear_patch_field(patch: &mut RulePatch, field: PatchField) {
    match field {
        PatchField::Enabled => patch.enabled = None,
        PatchField::Targets => patch.targets = None,
        PatchField::Filters => patch.filters = None,
        PatchField::Privacy => patch.privacy = None,
        PatchField::Delivery => patch.delivery = None,
        PatchField::QuietHours => patch.quiet_hours = None,
    }
}

fn patch_is_empty(patch: &RulePatch) -> bool {
    patch.enabled.is_none()
        && patch.targets.is_none()
        && patch.filters.is_none()
        && patch.privacy.is_none()
        && patch.delivery.is_none()
        && patch.quiet_hours.is_none()
}

fn validate_channel(channel: &ChannelRecord) -> Result<(), AppError> {
    let kind_matches = matches!(
        (&channel.kind, &channel.public_config),
        (ChannelKind::DingTalk, ChannelPublicConfig::DingTalk { .. })
            | (ChannelKind::WeCom, ChannelPublicConfig::WeCom)
    );
    if channel.name.trim().is_empty()
        || channel.name.len() > MAX_CHANNEL_NAME_BYTES
        || !kind_matches
        || !valid_credential_ref(&channel.credential_ref)
    {
        return Err(configuration_error("channel_invalid", "channel is invalid"));
    }
    Ok(())
}

fn valid_credential_ref(value: &str) -> bool {
    value.starts_with("cc-reminder/channel/")
        && value.len() > "cc-reminder/channel/".len()
        && value.len() <= MAX_CREDENTIAL_REF_BYTES
        && !value.contains(char::is_whitespace)
        && !value.contains('=')
}

fn valid_project_path(path: &Path) -> bool {
    !path.as_os_str().is_empty()
        && path
            .to_str()
            .is_some_and(|value| value.len() <= MAX_PROJECT_PATH_BYTES)
}

fn bounded_list<T>(items: Vec<T>) -> Result<Vec<T>, AppError> {
    if items.len() > MAX_CONFIG_LIST_ITEMS {
        Err(configuration_error(
            "list_limit_exceeded",
            "configuration list is too large",
        ))
    } else {
        Ok(items)
    }
}

fn validate_settings(settings: &AppSettings) -> Result<(), AppError> {
    if !(1..=MAX_RETENTION_DAYS).contains(&settings.event_retention_days)
        || !(1..=MAX_RETENTION_DAYS).contains(&settings.log_retention_days)
        || settings
            .notification_pause
            .as_ref()
            .is_some_and(|pause| pause.until <= pause.started_at)
        // The persisted frontend-reported UTC offset must be expressible as a
        // chrono FixedOffset; anything outside ±24h would poison quiet-hours
        // evaluation.
        || chrono::FixedOffset::east_opt(settings.local_offset_seconds).is_none()
    {
        return Err(configuration_error(
            "settings_invalid",
            "settings are invalid",
        ));
    }
    Ok(())
}

fn list_cache_projects(connection: &Connection) -> Result<Vec<ProjectMatchCacheProject>, AppError> {
    let projects = {
        let mut statement = connection
            .prepare("SELECT id, length(name) FROM projects ORDER BY name, id LIMIT 201")
            .map_err(|_| query_error())?;
        statement
            .query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
            })
            .map_err(|_| query_error())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|_| query_error())?
    };
    if projects.len() > MAX_PROJECT_CACHE_PROJECTS {
        return Err(cache_too_large());
    }
    let mut cache_bytes = 2;
    let mut cached = Vec::with_capacity(projects.len());
    for (id, name_bytes) in projects {
        reserve_cache_bytes(&mut cache_bytes, name_bytes)?;
        let project_id = parse_uuid(&id)?;
        let display_name = connection
            .query_row("SELECT name FROM projects WHERE id = ?1", [&id], |row| {
                row.get(0)
            })
            .map_err(|_| query_error())?;
        let paths = {
            let mut statement = connection
                .prepare(
                    "SELECT id, length(canonical_path) FROM project_paths
                     WHERE project_id = ?1 ORDER BY canonical_path LIMIT 201",
                )
                .map_err(|_| query_error())?;
            statement
                .query_map([&id], |row| {
                    Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
                })
                .map_err(|_| query_error())?
                .collect::<Result<Vec<_>, _>>()
                .map_err(|_| query_error())?
        };
        if paths.len() > MAX_PROJECT_CACHE_PATHS_PER_PROJECT {
            return Err(cache_too_large());
        }
        let mut canonical_paths = Vec::with_capacity(paths.len());
        for (path_id, path_bytes) in paths {
            reserve_cache_bytes(&mut cache_bytes, path_bytes)?;
            let path: String = connection
                .query_row(
                    "SELECT canonical_path FROM project_paths WHERE id = ?1",
                    [&path_id],
                    |row| row.get(0),
                )
                .map_err(|_| query_error())?;
            canonical_paths.push(path.into());
        }
        cached.push(ProjectMatchCacheProject {
            id: project_id,
            display_name,
            canonical_paths,
        });
    }
    Ok(cached)
}

fn reserve_cache_bytes(total: &mut usize, value_bytes: i64) -> Result<(), AppError> {
    let value_bytes = usize::try_from(value_bytes).map_err(|_| cache_too_large())?;
    let estimated = value_bytes
        .checked_mul(6)
        .and_then(|value| value.checked_add(64));
    *total = total
        .checked_add(estimated.ok_or_else(cache_too_large)?)
        .ok_or_else(cache_too_large)?;
    if *total > MAX_PROJECT_CACHE_BYTES {
        Err(cache_too_large())
    } else {
        Ok(())
    }
}

fn global_rule_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<StoredGlobalRule> {
    let id: String = row.get(0)?;
    let agent: String = row.get(1)?;
    let source_event = row.get(2)?;
    let version: i64 = row.get(3)?;
    let config: String = row.get(4)?;
    stored_result((|| -> Result<_, AppError> {
        Ok(StoredGlobalRule {
            id: parse_uuid(&id)?,
            agent: db_parse(&agent)?,
            source_event,
            version: u64::try_from(version).map_err(|_| stored_data_error())?,
            config: serde_json::from_str(&config).map_err(|_| stored_data_error())?,
        })
    })())
}

fn patch_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<StoredRulePatch> {
    let project_id: String = row.get(0)?;
    let agent: String = row.get(1)?;
    let source_event = row.get(2)?;
    let version: i64 = row.get(3)?;
    let patch: String = row.get(4)?;
    stored_result((|| -> Result<_, AppError> {
        Ok(StoredRulePatch {
            project_id: parse_uuid(&project_id)?,
            agent: db_parse(&agent)?,
            source_event,
            version: u64::try_from(version).map_err(|_| stored_data_error())?,
            patch: serde_json::from_str(&patch).map_err(|_| stored_data_error())?,
        })
    })())
}

fn project_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<ProjectRecord> {
    let id: String = row.get(0)?;
    let name = row.get(1)?;
    let root: String = row.get(2)?;
    let mode: String = row.get(3)?;
    let created: String = row.get(4)?;
    let updated: String = row.get(5)?;
    stored_result((|| -> Result<_, AppError> {
        Ok(ProjectRecord {
            id: parse_uuid(&id)?,
            name,
            canonical_root: root.into(),
            worktree_mode: db_parse(&mode)?,
            created_at: parse_time(&created)?,
            updated_at: parse_time(&updated)?,
        })
    })())
}

fn project_path_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<ProjectPathRecord> {
    let id: String = row.get(0)?;
    let project_id: String = row.get(1)?;
    let path: String = row.get(2)?;
    let kind: String = row.get(3)?;
    stored_result((|| -> Result<_, AppError> {
        Ok(ProjectPathRecord {
            id: parse_uuid(&id)?,
            project_id: parse_uuid(&project_id)?,
            canonical_path: path.into(),
            kind: db_parse(&kind)?,
        })
    })())
}

fn channel_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<ChannelRecord> {
    let id: String = row.get(0)?;
    let kind: String = row.get(1)?;
    let name = row.get(2)?;
    let credential_ref = row.get(3)?;
    let config: String = row.get(4)?;
    let health: String = row.get(5)?;
    let paused_reason_code = row.get(6)?;
    let consecutive_auth_failures = row.get(7)?;
    let last: Option<String> = row.get(8)?;
    let next: Option<String> = row.get(9)?;
    stored_result((|| -> Result<_, AppError> {
        Ok(ChannelRecord {
            id: parse_uuid(&id)?,
            kind: db_parse(&kind)?,
            name,
            credential_ref,
            public_config: serde_json::from_str(&config).map_err(|_| stored_data_error())?,
            health_status: db_parse(&health)?,
            paused_reason_code,
            consecutive_auth_failures,
            last_succeeded_at: last.as_deref().map(parse_time).transpose()?,
            next_allowed_at: next.as_deref().map(parse_time).transpose()?,
        })
    })())
}

fn private_new_file(path: &Path) -> Result<File, AppError> {
    #[cfg(unix)]
    use std::os::unix::fs::OpenOptionsExt;
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        options.mode(0o600);
    }
    options.open(path).map_err(|_| cache_error())
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
fn agent_text(agent: AgentKind) -> &'static str {
    agent.as_str()
}
fn path_text(path: &Path) -> Result<String, AppError> {
    path.to_str()
        .map(str::to_owned)
        .ok_or_else(|| configuration_error("path_invalid", "project path is invalid"))
}
fn parse_uuid(value: &str) -> Result<Uuid, AppError> {
    Uuid::parse_str(value).map_err(|_| stored_data_error())
}
fn parse_time(value: &str) -> Result<DateTime<Utc>, AppError> {
    DateTime::parse_from_rfc3339(value)
        .map(|time| time.with_timezone(&Utc))
        .map_err(|_| stored_data_error())
}
fn stored_result<T>(value: Result<T, AppError>) -> rusqlite::Result<T> {
    value.map_err(|_| rusqlite::Error::InvalidQuery)
}
fn now_text() -> String {
    Utc::now().to_rfc3339()
}
fn configuration_error(code: &str, message: &str) -> AppError {
    AppError {
        domain: ErrorDomain::Configuration,
        code: format!("configuration.{code}"),
        message: message.to_owned(),
        suggested_action: None,
    }
}
fn not_found() -> AppError {
    configuration_error("not_found", "configuration record was not found")
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
fn cache_error() -> AppError {
    storage_error(
        "storage.project_cache_failed",
        "project cache could not be regenerated",
    )
}
fn cache_too_large() -> AppError {
    storage_error(
        "storage.project_cache_too_large",
        "project cache exceeds its size limit",
    )
}
fn map_path_error(_: rusqlite::Error) -> AppError {
    configuration_error("path_conflict", "project path is already registered")
}
fn map_channel_error(_: rusqlite::Error) -> AppError {
    configuration_error(
        "channel_conflict",
        "channel credential reference is already registered",
    )
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use chrono::{TimeZone, Utc};
    use rusqlite::params;
    use tempfile::{TempDir, tempdir};
    use uuid::Uuid;

    use crate::events::catalog::{CapabilityCatalog, catalog_for};
    use crate::model::{
        AgentKind, AppSettings, ChannelHealth, ChannelKind, ChannelPublicConfig, ChannelRecord,
        Locale, PatchField, ProjectId, ProjectPathKind, ProjectPathRecord, ProjectRecord,
        RulePatch, Theme, WorktreeMode,
    };
    use crate::storage::config::ConfigRepository;
    use crate::storage::db::Database;

    #[test]
    fn first_start_seeds_one_complete_global_rule_per_catalogued_hook() {
        let (_root, repository) = test_config_repository();

        let report = repository
            .ensure_global_rules(&verified_catalogs())
            .unwrap();
        let rules = repository.list_global_rules().unwrap();

        assert_eq!(report.inserted, 41);
        assert_eq!(rules.len(), 41);
        assert_eq!(rules.iter().filter(|rule| rule.config.enabled).count(), 6);
    }

    #[test]
    fn catalog_refresh_adds_missing_rules_without_overwriting_user_configuration() {
        let (_root, repository) = seeded_config_repository();
        let mut custom = repository
            .get_global_rule(AgentKind::Codex, "Stop")
            .unwrap();
        custom.config.enabled = false;
        custom.version += 1;
        repository.save_global_rule(&custom).unwrap();

        repository
            .ensure_global_rules(&catalogs_with_one_new_event())
            .unwrap();

        assert_eq!(
            repository
                .get_global_rule(AgentKind::Codex, "Stop")
                .unwrap(),
            custom
        );
        assert!(
            !repository
                .get_global_rule(AgentKind::Codex, "NewCatalogEvent")
                .unwrap()
                .config
                .enabled
        );
    }

    #[test]
    fn catalog_refreshed_rule_remains_a_valid_save_target() {
        let (_root, repository) = seeded_config_repository();
        repository
            .ensure_global_rules(&catalogs_with_one_new_event())
            .unwrap();
        let mut rule = repository
            .get_global_rule(AgentKind::Codex, "NewCatalogEvent")
            .unwrap();
        rule.config.enabled = true;

        repository.save_global_rule(&rule).unwrap();

        assert!(
            repository
                .get_global_rule(AgentKind::Codex, "NewCatalogEvent")
                .unwrap()
                .config
                .enabled
        );
    }

    #[test]
    fn catalog_seed_rejects_unbounded_catalog_input() {
        let (_root, repository) = test_config_repository();
        let catalog = verified_catalogs().pop().unwrap();

        assert_eq!(
            repository
                .ensure_global_rules(&vec![catalog; 3])
                .unwrap_err()
                .code,
            "configuration.catalog_invalid"
        );
    }

    #[test]
    fn global_rule_saves_increment_versions_and_keep_empty_target_lists() {
        let (_root, repository) = seeded_config_repository();
        let mut rule = repository
            .get_global_rule(AgentKind::Codex, "Stop")
            .unwrap();
        let version = rule.version;
        rule.config.targets = Vec::new();

        repository.save_global_rule(&rule).unwrap();
        let stored = repository
            .get_global_rule(AgentKind::Codex, "Stop")
            .unwrap();

        assert_eq!(stored.version, version + 1);
        assert!(stored.config.targets.is_empty());
    }

    #[test]
    fn project_patch_round_trip_preserves_explicit_quiet_clear_and_reset_field() {
        let (_root, repository) = seeded_config_repository();
        let project = project_record();
        repository.save_project(&project).unwrap();
        let patch = RulePatch {
            enabled: Some(false),
            quiet_hours: Some(None),
            ..RulePatch::default()
        };

        repository
            .save_project_patch(project.id, AgentKind::Codex, "Stop", &patch)
            .unwrap();
        assert_eq!(
            repository
                .get_project_patch(project.id, AgentKind::Codex, "Stop")
                .unwrap()
                .patch
                .quiet_hours,
            Some(None)
        );

        repository
            .reset_project_patch_field(project.id, AgentKind::Codex, "Stop", PatchField::QuietHours)
            .unwrap();
        assert_eq!(
            repository
                .get_project_patch(project.id, AgentKind::Codex, "Stop")
                .unwrap()
                .patch
                .quiet_hours,
            None
        );
    }

    #[test]
    fn targets_only_project_patch_still_enforces_rule_bounds() {
        let (_root, repository) = seeded_config_repository();
        let project = project_record();
        repository.save_project(&project).unwrap();
        let channel = channel_record("cc-reminder/channel/fake-id");
        repository.save_channel(&channel).unwrap();
        let patch = RulePatch {
            targets: Some(
                (0..21)
                    .map(|_| crate::model::TargetConfig {
                        channel_id: channel.id,
                        template: None,
                    })
                    .collect(),
            ),
            ..RulePatch::default()
        };

        assert_eq!(
            repository
                .save_project_patch(project.id, AgentKind::Codex, "Stop", &patch)
                .unwrap_err()
                .code,
            "rule_invalid"
        );
    }

    #[test]
    fn project_paths_enforce_unique_canonical_paths_and_cascade_with_project() {
        let (_root, repository) = test_config_repository();
        let project = project_record();
        repository.save_project(&project).unwrap();
        let path = ProjectPathRecord {
            id: Uuid::now_v7(),
            project_id: project.id,
            canonical_path: PathBuf::from("/work/cc-reminder-worktree"),
            kind: ProjectPathKind::Worktree,
        };
        repository.save_project_path(&path).unwrap();
        assert_eq!(repository.list_project_paths(project.id).unwrap().len(), 2);

        let mut other = project_record();
        other.id = Uuid::now_v7();
        other.canonical_root = PathBuf::from("/work/other");
        repository.save_project(&other).unwrap();
        let duplicate = ProjectPathRecord {
            id: Uuid::now_v7(),
            project_id: other.id,
            ..path.clone()
        };
        assert_eq!(
            repository.save_project_path(&duplicate).unwrap_err().code,
            "configuration.path_conflict"
        );
        repository.delete_project(project.id).unwrap();
        assert!(
            repository
                .list_project_paths(project.id)
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn project_roots_cannot_reassign_another_projects_canonical_path() {
        let (_root, repository) = test_config_repository();
        let project = project_record();
        repository.save_project(&project).unwrap();
        let duplicate = ProjectRecord {
            id: Uuid::now_v7(),
            ..project
        };

        assert_eq!(
            repository.save_project(&duplicate).unwrap_err().code,
            "configuration.path_conflict"
        );
    }

    #[test]
    fn project_cache_is_written_after_path_changes_with_registered_paths_only() {
        let (root, repository) = test_config_repository();
        let project = project_record();
        repository.save_project(&project).unwrap();

        let cache =
            std::fs::read_to_string(root.path().join("com.ccreminder.app/project-paths.json"))
                .unwrap();

        assert!(cache.contains(&project.id.to_string()));
        assert!(cache.contains("/work/cc-reminder"));
        assert!(!cache.contains("credential_ref"));
    }

    #[test]
    fn cache_failure_is_persisted_until_regeneration_succeeds() {
        let (root, repository) = test_config_repository();
        std::fs::create_dir(root.path().join("com.ccreminder.app/project-paths.json")).unwrap();

        assert_eq!(
            repository.save_project(&project_record()).unwrap_err().code,
            "storage.project_cache_failed"
        );
        assert_eq!(
            repository.project_cache_health().unwrap(),
            crate::model::ProjectCacheHealth::RegenerationFailed
        );

        std::fs::remove_dir(root.path().join("com.ccreminder.app/project-paths.json")).unwrap();
        repository.regenerate_project_cache().unwrap();
        assert_eq!(
            repository.project_cache_health().unwrap(),
            crate::model::ProjectCacheHealth::Healthy
        );
    }

    #[test]
    fn oversized_cache_records_regeneration_failure() {
        let (_root, repository) = test_config_repository();
        let project = project_record();
        repository.save_project(&project).unwrap();
        let connection = rusqlite::Connection::open(repository.database_path()).unwrap();
        for index in 0..270 {
            connection
                .execute(
                    "INSERT INTO project_paths (id, project_id, canonical_path, kind) VALUES (?1, ?2, ?3, 'alias')",
                    params![
                        Uuid::now_v7().to_string(),
                        project.id.to_string(),
                        format!("/work/{index}-{}", "x".repeat(4_080)),
                    ],
                )
                .unwrap();
        }

        assert_eq!(
            repository.regenerate_project_cache().unwrap_err().code,
            "storage.project_cache_too_large"
        );
        assert_eq!(
            repository.project_cache_health().unwrap(),
            crate::model::ProjectCacheHealth::RegenerationFailed
        );
    }

    #[test]
    fn cache_project_read_limit_records_regeneration_failure() {
        let (_root, repository) = test_config_repository();
        let connection = rusqlite::Connection::open(repository.database_path()).unwrap();
        for index in 0..201 {
            let id = Uuid::now_v7();
            connection
                .execute(
                    "INSERT INTO projects (id, name, canonical_root, worktree_mode, created_at, updated_at)
                     VALUES (?1, ?2, ?3, 'alias', '2026-08-03T12:00:00Z', '2026-08-03T12:00:00Z')",
                    params![id.to_string(), format!("project-{index}"), format!("/work/{index}")],
                )
                .unwrap();
        }

        assert_eq!(
            repository.regenerate_project_cache().unwrap_err().code,
            "storage.project_cache_too_large"
        );
        assert_eq!(
            repository.project_cache_health().unwrap(),
            crate::model::ProjectCacheHealth::RegenerationFailed
        );
    }

    #[test]
    fn channel_storage_accepts_only_public_config_and_opaque_credential_reference() {
        let (_root, repository) = test_config_repository();
        repository
            .save_channel(&channel_record("cc-reminder/channel/fake-id"))
            .unwrap();

        let bytes = std::fs::read(repository.database_path()).unwrap();
        assert!(!String::from_utf8_lossy(&bytes).contains("access_token="));
    }

    #[test]
    fn channel_reference_is_unique_and_active_rules_prevent_deletion() {
        let (_root, repository) = seeded_config_repository();
        let channel = channel_record("cc-reminder/channel/fake-id");
        repository.save_channel(&channel).unwrap();
        let duplicate = ChannelRecord {
            id: Uuid::now_v7(),
            ..channel.clone()
        };
        assert_eq!(
            repository.save_channel(&duplicate).unwrap_err().code,
            "configuration.channel_conflict"
        );

        let mut rule = repository
            .get_global_rule(AgentKind::Codex, "Stop")
            .unwrap();
        rule.config.targets = vec![crate::model::TargetConfig {
            channel_id: channel.id,
            template: None,
        }];
        repository.save_global_rule(&rule).unwrap();
        assert_eq!(
            repository.delete_channel(channel.id).unwrap_err().code,
            "configuration.channel_in_use"
        );
    }

    #[test]
    fn project_override_target_prevents_channel_deletion() {
        let (_root, repository) = seeded_config_repository();
        let project = project_record();
        let channel = channel_record("cc-reminder/channel/fake-id");
        repository.save_project(&project).unwrap();
        repository.save_channel(&channel).unwrap();
        repository
            .save_project_patch(
                project.id,
                AgentKind::Codex,
                "Stop",
                &RulePatch {
                    targets: Some(vec![crate::model::TargetConfig {
                        channel_id: channel.id,
                        template: None,
                    }]),
                    ..RulePatch::default()
                },
            )
            .unwrap();

        assert_eq!(
            repository.delete_channel(channel.id).unwrap_err().code,
            "configuration.channel_in_use"
        );
    }

    #[test]
    fn inherited_target_on_project_enablement_prevents_channel_deletion() {
        let (_root, repository) = seeded_config_repository();
        let project = project_record();
        let channel = channel_record("cc-reminder/channel/fake-id");
        repository.save_project(&project).unwrap();
        repository.save_channel(&channel).unwrap();
        let mut global = repository
            .get_global_rule(AgentKind::Codex, "Stop")
            .unwrap();
        global.config.enabled = false;
        global.config.targets = vec![crate::model::TargetConfig {
            channel_id: channel.id,
            template: None,
        }];
        repository.save_global_rule(&global).unwrap();
        repository
            .save_project_patch(
                project.id,
                AgentKind::Codex,
                "Stop",
                &RulePatch {
                    enabled: Some(true),
                    ..RulePatch::default()
                },
            )
            .unwrap();

        assert_eq!(
            repository.delete_channel(channel.id).unwrap_err().code,
            "configuration.channel_in_use"
        );
    }

    #[test]
    fn settings_default_and_reject_out_of_bounds_retention() {
        let (_root, repository) = test_config_repository();
        assert_eq!(repository.get_settings().unwrap(), AppSettings::default());

        let invalid = AppSettings {
            event_retention_days: 0,
            ..AppSettings::default()
        };
        assert_eq!(
            repository.save_settings(&invalid).unwrap_err().code,
            "configuration.settings_invalid"
        );
        let valid = AppSettings {
            event_retention_days: 14,
            log_retention_days: 14,
            locale: Locale::En,
            theme: Theme::Dark,
            ..AppSettings::default()
        };
        assert_eq!(repository.save_settings(&valid).unwrap(), valid);
    }

    #[test]
    fn settings_reject_an_implausible_local_offset() {
        let (_root, repository) = test_config_repository();
        // A frontend-reported UTC offset outside chrono's ±24h FixedOffset
        // range would poison quiet-hours evaluation, so it must never persist.
        let invalid = AppSettings {
            local_offset_seconds: 100_000_000,
            ..AppSettings::default()
        };
        assert_eq!(
            repository.save_settings(&invalid).unwrap_err().code,
            "configuration.settings_invalid"
        );
        // A real-world east offset (+08:00) persists fine.
        let valid = AppSettings {
            local_offset_seconds: 8 * 3600,
            ..AppSettings::default()
        };
        assert_eq!(repository.save_settings(&valid).unwrap(), valid);
    }

    fn test_config_repository() -> (TempDir, ConfigRepository) {
        let root = tempdir().unwrap();
        let database = Database::open(
            &root
                .path()
                .join("com.ccreminder.app")
                .join("cc-reminder.sqlite3"),
        )
        .unwrap();
        (root, ConfigRepository::new(database))
    }

    fn seeded_config_repository() -> (TempDir, ConfigRepository) {
        let (root, repository) = test_config_repository();
        repository
            .ensure_global_rules(&verified_catalogs())
            .unwrap();
        (root, repository)
    }

    fn verified_catalogs() -> Vec<CapabilityCatalog> {
        vec![
            catalog_for(AgentKind::ClaudeCode, &semver::Version::new(2, 1, 218)).catalog,
            catalog_for(AgentKind::Codex, &semver::Version::new(0, 145, 0)).catalog,
        ]
    }

    fn catalogs_with_one_new_event() -> Vec<CapabilityCatalog> {
        let mut catalogs = verified_catalogs();
        let codex = catalogs
            .iter_mut()
            .find(|catalog| catalog.agent == AgentKind::Codex)
            .unwrap();
        let mut event = codex.hooks[0].clone();
        event.source_event = "NewCatalogEvent".to_owned();
        codex.hooks.push(event);
        catalogs
    }

    fn project_record() -> ProjectRecord {
        let now = Utc.with_ymd_and_hms(2026, 8, 3, 12, 0, 0).unwrap();
        ProjectRecord {
            id: project_id(),
            name: "CC Reminder".to_owned(),
            canonical_root: PathBuf::from("/work/cc-reminder"),
            worktree_mode: WorktreeMode::Alias,
            created_at: now,
            updated_at: now,
        }
    }

    fn channel_record(credential_ref: &str) -> ChannelRecord {
        ChannelRecord {
            id: Uuid::now_v7(),
            kind: ChannelKind::DingTalk,
            name: "Engineering".to_owned(),
            credential_ref: credential_ref.to_owned(),
            public_config: ChannelPublicConfig::DingTalk {
                keyword_prefix: Some("CC".to_owned()),
            },
            health_status: ChannelHealth::Unknown,
            paused_reason_code: None,
            consecutive_auth_failures: 0,
            last_succeeded_at: None,
            next_allowed_at: None,
        }
    }

    fn project_id() -> ProjectId {
        Uuid::now_v7()
    }
}
