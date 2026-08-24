// Notification History page (Task 19): semantic table with time/project/Hook/
// channel/result filters, bounded pagination honoring next_offset, one detail
// drawer carrying the redacted document and the attempt timeline, and manual
// retry for ELIGIBLE failed jobs only (expired is terminal).
//
// Privacy: the drawer renders only backend-redacted content. Unmatched
// working directories appear solely as their fingerprint hash — a resolved
// path never reaches this component through the contract, and no field that
// could carry one is ever rendered.
import { useCallback, useEffect, useRef, useState, type KeyboardEvent, type ReactNode } from "react";
import { X } from "lucide-react";

import { usePageBackend, type Backend } from "../lib/backend";
import { errorOf, type PageError } from "../lib/errors";
import type {
  DeliveryStatusCode,
  HistoryItem,
  ListHistoryInput,
  LocaleCode,
} from "../lib/contracts";
import { deliveryStatusText, dictionary } from "../lib/i18n";

const PAGE_SIZE = 10;

interface Filters {
  occurred_from: string;
  occurred_until: string;
  project_id: string;
  source_event: string;
  channel_id: string;
  delivery_status: string;
}

const EMPTY_FILTERS: Filters = {
  occurred_from: "",
  occurred_until: "",
  project_id: "",
  source_event: "",
  channel_id: "",
  delivery_status: "",
};

function filterInput(f: Filters): ListHistoryInput {
  return {
    // <input type="date"> yields bare YYYY-MM-DD; the core's DateTime<Utc>
    // needs RFC3339. "From" covers its whole first day, "until" its last.
    occurred_from: f.occurred_from === "" ? null : `${f.occurred_from}T00:00:00Z`,
    occurred_until: f.occurred_until === "" ? null : `${f.occurred_until}T23:59:59Z`,
    project_id: f.project_id === "" ? null : f.project_id,
    source_event: f.source_event === "" ? null : f.source_event,
    channel_id: f.channel_id === "" ? null : f.channel_id,
    delivery_status:
      f.delivery_status === "" ? null : (f.delivery_status as DeliveryStatusCode),
  };
}

export function HistoryPage({
  locale = "zh_cn",
  backend: injected,
  initialDeliveryStatus = null,
}: {
  locale?: LocaleCode;
  backend?: Backend;
  initialDeliveryStatus?: DeliveryStatusCode | null;
}): ReactNode {
  const backend = usePageBackend(injected);
  const t = dictionary(locale);
  const [filters, setFilters] = useState<Filters>(() => ({
    ...EMPTY_FILTERS,
    delivery_status: initialDeliveryStatus ?? "",
  }));
  /** Free-text Hook filter commits on Enter; the draft stays local meanwhile. */
  const [hookDraft, setHookDraft] = useState("");
  const [items, setItems] = useState<HistoryItem[] | null>(null);
  const [nextOffset, setNextOffset] = useState<number | null>(null);
  const [loadingMore, setLoadingMore] = useState(false);
  const [loadError, setLoadError] = useState<PageError | null>(null);
  const [projects, setProjects] = useState<{ id: string; name: string }[]>([]);
  const [channels, setChannels] = useState<{ id: string; name: string }[]>([]);
  /** Open detail drawer; item fills in asynchronously from get_history_detail. */
  const [detailEventId, setDetailEventId] = useState<string | null>(null);
  const [detailItem, setDetailItem] = useState<HistoryItem | null>(null);
  const [detailError, setDetailError] = useState<PageError | null>(null);
  /** Latest drawer request; late get_history_detail responses for other ids
   *  are dropped instead of leaking into the open drawer. */
  const detailRequestRef = useRef<string | null>(null);
  const [retryTarget, setRetryTarget] = useState<HistoryItem | null>(null);
  const [retrying, setRetrying] = useState(false);
  const [retryQueuedNotice, setRetryQueuedNotice] = useState(false);
  const [actionError, setActionError] = useState<PageError | null>(null);
  const [bgNotice, setBgNotice] = useState("");
  /** Focus management: return to the initiating control when dialogs close. */
  const triggerRef = useRef<HTMLElement | null>(null);
  const retryTriggerRef = useRef<HTMLButtonElement | null>(null);
  const retryConfirmRef = useRef<HTMLButtonElement | null>(null);
  const closeRef = useRef<HTMLButtonElement | null>(null);
  /** Set when the retry dialog closes; a post-commit effect restores focus
   *  (a synchronous focus call races the dialog's own unmount). */
  const pendingFocusRestore = useRef(false);

  useEffect(() => {
    if (retryTarget === null && pendingFocusRestore.current) {
      pendingFocusRestore.current = false;
      retryTriggerRef.current?.focus();
    }
  }, [retryTarget]);

  // Focus moves to the confirm button when the retry dialog opens.
  useEffect(() => {
    if (retryTarget !== null) {
      retryConfirmRef.current?.focus();
    }
  }, [retryTarget]);

  /** Monotonic request sequence: only the latest load may touch state, so a
   *  slow response can never overwrite a newer filter's results. */
  const seqRef = useRef(0);

  const load = useCallback(
    async (offset: number, append: boolean): Promise<boolean> => {
      const seq = ++seqRef.current;
      try {
        const page = await backend.listHistory({
          ...filterInput(filters),
          offset,
          limit: PAGE_SIZE,
        });
        if (seq !== seqRef.current) {
          return false;
        }
        setItems((prev) => (append && prev !== null ? [...prev, ...page.items] : page.items));
        setNextOffset(page.next_offset);
        setLoadError(null);
        return true;
      } catch (e: unknown) {
        // A superseded failure is discarded too; an append failure keeps the
        // already-loaded rows.
        if (seq !== seqRef.current) {
          return false;
        }
        if (!append) {
          setItems([]);
          setNextOffset(null);
        }
        setLoadError(errorOf(e));
        return false;
      }
    },
    [backend, filters],
  );

  useEffect(() => {
    void load(0, false);
  }, [load]);

  useEffect(() => {
    let cancelled = false;
    backend
      .listProjects()
      .then((list) => {
        if (!cancelled) setProjects(list.map((p) => ({ id: p.id, name: p.name })));
      })
      .catch(() => {});
    backend
      .listChannels()
      .then((list) => {
        if (!cancelled) setChannels(list.map((c) => ({ id: c.id, name: c.name })));
      })
      .catch(() => {});
    return () => {
      cancelled = true;
    };
  }, [backend]);

  // Background refresh on core pushes: refetch page one politely.
  useEffect(() => {
    const subscription = backend.subscribe("core://history-changed", (revision: number) => {
      load(0, false)
        .then((applied) => {
          if (applied) {
            setBgNotice(`${t.historyUpdated}(#${revision})`);
          }
        })
        .catch(() => {});
    });
    return () => {
      subscription.then((unlisten) => unlisten()).catch(() => {});
    };
  }, [backend, load, t.historyUpdated]);

  // Focus moves into the drawer once it opens. The request id is checked on
  // resolution so switching drawers discards the previous event's response.
  useEffect(() => {
    detailRequestRef.current = detailEventId;
    if (detailEventId !== null) {
      const requestedId = detailEventId;
      setDetailItem(null);
      setDetailError(null);
      closeRef.current?.focus();
      backend
        .getHistoryDetail({ event_id: requestedId })
        .then((page) => {
          if (detailRequestRef.current === requestedId) {
            setDetailItem(page.items[0] ?? null);
          }
        })
        .catch((e: unknown) => {
          if (detailRequestRef.current === requestedId) {
            setDetailError(errorOf(e));
          }
        });
    }
  }, [backend, detailEventId]);

  function closeDrawer(): void {
    setDetailEventId(null);
    setDetailItem(null);
    triggerRef.current?.focus();
  }

  function commitHookFilter(): void {
    setFilters((prev) => ({ ...prev, source_event: hookDraft.trim() }));
  }

  function onHookKeyDown(event: KeyboardEvent<HTMLInputElement>): void {
    if (event.key === "Enter") {
      event.preventDefault();
      commitHookFilter();
    }
  }

  async function confirmRetry(): Promise<void> {
    if (retryTarget === null || retryTarget.delivery_job_id === null) {
      return;
    }
    setRetrying(true);
    setActionError(null);
    try {
      await backend.manualRetryDelivery({ job_id: retryTarget.delivery_job_id });
      closeRetryDialog();
      setRetryQueuedNotice(true);
    } catch (e: unknown) {
      closeRetryDialog();
      setActionError(errorOf(e));
    } finally {
      setRetrying(false);
    }
  }

  function closeRetryDialog(): void {
    pendingFocusRestore.current = true;
    setRetryTarget(null);
  }

  function rowKeyDown(event: KeyboardEvent<HTMLTableRowElement>, item: HistoryItem): void {
    if (event.key === "Enter" || event.key === " ") {
      event.preventDefault();
      triggerRef.current = event.currentTarget;
      setDetailEventId(item.event_id);
    }
  }

  const channelName = (id: string | null): string =>
    id === null
      ? "—"
      : (channels.find((c) => c.id === id)?.name ?? id);

  return (
    <section aria-label={t.navHistory}>
      <h1>{t.navHistory}</h1>

      <p role="status" className="sr-only">
        {bgNotice}
      </p>
      {retryQueuedNotice && (
        <p role="status" className="muted">
          {t.retryQueued}
        </p>
      )}
      {actionError !== null && (
        <p role="alert">
          {actionError.message}
          {actionError.suggested_action !== null && <>（{actionError.suggested_action}）</>}
        </p>
      )}

      <div className="history-toolbar">
        <label htmlFor="hist-from">{t.filterTimeFrom}</label>
        <input
          id="hist-from"
          type="date"
          value={filters.occurred_from}
          onChange={(event) =>
            setFilters({ ...filters, occurred_from: event.target.value })
          }
        />
        <label htmlFor="hist-until">{t.filterTimeUntil}</label>
        <input
          id="hist-until"
          type="date"
          value={filters.occurred_until}
          onChange={(event) =>
            setFilters({ ...filters, occurred_until: event.target.value })
          }
        />
        <label htmlFor="hist-project">{t.colProject}</label>
        <select
          id="hist-project"
          value={filters.project_id}
          onChange={(event) => setFilters({ ...filters, project_id: event.target.value })}
        >
          <option value="">{t.filterAll}</option>
          {projects.map((project) => (
            <option key={project.id} value={project.id}>
              {project.name}
            </option>
          ))}
        </select>
        <label htmlFor="hist-hook">{t.colHook}</label>
        <input
          id="hist-hook"
          type="text"
          autoComplete="off"
          value={hookDraft}
          onChange={(event) => setHookDraft(event.target.value)}
          onKeyDown={onHookKeyDown}
        />
        <label htmlFor="hist-channel">{t.navChannels}</label>
        <select
          id="hist-channel"
          value={filters.channel_id}
          onChange={(event) => setFilters({ ...filters, channel_id: event.target.value })}
        >
          <option value="">{t.filterAll}</option>
          {channels.map((channel) => (
            <option key={channel.id} value={channel.id}>
              {channel.name}
            </option>
          ))}
        </select>
        <label htmlFor="hist-result">{t.filterResult}</label>
        <select
          id="hist-result"
          value={filters.delivery_status}
          onChange={(event) =>
            setFilters({ ...filters, delivery_status: event.target.value })
          }
        >
          <option value="">{t.filterAll}</option>
          {(["pending", "sending", "retry_wait", "succeeded", "failed", "expired"] as const).map(
            (code) => (
              <option key={code} value={code}>
                {deliveryStatusText(t, code)}
              </option>
            ),
          )}
        </select>
      </div>

      {loadError !== null && (
        <p role="alert">
          {loadError.message}
          {loadError.suggested_action !== null && <>（{loadError.suggested_action}）</>}
        </p>
      )}

      <table className="rules-table">
        <thead>
          <tr>
            <th>{t.colTime}</th>
            <th>{t.colHook}</th>
            <th>{t.colProject}</th>
            <th>{t.navChannels}</th>
            <th>{t.filterResult}</th>
            <th>{t.colSwitch}</th>
          </tr>
        </thead>
        <tbody>
          {(items ?? []).map((item) => (
            <tr
              key={item.event_id}
              tabIndex={0}
              onClick={(event) => {
                triggerRef.current = event.currentTarget;
                setDetailEventId(item.event_id);
              }}
              onKeyDown={(event) => rowKeyDown(event, item)}
            >
              <td>{new Date(item.occurred_at).toLocaleString()}</td>
              <td>{item.source_event}</td>
              <td>{item.project_display_name ?? t.unmatchedProject}</td>
              <td>{channelName(item.channel_id)}</td>
              <td>{deliveryStatusText(t, item.delivery_status)}</td>
              <td onClick={(event) => event.stopPropagation()}>
                {item.delivery_status === "failed" && item.delivery_job_id !== null ? (
                  <button
                    type="button"
                    className="cc-focusable"
                    onClick={(event) => {
                      retryTriggerRef.current = event.currentTarget;
                      setRetryQueuedNotice(false);
                      setRetryTarget(item);
                    }}
                  >
                    {t.retryFailedJob}
                  </button>
                ) : item.delivery_status === "expired" && item.delivery_job_id !== null ? (
                  <button type="button" disabled title={t.retryExpiredHint}>
                    {t.retryExpiredJob}
                  </button>
                ) : null}
              </td>
            </tr>
          ))}
        </tbody>
      </table>

      {items === null && loadError === null && <p className="muted">{t.loading}</p>}
      {items !== null && items.length === 0 && loadError === null && (
        <p className="muted">{t.emptyHistory}</p>
      )}

      {nextOffset !== null && items !== null && items.length > 0 && (
        <div className="row-end">
          <button
            type="button"
            className="cc-focusable"
            disabled={loadingMore}
            onClick={() => {
              setLoadingMore(true);
              load(nextOffset, true)
                .finally(() => setLoadingMore(false));
            }}
          >
            {loadingMore ? t.loading : t.loadMore}
          </button>
        </div>
      )}

      {/* Single detail drawer: redacted document + attempt timeline. */}
      {detailEventId !== null && (
        <aside role="dialog" aria-label={t.detailTitle} className="drawer">
          <div className="drawer-head">
            <h2>{t.detailTitle}</h2>
            <button
              type="button"
              ref={closeRef}
              className="icon-btn cc-focusable"
              aria-label={t.drawerClose}
              onClick={closeDrawer}
            >
              <X size={14} aria-hidden="true" />
            </button>
          </div>
          {detailError !== null && (
            <p role="alert">
              {detailError.message}
              {detailError.suggested_action !== null && <>（{detailError.suggested_action}）</>}
            </p>
          )}
          {detailItem === null && detailError === null && (
            <p className="muted">{t.loading}</p>
          )}
          {detailItem !== null && <DetailBody item={detailItem} t={t} />}
        </aside>
      )}

      {/* Manual-retry confirmation: only failed jobs reach this dialog. */}
      {retryTarget !== null && (
        <div className="dialog-overlay">
          <div role="dialog" aria-label={t.retryConfirmTitle} className="dialog">
            <h2>{t.retryConfirmTitle}</h2>
            <p>
              {retryTarget.source_event} ·{" "}
              {new Date(retryTarget.occurred_at).toLocaleString()}
            </p>
            <p className="trust-item">{t.retryConfirmNote}</p>
            <div className="row-end">
              <button
                type="button"
                className="cc-focusable"
                onClick={closeRetryDialog}
              >
                {t.cancel}
              </button>
              <button
                type="button"
                ref={retryConfirmRef}
                className="primary cc-focusable"
                disabled={retrying}
                onClick={() => {
                  void confirmRetry();
                }}
              >
                {t.confirmRetry}
              </button>
            </div>
          </div>
        </div>
      )}
    </section>
  );
}

/** Drawer body — renders ONLY contract-safe redacted fields. */
function DetailBody({
  item,
  t,
}: {
  item: HistoryItem;
  t: ReturnType<typeof dictionary>;
}): ReactNode {
  return (
    <>
      <dl className="detail-meta">
        <dt>{t.colHook}</dt>
        <dd>{item.source_event}</dd>
        <dt>{t.detailOccurred}</dt>
        <dd>{new Date(item.occurred_at).toLocaleString()}</dd>
        <dt>{t.detailReceived}</dt>
        <dd>{new Date(item.received_at).toLocaleString()}</dd>
        <dt>{t.colProject}</dt>
        <dd>{item.project_display_name ?? t.unmatchedProject}</dd>
        {item.model !== null && (
          <>
            <dt>{t.detailModel}</dt>
            <dd>{item.model}</dd>
          </>
        )}
        {item.permission_mode !== null && (
          <>
            <dt>{t.detailMode}</dt>
            <dd>{item.permission_mode}</dd>
          </>
        )}
      </dl>

      {/* Unmatched working directories appear ONLY as their hash fingerprint;
          the contract carries no path and none is reconstructed here. */}
      {item.unmatched_cwd_fingerprint !== null && (
        <p className="muted">
          {t.detailUnmatchedFingerprint}: <code className="inline-code">{item.unmatched_cwd_fingerprint}</code>
        </p>
      )}

      {item.document !== null ? (
        <section aria-label={t.previewTitle} className="preview-doc">
          <p className="preview-title">{item.document.title}</p>
          <ul>
            {item.document.facts.map(([key, value]) => (
              <li key={key}>
                {key}: {value}
              </li>
            ))}
          </ul>
          <pre>{item.document.body}</pre>
          {item.document.footer !== null && <p className="muted">{item.document.footer}</p>}
        </section>
      ) : (
        <p className="muted">{t.detailNoDocument}</p>
      )}

      <section aria-label={t.detailAttempts}>
        <h3>{t.detailAttempts}</h3>
        {item.attempts.length === 0 ? (
          <p className="muted">—</p>
        ) : (
          <ol className="attempt-list">
            {item.attempts.map((attempt) => (
              <li key={attempt.attempt_number}>
                #{attempt.attempt_number} · {new Date(attempt.started_at).toLocaleString()} →{" "}
                {new Date(attempt.completed_at).toLocaleString()}
                <br />
                {deliveryStatusText(t, attempt.outcome)}
                {attempt.http_status !== null && <> · HTTP {attempt.http_status}</>}
                {attempt.platform_code !== null && <> · {t.platformCodeLabel} {attempt.platform_code}</>}
                {attempt.error_code !== null && <> · {attempt.error_code}</>}
                {attempt.retry_at !== null && (
                  <>
                    <br />
                    <span className="muted">
                      retry_at: {new Date(attempt.retry_at).toLocaleString()}
                    </span>
                  </>
                )}
                {attempt.redacted_detail !== null && (
                  <>
                    <br />
                    <span className="muted">{attempt.redacted_detail}</span>
                  </>
                )}
              </li>
            ))}
          </ol>
        )}
      </section>
    </>
  );
}
