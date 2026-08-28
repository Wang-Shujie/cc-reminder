// Agent Integration page (Task 18): one row per agent with Detect / Install /
// Repair / Upgrade Helper / Uninstall mapped to the closed `apply_hook_action`
// enum, per-agent drift from the shared derivation, per-event health of the
// last applied result, and the Codex /hooks trust handoff.
import { useCallback, useEffect, useState, type ReactNode } from "react";
import { ArrowRight, ClipboardCopy } from "lucide-react";

import { usePageBackend, type Backend } from "../../lib/backend";
import { useCoreQuery } from "../../lib/useCoreQuery";
import {
  AGENT_CONFIRMATION_REQUIRED,
  errorOf,
  errorCodeOf,
  type PageError,
} from "../../lib/errors";
import type {
  AgentIntegrationSummary,
  AgentKindCode,
  HookActionCode,
  HookApplyEntry,
  HookRuleRow,
  ListHookRulesInput,
  LocaleCode,
} from "../../lib/contracts";
import { dictionary, type Dictionary } from "../../lib/i18n";

const AGENTS: readonly AgentKindCode[] = ["claude-code", "codex"];

function agentName(t: Dictionary, agent: AgentKindCode): string {
  return agent === "claude-code" ? t.agentClaudeCode : t.agentCodex;
}

function hookActionLabel(
  t: Dictionary,
  prefix: "agentInstallPrefix" | "agentRepairPrefix" | "agentUninstallPrefix",
  agent: AgentKindCode,
): string {
  return `${t[prefix]}${agentName(t, agent)}${t.hookWord}`;
}

function detectionStateLabel(t: Dictionary, health: string): string {
  switch (health) {
    case "detected":
      return t.dsDetected;
    case "missing":
      return t.dsMissing;
    case "invalid_version":
      return t.dsInvalidVersion;
    case "process_failed":
      return t.dsProcessFailed;
    case "timed_out":
      return t.dsTimedOut;
    default:
      return health;
  }
}

function entryHealthLabel(t: Dictionary, health: HookApplyEntry["health"]): string {
  switch (health) {
    case "healthy":
      return t.ehHealthy;
    case "missing":
      return t.ehMissing;
    case "drifted":
      return t.ehDrifted;
    case "helper_mismatch":
      return t.ehHelperMismatch;
    case "needs_trust":
      return t.ehNeedsTrust;
    case "agent_upgrade_required":
      return t.ehAgentUpgradeRequired;
    default:
      return health;
  }
}

/**
 * v2-issues:待确认 hook 的触发建议(用户裁决 2026-08-28)。按事件归类给
 * 一条可直接复制的提示词;没有提示词形态的事件(SessionStart 等由会话
 * 生命周期触发)给操作指引,prompt 为 null 不显示复制按钮。
 */
function triggerSuggestion(
  t: Dictionary,
  event: string,
): { label: string; prompt: string | null } {
  switch (event) {
    case "SessionStart":
      return { label: t.triggerHintSessionStart, prompt: null };
    case "SessionEnd":
      return { label: t.triggerHintSessionEnd, prompt: null };
    case "PreCompact":
    case "PostCompact":
      return { label: t.triggerHintCompact, prompt: null };
    case "PermissionRequest":
      return { label: t.triggerHintPromptLabel, prompt: t.triggerPromptPermission };
    case "PreToolUse":
    case "PostToolUse":
      return { label: t.triggerHintPromptLabel, prompt: t.triggerPromptTool };
    case "SubagentStart":
    case "SubagentStop":
      return { label: t.triggerHintPromptLabel, prompt: t.triggerPromptSubagent };
    default:
      return { label: t.triggerHintPromptLabel, prompt: t.triggerPromptGeneric };
  }
}

interface Busy {
  agent: AgentKindCode;
}

export function AgentsPage({
  locale = "zh_cn",
  backend: injected,
  showHeading = true,
}: {
  locale?: LocaleCode;
  backend?: Backend;
  /** 集成页内嵌时隐藏自身标题(区块标题「通知来源」已承担层级)。 */
  showHeading?: boolean;
}): ReactNode {
  const backend = usePageBackend(injected);
  const t = dictionary(locale);

  /** Per-event health of the LAST successful apply per agent. */
  const [entries, setEntries] = useState<Partial<Record<AgentKindCode, HookApplyEntry[]>>>({});
  /** True once any Codex entry is observed working: the official /hooks
   *  confirmation is demonstrably complete, so remaining pending entries are
   *  "await their first real occurrence", not "user has work to do". */
  const codexConfirmed = (entries["codex"] ?? []).some(
    (entry) => entry.trust_status === "observed_working",
  );
  const [busy, setBusy] = useState<Busy | null>(null);
  const [detecting, setDetecting] = useState(false);
  const [error, setError] = useState<PageError | null>(null);
  const [uninstallTarget, setUninstallTarget] = useState<AgentKindCode | null>(null);
  const [trustGuideOpen, setTrustGuideOpen] = useState(false);
  /** 最近应用结果弹窗(用户裁决第四轮):动作完成后自动弹出一次,
   *  页面不再常驻结果区块;工具栏箭头可随时回看。 */
  const [resultOpen, setResultOpen] = useState(false);
  /** Set after the backend rejects with agent_confirmation_required; the open
   *  dialog discloses the compatibility caveat before retrying with true. */
  const [needsConsent, setNeedsConsent] = useState<{
    agent: AgentKindCode;
    action: HookActionCode;
  } | null>(null);

  // 统一请求层(架构提案 §1):集成摘要 + 全范围规则行一次取回
  // (drift 必须看见项目补丁:全局行 + 每个项目 scope)。失败语义保持
  // "空表 + 显式告警,其余页面可用"。
  const integrationsQuery = useCoreQuery(
    async (b) => {
      const projects = await b.listProjects().catch(() => []);
      const scopes: ListHookRulesInput[] = AGENTS.flatMap((agent) => [
        { agent, project_id: null },
        ...projects.map((p) => ({ agent, project_id: p.id })),
      ]);
      const [integrations, rowLists] = await Promise.all([
        b.listAgentIntegrations(),
        Promise.all(
          scopes.map((scope) => b.listHookRules(scope).catch(() => [])),
        ),
      ]);
      return { summaries: integrations, rows: rowLists.flat() };
    },
    [],
    [],
    backend,
  );
  const summaries: AgentIntegrationSummary[] | null = integrationsQuery.failed
    ? []
    : (integrationsQuery.data?.summaries ?? null);
  const rows: HookRuleRow[] = integrationsQuery.data?.rows ?? [];
  const loadFailed = integrationsQuery.failed;
  const refresh = integrationsQuery.refresh;

  function hasInstalledHooks(agent: AgentKindCode): boolean {
    return rows.some((row) => row.agent === agent && row.installed);
  }

  async function apply(
    agent: AgentKindCode,
    action: HookActionCode,
    confirmCompatibleVersion: boolean,
  ): Promise<void> {
    setBusy({ agent });
    setError(null);
    try {
      const result = await backend.applyHookAction({
        agent,
        action,
        expected_health_revision: 0,
        confirm_compatible_version: confirmCompatibleVersion,
      });
      // Uninstall leaves nothing applied: drop the entry entirely so
      // 最近应用结果 never shows stale pre-uninstall health.
      setEntries((prev) => {
        const next = { ...prev };
        if (action === "uninstall") {
          delete next[agent];
        } else {
          next[agent] = result.entries;
        }
        return next;
      });
      setNeedsConsent(null);
      setUninstallTarget(null);
      if (action !== "uninstall") {
        setResultOpen(true);
      }
      await refresh();
    } catch (e: unknown) {
      if (!confirmCompatibleVersion && errorCodeOf(e) === AGENT_CONFIRMATION_REQUIRED) {
        // Keep the dialog flow; the disclosure appears for round two.
        setNeedsConsent({ agent, action });
        return;
      }
      setNeedsConsent(null);
      setUninstallTarget(null);
      setError(errorOf(e));
    } finally {
      setBusy(null);
    }
  }

  async function detect(): Promise<void> {
    setDetecting(true);
    setError(null);
    try {
      // 检测结果由核心落库,列表接口本就实时探测——经请求层重取即得。
      await backend.detectAgents({ confirm_compatible_version: false });
      await refresh();
    } catch (e: unknown) {
      setError(errorOf(e));
    } finally {
      setDetecting(false);
    }
  }

  const summaryFor = (agent: AgentKindCode): AgentIntegrationSummary | undefined =>
    summaries?.find((s) => s.agent === agent);

  return (
    <section aria-label={t.navAgents}>
      {showHeading && <h2>{t.navAgents}</h2>}

      <div className="rules-toolbar">
        <div className="rules-toolbar-controls">
          <button
            type="button"
            className="cc-focusable"
            disabled={detecting}
            onClick={() => {
              void detect();
            }}
          >
            {detecting ? t.agentDetecting : t.agentDetect}
          </button>
          {(entries["claude-code"] !== undefined ||
            entries["codex"] !== undefined) && (
            <button
              type="button"
              className="cc-focusable link-arrow"
              onClick={() => setResultOpen(true)}
            >
              {t.lastApplied}
              <ArrowRight size={14} aria-hidden="true" />
            </button>
          )}
        </div>
      </div>

      {loadFailed && <p role="alert">{t.listLoadFailed}</p>}

      {/* 动作结果以弹窗呈现(用户裁决):内层保留 role=alert 语义。 */}
      {error !== null && (
        <div className="dialog-overlay">
          <div role="dialog" aria-label={t.navAgents} className="dialog">
            <h2>{t.navAgents}</h2>
            <p role="alert">
              {error.message}
              {error.suggested_action !== null && <>（{error.suggested_action}）</>}
            </p>
            <div className="row-end">
              <button
                type="button"
                className="cc-focusable"
                onClick={() => {
                  setError(null);
                }}
              >
                {t.drawerClose}
              </button>
            </div>
          </div>
        </div>
      )}

      <table className="rules-table">
        <thead>
          <tr>
            <th>{t.colAgent}</th>
            <th>{t.colVersion}</th>
            <th>{t.colState}</th>
            <th>{t.colSwitch}</th>
          </tr>
        </thead>
        <tbody>
          {AGENTS.map((agent) => {
            const summary = summaryFor(agent);
            const name = agentName(t, agent);
            const installed = summary?.installed ?? false;
            const unknownMajor =
              summary?.needs_compatible_version_confirmation ?? false;
            return (
              <tr key={agent}>
                <td>{name}</td>
                <td>{summary?.version ?? "—"}</td>
                <td>
                  {summary === null || summary === undefined ? (
                    "—"
                  ) : (
                    detectionStateLabel(t, summary.health)
                  )}
                  {installed && unknownMajor && (
                    <>
                      {" · "}
                      <span className="badge">{t.agentUpgradeNeeded}</span>
                    </>
                  )}
                </td>
                <td className="agent-actions">
                  {!installed && (
                    <button
                      type="button"
                      className="cc-focusable"
                      /* A missing agent binary cannot host hooks at all. */
                      disabled={busy !== null || summary?.health === "missing"}
                      onClick={() => {
                        void apply(agent, "install", false);
                      }}
                    >
                      {hookActionLabel(t, "agentInstallPrefix", agent)}
                    </button>
                  )}
                  {installed && !unknownMajor && (
                    <button
                      type="button"
                      className="cc-focusable"
                      disabled={busy !== null}
                      onClick={() => {
                        void apply(agent, hasInstalledHooks(agent) ? "repair" : "install", false);
                      }}
                    >
                      {/* Label follows the actual action: Repair only when
                          installed hooks exist, otherwise Install. */}
                      {hookActionLabel(
                        t,
                        hasInstalledHooks(agent)
                          ? "agentRepairPrefix"
                          : "agentInstallPrefix",
                        agent,
                      )}
                    </button>
                  )}
                  {installed && unknownMajor && (
                    <>
                      {/* Unknown major: app upgrade is the only way forward —
                          installing against an unverified version stays off. */}
                      <button
                        type="button"
                        className="cc-focusable"
                        disabled
                        title={t.agentUpgradeNeeded}
                      >
                        {hookActionLabel(t, "agentInstallPrefix", agent)}
                      </button>
                    </>
                  )}
                  {installed && (
                    <>
                      <button
                        type="button"
                        className="cc-focusable"
                        disabled={busy !== null}
                        onClick={() => {
                          void apply(agent, "upgrade_helper", false);
                        }}
                      >
                        {t.upgradeHelperAction}
                      </button>
                      <button
                        type="button"
                        className="cc-focusable"
                        disabled={busy !== null}
                        onClick={() => setUninstallTarget(agent)}
                      >
                        {hookActionLabel(t, "agentUninstallPrefix", agent)}
                      </button>
                    </>
                  )}
                </td>
              </tr>
            );
          })}
        </tbody>
      </table>
      {summaries === null && <p className="muted">{t.loading}</p>}

      {resultOpen &&
        (entries["claude-code"] !== undefined ||
          entries["codex"] !== undefined) && (
        <div className="dialog-overlay">
          <div role="dialog" aria-label={t.lastApplied} className="dialog">
            <h2>{t.lastApplied}</h2>
            {AGENTS.map((agent) =>
              (entries[agent] ?? []).length > 0 ? (
                <div key={agent}>
                  <h3>{agentName(t, agent)}</h3>
                  <ul>
                    {(entries[agent] ?? []).map((entry) => {
                      const pending =
                        entry.trust_status === "needs_user_confirmation";
                      const suggestion = pending
                        ? triggerSuggestion(t, entry.source_event)
                        : null;
                      return (
                        <li key={entry.source_event}>
                          {entry.source_event} · {entryHealthLabel(t, entry.health)}
                          {pending &&
                            ` · ${
                              codexConfirmed
                                ? t.ehAwaitingFirstRun
                                : t.ehNeedsTrust
                            }`}
                          {suggestion !== null && (
                            <div className="muted trigger-suggestion">
                              {suggestion.label}
                              {suggestion.prompt !== null && (
                                <>
                                  <code>{suggestion.prompt}</code>
                                  <button
                                    type="button"
                                    className="cc-focusable"
                                    aria-label={`${t.copyCommand} ${entry.source_event}`}
                                    onClick={() => {
                                      void navigator.clipboard?.writeText(
                                        suggestion.prompt ?? "",
                                      );
                                    }}
                                  >
                                    <ClipboardCopy size={12} aria-hidden="true" />{" "}
                                    {t.copyCommand}
                                  </button>
                                </>
                              )}
                            </div>
                          )}
                        </li>
                      );
                    })}
                  </ul>
                </div>
              ) : null,
            )}
            {(entries["codex"] ?? []).some(
              (entry) =>
                entry.trust_status === "needs_user_confirmation" ||
                entry.health === "needs_trust",
            ) && (
              <p className="trust-command">
                {/* Once any Codex entry is observed working, the official /hooks
                    confirmation is demonstrably done: remaining pending entries
                    just await their first real occurrence — inform, don't
                    re-instruct. */}
                {codexConfirmed ? (
                  <span>{t.trustDoneAwaiting}</span>
                ) : (
                  <button
                    type="button"
                    className="cc-focusable link-arrow"
                    onClick={() => {
                      setResultOpen(false);
                      setTrustGuideOpen(true);
                    }}
                  >
                    {t.trustViewGuide}
                    <ArrowRight size={14} aria-hidden="true" />
                  </button>
                )}
              </p>
            )}
            <div className="row-end">
              <button
                type="button"
                className="cc-focusable"
                onClick={() => setResultOpen(false)}
              >
                {t.drawerClose}
              </button>
            </div>
          </div>
        </div>
      )}

      {trustGuideOpen && (
        <div className="dialog-overlay">
          <div role="dialog" aria-label={t.trustGuideTitle} className="dialog">
            <h2>{t.trustGuideTitle}</h2>
            <p>{t.trustNotice}</p>
            <p className="trust-command">
              <code>{t.trustCommand}</code>
              <button
                type="button"
                className="cc-focusable"
                onClick={() => {
                  void navigator.clipboard?.writeText(t.trustCommand);
                }}
              >
                <ClipboardCopy size={14} aria-hidden="true" /> {t.copyCommand}
              </button>
            </p>
            <div className="row-end">
              <button
                type="button"
                className="cc-focusable"
                onClick={() => {
                  setTrustGuideOpen(false);
                  void detect();
                }}
              >
                {t.recheck}
              </button>
              <button
                type="button"
                className="cc-focusable"
                onClick={() => setTrustGuideOpen(false)}
              >
                {t.drawerClose}
              </button>
            </div>
          </div>
        </div>
      )}

      {uninstallTarget !== null && (
        <div className="dialog-overlay">
          <div
            role="dialog"
            aria-label={t.uninstallConfirmTitle}
            className="dialog"
          >
            <h2>{t.uninstallConfirmTitle}</h2>
            <p>{hookActionLabel(t, "agentUninstallPrefix", uninstallTarget)}</p>
            <p className="trust-item">{t.uninstallScopeNote}</p>
            <div className="row-end">
              <button
                type="button"
                className="cc-focusable"
                onClick={() => setUninstallTarget(null)}
              >
                {t.cancel}
              </button>
              <button
                type="button"
                className="primary cc-focusable"
                disabled={busy !== null}
                onClick={() => {
                  void apply(uninstallTarget, "uninstall", false);
                }}
              >
                {t.confirmUninstall}
              </button>
            </div>
          </div>
        </div>
      )}

      {needsConsent !== null && (
        <div className="dialog-overlay">
          <div
            role="dialog"
            aria-label={t.versionConsentTitle}
            className="dialog"
          >
            <h2>{t.versionConsentTitle}</h2>
            <p className="trust-item">{t.versionConsentDisclosure}</p>
            <div className="row-end">
              <button
                type="button"
                className="cc-focusable"
                onClick={() => setNeedsConsent(null)}
              >
                {t.cancel}
              </button>
              <button
                type="button"
                className="primary cc-focusable"
                disabled={busy !== null}
                onClick={() => {
                  void apply(needsConsent.agent, needsConsent.action, true);
                }}
              >
                {t.consentContinue}
              </button>
            </div>
          </div>
        </div>
      )}
    </section>
  );
}
