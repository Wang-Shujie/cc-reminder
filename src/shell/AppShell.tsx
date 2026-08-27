// Quiet desktop shell: 184px rail + 48px header + unframed content, one grid.
// Five destinations (v2.1 第四轮裁决:规则/项目拆分);Health colors are the
// only accents; everything else is neutral.
import { useEffect, useRef, useState, type ReactNode } from "react";
import {
  FolderGit2,
  LayoutDashboard,
  ListChecks,
  Settings as SettingsIcon,
  Webhook,
} from "lucide-react";

import { useBackend } from "../lib/backend";
import { HookRulesPage } from "../pages/rules/HookRulesPage";
import { IntegrationsPage } from "../pages/integrations/IntegrationsPage";
import { ProjectsPage } from "../pages/projects/ProjectsPage";
import { SettingsPage } from "../pages/settings/SettingsPage";
import { WorkbenchPage } from "../pages/workbench/WorkbenchPage";
import {
  CORE_EVENTS,
  type HealthSnapshot,
  type LocaleCode,
  type ThemeCode,
} from "../lib/contracts";
import { dictionary, type Dictionary } from "../lib/i18n";

export type PageId =
  | "workbench"
  | "rules"
  | "projects"
  | "integrations"
  | "settings";

const PAGES: readonly {
  id: PageId;
  icon: typeof Webhook;
  label: (d: Dictionary) => string;
}[] = [
  { id: "workbench", icon: LayoutDashboard, label: (d) => d.navWorkbench },
  { id: "rules", icon: ListChecks, label: (d) => d.navRules },
  { id: "projects", icon: FolderGit2, label: (d) => d.navProjects },
  { id: "integrations", icon: Webhook, label: (d) => d.navIntegrations },
  { id: "settings", icon: SettingsIcon, label: (d) => d.navSettings },
];

/** v1 页 ID → v2 目的地(读时映射,写时永远写新 ID)。 */
const LEGACY_PAGE_MAP: Record<string, PageId> = {
  overview: "workbench",
  history: "workbench",
  hooks: "rules",
  projects: "projects",
  agents: "integrations",
  channels: "integrations",
  settings: "settings",
};

const LAST_PAGE_KEY = "cc-reminder:last-page";

function savedPage(): PageId {
  const value = localStorage.getItem(LAST_PAGE_KEY);
  if (value !== null && value in LEGACY_PAGE_MAP) {
    return LEGACY_PAGE_MAP[value]!;
  }
  return PAGES.some((page) => page.id === value) ? (value as PageId) : "workbench";
}

export function AppShell({
  locale,
  theme,
}: {
  locale: LocaleCode;
  theme: ThemeCode;
}): ReactNode {
  const backend = useBackend();
  const t = dictionary(locale);
  const contentRef = useRef<HTMLElement | null>(null);
  const [page, setPage] = useState<PageId>(savedPage);
  const [health, setHealth] = useState<HealthSnapshot | null>(null);

  // One snapshot on mount; revision events trigger a refetch. Event payloads
  // are never trusted for state — only the revision number arrives here.
  useEffect(() => {
    let cancelled = false;
    const refresh = () => {
      backend
        .getHealthSnapshot()
        .then((snapshot) => {
          if (!cancelled) {
            setHealth(snapshot);
          }
        })
        .catch(() => {
          /* offline in tests / transient core error: keep last snapshot */
        });
    };
    refresh();
    const subscriptions = CORE_EVENTS.map((event) =>
      backend.subscribe(event, () => {
        refresh();
      }),
    );
    return () => {
      cancelled = true;
      for (const subscription of subscriptions) {
        subscription.then((unlisten) => unlisten()).catch(() => {});
      }
    };
  }, [backend]);

  useEffect(() => {
    document.documentElement.dataset.theme = theme;
  }, [theme]);

  function openPage(id: PageId): void {
    // Destinations always open at the top — a tall settings scroll must not
    // leak into the next page's first paint.
    setPage(id);
    localStorage.setItem(LAST_PAGE_KEY, id);
    if (contentRef.current !== null) {
      contentRef.current.scrollTop = 0;
    }
  }

  const overall = health?.overall ?? "ok";

  return (
    <div className="shell-root" data-overall={overall}>
      <header className="shell-header">
        <span className="shell-title">{t.statusTitle}</span>
        <span className={`health-dot health-${overall}`} aria-hidden="true" />
        <span className="shell-status">
          {overall === "ok"
            ? t.statusWordOk
            : overall === "warning"
              ? t.statusWordWarning
              : t.statusWordError}
        </span>
        <span className="shell-counts">
          {t.pendingJobs}: {health?.pending_jobs ?? 0} · {t.failedJobs}:{" "}
          {health?.failed_jobs ?? 0}
        </span>
      </header>
      <nav className="shell-nav" aria-label={t.navLabel}>
        {PAGES.map(({ id, icon: Icon, label }) => (
          <button
            key={id}
            type="button"
            className={`nav-item cc-focusable${page === id ? " nav-active" : ""}`}
            aria-current={page === id ? "page" : undefined}
            onClick={() => openPage(id)}
          >
            <Icon size={16} aria-hidden="true" />
            <span>{label(t)}</span>
          </button>
        ))}
      </nav>
      <main className="shell-content" ref={contentRef}>
        {page === "workbench" ? (
          <WorkbenchPage locale={locale} onNavigate={openPage} />
        ) : page === "rules" ? (
          <HookRulesPage locale={locale} />
        ) : page === "projects" ? (
          <ProjectsPage locale={locale} />
        ) : page === "integrations" ? (
          <IntegrationsPage locale={locale} onNavigate={openPage} />
        ) : (
          <SettingsPage locale={locale} />
        )}
      </main>
    </div>
  );
}
