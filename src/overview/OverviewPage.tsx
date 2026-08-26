// Overview page (Task 19): a compact metric strip, the shared health issue
// list and recent delivery failures — mirroring get_health_snapshot exactly.
// Not decorative: every issue carries a button navigating to the management
// page that fixes it.
import { useCallback, useEffect, useRef, useState, type ReactNode } from "react";
import { ArrowRight } from "lucide-react";

import { usePageBackend, type Backend } from "../lib/backend";
import type {
  DeliveryStatusCode,
  HealthSnapshot,
  HistoryItem,
  LocaleCode,
} from "../lib/contracts";
import { deliveryStatusText, dictionary, type Dictionary } from "../lib/i18n";

const RECENT_FAILURES_LIMIT = 5;

/** Where an issue's repair lives after the 4-destination reorg: a owning page
 *  (rules / integrations) or the workbench's notification log tab. Codes are
 *  prefixed by subsystem; unknown codes default to integrations where most
 *  repairs live. */
type IssueAction =
  | { kind: "page"; page: "rules" | "integrations" }
  | { kind: "history" };

function issueAction(issueCode: string): IssueAction {
  if (
    issueCode.startsWith("queue.") ||
    issueCode.startsWith("delivery.") ||
    issueCode.startsWith("spool")
  ) {
    return { kind: "history" };
  }
  if (issueCode.startsWith("hooks.") || issueCode.startsWith("projects.")) {
    return { kind: "page", page: "rules" };
  }
  return { kind: "page", page: "integrations" };
}

export function OverviewPage({
  locale = "zh_cn",
  backend: injected,
  onNavigate,
  onOpenHistory,
}: {
  locale?: LocaleCode;
  backend?: Backend;
  onNavigate?: (page: "rules" | "integrations") => void;
  onOpenHistory?: (deliveryStatus?: DeliveryStatusCode) => void;
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
        <h2>{t.navOverview}</h2>
        {loadFailed ? <p role="alert">{t.overviewLoadFailed}</p> : <p className="muted">{t.loading}</p>}
      </section>
    );
  }

  const actionLabel = (action: IssueAction): string =>
    action.kind === "history"
      ? t.gotoHistoryTab
      : action.page === "rules"
        ? t.gotoRules
        : t.gotoIntegrations;

  const lastSuccess =
    snapshot.last_success_at === null
      ? t.neverSucceeded
      : new Date(snapshot.last_success_at).toLocaleString();

  return (
    <section aria-label={t.navOverview}>
      <h2>{t.navOverview}</h2>

      {loadFailed && <p role="alert">{t.overviewLoadFailed}</p>}
      <p role="status" className="sr-only">
        {notice}
      </p>

      {/* Compact metric strip — counts mirror the shared snapshot verbatim. */}
      <ul className="metric-strip">
        <li className="metric-plate">
          <span className="metric-number">{snapshot.pending_jobs}</span>
          <span className="metric-label">{t.metricLabelPending}</span>
        </li>
        <li className="metric-plate">
          <span className="metric-number">{snapshot.retry_jobs}</span>
          <span className="metric-label">{t.metricLabelRetry}</span>
        </li>
        <li className="metric-plate">
          <span className="metric-number">{snapshot.failed_jobs}</span>
          <span className="metric-label">{t.metricLabelFailed}</span>
          <button
            type="button"
            className="cc-focusable link-arrow"
            onClick={() => onOpenHistory?.("failed")}
          >
            {t.viewFailedJobs}
            <ArrowRight size={14} aria-hidden="true" />
          </button>
        </li>
        <li className="metric-plate">
          <span className="metric-number">{snapshot.expired_jobs}</span>
          <span className="metric-label">{t.metricLabelExpired}</span>
        </li>
        <li className="metric-last muted">
          <span>{t.lastSuccessLabel.replace("{time}", lastSuccess)}</span>
        </li>
      </ul>

      {/* 导视纪律(用户裁决):无待处理问题即无牌面——空态不占位。 */}
      {snapshot.issues.length > 0 && (
        <section aria-label={t.overviewIssues}>
          <h2>{t.overviewIssues}</h2>
          <ul className="issue-list">
            {snapshot.issues.map((issue) => {
              const action = issueAction(issue.issue_code);
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
                    className="cc-focusable link-arrow"
                    onClick={() =>
                      action.kind === "history"
                        ? onOpenHistory?.()
                        : onNavigate?.(action.page)
                    }
                  >
                    {actionLabel(action)}
                    <ArrowRight size={14} aria-hidden="true" />
                  </button>
                </li>
              );
            })}
          </ul>
        </section>
      )}

      {/* 同上:无失败即无牌面;仅加载失败时保留提示行。 */}
      {(failuresErrored || (recentFailures?.length ?? 0) > 0) && (
        <section aria-label={t.recentFailuresTitle} className="page-subsection">
          <h2>{t.recentFailuresTitle}</h2>
          {failuresErrored ? (
            <p className="muted">{t.recentFailuresLoadFailed}</p>
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
                {recentFailures!.map((item) => (
                  <tr key={item.event_id} className="hazard-row">
                    <td>{new Date(item.occurred_at).toLocaleString()}</td>
                    <td>{item.source_event}</td>
                    <td>{deliveryStatusText(t, item.delivery_status)}</td>
                  </tr>
                ))}
              </tbody>
            </table>
          )}
        </section>
      )}
    </section>
  );
}
