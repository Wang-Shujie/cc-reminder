// Projects page (Task 18): canonical roots, path aliases, per-agent override
// counts. Adding a project is ONLY possible through a user-selected directory
// (injected folder picker; production wires the Tauri dialog plugin lazily).
// No whole-disk scan exists anywhere on this page.
import { useCallback, useEffect, useMemo, useState, type ReactNode } from "react";
import { Unlink } from "lucide-react";

import { usePageBackend, type Backend } from "../lib/backend";
import { errorOf, type PageError } from "../lib/errors";
import type {
  AgentKindCode,
  ListHookRulesInput,
  LocaleCode,
  ProjectId,
  ProjectSummary,
  WorktreeModeCode,
} from "../lib/contracts";
import { dictionary } from "../lib/i18n";

const AGENTS: readonly AgentKindCode[] = ["claude-code", "codex"];

/** Production folder picker: native directory-open dialog only, loaded lazily
 *  so tests (which inject their own picker) never touch the plugin module. */
async function nativeFolderPicker(): Promise<string | null> {
  const { open } = await import("@tauri-apps/plugin-dialog");
  const selection = await open({ directory: true, multiple: false });
  return typeof selection === "string" ? selection : null;
}

function basename(path: string): string {
  const parts = path.split(/[\\/]/).filter(Boolean);
  return parts[parts.length - 1] ?? path;
}

interface AddModalState {
  selectedPath: string;
}

export function ProjectsPage({
  locale = "zh_cn",
  backend: injected,
  dialog,
}: {
  locale?: LocaleCode;
  backend?: Backend;
  /** Injected folder picker returning the user-chosen directory or null. */
  dialog?: () => Promise<string | null>;
}): ReactNode {
  const backend = usePageBackend(injected);
  const t = dictionary(locale);
  const pickFolder = dialog ?? nativeFolderPicker;

  const [projects, setProjects] = useState<ProjectSummary[] | null>(null);
  const [agentFilter, setAgentFilter] = useState<"all" | AgentKindCode>("all");
  const [modal, setModal] = useState<AddModalState | null>(null);
  const [choice, setChoice] = useState<WorktreeModeCode>("alias");
  const [nameDraft, setNameDraft] = useState("");
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<PageError | null>(null);
  const [confirmRemovePath, setConfirmRemovePath] = useState<{
    path_id: string;
    path: string;
  } | null>(null);

  /** Per-project override rows fetched for BOTH agents so the Agent column
   *  shows who overrides while the count follows the selected agent. */
  const [overrides, setOverrides] = useState<
    Record<string, Partial<Record<AgentKindCode, number>>>
  >({});

  const refresh = useCallback(async (): Promise<void> => {
    const list = await backend.listProjects();
    setProjects(list);
    const scopes: ListHookRulesInput[] = list.flatMap((project) =>
      AGENTS.map((agent) => ({ agent, project_id: project.id })),
    );
    const ruleLists = await Promise.all(
      scopes.map((scope) => backend.listHookRules(scope).catch(() => [])),
    );
    const next: typeof overrides = {};
    scopes.forEach((scope, index) => {
      const count = (ruleLists[index] ?? []).filter(
        (row) => row.patched_fields.length > 0,
      ).length;
      next[scope.project_id as string] = {
        ...next[scope.project_id as string],
        [scope.agent]: count,
      };
    });
    setOverrides(next);
  }, [backend]);

  useEffect(() => {
    refresh().catch(() => setProjects([]));
  }, [refresh]);

  async function addProject(): Promise<void> {
    setError(null);
    // The picker is the ONLY entry point: no scan, no manual path typing.
    const picked = await pickFolder();
    if (picked === null || picked === "") {
      return;
    }
    setNameDraft(basename(picked));
    // Default per design: a worktree joins the existing project as an alias.
    setChoice("alias");
    setModal({ selectedPath: picked });
  }

  async function save(): Promise<void> {
    if (modal === null) {
      return;
    }
    setSaving(true);
    setError(null);
    try {
      await backend.saveProject({
        project_id: null,
        name: nameDraft.trim() === "" ? basename(modal.selectedPath) : nameDraft.trim(),
        canonical_root: modal.selectedPath,
        worktree_mode: choice,
        // The core canonicalizes this server-side and probes only this
        // directory + its parents for the Git root.
        selected_path: modal.selectedPath,
      });
      setModal(null);
      await refresh();
    } catch (e: unknown) {
      setError(errorOf(e));
    } finally {
      setSaving(false);
    }
  }

  async function removeAlias(pathId: string): Promise<void> {
    setError(null);
    try {
      await backend.removeProjectAlias({ path_id: pathId });
      setConfirmRemovePath(null);
      await refresh();
    } catch (e: unknown) {
      setConfirmRemovePath(null);
      setError(errorOf(e));
    }
  }

  const overrideCount = (project: ProjectSummary): number => {
    const perAgent = overrides[project.id] ?? {};
    if (agentFilter === "all") {
      return Object.values(perAgent).reduce((sum, n) => sum + (n ?? 0), 0);
    }
    return perAgent[agentFilter] ?? 0;
  };

  const overrideAgents = useMemo(
    () =>
      new Map(
        (projects ?? []).map((project) => [
          project.id,
          AGENTS.filter((agent) => (overrides[project.id]?.[agent] ?? 0) > 0),
        ]),
      ),
    [projects, overrides],
  );

  return (
    <section aria-label={t.navProjects}>
      <h1>{t.navProjects}</h1>

      <div className="rules-toolbar">
        <div className="rules-toolbar-controls">
          <button
            type="button"
            className="cc-focusable"
            onClick={() => {
              void addProject();
            }}
          >
            {t.addProjectBtn}
          </button>
          <label htmlFor="project-agent-filter">{t.selectAgent}</label>
          <select
            id="project-agent-filter"
            aria-label={t.selectAgent}
            value={agentFilter}
            onChange={(event) => {
              const value = event.target.value;
              setAgentFilter(value === "all" ? "all" : (value as AgentKindCode));
            }}
          >
            <option value="all">{t.allAgentsOption}</option>
            <option value="claude-code">{t.agentClaudeCode}</option>
            <option value="codex">{t.agentCodex}</option>
          </select>
        </div>
      </div>

      {/* Explicit boundary statement: selection-driven inspection only. */}
      <p className="muted">{t.scanBoundaryNote}</p>

      {/* Page-level error only when no modal owns it (the modal renders its
          own alert so a failed save never produces two alerts at once). */}
      {modal === null && error !== null && (
        <p role="alert">
          {error.message}
          {error.suggested_action !== null && <>（{error.suggested_action}）</>}
        </p>
      )}

      <table className="rules-table">
        <thead>
          <tr>
            <th>{t.colProject}</th>
            <th>{t.colRoot}</th>
            <th>{t.colAliases}</th>
            <th>{t.colAgent}</th>
            <th>{t.colOverrides}</th>
          </tr>
        </thead>
        <tbody>
          {(projects ?? []).map((project) => (
            <tr key={project.id}>
              <td>{project.name}</td>
              <td>
                {project.canonical_root}
                {project.git_root !== undefined &&
                  project.git_root !== null &&
                  project.git_root !== project.canonical_root && (
                    <>
                      {" "}
                      <span className="muted">({project.git_root})</span>
                    </>
                  )}
              </td>
              <td>
                {project.paths.filter((path) => path.kind !== "root").length === 0 ? (
                  <span className="muted">—</span>
                ) : (
                  <ul className="alias-list">
                    {project.paths
                      .filter((path) => path.kind !== "root")
                      .map((path) => (
                        <li key={path.id}>
                          <span>{path.canonical_path}</span>
                          <button
                            type="button"
                            className="icon-btn cc-focusable"
                            aria-label={`${t.removeAliasBtn} ${path.canonical_path}`}
                            title={t.removeAliasBtn}
                            onClick={() =>
                              setConfirmRemovePath({
                                path_id: path.id,
                                path: path.canonical_path,
                              })
                            }
                          >
                            <Unlink size={14} aria-hidden="true" />
                          </button>
                        </li>
                      ))}
                  </ul>
                )}
              </td>
              <td>
                {(overrideAgents.get(project.id) ?? []).length === 0 ? (
                  <span className="muted">—</span>
                ) : (
                  (overrideAgents.get(project.id) ?? [])
                    .map((agent) => (agent === "codex" ? t.agentCodex : t.agentClaudeCode))
                    .join("、")
                )}
              </td>
              <td>{overrideCount(project)}</td>
            </tr>
          ))}
        </tbody>
      </table>
      {projects !== null && projects.length === 0 && <p className="muted">{t.noProjects}</p>}

      {modal !== null && (
        <div className="dialog-overlay">
          <div role="dialog" aria-label={t.addProjectBtn} className="dialog">
            <h2>{t.addProjectBtn}</h2>
            <p>
              {t.pickedPathLabel}: <code>{modal.selectedPath}</code>
            </p>
            <fieldset>
              <legend>{t.worktreeChoiceLegend}</legend>
              {/* Default: worktrees join the existing project as aliases. */}
              <label className="check-row">
                <input
                  type="radio"
                  name="worktree-mode"
                  checked={choice === "alias"}
                  onChange={() => setChoice("alias")}
                />
                <span>{t.aliasChoiceLabel}</span>
              </label>
              <label className="check-row">
                <input
                  type="radio"
                  name="worktree-mode"
                  checked={choice === "separate"}
                  onChange={() => setChoice("separate")}
                />
                <span>{t.separateChoiceLabel}</span>
              </label>
            </fieldset>
            <label htmlFor="project-name">{t.projectNameField}</label>
            <input
              id="project-name"
              value={nameDraft}
              onChange={(event) => setNameDraft(event.target.value)}
            />
            {error !== null && (
              <p role="alert">
                {error.message}
                {error.suggested_action !== null && <>（{error.suggested_action}）</>}
              </p>
            )}
            <div className="row-end">
              <button
                type="button"
                className="cc-focusable"
                onClick={() => setModal(null)}
              >
                {t.cancel}
              </button>
              <button
                type="button"
                className="primary cc-focusable"
                disabled={saving}
                onClick={() => {
                  void save();
                }}
              >
                {t.saveBtn}
              </button>
            </div>
          </div>
        </div>
      )}

      {confirmRemovePath !== null && (
        <div className="dialog-overlay">
          <div
            role="dialog"
            aria-label={t.removeAliasConfirmTitle}
            className="dialog"
          >
            <h2>{t.removeAliasConfirmTitle}</h2>
            <p>{confirmRemovePath.path}</p>
            <p className="trust-item">{t.removeAliasNote}</p>
            <div className="row-end">
              <button
                type="button"
                className="cc-focusable"
                onClick={() => setConfirmRemovePath(null)}
              >
                {t.cancel}
              </button>
              <button
                type="button"
                className="primary cc-focusable"
                onClick={() => {
                  void removeAlias(confirmRemovePath.path_id);
                }}
              >
                {t.confirmRemove}
              </button>
            </div>
          </div>
        </div>
      )}
    </section>
  );
}
