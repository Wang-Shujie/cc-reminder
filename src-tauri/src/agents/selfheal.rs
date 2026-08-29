//! v2-issues(2026-08-29 实锤复发):Claude Code 会话把内存中的空 hooks 态
//! 落盘,清掉 cc-reminder 安装的条目——磁盘被清、DB 仍记 healthy、应用侧
//! 无感知(第四轮事件同因,Claude Code 受害,Codex 无涉)。自愈循环周期性
//! 比对磁盘 owned 条目与 DB 记录,发现结构损伤即复用既有 Repair 事务
//! (读→合并→加密快照→原子替换→落库)恢复——不新发明任何写盘路径。
//!
//! 用户意图边界:
//! - 应用内 **卸载** 会删除 DB 行 → 循环跳过,绝不复活;
//! - 手工删除配置条目被视为事故(与第四轮定罪的清空同形态)→ 自动恢复;
//! - Codex 信任待确认(NeedsTrust)不是损伤——Repair 会重置信任,绝不因
//!   待确认触发;目录未验证(AgentUpgradeRequired)同理不触发。

use std::collections::BTreeSet;
use std::path::PathBuf;

use crate::agents::{EntryHealth, HookSelection};
use crate::error::AppError;
use crate::installer::helper::HelperInstaller;
use crate::installer::lifecycle::HookInstaller;
use crate::model::AgentKind;
use crate::storage::integrations::IntegrationRepository;
use crate::worker::{CancellationToken, CoreEvent, CoreEventSink};

/// 纯判定:存在结构损伤(Missing/Drifted/HelperMismatch)才修复。
pub(crate) fn should_repair(entries: &[crate::agents::HookEntryHealth]) -> bool {
    entries.iter().any(|e| {
        matches!(
            e.health,
            EntryHealth::Missing | EntryHealth::Drifted | EntryHealth::HelperMismatch
        )
    })
}

/// Agent 配置文件的稳定路径。与命令层 `build_installer` 的差异:此处永远/// 使用默认 home(自愈只管应用自己安装的默认路径;命令层另有 codex_home/// 测试注入覆盖)。
pub(crate) fn hook_config_path(agent: AgentKind, home: &std::path::Path) -> PathBuf {
    match agent {
        AgentKind::ClaudeCode => home.join(".claude").join("settings.json"),
        AgentKind::Codex => home.join(".codex").join("hooks.json"),
    }
}

/// 单个 agent 的一次自愈:inspect → 需要则 Repair。返回是否发生了修复。
pub(crate) fn heal_agent_once(
    installer: &HookInstaller,
    selection: &HookSelection,
) -> Result<bool, AppError> {
    let health = installer.inspect(selection)?;
    if !should_repair(&health.entries) {
        return Ok(false);
    }
    installer.apply(crate::installer::lifecycle::HookAction::Repair, selection)?;
    Ok(true)
}

/// 后台自愈循环:helper 环境构造一次(不可用则整个循环不启动——占位
/// helper 的安装事务会被 `update.helper_not_installed` 拒绝,轮询无意义),
/// 周期检查两个 agent。DB 无 hook 记录(未安装/已卸载)的 agent 跳过。
/// 首个 interval tick 立即执行:应用重启即恢复被清空的配置。
#[allow(clippy::too_many_arguments)] // 启动装配函数,依赖平铺比打包 struct 直白
pub async fn selfheal_loop(
    integrations: IntegrationRepository,
    cipher: crate::security::crypto::LazyFieldCipher,
    helper: HelperInstaller,
    home: PathBuf,
    events: CoreEventSink,
    diagnostics: std::sync::Arc<crate::diagnostics::Diagnostics>,
    period: std::time::Duration,
    cancel: CancellationToken,
) {
    let mut interval = tokio::time::interval(period);
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    loop {
        tokio::select! {
            _ = interval.tick() => {
                // cipher 未就绪(钥匙串后台加载中)→ 修复事务要写加密快照,
                // 本轮跳过,下轮再试。
                let Some(cipher_value) = cipher.get().ok() else {
                    continue;
                };
                for agent in [AgentKind::ClaudeCode, AgentKind::Codex] {
                    let Ok(records) = integrations.list_hooks(agent) else {
                        continue;
                    };
                    if records.is_empty() {
                        continue;
                    }
                    let selection = HookSelection {
                        events: records
                            .iter()
                            .map(|r| r.source_event.clone())
                            .collect::<BTreeSet<String>>(),
                        helper_path: helper.stable_path(),
                        helper_version: helper.manifest_version().clone(),
                    };
                    let installer = HookInstaller::new(
                        agent,
                        hook_config_path(agent, &home),
                        integrations.clone(),
                        Some((*cipher_value).clone()),
                        helper.clone(),
                    );
                    match heal_agent_once(&installer, &selection) {
                        Ok(true) => {
                            diagnostics.info(
                                "selfheal",
                                &format!("self-healed cleared/drifted hooks for {agent:?}"),
                            );
                            crate::worker::emit(
                                &events,
                                CoreEvent::HealthChanged { channel_id: None },
                            );
                        }
                        Ok(false) => {}
                        Err(err) => {
                            diagnostics.info(
                                "selfheal",
                                &format!("self-heal skipped for {agent:?}: {err}"),
                            );
                        }
                    }
                }
            }
            _ = cancel.cancelled() => return,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{heal_agent_once, hook_config_path, should_repair};
    use crate::agents::{EntryHealth, HookEntryHealth, HookSelection};
    use crate::installer::helper::HelperInstaller;
    use crate::installer::lifecycle::HookInstaller;
    use crate::model::{AgentKind, HookInstallationRecord, InstallationHealth, TrustStatus};
    use crate::security::crypto::FieldCipher;
    use crate::storage::db::Database;
    use crate::storage::integrations::IntegrationRepository;
    use std::collections::BTreeSet;

    fn entry(health: EntryHealth) -> HookEntryHealth {
        HookEntryHealth {
            source_event: "Stop".into(),
            command_fingerprint: "fp".into(),
            definition_fingerprint: "df".into(),
            trust_status: TrustStatus::NotRequired,
            health,
        }
    }

    #[test]
    fn needs_trust_and_unverified_catalog_never_trigger_a_repair() {
        // 仅待确认(Codex 官方信任未完成)或目录未验证不是损伤——Repair 会
        // 重置信任,绝不因它们触发自愈。
        assert!(!should_repair(&[entry(EntryHealth::Healthy)]));
        assert!(!should_repair(&[entry(EntryHealth::NeedsTrust)]));
        assert!(!should_repair(&[entry(EntryHealth::AgentUpgradeRequired)]));
        assert!(should_repair(&[entry(EntryHealth::Missing)]));
        assert!(should_repair(&[entry(EntryHealth::Drifted)]));
        assert!(should_repair(&[entry(EntryHealth::HelperMismatch)]));
    }

    #[test]
    fn cleared_config_file_is_restored_and_foreign_content_preserved() {
        // 实机事故形态:settings.json 的 hooks 被清成空数组,外来键(model 等)
        // 完好。自愈必须恢复 owned 条目且绝不触碰外来内容;已健康后不重复动作。
        let root = tempfile::tempdir().unwrap();
        // Database::open 强制 <dir>/com.ccreminder.app/cc-reminder.sqlite3 布局。
        let db_dir = root.path().join("com.ccreminder.app");
        std::fs::create_dir_all(&db_dir).unwrap();
        let database = Database::open(&db_dir.join("cc-reminder.sqlite3")).unwrap();
        let integrations = IntegrationRepository::new(database.clone());
        integrations
            .replace_hooks(
                AgentKind::ClaudeCode,
                &[HookInstallationRecord {
                    agent: AgentKind::ClaudeCode,
                    source_event: "Stop".into(),
                    command_fingerprint: "cmd-fp".into(),
                    definition_fingerprint: "def-fp".into(),
                    helper_version: "2.0.0".into(),
                    config_hash: "hash".into(),
                    trust_status: TrustStatus::NotRequired,
                    health_status: InstallationHealth::Healthy,
                    last_seen_at: None,
                }],
            )
            .unwrap();

        let config = hook_config_path(AgentKind::ClaudeCode, root.path());
        std::fs::create_dir_all(config.parent().unwrap()).unwrap();
        std::fs::write(&config, r#"{"model":"opus","hooks":{"Stop":[]}}"#).unwrap();

        let helper = HelperInstaller::undeployed_placeholder(root.path().join("bin"));
        std::fs::create_dir_all(root.path().join("bin")).unwrap();
        std::fs::write(helper.stable_path(), b"helper").unwrap();

        let installer = HookInstaller::new(
            AgentKind::ClaudeCode,
            config.clone(),
            integrations.clone(),
            Some(FieldCipher::from_key([3u8; 32])),
            helper.clone(),
        );
        let selection = HookSelection {
            events: BTreeSet::from(["Stop".to_string()]),
            helper_path: helper.stable_path(),
            helper_version: helper.manifest_version().clone(),
        };

        assert!(heal_agent_once(&installer, &selection).unwrap());
        let after = std::fs::read_to_string(&config).unwrap();
        assert!(after.contains("\"Stop\""), "owned event restored: {after}");
        assert!(
            after.contains("cc-reminder-hook"),
            "helper command back: {after}"
        );
        assert!(after.contains("opus"), "foreign content preserved: {after}");

        // 已健康:不再动作。
        assert!(!heal_agent_once(&installer, &selection).unwrap());
    }
}
