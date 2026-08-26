use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::time::Duration;

use chrono::Utc;
use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};

use crate::error::{AppError, ErrorDomain};
use crate::security::permissions::{
    ensure_current_user_dacl, ensure_private_directory, ensure_private_file,
};

const DESKTOP_BUSY_TIMEOUT: Duration = Duration::from_secs(2);
const INGRESS_BUSY_TIMEOUT: Duration = Duration::from_millis(20);
const MIGRATIONS: &[(i64, &str)] = &[(1, include_str!("../../../migrations/0001_initial.sql"))];

#[derive(Clone, Debug)]
pub struct Database {
    path: PathBuf,
}

impl Database {
    pub fn open(path: &Path) -> Result<Self, AppError> {
        prepare_database_path(path)?;
        let database = Self {
            path: path.to_path_buf(),
        };
        database.migrate()?;
        ensure_private_file(path)?;
        Ok(database)
    }

    pub fn open_ingress_writer(path: &Path) -> Result<Connection, AppError> {
        prepare_database_path(path)?;
        let connection = open_connection(path, INGRESS_BUSY_TIMEOUT)?;
        if schema_version_on(&connection)? < 1 {
            return Err(storage_error(
                "storage.schema_unavailable",
                "database schema is not ready",
            ));
        }
        Ok(connection)
    }

    pub fn migrate(&self) -> Result<(), AppError> {
        let mut connection = open_connection_set_wal(&self.path, DESKTOP_BUSY_TIMEOUT)?;
        apply_migrations(&mut connection, MIGRATIONS)
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub(crate) fn connect(&self) -> Result<Connection, AppError> {
        open_connection(&self.path, DESKTOP_BUSY_TIMEOUT)
    }

    pub fn table_names(&self) -> Result<BTreeSet<String>, AppError> {
        let connection = self.connect()?;
        let mut statement = connection
            .prepare(
                "SELECT name FROM sqlite_master
                 WHERE type = 'table' AND name NOT LIKE 'sqlite_%'
                 ORDER BY name",
            )
            .map_err(|_| query_error())?;
        let names = statement
            .query_map([], |row| row.get::<_, String>(0))
            .map_err(|_| query_error())?
            .collect::<Result<BTreeSet<_>, _>>()
            .map_err(|_| query_error())?;
        Ok(names)
    }

    pub fn pragma_string(&self, name: &str) -> Result<String, AppError> {
        let connection = self.connect()?;
        connection
            .pragma_query_value(None, name, |row| row.get(0))
            .map_err(|_| query_error())
    }

    pub fn schema_version(&self) -> Result<i64, AppError> {
        let connection = self.connect()?;
        schema_version_on(&connection)
    }
}

fn prepare_database_path(path: &Path) -> Result<(), AppError> {
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty());
    if path.file_name().and_then(|name| name.to_str()) != Some("cc-reminder.sqlite3")
        || parent
            .and_then(|directory| directory.file_name())
            .and_then(|name| name.to_str())
            != Some("com.ccreminder.app")
    {
        return Err(storage_error(
            "storage.invalid_database_path",
            "database path is invalid",
        ));
    }
    let parent = parent.expect("validated database parent");
    ensure_private_directory(parent)?;
    #[cfg(windows)]
    ensure_current_user_dacl(parent)?;
    ensure_private_file(path)?;
    ensure_current_user_dacl(path)
}

/// 打开一个工作连接:只做连接级配置(busy_timeout / foreign_keys /
/// synchronous——均为连接本地,不加库级锁)。
///
/// 实机法医教训(2026-08-27):journal_mode=WAL 持久化在库头,过去在每个
/// per-op 连接上重复 PRAGMA journal_mode,与"最后一个连接关闭时
/// checkpoint 移除 -wal/-shm"的窗口相撞,会在 walIndexReadHdr/unixShmMap
/// 处把 opener 永久卡死,进而冻住全部写入(hook 全部静默超时、外部读
/// BUSY)。因此 journal_mode 仅由 migrate() 在启动单线程时设置一次。
fn open_connection(path: &Path, busy_timeout: Duration) -> Result<Connection, AppError> {
    let connection = Connection::open(path)
        .map_err(|_| storage_error("storage.open_failed", "database could not be opened"))?;
    connection
        .busy_timeout(busy_timeout)
        .map_err(|_| storage_error("storage.open_failed", "database could not be configured"))?;
    connection
        .pragma_update(None, "foreign_keys", true)
        .map_err(|_| storage_error("storage.open_failed", "database could not be configured"))?;
    connection
        .pragma_update(None, "synchronous", "NORMAL")
        .map_err(|_| storage_error("storage.open_failed", "database could not be configured"))?;
    Ok(connection)
}

/// 仅初始化路径使用:设置持久化 journal_mode(WAL)。库级排他操作,
/// 决不放进 per-op 热路径。
fn open_connection_set_wal(path: &Path, busy_timeout: Duration) -> Result<Connection, AppError> {
    let connection = open_connection(path, busy_timeout)?;
    connection
        .pragma_update(None, "journal_mode", "WAL")
        .map_err(|_| storage_error("storage.open_failed", "database could not be configured"))?;
    Ok(connection)
}

fn apply_migrations(
    connection: &mut Connection,
    migrations: &[(i64, &str)],
) -> Result<(), AppError> {
    for &(version, sql) in migrations {
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| migration_error())?;
        if schema_version_on(&transaction)? >= version {
            transaction.commit().map_err(|_| migration_error())?;
            continue;
        }
        transaction
            .execute_batch(sql)
            .map_err(|_| migration_error())?;
        transaction
            .execute(
                "INSERT INTO schema_migrations(version, applied_at) VALUES (?1, ?2)",
                params![version, Utc::now().to_rfc3339()],
            )
            .map_err(|_| migration_error())?;
        transaction.commit().map_err(|_| migration_error())?;
    }
    Ok(())
}

fn schema_version_on(connection: &Connection) -> Result<i64, AppError> {
    let migrations_table_exists = connection
        .query_row(
            "SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = 'schema_migrations'",
            [],
            |_| Ok(()),
        )
        .optional()
        .map_err(|_| query_error())?
        .is_some();
    if !migrations_table_exists {
        return Ok(0);
    }
    connection
        .query_row(
            "SELECT COALESCE(MAX(version), 0) FROM schema_migrations",
            [],
            |row| row.get(0),
        )
        .map_err(|_| query_error())
}

fn migration_error() -> AppError {
    storage_error(
        "storage.migration_failed",
        "database migration could not be applied",
    )
}

fn query_error() -> AppError {
    storage_error("storage.query_failed", "database query failed")
}

pub(crate) fn storage_error(code: &str, message: &str) -> AppError {
    AppError {
        domain: ErrorDomain::Storage,
        code: code.to_owned(),
        message: message.to_owned(),
        suggested_action: None,
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    #[cfg(unix)]
    use std::os::unix::fs::PermissionsExt;

    use tempfile::{TempDir, tempdir};

    use super::{Database, MIGRATIONS, apply_migrations};

    fn database_path(root: &TempDir) -> std::path::PathBuf {
        root.path()
            .join("com.ccreminder.app")
            .join("cc-reminder.sqlite3")
    }

    #[test]
    fn migration_creates_every_v1_table_and_enables_wal() {
        let root = tempdir().unwrap();
        let path = database_path(&root);
        let db = Database::open(&path).unwrap();
        let tables = db.table_names().unwrap();

        assert_eq!(
            tables,
            BTreeSet::from(
                [
                    "agent_installations",
                    "app_settings",
                    "channels",
                    "config_snapshots",
                    "delivery_attempts",
                    "delivery_jobs",
                    "events",
                    "global_rules",
                    "hook_installations",
                    "ingress_events",
                    "project_paths",
                    "project_rule_overrides",
                    "projects",
                    "schema_migrations",
                ]
                .map(str::to_owned),
            )
        );
        assert_eq!(db.pragma_string("journal_mode").unwrap(), "wal");
        let connection = db.connect().unwrap();
        for (pragma, expected) in [
            ("foreign_keys", 1_i64),
            ("synchronous", 1),
            ("busy_timeout", 2_000),
        ] {
            let actual = connection
                .pragma_query_value(None, pragma, |row| row.get::<_, i64>(0))
                .unwrap();
            assert_eq!(actual, expected, "unexpected {pragma} value");
        }
        assert_eq!(db.schema_version().unwrap(), 1);
    }

    #[test]
    fn applying_migrations_twice_is_idempotent() {
        let root = tempdir().unwrap();
        let path = database_path(&root);
        Database::open(&path).unwrap();

        let reopened = Database::open(&path).unwrap();

        assert_eq!(reopened.schema_version().unwrap(), 1);
    }

    #[test]
    fn invalid_migration_rolls_back_its_schema_and_version_row() {
        let root = tempdir().unwrap();
        let db = Database::open(&database_path(&root)).unwrap();
        let mut connection = db.connect().unwrap();
        let mut migrations = MIGRATIONS.to_vec();
        migrations.push((
            2,
            "CREATE TABLE migration_rollback_probe (id INTEGER PRIMARY KEY); invalid sql;",
        ));

        let error = apply_migrations(&mut connection, &migrations).unwrap_err();

        assert_eq!(error.code, "storage.migration_failed");
        assert_eq!(db.schema_version().unwrap(), 1);
        assert!(
            !db.table_names()
                .unwrap()
                .contains("migration_rollback_probe")
        );
    }

    #[test]
    fn ingress_writer_refuses_an_unmigrated_database_and_uses_twenty_ms_timeout() {
        let root = tempdir().unwrap();
        let path = database_path(&root);

        let error = Database::open_ingress_writer(&path).unwrap_err();
        assert_eq!(error.code, "storage.schema_unavailable");

        Database::open(&path).unwrap();
        let writer = Database::open_ingress_writer(&path).unwrap();
        let busy_timeout: i64 = writer
            .pragma_query_value(None, "busy_timeout", |row| row.get(0))
            .unwrap();
        let foreign_keys: u8 = writer
            .pragma_query_value(None, "foreign_keys", |row| row.get(0))
            .unwrap();
        let synchronous: u8 = writer
            .pragma_query_value(None, "synchronous", |row| row.get(0))
            .unwrap();

        assert_eq!(busy_timeout, 20);
        assert_eq!(foreign_keys, 1);
        assert_eq!(synchronous, 1);
    }

    #[cfg(unix)]
    #[test]
    fn database_opens_reject_shared_parents_without_hardening_them() {
        let root = tempdir().unwrap();
        let shared_directory = root.path().join("shared");
        std::fs::create_dir(&shared_directory).unwrap();
        std::fs::set_permissions(&shared_directory, std::fs::Permissions::from_mode(0o755))
            .unwrap();
        let path = shared_directory.join("cc-reminder.sqlite3");

        let error = Database::open(&path).unwrap_err();
        assert_eq!(error.code, "storage.invalid_database_path");
        assert!(!error.message.contains(&path.display().to_string()));
        assert_eq!(
            std::fs::metadata(&shared_directory)
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o755
        );

        let error = Database::open_ingress_writer(&path).unwrap_err();
        assert_eq!(error.code, "storage.invalid_database_path");
        assert!(!error.message.contains(&path.display().to_string()));
        assert_eq!(
            std::fs::metadata(&shared_directory)
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o755
        );
    }

    #[cfg(unix)]
    #[test]
    fn opening_database_hardens_an_existing_parent_directory() {
        let root = tempdir().unwrap();
        let directory = root.path().join("com.ccreminder.app");
        std::fs::create_dir(&directory).unwrap();
        std::fs::set_permissions(&directory, std::fs::Permissions::from_mode(0o755)).unwrap();
        let path = directory.join("cc-reminder.sqlite3");

        Database::open(&path).unwrap();

        assert_eq!(
            std::fs::metadata(&directory).unwrap().permissions().mode() & 0o777,
            0o700
        );
        assert_eq!(
            std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }
}
