// Agent Integration page (Task 18): one row per agent with Detect / Install /
// Repair / Upgrade Helper / Uninstall mapped to the closed `apply_hook_action`
// enum, per-agent drift from the shared derivation, per-event health of the
// last applied result, and the Codex /hooks trust handoff.
import { useCallback, useEffect, useState, type ReactNode } from "react";
import { ClipboardCopy } from "lucide-react";

import { usePageBackend, type Backend } from "../lib/backend";
import { driftEvents, mergeRowsForDrift } from "../lib/drift";
import {
  AGENT_CONFIRMATION_REQUIRED,
  errorOf,
  errorCodeOf,
  type PageError,
} from "../lib/errors";
import type {
  AgentIntegrationSummary,
  AgentKindCode,
  HookActionCode,
  HookApplyEntry,
  HookRuleRow,
  ListHookRulesInput,
  LocaleCode,
} from "../lib/contracts";
import { dictionary, type Dictionary } from "../lib/i18n";

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

interface Busy {
  agent: AgentKindCode;
}

export function AgentsPage({
  locale = "zh_cn",
  backend: injected,
}: {
  locale?: LocaleCode;
  backend?: Backend;
}): ReactNode {
  const backend = usePageBackend(injected);
  const t = dictionary(locale);
  const [summaries, setSummaries] = useState<AgentIntegrationSummary[] | null>(null);
  const [rows, setRows] = useState<HookRuleRow[]>([]);
  /** Per-event health of the LAST successful apply per agent. */
  const [entries, setEntries] = useState<Partial<Record<AgentKindCode, HookApplyEntry[]>>>({});
  const [busy, setBusy] = useState<Busy | null>(null);
  const [detecting, setDetecting] = useState(false);
  const [error, setError] = useState<PageError | null>(null);
  const [uninstallTarget, setUninstallTarget] = useState<AgentKindCode | null>(null);
  /** Set after the backend rejects with agent_confirmation_required; the open
   *  dialog discloses the compatibility caveat before retrying with true. */
  const [needsConsent, setNeedsConsent] = useState<{
    agent: AgentKindCode;
    action: HookActionCode;
  } | null>(null);

  const refresh = useCallback(async (): Promise<void> => {
    // Drift must see project patches too: global rows + every project scope.
    const projects = await backend.listProjects().catch(() => []);
    const scopes: ListHookRulesInput[] = AGENTS.flatMap((agent) => [
      { agent, project_id: null },
      ...projects.map((p) => ({ agent, project_id: p.id })),
    ]);
    const [integrations, rowLists] = await Promise.all([
      backend.listAgentIntegrations(),
      Promise.all(scopes.map((scope) => backend.listHookRules(scope).catch(() => []))),
    ]);
    setSummaries(integrations);
    setRows(rowLists.flat());
  }, [backend]);

  useEffect(() => {
    refresh().catch(() => setSummaries([]));
  }, [refresh]);

  function hasInstalledHooks(agent: AgentKindCode): boolean {
    return rows.some((row) => row.agent === agent && row.installed);
  }

  function hasDrift(agent: AgentKindCode): boolean {
    return driftEvents(mergeRowsForDrift(rows.filter((row) => row.agent === agent))).added
      .length > 0;
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
      setEntries((prev) => ({ ...prev, [agent]: result.entries }));
      setNeedsConsent(null);
      setUninstallTarget(null);
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
      setSummaries(
        await backend.detectAgents({ confirm_compatible_version: false }),
      );
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
      <h1>{t.navAgents}</h1>

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
        </div>
      </div>

      {error !== null && (
        <p role="alert">
          {error.message}
          {error.suggested_action !== null && <>（{error.suggested_action}）</>}
        </p>
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
                      {hookActionLabel(
                        t,
                        hasInstalledHooks(agent) || hasDrift(agent)
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

      {(entries["claude-code"] !== undefined || entries["codex"] !== undefined) && (
        <section aria-label={t.lastApplied}>
          <h2>{t.lastApplied}</h2>
          {AGENTS.map((agent) =>
            (entries[agent] ?? []).length > 0 ? (
              <div key={agent}>
                <h3>{agentName(t, agent)}</h3>
                <ul>
                  {(entries[agent] ?? []).map((entry) => (
                    <li key={entry.source_event}>
                      {entry.source_event} · {entryHealthLabel(t, entry.health)}
                    </li>
                  ))}
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
              <span>
                {t.trustNotice} <code>{t.trustCommand}</code>
              </span>
              <button
                type="button"
                className="cc-focusable"
                onClick={() => {
                  void navigator.clipboard?.writeText(t.trustCommand);
                }}
              >
                <ClipboardCopy size={14} aria-hidden="true" /> {t.copyCommand}
              </button>
              <button
                type="button"
                className="cc-focusable"
                onClick={() => {
                  void detect();
                }}
              >
                {t.recheck}
              </button>
            </p>
          )}
        </section>
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
