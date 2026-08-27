// Channels page (Task 18): channel list + ONE persistent form region (add or
// credential-replace mode — never a card inside a card). Saved credentials are
// never placed into an input or any DOM node: the read model carries only
// `credential_present`, and the Webhook input always starts empty.
import { useState, type ReactNode } from "react";
import { Trash2 } from "lucide-react";

import { usePageBackend, type Backend } from "../../lib/backend";
import { useCoreQuery } from "../../lib/useCoreQuery";
import { errorOf, type PageError } from "../../lib/errors";
import type {
  ChannelId,
  ChannelKindCode,
  ChannelSummary,
  DeliveryReceiptDto,
  LocaleCode,
} from "../../lib/contracts";
import { dictionary } from "../../lib/i18n";
import { ChannelGuide } from "./ChannelGuide";

interface FormState {
  name: string;
  kind: ChannelKindCode;
  webhook: string;
  signingSecret: string;
  keywordPrefix: string;
}

const EMPTY_FORM: FormState = {
  name: "",
  kind: "we_com",
  webhook: "",
  signingSecret: "",
  keywordPrefix: "",
};

function kindLabel(t: ReturnType<typeof dictionary>, kind: ChannelKindCode): string {
  return kind === "ding_talk" ? t.kindDingTalk : t.kindWeCom;
}

export function ChannelsPage({
  locale = "zh_cn",
  backend: injected,
  variant = "full",
}: {
  locale?: LocaleCode;
  backend?: Backend;
  /** full = 表格+表单(默认);manage = 仅表格与凭据替换(集成页);
   *  add = 仅添加表单(设置页,用户裁决的拆分)。 */
  variant?: "full" | "manage" | "add";
}): ReactNode {
  const backend = usePageBackend(injected);
  const t = dictionary(locale);
  // 统一请求层(架构提案 §1):渠道表随 health-changed(渠道暂停/恢复)
  // 自动刷新;失败语义保持"空表 + 显式告警"。
  const channelsQuery = useCoreQuery(
    (b) => b.listChannels(),
    [],
    ["core://health-changed"],
    backend,
  );
  const channels: ChannelSummary[] | null = channelsQuery.failed
    ? []
    : channelsQuery.data;
  const loadFailed = channelsQuery.failed;
  const refresh = channelsQuery.refresh;
  const [editingId, setEditingId] = useState<ChannelId | null>(null);
  const [form, setForm] = useState<FormState>(EMPTY_FORM);
  const [saving, setSaving] = useState(false);
  const [testingId, setTestingId] = useState<ChannelId | null>(null);
  const [deleting, setDeleting] = useState(false);
  /** Targeted confirmations: sending a test reaches a REAL group; deletion is
   *  destructive and may be blocked by rules targeting the channel. */
  const [testConfirm, setTestConfirm] = useState<ChannelSummary | null>(null);
  const [deleteConfirm, setDeleteConfirm] = useState<ChannelSummary | null>(null);
  const [error, setError] = useState<PageError | null>(null);
  const [receipts, setReceipts] = useState<
    Record<string, DeliveryReceiptDto & { at: Date }>
  >({});

  const editing = editingId === null ? null : (channels ?? []).find((c) => c.id === editingId) ?? null;

  function startAdd(): void {
    setEditingId(null);
    setForm(EMPTY_FORM);
    setError(null);
  }

  function startReplace(channel: ChannelSummary): void {
    // Saved credential material is NEVER loaded back into the form.
    setEditingId(channel.id);
    setForm({ ...EMPTY_FORM });
    setError(null);
  }

  async function save(): Promise<void> {
    setSaving(true);
    setError(null);
    try {
      if (editing !== null) {
        // Replace mode: an empty webhook means "nothing to replace".
        await backend.replaceChannelCredential({
          channel_id: editing.id,
          credential:
            editing.kind === "ding_talk"
              ? {
                  kind: "ding_talk",
                  webhook: form.webhook,
                  signing_secret: form.signingSecret === "" ? null : form.signingSecret,
                }
              : { kind: "we_com", webhook: form.webhook },
        });
      } else {
        await backend.saveChannel({
          channel_id: null,
          name: form.name,
          keyword_prefix:
            form.kind === "ding_talk" && form.keywordPrefix !== ""
              ? form.keywordPrefix
              : null,
          credential:
            form.kind === "ding_talk"
              ? {
                  kind: "ding_talk",
                  webhook: form.webhook,
                  signing_secret: form.signingSecret === "" ? null : form.signingSecret,
                }
              : { kind: "we_com", webhook: form.webhook },
        });
      }
      setForm(EMPTY_FORM);
      setEditingId(null);
      await refresh();
    } catch (e: unknown) {
      setError(errorOf(e));
    } finally {
      setSaving(false);
    }
  }

  async function runTest(channel: ChannelSummary): Promise<void> {
    setTestingId(channel.id);
    setError(null);
    try {
      const receipt = await backend.testChannel({ channel_id: channel.id });
      setReceipts((prev) => ({ ...prev, [channel.id]: { ...receipt, at: new Date() } }));
    } catch (e: unknown) {
      setError(errorOf(e));
    } finally {
      setTestingId(null);
      setTestConfirm(null);
    }
  }

  async function remove(channel: ChannelSummary): Promise<void> {
    setDeleting(true);
    setError(null);
    try {
      await backend.deleteChannel({ channel_id: channel.id });
      setDeleteConfirm(null);
      if (editing?.id === channel.id) {
        startAdd();
      }
      await refresh();
    } catch (e: unknown) {
      // e.g. configuration.channel_in_use while rules still target it.
      setDeleteConfirm(null);
      setError(errorOf(e));
    } finally {
      setDeleting(false);
    }
  }

  const showTable = variant !== "add";
  const showAddEntry = variant === "full";
  const showForm = variant === "add" || variant === "full" || editingId !== null;

  return (
    <section aria-label={t.navChannels}>
      {showTable && <h2>{t.navChannels}</h2>}

      {showAddEntry && (
        <div className="rules-toolbar">
          <div className="rules-toolbar-controls">
            <button type="button" className="cc-focusable" onClick={startAdd}>
              {t.addChannelAction}
            </button>
          </div>
        </div>
      )}

      {loadFailed && <p role="alert">{t.listLoadFailed}</p>}

      {error !== null && (
        <p role="alert">
          {error.message}
          {error.suggested_action !== null && <>（{error.suggested_action}）</>}
        </p>
      )}

      {showTable && (
      <table className="rules-table">
        <thead>
          <tr>
            <th>{t.channelColName}</th>
            <th>{t.channelColKind}</th>
            <th>{t.channelColCredential}</th>
            <th>{t.channelColHealth}</th>
            <th>{t.lastSuccessCol}</th>
            <th>{t.colSwitch}</th>
          </tr>
        </thead>
        <tbody>
          {(channels ?? []).map((channel) => (
            <tr key={channel.id}>
              <td>{channel.name}</td>
              <td>{kindLabel(t, channel.kind)}</td>
              <td>{channel.credential_present ? t.savedCredentialBadge : "—"}</td>
              <td>
                {channel.paused && (
                  <>
                    <span className="badge">{t.pausedBadge}</span>{" "}
                    <span className="muted">{t.authPausedNote}</span>{" "}
                  </>
                )}
                <span className="muted">{channel.health}</span>
              </td>
              <td>
                {channel.last_succeeded_at === null
                  ? t.neverSucceeded
                  : new Date(channel.last_succeeded_at).toLocaleString()}
              </td>
              <td className="agent-actions">
                <button
                  type="button"
                  className="cc-focusable"
                  disabled={testingId !== null}
                  onClick={() => setTestConfirm(channel)}
                >
                  {t.testSendBtn}
                </button>
                <button
                  type="button"
                  className="cc-focusable"
                  aria-label={`${t.replaceCredentialAction} ${channel.name}`}
                  onClick={() => startReplace(channel)}
                >
                  {t.replaceCredentialAction}
                </button>
                <button
                  type="button"
                  className="icon-btn cc-focusable"
                  aria-label={`${t.deleteChannelAction} ${channel.name}`}
                  title={t.deleteChannelAction}
                  disabled={deleting}
                  onClick={() => setDeleteConfirm(channel)}
                >
                  <Trash2 size={14} aria-hidden="true" />
                </button>
              </td>
            </tr>
          ))}
        </tbody>
      </table>
      )}
      {showTable && channels !== null && channels.length === 0 && (
        <p className="muted">{t.emptyChannels}</p>
      )}

      {showTable && (Object.keys(receipts).length > 0) && (
        <section aria-label={t.testResultsTitle}>
          <h2>{t.testResultsTitle}</h2>
          <ul>
            {Object.entries(receipts).map(([id, receipt]) => {
              const channel = (channels ?? []).find((c) => c.id === id);
              return (
                <li key={id}>
                  {channel?.name ?? id}: HTTP {receipt.http_status}
                  {receipt.platform_code !== null && (
                    <>
                      {" · "}
                      {t.platformCodeLabel} {receipt.platform_code}
                      {receipt.platform_code === "45033" && (
                        <span className="muted">（{t.markdownFallbackNote}）</span>
                      )}
                    </>
                  )}
                </li>
              );
            })}
          </ul>
          {/* v2-issues: 失败速查直接列在测试发送结果旁。 */}
          <p className="muted field-hint">
            {t.troubleshootTitle}
            {t.troubleshootBody}
          </p>
        </section>
      )}

      {/* The single channel form: add mode or credential-replace mode. */}
      {showForm && (
      <form
        className="channel-form"
        aria-label={editing !== null ? t.replaceCredentialAction : t.addChannelAction}
        onSubmit={(event) => {
          event.preventDefault();
          void save();
        }}
      >
        <h2>{editing !== null ? t.replaceCredentialAction : t.addChannelAction}</h2>
        {/* v2-issues:分步指引随平台切换(添加模式),与 operations.md §5 同源。 */}
        <ChannelGuide
          locale={locale}
          kind={editing === null ? form.kind : editing.kind}
        />
        {editing !== null && (
          <>
            <p>
              {t.channelColName}: <strong>{editing.name}</strong>
            </p>
            <p className="muted">{kindLabel(t, editing.kind)}</p>
            {editing.credential_present && (
              <p>
                <span className="tag">{t.savedCredentialBadge}</span>{" "}
                <span className="muted">{t.credentialReplaceHint}</span>
              </p>
            )}
          </>
        )}
        {editing === null && (
          <>
            <label htmlFor="channel-name">{t.channelName}</label>
            <input
              id="channel-name"
              value={form.name}
              onChange={(event) => setForm({ ...form, name: event.target.value })}
            />
            <label htmlFor="channel-kind">{t.channelKind}</label>
            <select
              id="channel-kind"
              value={form.kind}
              onChange={(event) =>
                setForm({ ...form, kind: event.target.value as ChannelKindCode })
              }
            >
              <option value="we_com">{t.kindWeCom}</option>
              <option value="ding_talk">{t.kindDingTalk}</option>
            </select>
          </>
        )}
        <label htmlFor="channel-webhook">{t.webhookField}</label>
        <input
          id="channel-webhook"
          type="text"
          autoComplete="off"
          value={form.webhook}
          onChange={(event) => setForm({ ...form, webhook: event.target.value })}
        />
        {(editing === null ? form.kind === "ding_talk" : editing.kind === "ding_talk") && (
          <>
            <label htmlFor="channel-secret">{t.signingSecret}</label>
            <input
              id="channel-secret"
              type="password"
              autoComplete="new-password"
              value={form.signingSecret}
              onChange={(event) =>
                setForm({ ...form, signingSecret: event.target.value })
              }
            />
            <p className="muted field-hint">{t.secretHint}</p>
            {editing === null && (
              <>
                <label htmlFor="channel-prefix">{t.keywordPrefixField}</label>
                <input
                  id="channel-prefix"
                  value={form.keywordPrefix}
                  onChange={(event) =>
                    setForm({ ...form, keywordPrefix: event.target.value })
                  }
                />
                <p className="muted field-hint">{t.keywordHint}</p>
              </>
            )}
          </>
        )}
        <div className="row-end">
          <button
            type="submit"
            className="primary cc-focusable"
            disabled={
              saving ||
              form.webhook === "" ||
              (editing === null && form.name.trim() === "")
            }
          >
            {t.saveChannel}
          </button>
        </div>
      </form>
      )}

      {testConfirm !== null && (
        <div className="dialog-overlay">
          <div
            role="dialog"
            aria-label={t.testSendConfirmTitle}
            className="dialog"
          >
            <h2>{t.testSendConfirmTitle}</h2>
            <p>
              {testConfirm.name} — {t.testSendWarning}
            </p>
            <div className="row-end">
              <button
                type="button"
                className="cc-focusable"
                onClick={() => setTestConfirm(null)}
              >
                {t.cancel}
              </button>
              <button
                type="button"
                className="primary cc-focusable"
                disabled={testingId !== null}
                onClick={() => {
                  void runTest(testConfirm);
                }}
              >
                {t.confirmSend}
              </button>
            </div>
          </div>
        </div>
      )}

      {deleteConfirm !== null && (
        <div className="dialog-overlay">
          <div
            role="dialog"
            aria-label={t.deleteChannelConfirmTitle}
            className="dialog"
          >
            <h2>{t.deleteChannelConfirmTitle}</h2>
            <p>{deleteConfirm.name}</p>
            <p className="trust-item">{t.deleteChannelNote}</p>
            <div className="row-end">
              <button
                type="button"
                className="cc-focusable"
                onClick={() => setDeleteConfirm(null)}
              >
                {t.cancel}
              </button>
              <button
                type="button"
                className="primary cc-focusable"
                disabled={deleting}
                onClick={() => {
                  void remove(deleteConfirm);
                }}
              >
                {t.confirmDelete}
              </button>
            </div>
          </div>
        </div>
      )}
    </section>
  );
}
