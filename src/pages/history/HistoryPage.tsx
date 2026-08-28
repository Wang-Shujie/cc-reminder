// Notification History page (Task 19): semantic table with time/project/Hook/
// channel/result filters, bounded pagination honoring next_offset, one detail
// drawer carrying the redacted document and the attempt timeline, and manual
// retry for ELIGIBLE failed jobs only (expired is terminal).
//
// Privacy: the drawer renders only backend-redacted content. Unmatched
// working directories appear solely as their fingerprint hash — a resolved
// path never reaches this component through the contract, and no field that
// could carry one is ever rendered.
import { useCallback, useEffect, useMemo, useRef, useState, type KeyboardEvent, type ReactNode } from "react";
import { X } from "lucide-react";

import { usePageBackend, type Backend } from "../../lib/backend";
import { errorOf, type PageError } from "../../lib/errors";
import type {
  AgentKindCode,
  CoreEventName,
  DeliveryStatusCode,
  HistoryItem,
  ListHistoryInput,
  LocaleCode,
} from "../../lib/contracts";
import { deliveryStatusText, dictionary } from "../../lib/i18n";

const PAGE_SIZE = 10;

interface Filters {
  occurred_from: string;
  occurred_until: string;
  project_id: string;
  source: string;
  source_event: string;
  channel_id: string;
  delivery_status: string;
}

const EMPTY_FILTERS: Filters = {
  occurred_from: "",
  occurred_until: "",
  project_id: "",
  source: "",
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
    source: f.source === "" ? null : (f.source as AgentKindCode),
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

  /** 常用预设筛选(用户裁决第三轮):全部/失败/排队中/重试中。
   *  预设只改结果状态与时间窗;项目/渠道/Hook 关键字保留叠加。 */
  type PresetCode = "all" | "failed" | "queued" | "retry";
  const presetStatus: Record<Exclude<PresetCode, "all">, string> = {
    failed: "failed",
    queued: "pending",
    retry: "retry_wait",
  };
  const activePreset: PresetCode | null =
    filters.occurred_from === "" && filters.occurred_until === ""
      ? filters.delivery_status === ""
        ? "all"
        : (Object.entries(presetStatus).find(
            ([, status]) => status === filters.delivery_status,
          )?.[0] as PresetCode | undefined) ?? null
      : null;

  function applyPreset(code: PresetCode): void {
    updateFilter({
      delivery_status: code === "all" ? "" : presetStatus[code],
      occurred_from: "",
      occurred_until: "",
    });
  }
  const [items, setItems] = useState<HistoryItem[] | null>(null);
  /** 翻页状态(用户裁决 2026-08-27):pageIndex 从 0 计;hasNext 由后端
   *  next_offset 是否为 null 推得(后端不回总数,页码显示当前页序号)。 */
  const [pageIndex, setPageIndex] = useState(0);
  const [hasNext, setHasNext] = useState(false);
  const [loadError, setLoadError] = useState<PageError | null>(null);
  /** Hook 下拉选项目录:按 agent 分组的 source_event 集合。 */
  const [hookCatalog, setHookCatalog] = useState<Partial<Record<AgentKindCode, string[]>>>({});
  const hookOptions = useMemo(() => {
    const lists =
      filters.source === ""
        ? Object.values(hookCatalog)
        : [hookCatalog[filters.source as AgentKindCode] ?? []];
    return [...new Set(lists.flat())].sort();
  }, [hookCatalog, filters.source]);
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
    async (targetPage: number): Promise<boolean> => {
      const seq = ++seqRef.current;
      try {
        const result = await backend.listHistory({
          ...filterInput(filters),
          offset: targetPage * PAGE_SIZE,
          limit: PAGE_SIZE,
        });
        if (seq !== seqRef.current) {
          return false;
        }
        setItems(result.items);
        setHasNext(result.next_offset !== null);
        setLoadError(null);
        return true;
      } catch (e: unknown) {
        // A superseded failure is discarded too.
        if (seq !== seqRef.current) {
          return false;
        }
        setItems([]);
        setHasNext(false);
        setLoadError(errorOf(e));
        return false;
      }
    },
    [backend, filters],
  );

  /// 原位刷新:按当前已加载行数重取,分页位置与「加载更多」链不变。
  /// 投递状态(结果列)由 worker 在每次发送完成后以 queue-changed 推送,
  /// 本方法是其与 history-changed 共用的刷新入口。
  const refreshVisible = useCallback(
    async (): Promise<boolean> => {
      const seq = ++seqRef.current;
      try {
        const result = await backend.listHistory({
          ...filterInput(filters),
          offset: pageIndex * PAGE_SIZE,
          limit: PAGE_SIZE,
        });
        if (seq !== seqRef.current) {
          return false;
        }
        setItems(result.items);
        setHasNext(result.next_offset !== null);
        setLoadError(null);
        return true;
      } catch (e: unknown) {
        if (seq !== seqRef.current) {
          return false;
        }
        setLoadError(errorOf(e));
        return false;
      }
    },
    [backend, filters, pageIndex],
  );

  useEffect(() => {
    void load(pageIndex);
  }, [load, pageIndex]);

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

  // Hook 选项目录来自规则目录(两个 agent 各取一次;失败时退化为"全部")。
  useEffect(() => {
    let cancelled = false;
    const agents = ["claude-code", "codex"] as const;
    Promise.all(
      agents.map((agent) =>
        backend
          .listHookRules({ agent })
          .then((rows) => [agent, [...new Set(rows.map((r) => r.source_event))]] as const),
      ),
    )
      .then((entries) => {
        if (!cancelled) {
          setHookCatalog(Object.fromEntries(entries));
        }
      })
      .catch(() => {});
    return () => {
      cancelled = true;
    };
  }, [backend]);

  // Background refresh on core pushes: history-changed = 新事件/清空;
  // queue-changed = 投递状态迁移(发送完成/失败/重试排队)——「结果」列
  // 的实时性来源(用户反馈:此前仅在重载表格后才更新)。
  useEffect(() => {
    const topics: CoreEventName[] = [
      "core://history-changed",
      "core://queue-changed",
    ];
    const subscriptions = topics.map((topic) =>
      backend.subscribe(topic, (revision: number) => {
        refreshVisible()
          .then((applied) => {
            if (applied) {
              setBgNotice(`${t.historyUpdated}(#${revision})`);
            }
          })
          .catch(() => {});
      }),
    );
    return () => {
      for (const subscription of subscriptions) {
        subscription.then((unlisten) => unlisten()).catch(() => {});
      }
    };
  }, [backend, refreshVisible, t.historyUpdated]);

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

  /** 任何筛选变更都回到第 1 页(翻页语义下的标准行为)。 */
  function updateFilter(patch: Partial<Filters>): void {
    setPageIndex(0);
    setFilters((prev) => ({ ...prev, ...patch }));
  }

  /** 切 Agent 时,若已选 Hook 不在新目录里则一并清空。 */
  function setAgentFilter(value: string): void {
    setPageIndex(0);
    setFilters((prev) => {
      const options =
        value === ""
          ? Object.values(hookCatalog).flat()
          : (hookCatalog[value as AgentKindCode] ?? []);
      return {
        ...prev,
        source: value,
        source_event: options.includes(prev.source_event) ? prev.source_event : "",
      };
    });
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
      <h2>{t.navHistory}</h2>

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

      {/* 预设行(用户裁决):常用筛选一键直达,替代"查看失败任务"式跳转。 */}
      <div className="history-presets" role="group" aria-label={t.presetLabel}>
        {(
          [
            ["all", t.filterAll],
            ["failed", t.presetFailed],
            ["queued", t.presetQueued],
            ["retry", t.presetRetry],
          ] as const
        ).map(([code, label]) => (
          <button
            key={code}
            type="button"
            className={`cc-focusable history-preset${activePreset === code ? " preset-active" : ""}`}
            aria-pressed={activePreset === code}
            onClick={() => applyPreset(code)}
          >
            {label}
          </button>
        ))}
      </div>

      {/* 高级筛选:标签包裹控件的行内排布,不再文字与选框分行。 */}
      <div className="history-filters">
        <label htmlFor="hist-from">
          {t.filterTimeFrom}
          <input
            id="hist-from"
            type="date"
            value={filters.occurred_from}
            onChange={(event) =>
              updateFilter({ occurred_from: event.target.value })
            }
          />
        </label>
        <label htmlFor="hist-until">
          {t.filterTimeUntil}
          <input
            id="hist-until"
            type="date"
            value={filters.occurred_until}
            onChange={(event) =>
              updateFilter({ occurred_until: event.target.value })
            }
          />
        </label>
        <label htmlFor="hist-project">
          {t.colProject}
          <select
            id="hist-project"
            value={filters.project_id}
            onChange={(event) =>
              updateFilter({ project_id: event.target.value })
            }
          >
            <option value="">{t.filterAll}</option>
            {projects.map((project) => (
              <option key={project.id} value={project.id}>
                {project.name}
              </option>
            ))}
          </select>
        </label>
        <label htmlFor="hist-agent">
          {t.colAgent}
          <select
            id="hist-agent"
            value={filters.source}
            onChange={(event) => setAgentFilter(event.target.value)}
          >
            <option value="">{t.filterAll}</option>
            <option value="claude-code">{t.agentClaudeCode}</option>
            <option value="codex">{t.agentCodex}</option>
          </select>
        </label>
        <label htmlFor="hist-hook">
          {t.colHook}
          <select
            id="hist-hook"
            value={filters.source_event}
            onChange={(event) => updateFilter({ source_event: event.target.value })}
          >
            <option value="">{t.filterAll}</option>
            {hookOptions.map((hook) => (
              <option key={hook} value={hook}>
                {hook}
              </option>
            ))}
          </select>
        </label>
        <label htmlFor="hist-channel">
          {t.navChannels}
          <select
            id="hist-channel"
            value={filters.channel_id}
            onChange={(event) =>
              updateFilter({ channel_id: event.target.value })
            }
          >
            <option value="">{t.filterAll}</option>
            {channels.map((channel) => (
              <option key={channel.id} value={channel.id}>
                {channel.name}
              </option>
            ))}
          </select>
        </label>
        <label htmlFor="hist-result">
          {t.filterResult}
          <select
            id="hist-result"
            value={filters.delivery_status}
            onChange={(event) =>
              updateFilter({ delivery_status: event.target.value })
            }
          >
            <option value="">{t.filterAll}</option>
            {(
              [
                "pending",
                "sending",
                "retry_wait",
                "succeeded",
                "failed",
                "expired",
              ] as const
            ).map((code) => (
              <option key={code} value={code}>
                {deliveryStatusText(t, code)}
              </option>
            ))}
          </select>
        </label>
      </div>

      {loadError !== null && (
        <p role="alert">
          {loadError.message}
          {loadError.suggested_action !== null && <>（{loadError.suggested_action}）</>}
        </p>
      )}

      {/* 滚动体:仅记录区滚动,标题与筛选恒定(用户裁决第三轮)。 */}
      <div className="history-body">
      <table className="rules-table">
        <thead>
          <tr>
            <th>{t.colTime}</th>
            <th>{t.colAgent}</th>
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
              className={item.delivery_status === "failed" ? "hazard-row" : undefined}
              tabIndex={0}
              onClick={(event) => {
                triggerRef.current = event.currentTarget;
                setDetailEventId(item.event_id);
              }}
              onKeyDown={(event) => rowKeyDown(event, item)}
            >
              <td>{new Date(item.occurred_at).toLocaleString()}</td>
              <td>{item.source === "codex" ? t.agentCodex : t.agentClaudeCode}</td>
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

      {/* 翻页(用户裁决 2026-08-27):上一页/页码/下一页;后端游标只给
          next_offset,故显示当前页序号而非总页数。 */}
      {(pageIndex > 0 || hasNext) && items !== null && items.length > 0 && (
        <div className="pager">
          <button
            type="button"
            className="cc-focusable"
            disabled={pageIndex === 0}
            onClick={() => setPageIndex(pageIndex - 1)}
          >
            {t.prevPage}
          </button>
          <span className="muted">{t.pageInfo.replace("{page}", String(pageIndex + 1))}</span>
          <button
            type="button"
            className="cc-focusable"
            disabled={!hasNext}
            onClick={() => setPageIndex(pageIndex + 1)}
          >
            {t.nextPage}
          </button>
        </div>
      )}
      </div>

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
