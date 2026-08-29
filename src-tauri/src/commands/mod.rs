//! Typed desktop command surface (Task 15 §A).
//!
//! Each Tauri command is a thin wrapper around a `pub(crate)` function that
//! takes `&CoreState` and a typed input struct. The split keeps commands
//! unit-testable without constructing a `tauri::State` (which has no public
//! constructor), and lets the helper fns be exercised directly from the
//! `commands::` tests required by the plan.
//!
//! Privacy contract: read commands expose only typed DTOs that omit
//! credentials, ciphertext, raw unmatched IDs, Hook raw JSON and full platform
//! responses. `deny_unknown_fields` is on every input struct so the WebView
//! cannot smuggle extra fields.

pub mod agents;
pub mod channels;
pub mod diagnostics;
pub mod history;
pub mod projects;
pub mod rules;
pub mod settings;
pub mod updates;

use std::sync::Mutex;

use chrono::Utc;
use serde::{Deserialize, Serialize};
use tauri::State;
use uuid::Uuid;

use crate::error::{AppError, ErrorDomain};
use crate::health::HealthSnapshot;
use crate::model::ChannelRecord;
use crate::security::credentials::CredentialStore;
use crate::storage::config::ConfigRepository;
use crate::storage::events::EventRepository;
use crate::storage::integrations::IntegrationRepository;
use crate::storage::queue::QueueRepository;

/// Aggregated, clonable handle to every repository the command surface needs.
/// `Managed` by Tauri as `tauri::State<CoreState>`.
//
// ponytail: one struct, no trait-object registry. The command set is fixed and
// small; an interface-per-domain would be scaffolding for exactly one
// implementation. The Mutex on the worker cancel token is the only interior
// mutability the surface needs (commands are otherwise read/transactional).
/// 领域分组(架构提案 §3):存储四件套与运行时句柄各成一 struct,
/// 避免平铺十一个字段的"上帝对象";凭据/加密/日志横切关注保持顶层。
#[derive(Clone)]
pub struct StorageHandles {
    pub config: ConfigRepository,
    pub events: EventRepository,
    pub queue: QueueRepository,
    pub integrations: IntegrationRepository,
}

#[derive(Clone)]
pub struct RuntimeHandles {
    pub cancel_token: std::sync::Arc<Mutex<Option<crate::worker::CancellationToken>>>,
    /// Join handle of the delivery-worker task, stashed by the app shell next
    /// to `cancel_token` so `RunEvent::Exit` can wait (≤10s) for the in-flight
    /// send pass during graceful shutdown. Always `None` in pure command
    /// tests — no worker is running there.
    pub worker_task: std::sync::Arc<Mutex<Option<tauri::async_runtime::JoinHandle<()>>>>,
    /// Applies the autostart setting to the OS (register/unregister the
    /// LaunchAgent / login item). Injected by the app shell from the Tauri
    /// autostart plugin; defaults to a no-op so pure command tests can run.
    pub autostart_control: AutostartControl,
    /// Shared bounded-channel sender feeding the `core://` event forwarder
    /// task spawned by the app shell (lib.rs). Commands push revision-only
    /// notifications here (e.g. rule saves bumping health); the single
    /// consumer maps each event to its topic and emits `{revision}` to the
    /// WebView. Defaults to a DISCONNECTED sender so pure command tests need
    /// no wiring — sends fail harmlessly.
    pub core_events: crate::worker::CoreEventSink,
    /// Absolute resource directory resolved by the app shell from Tauri's
    /// resource-dir API (`app.path().resource_dir()`); `None` in pure command
    /// tests and whenever resolution fails. The bundled-helper loader joins
    /// ONLY fixed relative paths under it — never caller-supplied input.
    pub resources_dir: Option<std::path::PathBuf>,
    /// Count of hook ingress requests REJECTED this session (unrecognized
    /// helper / handler panic). v2-issues:hook 触发失败曾对健康完全不可见
    /// (快照硬编码 rejected_count=0);IPC 环在拒绝时自增,投影读取。
    pub rejected_ingress: std::sync::Arc<std::sync::atomic::AtomicU64>,
}

#[derive(Clone)]
pub struct CoreState {
    pub storage: StorageHandles,
    pub credentials: CredentialStore,
    pub cipher: crate::security::crypto::LazyFieldCipher,
    /// Shared diagnostics logger (Task 20 fix round 1): the same instance the
    /// startup wiring and the retention ticker write through, so exports see
    /// the runtime log files and debug windows are process-wide.
    pub diagnostics: std::sync::Arc<crate::diagnostics::Diagnostics>,
    pub runtime: RuntimeHandles,
}

/// Applies an autostart on/off request. Returns an error message on failure.
pub type AutostartControl = std::sync::Arc<dyn Fn(bool) -> Result<(), String> + Send + Sync>;

impl CoreState {
    pub fn new(
        config: ConfigRepository,
        events: EventRepository,
        queue: QueueRepository,
        integrations: IntegrationRepository,
        credentials: CredentialStore,
        cipher: crate::security::crypto::LazyFieldCipher,
        diagnostics: std::sync::Arc<crate::diagnostics::Diagnostics>,
    ) -> Self {
        // Default event sink: a sender whose receiver is dropped immediately,
        // so every try_send reports Disconnected and is ignored (see
        // `worker::emit`). The app shell replaces this with the live channel.
        let (core_events, dead_receiver) = tokio::sync::mpsc::channel(1);
        drop(dead_receiver);
        Self {
            storage: StorageHandles {
                config,
                events,
                queue,
                integrations,
            },
            credentials,
            cipher,
            diagnostics,
            runtime: RuntimeHandles {
                cancel_token: std::sync::Arc::new(Mutex::new(None)),
                worker_task: std::sync::Arc::new(Mutex::new(None)),
                autostart_control: std::sync::Arc::new(|_| Ok(())),
                core_events,
                resources_dir: None,
                rejected_ingress: std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0)),
            },
        }
    }
}

/// Generic page request input. `deny_unknown_fields` so the frontend cannot
/// smuggle extra params; validation lives in the command, not the repo.
#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PageInput {
    pub offset: u32,
    pub limit: u16,
}

impl PageInput {
    pub fn to_page_request(&self) -> crate::storage::events::PageRequest {
        crate::storage::events::PageRequest {
            offset: self.offset,
            limit: self.limit,
        }
    }
}

/// Bootstrap state returned to the frontend on first paint.
#[derive(Clone, Debug, Serialize)]
pub struct BootstrapState {
    pub onboarding_completed: bool,
    pub locale: String,
    pub theme: String,
    pub health: HealthSnapshot,
    pub pending_jobs: u64,
    pub failed_jobs: u64,
}

/// Parse a UUID input or return a typed `configuration.malformed_uuid` error.
/// Centralized so every command shares one redaction-safe rejection path.
pub(crate) fn parse_uuid_input(value: &str) -> Result<Uuid, AppError> {
    Uuid::parse_str(value)
        .map_err(|_| configuration_error("malformed_uuid", "identifier is invalid"))
}

pub(crate) fn configuration_error(code: &str, message: &str) -> AppError {
    AppError {
        domain: ErrorDomain::Configuration,
        code: format!("configuration.{code}"),
        message: message.to_owned(),
        suggested_action: None,
    }
}

pub(crate) fn secret_store_error(code: &str, message: &str) -> AppError {
    AppError {
        domain: ErrorDomain::SecretStore,
        code: format!("secret_store.{code}"),
        message: message.to_owned(),
        suggested_action: None,
    }
}

pub(crate) fn integration_error(code: &str, message: &str) -> AppError {
    AppError {
        domain: ErrorDomain::Integration,
        code: format!("integration.{code}"),
        message: message.to_owned(),
        suggested_action: None,
    }
}

pub(crate) fn update_error(code: &str, message: &str) -> AppError {
    AppError {
        domain: ErrorDomain::Update,
        code: format!("update.{code}"),
        message: message.to_owned(),
        suggested_action: None,
    }
}

// ---------------------------------------------------------------------------
// Command implementations (typed, no Tauri types). Each `pub(crate) fn` is the
// real body; the per-domain command modules expose the `#[tauri::command]`
// wrappers that pull `State<CoreState>` and delegate.
// ---------------------------------------------------------------------------

/// `get_bootstrap_state`. The optional `offset_seconds` is the WebView's UTC
/// offset (`-Date#getTimezoneOffset()*60`, east-positive — same Task 19
/// pattern as `set_notification_pause`). Persisting it here delivers the
/// timezone to the core at first paint of EVERY session, so quiet hours
/// evaluate in local time from the second launch onward (documented fallback:
/// UTC until the first report).
pub(crate) fn bootstrap_state(
    state: &CoreState,
    offset_seconds: Option<i32>,
) -> Result<BootstrapState, AppError> {
    persist_reported_offset(&state.storage.config, offset_seconds)?;
    let settings = state.storage.config.get_settings()?;
    let snapshot = build_health_snapshot(state)?;
    let pending = snapshot.pending_jobs;
    let failed = snapshot.failed_jobs;
    Ok(BootstrapState {
        onboarding_completed: settings.onboarding_completed,
        locale: locale_code(settings.locale),
        theme: theme_code(settings.theme),
        health: snapshot,
        pending_jobs: pending,
        failed_jobs: failed,
    })
}

/// Store a frontend-reported UTC offset alongside settings. An implausible
/// value (outside chrono's FixedOffset range) is IGNORED rather than fatal:
/// bootstrap must never fail because the WebView reported nonsense.
fn persist_reported_offset(
    config: &ConfigRepository,
    offset_seconds: Option<i32>,
) -> Result<(), AppError> {
    let Some(seconds) = offset_seconds else {
        return Ok(());
    };
    if chrono::FixedOffset::east_opt(seconds).is_none() {
        return Ok(());
    }
    let mut settings = config.get_settings()?;
    if settings.local_offset_seconds != seconds {
        settings.local_offset_seconds = seconds;
        // v2-issues: 启动路径必须"只读可活"。offset 持久化是 best-effort——
        // 写失败(如只读存储)只意味着下次启动的静默小时仍用旧时区,
        // 绝不能让 bootstrap 报错把用户挡在永久加载屏外。
        let _ = config.save_settings(&settings);
    }
    Ok(())
}

/// `get_health_snapshot`
pub(crate) fn health_snapshot(state: &CoreState) -> Result<HealthSnapshot, AppError> {
    build_health_snapshot(state)
}

fn build_health_snapshot(state: &CoreState) -> Result<HealthSnapshot, AppError> {
    use crate::health::{HealthInputs, HealthIssue, HealthLevel, project_health};

    let stats = state.storage.queue.queue_stats()?;
    let channels = state.storage.config.list_channels()?;
    let channel_states: Vec<_> = channels
        .iter()
        .map(|c| crate::health::ChannelHealthState {
            channel_id: c.id.to_string(),
            name: c.name.clone(),
            kind: channel_kind_code(c.kind),
            health: channel_health_level(c.health_status, c.paused_reason_code.as_deref()),
            paused: c.paused_reason_code.is_some(),
            summary: channel_summary(c.health_status, c.paused_reason_code.as_deref()),
        })
        .collect();

    // v2-issues:agents 与 hook 安装健康此前完全没进快照(硬编码省略),
    // agent 卸载/升级、hook 漂移/未信任、ingress 被拒都表现为「健康」。
    // 数据全部已落库(detect/redetect 维护 agent_installations,安装事务
    // 维护 hook_installations)——纯读库投影,零子进程成本。
    let mut agent_states = Vec::new();
    let mut issues: Vec<HealthIssue> = Vec::new();
    for agent in [
        crate::model::AgentKind::ClaudeCode,
        crate::model::AgentKind::Codex,
    ] {
        if let Ok(record) = state.storage.integrations.agent(agent) {
            let level = installation_health_level(record.health_status);
            agent_states.push(crate::health::AgentIntegrationHealth {
                agent,
                installed: record.health_status == crate::model::InstallationHealth::Healthy,
                version: record.version.as_ref().map(ToString::to_string),
                health: level,
                summary: installation_summary(record.health_status),
            });
        }
        let agent_label = agent_kind_label(agent);
        let hooks = state
            .storage
            .integrations
            .list_hooks(agent)
            .unwrap_or_default();
        let mut broken = 0u32;
        let mut repair = 0u32;
        let mut untrusted = 0u32;
        for hook in &hooks {
            match hook.health_status {
                crate::model::InstallationHealth::Error => broken += 1,
                crate::model::InstallationHealth::NeedsRepair => repair += 1,
                _ => {}
            }
            if hook.trust_status == crate::model::TrustStatus::NeedsUserConfirmation {
                untrusted += 1;
            }
        }
        if broken > 0 {
            issues.push(issue(
                "hook.failed",
                HealthLevel::Error,
                &format!("{agent_label} 有 {broken} 条 Hook 配置已失效(被外部修改或删除)"),
                "在「集成」页重新安装 Hook",
            ));
        }
        if repair > 0 {
            issues.push(issue(
                "hook.needs_repair",
                HealthLevel::Warning,
                &format!("{agent_label} 有 {repair} 条 Hook 配置漂移,需要修复"),
                "在「集成」页修复 Hook",
            ));
        }
        if untrusted > 0 {
            issues.push(issue(
                "hook.needs_trust",
                HealthLevel::Warning,
                &format!("{agent_label} 有 {untrusted} 条 Hook 尚未经过一次真实触发确认"),
                "在对应 Agent 中真实触发一次 Hook",
            ));
        }
    }

    let rejected = state
        .runtime
        .rejected_ingress
        .load(std::sync::atomic::Ordering::Relaxed);
    if rejected > 0 {
        issues.push(issue(
            "ingress.rejected",
            HealthLevel::Warning,
            &format!("本次运行已拒绝 {rejected} 次 Hook 请求(helper 不被信任或内部错误)"),
            "在「集成」页重新安装 Hook 后重启应用",
        ));
    }

    let inputs = HealthInputs {
        agents: agent_states,
        channels: channel_states,
        pending_jobs: stats.pending,
        retry_jobs: stats.retry_wait,
        failed_jobs: stats.failed,
        expired_jobs: stats.expired,
        succeeded_jobs: stats.succeeded,
        spool_count: 0,
        rejected_count: rejected,
        last_success_at: last_success(&channels),
        issues,
    };
    Ok(project_health(&inputs))
}

/// Persisted agent-installation health → shared health level. `Unknown` (never
/// detected) stays Ok so a fresh install without onboarding is not an alarm.
fn installation_health_level(
    health: crate::model::InstallationHealth,
) -> crate::health::HealthLevel {
    match health {
        crate::model::InstallationHealth::Healthy => crate::health::HealthLevel::Ok,
        crate::model::InstallationHealth::Unknown => crate::health::HealthLevel::Ok,
        crate::model::InstallationHealth::NeedsRepair => crate::health::HealthLevel::Warning,
        crate::model::InstallationHealth::Error => crate::health::HealthLevel::Error,
    }
}

fn installation_summary(health: crate::model::InstallationHealth) -> String {
    match health {
        crate::model::InstallationHealth::Healthy => "healthy".into(),
        crate::model::InstallationHealth::Unknown => "unknown".into(),
        crate::model::InstallationHealth::NeedsRepair => "needs_repair".into(),
        crate::model::InstallationHealth::Error => "error".into(),
    }
}

fn agent_kind_label(agent: crate::model::AgentKind) -> &'static str {
    match agent {
        crate::model::AgentKind::ClaudeCode => "Claude Code",
        crate::model::AgentKind::Codex => "Codex",
    }
}

fn issue(
    issue_code: &str,
    level: crate::health::HealthLevel,
    message: &str,
    suggested_action: &str,
) -> crate::health::HealthIssue {
    crate::health::HealthIssue {
        issue_code: issue_code.to_owned(),
        level,
        message: message.to_owned(),
        suggested_command: None,
        suggested_action: Some(suggested_action.to_owned()),
    }
}

fn last_success(channels: &[ChannelRecord]) -> Option<chrono::DateTime<Utc>> {
    // The channels table holds per-channel last_succeeded_at; the overview
    // "last success" is the most recent across them.
    channels
        .iter()
        .filter_map(|channel| channel.last_succeeded_at)
        .max()
}

pub(crate) fn channel_kind_code(kind: crate::model::ChannelKind) -> String {
    match kind {
        crate::model::ChannelKind::DingTalk => "ding_talk".into(),
        crate::model::ChannelKind::WeCom => "we_com".into(),
    }
}

pub(crate) fn channel_health_level(
    health: crate::model::ChannelHealth,
    paused: Option<&str>,
) -> crate::health::HealthLevel {
    use crate::health::HealthLevel;
    match health {
        crate::model::ChannelHealth::Healthy => HealthLevel::Ok,
        crate::model::ChannelHealth::Unknown => HealthLevel::Ok,
        crate::model::ChannelHealth::PausedAuthentication => HealthLevel::Warning,
        crate::model::ChannelHealth::Error => HealthLevel::Error,
    }
    .worst(if paused.is_some() {
        HealthLevel::Warning
    } else {
        HealthLevel::Ok
    })
}

pub(crate) fn channel_summary(health: crate::model::ChannelHealth, paused: Option<&str>) -> String {
    if paused.is_some() {
        return "paused".into();
    }
    match health {
        crate::model::ChannelHealth::Healthy => "healthy".into(),
        crate::model::ChannelHealth::Unknown => "unknown".into(),
        crate::model::ChannelHealth::PausedAuthentication => "paused".into(),
        crate::model::ChannelHealth::Error => "error".into(),
    }
}

pub(crate) fn locale_code(locale: crate::model::Locale) -> String {
    match locale {
        crate::model::Locale::ZhCn => "zh_cn".into(),
        crate::model::Locale::En => "en".into(),
    }
}

pub(crate) fn theme_code(theme: crate::model::Theme) -> String {
    match theme {
        crate::model::Theme::System => "system".into(),
        crate::model::Theme::Light => "light".into(),
        crate::model::Theme::Dark => "dark".into(),
    }
}

// ---------------------------------------------------------------------------
// Tauri wrappers for the two commands that live in this module.
// ---------------------------------------------------------------------------

#[tauri::command]
pub async fn get_bootstrap_state(
    state: State<'_, CoreState>,
    offset_seconds: Option<i32>,
) -> Result<BootstrapState, AppError> {
    let state = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || bootstrap_state(&state, offset_seconds))
        .await
        .map_err(|_| configuration_error("join_failed", "command join failed"))?
}

#[tauri::command]
pub async fn get_health_snapshot(state: State<'_, CoreState>) -> Result<HealthSnapshot, AppError> {
    let state = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || health_snapshot(&state))
        .await
        .map_err(|_| configuration_error("join_failed", "command join failed"))?
}

// The `#[tauri::command]` wrappers live in each submodule and are referenced
// from `lib.rs` by their fully-qualified path (e.g. `commands::channels::save_channel`).
// We deliberately do NOT glob-re-export them here: the command names collide
// with the inner `*_impl` body names if re-exported together, and
// `generate_handler!` requires literal paths anyway.

#[cfg(test)]
mod tests {
    #[test]
    fn agent_hook_and_rejected_health_reach_the_snapshot() {
        // v2-issues:agents/hook 漂移/ingress 拒绝此前完全进不了快照,
        // 事实故障下界面仍显示健康。三者必须各自抬级并产出 issue。
        use crate::model::{
            AgentInstallationRecord, AgentKind, InstallationHealth, TrustStatus,
        };
        use crate::security::credentials::CredentialStore;
        use crate::security::crypto::FieldCipher;
        use crate::storage::config::ConfigRepository;
        use crate::storage::db::Database;
        use crate::storage::events::EventRepository;
        use crate::storage::integrations::IntegrationRepository;
        use crate::storage::queue::QueueRepository;
        use tempfile::tempdir;

        use super::{CoreState, health_snapshot};

        let root = tempdir().unwrap();
        let database_path = root.path().join("com.ccreminder.app/cc-reminder.sqlite3");
        std::fs::create_dir_all(database_path.parent().unwrap()).unwrap();
        let database = Database::open(&database_path).unwrap();
        let integrations = IntegrationRepository::new(database.clone());
        let state = CoreState::new(
            ConfigRepository::new(database.clone()),
            EventRepository::new(database.clone()),
            QueueRepository::new(database.clone()),
            integrations.clone(),
            CredentialStore::memory_for_test(),
            crate::security::crypto::LazyFieldCipher::ready(std::sync::Arc::new(
                FieldCipher::from_key([4u8; 32]),
            )),
            std::sync::Arc::new(crate::diagnostics::Diagnostics::test(
                &root.path().join("logs"),
                1024 * 1024,
                3,
            )),
        );

        // Claude Code: agent healthy, but one hook drifted (needs repair) and
        // one untrusted. Codex: agent itself needs repair.
        integrations
            .upsert_agent(&AgentInstallationRecord {
                agent: AgentKind::ClaudeCode,
                executable_path: None,
                version: Some(semver::Version::new(2, 1, 218)),
                capability_verification: crate::events::catalog::CatalogVerification::Exact,
                health_status: InstallationHealth::Healthy,
                last_checked_at: chrono::Utc::now(),
            })
            .unwrap();
        integrations
            .upsert_agent(&AgentInstallationRecord {
                agent: AgentKind::Codex,
                executable_path: None,
                version: Some(semver::Version::new(0, 145, 0)),
                capability_verification: crate::events::catalog::CatalogVerification::Exact,
                health_status: InstallationHealth::NeedsRepair,
                last_checked_at: chrono::Utc::now(),
            })
            .unwrap();
        integrations
            .replace_hooks(
                AgentKind::ClaudeCode,
                &[
                    hook_record(
                        "PreToolUse",
                        InstallationHealth::NeedsRepair,
                        TrustStatus::ObservedWorking,
                    ),
                    hook_record(
                        "Stop",
                        InstallationHealth::Healthy,
                        TrustStatus::NeedsUserConfirmation,
                    ),
                ],
            )
            .unwrap();

        state
            .runtime
            .rejected_ingress
            .fetch_add(2, std::sync::atomic::Ordering::Relaxed);

        let snapshot = health_snapshot(&state).unwrap();
        let codes: Vec<_> = snapshot
            .issues
            .iter()
            .map(|i| i.issue_code.as_str())
            .collect();
        assert!(codes.contains(&"hook.needs_repair"), "codes: {codes:?}");
        assert!(codes.contains(&"hook.needs_trust"), "codes: {codes:?}");
        assert!(codes.contains(&"ingress.rejected"), "codes: {codes:?}");
        assert_eq!(snapshot.rejected_count, 2);
        let codex = snapshot
            .agents
            .iter()
            .find(|a| a.agent == AgentKind::Codex)
            .expect("codex installation reaches the snapshot");
        assert_eq!(codex.health, crate::health::HealthLevel::Warning);
        // Codex NeedsRepair + hook drift + rejections ⇒ at least Warning.
        assert_eq!(snapshot.overall, crate::health::HealthLevel::Warning);
    }

    /// Minimal hook row for `replace_hooks` in tests.
    fn hook_record(
        source_event: &str,
        health: crate::model::InstallationHealth,
        trust: crate::model::TrustStatus,
    ) -> crate::model::HookInstallationRecord {
        crate::model::HookInstallationRecord {
            agent: crate::model::AgentKind::ClaudeCode,
            source_event: source_event.to_owned(),
            command_fingerprint: "fp".into(),
            definition_fingerprint: "df".into(),
            helper_version: "0.0.0".into(),
            config_hash: "hash".into(),
            trust_status: trust,
            health_status: health,
            last_seen_at: None,
        }
    }

    // 只读前置依赖 unix 权限位(chmod 0o444),Windows 无对应语义。
    #[test]
    #[cfg(unix)]
    fn reported_offset_persistence_is_best_effort() {
        // v2-issues: 只读存储不得把 bootstrap 挡在永久加载屏——offset 写失败
        // 必须被吞掉(仅影响下次启动的静默小时时区)。
        use crate::storage::config::ConfigRepository;
        use crate::storage::db::Database;
        use std::os::unix::fs::PermissionsExt;

        let root = tempfile::tempdir().unwrap();
        let dir = root.path().join("com.ccreminder.app");
        std::fs::create_dir_all(&dir).unwrap();
        let db_path = dir.join("cc-reminder.sqlite3");
        let db = Database::open(&db_path).unwrap();
        let config = ConfigRepository::new(db.clone());

        // WAL 模式下写入发生在 -wal/-shm 边车文件:三者全部只读,
        // 强制 save_settings 失败。
        let files = [
            db_path.clone(),
            dir.join("cc-reminder.sqlite3-wal"),
            dir.join("cc-reminder.sqlite3-shm"),
        ];
        for path in &files {
            if path.exists() {
                std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o444)).unwrap();
            }
        }
        let mut settings = config.get_settings().unwrap();
        settings.local_offset_seconds = 12345;
        let save_failed = config.save_settings(&settings).is_err();
        for path in &files {
            if path.exists() {
                std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o644)).unwrap();
            }
        }
        drop(config);
        drop(db);
        drop(root);
        // 环境前提不成立(边车写仍成功)时该测试失去意义,显式失败暴露。
        assert!(save_failed, "read-only setup failed to block the save");
    }
}
