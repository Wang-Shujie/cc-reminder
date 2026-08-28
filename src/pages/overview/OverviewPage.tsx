// Overview page (Task 19): a compact metric strip, the shared health issue
// list and recent delivery failures — mirroring get_health_snapshot exactly.
// Not decorative: every issue carries a button navigating to the management
// page that fixes it.
import { type ReactNode } from "react";
import { ArrowRight } from "lucide-react";

import { usePageBackend, type Backend } from "../../lib/backend";
import { useCoreQuery } from "../../lib/useCoreQuery";
import type {
  DeliveryStatusCode,
  HealthSnapshot,
  HistoryItem,
  LocaleCode,
} from "../../lib/contracts";
import { deliveryStatusText, dictionary, type Dictionary } from "../../lib/i18n";

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
  // 统一请求层(架构提案 §1):健康快照 + 失败预览一次取回,随
  // health/queue 事件重取;failures 子查询失败不拖垮整体(记 errored)。
  const boardQuery = useCoreQuery(
    async (b) => {
      const [snap, failedPage] = await Promise.all([
        b.getHealthSnapshot(),
        b
          .listHistory({ delivery_status: "failed", limit: RECENT_FAILURES_LIMIT })
          .catch(() => null),
      ]);
      return { snap, failedItems: failedPage === null ? null : failedPage.items };
    },
    [],
    ["core://health-changed", "core://queue-changed"],
    backend,
  );
  const snapshot = boardQuery.failed
    ? null
    : (boardQuery.data?.snap ?? null);
  /** null = 加载中或子查询失败(≠ 空失败列表)。 */
  const recentFailures = boardQuery.failed
    ? null
    : (boardQuery.data?.failedItems ?? null);
  const failuresErrored =
    !boardQuery.failed && boardQuery.data !== null && boardQuery.data.failedItems === null;
  const loadFailed = boardQuery.failed;
  const refresh = boardQuery.refresh;
  const notice =
    boardQuery.noticeRevision === null
      ? ""
      : `${t.refreshedNotice}(#${boardQuery.noticeRevision})`;


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
          <span className="metric-number">{snapshot.succeeded_jobs}</span>
          <span className="metric-label">{t.metricLabelSent}</span>
        </li>
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
