// Overview page (Task 19): a compact metric strip, the shared health issue
// list and recent delivery failures — mirroring get_health_snapshot exactly.
// Not decorative: every issue carries a button navigating to the management
// page that fixes it.
import { useCallback, useEffect, useRef, useState, type ReactNode } from "react";

import { usePageBackend, type Backend } from "../lib/backend";
import type {
  DeliveryStatusCode,
  HealthSnapshot,
  HistoryItem,
  LocaleCode,
} from "../lib/contracts";
import { deliveryStatusText, dictionary, type Dictionary } from "../lib/i18n";
import type { PageId } from "../shell/AppShell";

/** Optional preset applied to the History page after an overview jump. */
export interface HistorySeed {
  delivery_status?: DeliveryStatusCode;
}

const RECENT_FAILURES_LIMIT = 5;

/** Owning management page for an issue code. Codes are prefixed by subsystem;
 *  unknown codes default to Agent integrations where most repairs live. */
function issuePage(issueCode: string): PageId {
  if (issueCode.startsWith("channel.") || issueCode.startsWith("credentials.")) {
    return "channels";
  }
  if (
    issueCode.startsWith("queue.") ||
    issueCode.startsWith("delivery.") ||
    issueCode.startsWith("spool")
  ) {
    return "history";
  }
  return "agents";
}

function metricText(template: string, n: number): string {
  return template.replace("{n}", String(n));
}

export function OverviewPage({
  locale = "zh_cn",
  backend: injected,
  onNavigate,
}: {
  locale?: LocaleCode;
  backend?: Backend;
  onNavigate?: (page: PageId, seed?: HistorySeed) => void;
}): ReactNode {
  const backend = usePageBackend(injected);
  const t = dictionary(locale);
  const [snapshot, setSnapshot] = useState<HealthSnapshot | null>(null);
  const [recentFailures, setRecentFailures] = useState<HistoryItem[] | null>(null);
  /** True when the failures query itself failed (≠ an empty failure list). */
  const [failuresErrored, setFailuresErrored] = useState(false);
  const [loadFailed, setLoadFailed] = useState(false);
  /** Polite live-region content for background refreshes (never moves focus). */
  const [notice, setNotice] = useState("");
  const mounted = useRef(true);

  useEffect(() => {
    mounted.current = true;
    return () => {
      mounted.current = false;
    };
  }, []);

  const refresh = useCallback(async (): Promise<void> => {
    const [snap, failedPage] = await Promise.all([
      backend.getHealthSnapshot(),
      backend
        .listHistory({ delivery_status: "failed", limit: RECENT_FAILURES_LIMIT })
        .catch(() => null),
    ]);
    if (!mounted.current) return;
    setSnapshot(snap);
    if (failedPage === null) {
      // The failures query failed: say so instead of claiming 没有失败任务.
      setRecentFailures(null);
      setFailuresErrored(true);
    } else {
      setRecentFailures(failedPage.items);
      setFailuresErrored(false);
    }
  }, [backend]);

  useEffect(() => {
    let cancelled = false;
    refresh().catch(() => {
      if (!cancelled) {
        setLoadFailed(true);
      }
    });
    return () => {
      cancelled = true;
    };
  }, [refresh]);

  useEffect(() => {
    const events = ["core://health-changed", "core://queue-changed"] as const;
    const subscriptions = events.map((event) =>
      backend.subscribe(event, (revision: number) => {
        refresh()
          .then(() => {
            if (mounted.current) {
              setNotice(`${t.refreshedNotice}(#${revision})`);
            }
          })
          .catch(() => {
            /* keep the last snapshot on transient failures */
          });
      }),
    );
    return () => {
      for (const subscription of subscriptions) {
        subscription.then((unlisten) => unlisten()).catch(() => {});
      }
    };
  }, [backend, refresh, t.refreshedNotice]);

  if (snapshot === null) {
    return (
      <section aria-label={t.navOverview}>
        <h1>{t.navOverview}</h1>
        {loadFailed ? <p role="alert">{t.overviewLoadFailed}</p> : <p className="muted">{t.loading}</p>}
      </section>
    );
  }

  const navLabelFor = (page: PageId): string =>
    page === "agents" ? t.gotoAgents : page === "channels" ? t.gotoChannels : t.gotoHistory;

  const lastSuccess =
    snapshot.last_success_at === null
      ? t.neverSucceeded
      : new Date(snapshot.last_success_at).toLocaleString();

  return (
    <section aria-label={t.navOverview}>
      <h1>{t.navOverview}</h1>

      {loadFailed && <p role="alert">{t.overviewLoadFailed}</p>}
      <p role="status" className="sr-only">
        {notice}
      </p>

      {/* Compact metric strip — counts mirror the shared snapshot verbatim. */}
      <ul className="metric-strip">
        <li>
          <span>{metricText(t.metricPending, snapshot.pending_jobs)}</span>
        </li>
        <li>
          <span>{metricText(t.metricRetry, snapshot.retry_jobs)}</span>
        </li>
        <li>
          <span>{metricText(t.metricFailed, snapshot.failed_jobs)}</span>
          <button
            type="button"
            className="cc-focusable"
            onClick={() => onNavigate?.("history", { delivery_status: "failed" })}
          >
            {t.viewFailedJobs}
          </button>
        </li>
        <li>
          <span>{metricText(t.metricExpired, snapshot.expired_jobs)}</span>
        </li>
        <li className="muted">
          <span>{t.lastSuccessLabel.replace("{time}", lastSuccess)}</span>
        </li>
      </ul>

      <section aria-label={t.overviewIssues}>
        <h2>{t.overviewIssues}</h2>
        {snapshot.issues.length === 0 ? (
          <p className="muted">{t.noIssues}</p>
        ) : (
          <ul className="issue-list">
            {snapshot.issues.map((issue) => {
              const target = issuePage(issue.issue_code);
              return (
                <li key={`${issue.issue_code}:${issue.message}`}>
                  {issue.level !== "ok" && (
                    <span
                      className={`health-dot health-${issue.level}`}
                      aria-hidden="true"
                    />
                  )}
                  <span>{issue.message}</span>
                  {issue.suggested_command !== null && (
                    <code className="inline-code">{issue.suggested_command}</code>
                  )}
                  {issue.suggested_action !== null && (
                    <span className="muted">{issue.suggested_action}</span>
                  )}
                  <button
                    type="button"
                    className="cc-focusable"
                    onClick={() => onNavigate?.(target)}
                  >
                    {navLabelFor(target)}
                  </button>
                </li>
              );
            })}
          </ul>
        )}
      </section>

      <section aria-label={t.recentFailuresTitle} className="page-subsection">
        <h2>{t.recentFailuresTitle}</h2>
        {recentFailures === null ? (
          failuresErrored ? (
            <p className="muted">{t.recentFailuresLoadFailed}</p>
          ) : null
        ) : recentFailures.length === 0 ? (
          <p className="muted">{t.noRecentFailures}</p>
        ) : (
          <table className="rules-table">
            <thead>
              <tr>
                <th>{t.colTime}</th>
                <th>{t.colHook}</th>
                <th>{t.filterResult}</th>
              </tr>
            </thead>
            <tbody>
              {recentFailures.map((item) => (
                <tr key={item.event_id}>
                  <td>{new Date(item.occurred_at).toLocaleString()}</td>
                  <td>{item.source_event}</td>
                  <td>{deliveryStatusText(t, item.delivery_status)}</td>
                </tr>
              ))}
            </tbody>
          </table>
        )}
      </section>
    </section>
  );
}
