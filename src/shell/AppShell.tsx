// Quiet desktop shell: 184px rail + 48px header + unframed content, one grid.
// Health colors are the only accents; everything else is neutral.
import { useEffect, useState, type ReactNode } from "react";
import {
  Bot,
  FolderGit2,
  History,
  LayoutDashboard,
  ListChecks,
  Settings as SettingsIcon,
  Webhook,
} from "lucide-react";

import { useBackend } from "../lib/backend";
import { AgentsPage } from "../agents/AgentsPage";
import { ChannelsPage } from "../channels/ChannelsPage";
import { HookRulesPage } from "../hooks/HookRulesPage";
import { ProjectsPage } from "../projects/ProjectsPage";
import {
  CORE_EVENTS,
  type HealthSnapshot,
  type LocaleCode,
  type ThemeCode,
} from "../lib/contracts";
import { dictionary, type Dictionary } from "../lib/i18n";

export type PageId =
  | "overview"
  | "agents"
  | "hooks"
  | "channels"
  | "projects"
  | "history"
  | "settings";

const PAGES: readonly {
  id: PageId;
  icon: typeof Bot;
  label: (d: Dictionary) => string;
}[] = [
  { id: "overview", icon: LayoutDashboard, label: (d) => d.navOverview },
  { id: "agents", icon: Bot, label: (d) => d.navAgents },
  { id: "hooks", icon: ListChecks, label: (d) => d.navHooks },
  { id: "channels", icon: Webhook, label: (d) => d.navChannels },
  { id: "projects", icon: FolderGit2, label: (d) => d.navProjects },
  { id: "history", icon: History, label: (d) => d.navHistory },
  { id: "settings", icon: SettingsIcon, label: (d) => d.navSettings },
];

const LAST_PAGE_KEY = "cc-reminder:last-page";

function savedPage(): PageId {
  const value = localStorage.getItem(LAST_PAGE_KEY);
  return PAGES.some((page) => page.id === value) ? (value as PageId) : "hooks";
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
    setPage(id);
    localStorage.setItem(LAST_PAGE_KEY, id);
  }

  function labelFor(id: PageId): string {
    const page = PAGES.find((candidate) => candidate.id === id);
    return page ? page.label(t) : "";
  }

  const overall = health?.overall ?? "ok";
  const active = PAGES.find((candidate) => candidate.id === page);

  return (
    <div className="shell-root" data-overall={overall}>
      <header className="shell-header">
        <span className="shell-title">{t.statusTitle}</span>
        <span className={`health-dot health-${overall}`} aria-hidden="true" />
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
      <main className="shell-content">
        {page === "hooks" ? (
          <HookRulesPage locale={locale} />
        ) : page === "agents" ? (
          <AgentsPage locale={locale} />
        ) : page === "channels" ? (
          <ChannelsPage locale={locale} />
        ) : page === "projects" ? (
          <ProjectsPage locale={locale} />
        ) : (
          <>
            <h1>{labelFor(page)}</h1>
            <p className="muted">{active ? t.pagePlaceholder : ""}</p>
          </>
        )}
      </main>
    </div>
  );
}
