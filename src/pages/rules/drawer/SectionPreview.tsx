// 「预览与测试」分区:自带 250ms 防抖预览、单调请求序号丢弃过期响应、
// 发送测试确认弹窗与 sentOk 回执(逻辑与 JSX 原样移出,架构提案 §4)。
import { useEffect, useRef, useState, type ReactNode } from "react";
import { Send } from "lucide-react";

import { useBackend } from "../../../lib/backend";
import type {
  AgentKindCode,
  ChannelId,
  ChannelSummary,
  NotificationDocument,
} from "../../../lib/contracts";
import type { Dictionary } from "../../../lib/i18n";

const PREVIEW_DEBOUNCE_MS = 250;

export function SectionPreview({
  t,
  agent,
  sourceEvent,
  projectId,
  editable,
  channels,
  resetTick,
  onError,
}: {
  t: Dictionary;
  agent: AgentKindCode;
  sourceEvent: string;
  projectId: string | null;
  editable: boolean;
  channels: ChannelSummary[];
  /** 壳的重同步计数:变化即复位预览/回执/渠道选择(原 drawer 语义)。 */
  resetTick: number;
  onError: (message: string) => void;
}): ReactNode {
  const backend = useBackend();
  const [previewDoc, setPreviewDoc] = useState<NotificationDocument | null>(null);
  const [previewError, setPreviewError] = useState<string | null>(null);
  const [sentOk, setSentOk] = useState(false);
  const [sendDialogOpen, setSendDialogOpen] = useState(false);
  const [sendChannelId, setSendChannelId] = useState<ChannelId | "">(
    () => channels[0]?.id ?? "",
  );

  // 重同步复位:回执与渠道选择回到干净瞬态(初始 tick 也无害,目标值同初值)。
  useEffect(() => {
    setSentOk(false);
    setSendChannelId(channels[0]?.id ?? "");
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [resetTick]);

  // Debounced redacted preview: monotonic request id drops stale responses.
  // resetTick 在依赖里:壳的每次重同步(补丁提交/恢复继承)都重新拉取预览,
  // 否则复位清掉的 previewDoc 不会再回来(满载 e2e 抓到的潜伏 bug)。
  const requestSeq = useRef(0);
  useEffect(() => {
    const id = ++requestSeq.current;
    setPreviewDoc(null);
    setPreviewError(null);
    const timer = setTimeout(() => {
      backend
        .previewNotification({
          agent,
          source_event: sourceEvent,
          project_id: projectId,
        })
        .then((doc) => {
          if (requestSeq.current === id) {
            setPreviewDoc(doc);
            setPreviewError(null);
          }
        })
        .catch((e: unknown) => {
          if (requestSeq.current === id) {
            setPreviewDoc(null);
            setPreviewError(e instanceof Error ? e.message : String(e));
          }
        });
    }, PREVIEW_DEBOUNCE_MS);
    return () => {
      clearTimeout(timer);
    };
  // Deps deliberately exclude unsaved text: the preview shows the SAVED
  // config (已保存配置的预览), so typing must not imply a refetch.
  }, [backend, agent, sourceEvent, projectId, resetTick]);

  return (
    <>
      {previewError !== null && <p role="alert">{previewError}</p>}
      {previewDoc !== null && (
        <div className="preview-doc">
          <p className="preview-title">{previewDoc.title}</p>
          <ul>
            {previewDoc.facts.map(([name, value]) => (
              <li key={name}>
                {name}: {value}
              </li>
            ))}
          </ul>
          <pre>{previewDoc.body}</pre>
          {previewDoc.footer !== null && <p className="muted">{previewDoc.footer}</p>}
        </div>
      )}
      {sentOk && <p className="muted">{t.sentOk}</p>}
      <button
        type="button"
        className="cc-focusable"
        disabled={!editable || channels.length === 0}
        onClick={() => setSendDialogOpen(true)}
      >
        <Send size={14} aria-hidden="true" /> {t.sendTestAction}
      </button>

      {sendDialogOpen && sendChannelId !== "" && (
        <div className="dialog-overlay">
          <div
            role="dialog"
            aria-label={`${t.sendConfirmTitle}「${
              channels.find((c) => c.id === sendChannelId)?.name ?? ""
            }」`}
            className="dialog"
          >
            <h2>
              {t.sendConfirmTitle}「{channels.find((c) => c.id === sendChannelId)?.name ?? ""}」
            </h2>
            {channels.map((channel) => (
              <label key={channel.id} className="check-row">
                <input
                  type="radio"
                  name="send-test-channel"
                  checked={sendChannelId === channel.id}
                  onChange={() => setSendChannelId(channel.id)}
                />
                <span>{channel.name}</span>
              </label>
            ))}
            <div className="row-end">
              <button
                type="button"
                className="cc-focusable"
                onClick={() => setSendDialogOpen(false)}
              >
                {t.cancel}
              </button>
              <button
                type="button"
                className="primary cc-focusable"
                onClick={() => {
                  backend
                    .sendRuleTest({
                      agent,
                      source_event: sourceEvent,
                      channel_id: sendChannelId as ChannelId,
                    })
                    .then(() => {
                      setSendDialogOpen(false);
                      setSentOk(true);
                    })
                    .catch((e: unknown) => {
                      setSendDialogOpen(false);
                      onError(e instanceof Error ? e.message : String(e));
                    });
                }}
              >
                {t.confirmSend}
              </button>
            </div>
          </div>
        </div>
      )}
    </>
  );
}
