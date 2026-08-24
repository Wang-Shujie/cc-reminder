// Hook Rules capability table (Task 17): every catalogued row for the selected
// Agent tab, filtered client-side, with a right-hand configuration drawer.
// Health colors are reserved for health; badges stay neutral.
import { useCallback, useEffect, useMemo, useState, type KeyboardEvent, type ReactNode } from "react";
import { Search, X } from "lucide-react";

import { useBackend } from "../lib/backend";
import type {
  AgentKindCode,
  ChannelSummary,
  HealthSnapshot,
  HookRuleRow,
  ListHookRulesInput,
  LocaleCode,
  PatchFieldCode,
  ProjectSummary,
} from "../lib/contracts";
import { dictionary } from "../lib/i18n";
import { HookRuleDrawer } from "./HookRuleDrawer";

/** Scope of the table: global rules, or effective rules for one project. */
export type RulesScope =
  | { scope: "global" }
  | { scope: "project"; project_id: string; project_name: string };

const DRIFT_ISSUE_CODE = "hooks.selection_out_of_date";

type EnabledFilter = "all" | "enabled" | "disabled";
type SensitivityFilter = "all" | "public" | "sensitive" | "forbidden";

export function HookRulesPage({
  locale,
  initialScope,
  health: healthProp,
}: {
  locale: LocaleCode;
  initialScope?: RulesScope;
  /** Shared health from AppShell; when absent the page fetches its own. */
  health?: HealthSnapshot | null;
}): ReactNode {
  const backend = useBackend();
  const t = dictionary(locale);
  const [scope, setScope] = useState<RulesScope>(initialScope ?? { scope: "global" });
  const [agent, setAgent] = useState<AgentKindCode>("claude-code");
  const [rows, setRows] = useState<HookRuleRow[] | null>(null);
  const [channels, setChannels] = useState<ChannelSummary[]>([]);
  const [projects, setProjects] = useState<ProjectSummary[]>([]);
  const [ownHealth, setOwnHealth] = useState<HealthSnapshot | null>(null);
  const [query, setQuery] = useState("");
  const [phase, setPhase] = useState("all");
  const [enabledFilter, setEnabledFilter] = useState<EnabledFilter>("all");
  const [sensitivity, setSensitivity] = useState<SensitivityFilter>("all");
  const [drawerEvent, setDrawerEvent] = useState<string | null>(null);
  const [confirmingApply, setConfirmingApply] = useState(false);

  const projectId = scope.scope === "project" ? scope.project_id : null;
  // Prop presence is stable per mount; keeps `refresh` identity calm.
  const wantsOwnHealth = healthProp === undefined;

  const refresh = useCallback(async (): Promise<void> => {
    const input: ListHookRulesInput = { agent, project_id: projectId };
    const [ruleRows, channelList, snapshot] = await Promise.all([
      backend.listHookRules(input),
      backend.listChannels().catch(() => []),
      wantsOwnHealth ? backend.getHealthSnapshot().catch(() => null) : Promise.resolve(null),
    ]);
    setRows(ruleRows);
    setChannels(channelList);
    if (wantsOwnHealth) {
      setOwnHealth(snapshot);
    }
  }, [backend, agent, projectId, wantsOwnHealth]);

  useEffect(() => {
    let cancelled = false;
    backend
      .listProjects()
      .then((list) => {
        if (!cancelled) {
          setProjects(list);
        }
      })
      .catch(() => {});
    return () => {
      cancelled = true;
    };
  }, [backend]);

  useEffect(() => {
    let cancelled = false;
    setRows(null);
    refresh().catch(() => {
      if (!cancelled) {
        setRows([]);
      }
    });
    return () => {
      cancelled = true;
    };
  }, [refresh]);

  const agentRows = useMemo(
    () => (rows ?? []).filter((row) => row.agent === agent),
    [rows, agent],
  );

  const phases = useMemo(
    () => Array.from(new Set(agentRows.map((row) => row.phase))).sort(),
    [agentRows],
  );

  const filtered = useMemo(
    () =>
      agentRows.filter((row) => {
        const needle = query.trim().toLowerCase();
        const matchesQuery = needle === "" || row.source_event.toLowerCase().includes(needle);
        const matchesPhase = phase === "all" || row.phase === phase;
        const matchesEnabled =
          enabledFilter === "all" || (enabledFilter === "enabled") === row.enabled;
        const matchesSensitivity = sensitivity === "all" || row.sensitivity === sensitivity;
        return matchesQuery && matchesPhase && matchesEnabled && matchesSensitivity;
      }),
    [agentRows, query, phase, enabledFilter, sensitivity],
  );

  /** Effective enablement drives the drift diff (global-scope view). */
  const addedEvents = agentRows.filter((row) => row.available && row.enabled && !row.installed);
  const removedEvents = agentRows.filter((row) => row.installed && !row.enabled);
  const activeHealth = healthProp !== undefined ? healthProp : ownHealth;
  const drift =
    activeHealth?.issues.some((issue) => issue.issue_code === DRIFT_ISSUE_CODE) ?? false;

  async function toggleRow(row: HookRuleRow, next: boolean): Promise<void> {
    if (scope.scope === "project") {
      await backend.saveProjectRulePatch({
        project_id: scope.project_id,
        agent,
        source_event: row.source_event,
        patch: { enabled: next },
      });
    } else {
      await backend.saveGlobalRule({
        agent,
        source_event: row.source_event,
        config: { ...row.config, enabled: next },
      });
    }
    await refresh();
  }

  async function applyRepair(): Promise<void> {
    try {
      await backend.applyHookAction({
        agent,
        action: "repair",
        expected_health_revision: 0,
        // The explicit confirmation dialog above IS the version acknowledgment.
        confirm_compatible_version: true,
      });
    } finally {
      setConfirmingApply(false);
      await refresh();
    }
  }

  function openDrawer(row: HookRuleRow): void {
    setDrawerEvent(row.source_event);
  }

  function onRowKeyDown(event: KeyboardEvent<HTMLTableRowElement>, row: HookRuleRow): void {
    if (event.key === "Enter" || event.key === " ") {
      event.preventDefault();
      openDrawer(row);
    }
  }

  const drawerRow =
    drawerEvent === null
      ? null
      : (agentRows.find((r) => r.source_event === drawerEvent) ?? null);
  const projectNameById = new Map(projects.map((p) => [p.id, p.name]));
  const scopeProjectOptions: { id: string; name: string }[] =
    projects.length > 0
      ? projects.map((p) => ({ id: p.id, name: p.name }))
      : scope.scope === "project"
        ? [{ id: scope.project_id, name: scope.project_name }]
        : [];

  return (
    <section aria-label={t.navHooks}>
      <h1>{t.navHooks}</h1>

      {drift && (
        <p className="drift-status" role="status">
          <span>{t.driftHint}</span>
          <button
            type="button"
            className="cc-focusable"
            onClick={() => setConfirmingApply(true)}
          >
            {t.applyHookChanges}
          </button>
        </p>
      )}

      <div className="rules-toolbar">
        <div role="tablist" aria-label={t.colAgent} className="rules-tabs">
          {(
            [
              ["claude-code", t.agentClaudeCode],
              ["codex", t.agentCodex],
            ] as const
          ).map(([code, label]) => (
            <button
              key={code}
              role="tab"
              type="button"
              aria-selected={agent === code}
              className={`cc-focusable rules-tab${agent === code ? " rules-tab-active" : ""}`}
              onClick={() => setAgent(code)}
            >
              {label}
            </button>
          ))}
        </div>
        <div role="radiogroup" aria-label={t.scopeLabel} className="rules-toolbar-controls">
          <button
            type="button"
            role="radio"
            className={`cc-focusable${scope.scope === "global" ? " scope-active" : ""}`}
            aria-checked={scope.scope === "global"}
            onClick={() => setScope({ scope: "global" })}
          >
            {t.scopeGlobal}
          </button>
          <button
            type="button"
            role="radio"
            className={`cc-focusable${scope.scope === "project" ? " scope-active" : ""}`}
            aria-checked={scope.scope === "project"}
            onClick={() => {
              const first = scopeProjectOptions[0];
              if (first) {
                setScope({
                  scope: "project",
                  project_id: scope.scope === "project" ? scope.project_id : first.id,
                  project_name:
                    scope.scope === "project"
                      ? scope.project_name
                      : (projectNameById.get(first.id) ?? first.name),
                });
              }
            }}
          >
            {t.scopeProject}
          </button>
          {scope.scope === "project" && (
            <select
              aria-label={t.scopeProject}
              value={scope.project_id}
              onChange={(event) => {
                const id = event.target.value;
                setScope({
                  scope: "project",
                  project_id: id,
                  project_name:
                    projectNameById.get(id) ??
                    scopeProjectOptions.find((option) => option.id === id)?.name ??
                    id,
                });
              }}
            >
              {scopeProjectOptions.map((option) => (
                <option key={option.id} value={option.id}>
                  {option.name}
                </option>
              ))}
            </select>
          )}
        </div>
        <div className="rules-toolbar-filters">
          <div className="search-box">
            <Search size={14} aria-hidden="true" />
            <input
              type="search"
              role="searchbox"
              aria-label={t.searchHook}
              value={query}
              onChange={(event) => setQuery(event.target.value)}
            />
            {query !== "" && (
              <button
                type="button"
                className="icon-btn cc-focusable"
                aria-label={t.clearSearch}
                title={t.clearSearch}
                onClick={() => setQuery("")}
              >
                <X size={14} aria-hidden="true" />
              </button>
            )}
          </div>
          <label htmlFor="rules-phase">{t.phaseLabel}</label>
          <select
            id="rules-phase"
            value={phase}
            onChange={(event) => setPhase(event.target.value)}
          >
            <option value="all">{t.filterAll}</option>
            {phases.map((value) => (
              <option key={value} value={value}>
                {value}
              </option>
            ))}
          </select>
          <label htmlFor="rules-enabled">{t.enabledFilterLabel}</label>
          <select
            id="rules-enabled"
            value={enabledFilter}
            onChange={(event) => setEnabledFilter(event.target.value as EnabledFilter)}
          >
            <option value="all">{t.filterAll}</option>
            <option value="enabled">{t.enabledOn}</option>
            <option value="disabled">{t.enabledOff}</option>
          </select>
          <label htmlFor="rules-sensitivity">{t.sensitivityFilterLabel}</label>
          <select
            id="rules-sensitivity"
            value={sensitivity}
            onChange={(event) => setSensitivity(event.target.value as SensitivityFilter)}
          >
            <option value="all">{t.filterAll}</option>
            <option value="public">{t.severityPublic}</option>
            <option value="sensitive">{t.severitySensitive}</option>
            <option value="forbidden">{t.severityForbidden}</option>
          </select>
        </div>
      </div>

      <table className="rules-table">
        <thead>
          <tr>
            <th>{t.colSwitch}</th>
            <th>Hook</th>
            <th>{t.colPhase}</th>
            <th>Agent</th>
            <th>{t.colFrequency}</th>
            <th>{t.colChannels}</th>
            <th>{t.colSource}</th>
            <th>{t.colStatus}</th>
          </tr>
        </thead>
        <tbody>
          {filtered.map((row) => {
            const channelNames = row.config.targets
              .map((target) => channels.find((c) => c.id === target.channel_id)?.name)
              .filter((name): name is string => Boolean(name));
            return (
              <tr
                key={row.source_event}
                tabIndex={0}
                className={row.available ? "" : "row-unavailable"}
                onClick={() => openDrawer(row)}
                onKeyDown={(event) => onRowKeyDown(event, row)}
              >
                <td onClick={(event) => event.stopPropagation()}>
                  <input
                    type="checkbox"
                    role="switch"
                    aria-label={`${t.switchRowPrefix} ${row.source_event}`}
                    checked={row.enabled}
                    disabled={!row.available}
                    onChange={(event) => {
                      void toggleRow(row, event.target.checked);
                    }}
                  />
                </td>
                <td>
                  {row.source_event}
                  {row.status === "experimental" && (
                    <span className="badge">{t.experimentalBadge}</span>
                  )}
                  {row.status === "deprecated" && <span className="badge">{t.deprecatedBadge}</span>}
                </td>
                <td>{row.phase}</td>
                <td>{row.agent === "claude-code" ? t.agentClaudeCode : t.agentCodex}</td>
                <td>
                  {row.high_frequency ? (
                    <span className="badge badge-strong">{t.highFrequency}</span>
                  ) : (
                    <span className="muted">{t.normalFrequency}</span>
                  )}
                </td>
                <td>{channelNames.length > 0 ? channelNames.join("、") : "—"}</td>
                <td>
                  <span className={row.patched_fields.length > 0 ? "" : "muted"}>
                    {row.patched_fields.length > 0 ? t.sourceOverridden : t.sourceGlobal}
                  </span>
                </td>
                <td>{!row.available && <span className="badge">{t.unsupportedVersion}</span>}</td>
              </tr>
            );
          })}
        </tbody>
      </table>
      {rows !== null && filtered.length === 0 && <p className="muted">{t.emptyRules}</p>}

      {drawerRow !== null && (
        <HookRuleDrawer
          key={`${drawerRow.agent}:${drawerRow.source_event}:${scope.scope}`}
          locale={locale}
          agent={agent}
          rule={drawerRow}
          scope={scope}
          channels={channels}
          onClose={() => setDrawerEvent(null)}
          onChanged={() => {
            void refresh();
          }}
        />
      )}

      {confirmingApply && (
        <div className="dialog-overlay">
          <div role="dialog" aria-label={t.confirmApplyTitle} className="dialog">
            <h2>{t.confirmApplyTitle}</h2>
            <h3>{t.applyAdded}</h3>
            {addedEvents.length > 0 ? (
              <ul>
                {addedEvents.map((row) => (
                  <li key={row.source_event}>{row.source_event}</li>
                ))}
              </ul>
            ) : (
              <p className="muted">—</p>
            )}
            <h3>{t.applyRemoved}</h3>
            {removedEvents.length > 0 ? (
              <ul>
                {removedEvents.map((row) => (
                  <li key={row.source_event}>{row.source_event}</li>
                ))}
              </ul>
            ) : (
              <p className="muted">—</p>
            )}
            {agent === "codex" && <p className="trust-item">{t.codexReviewWarn}</p>}
            <div className="row-end">
              <button type="button" className="cc-focusable" onClick={() => setConfirmingApply(false)}>
                {t.cancel}
              </button>
              <button
                type="button"
                className="primary cc-focusable"
                onClick={() => {
                  void applyRepair();
                }}
              >
                {t.confirmApply}
              </button>
            </div>
          </div>
        </div>
      )}
    </section>
  );
}
