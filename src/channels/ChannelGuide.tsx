// 可折叠「如何获取 Webhook?」指引(v2-issues 用户反馈):内容随平台
// 切换,收起状态记入 localStorage(首次默认展开)。文案与
// docs/operations.md §5 保持同源。动图素材(脱敏)列为用户提供项。
import { useState, type ReactNode } from "react";

import type { ChannelKindCode, LocaleCode } from "../lib/contracts";
import { dictionary } from "../lib/i18n";

const COLLAPSED_KEY = "cc-reminder:channel-guide-collapsed";

export function ChannelGuide({
  locale,
  kind,
}: {
  locale: LocaleCode;
  kind: ChannelKindCode;
}): ReactNode {
  const t = dictionary(locale);
  const [open, setOpen] = useState(() => localStorage.getItem(COLLAPSED_KEY) !== "1");

  return (
    <div className="channel-guide">
      <button
        type="button"
        className="cc-focusable link-arrow"
        aria-expanded={open}
        onClick={() => {
          const next = !open;
          setOpen(next);
          localStorage.setItem(COLLAPSED_KEY, next ? "0" : "1");
        }}
      >
        {t.guideToggle}
      </button>
      {open && (
        <p className="channel-guide-body">
          {kind === "ding_talk" ? t.guideDingTalk : t.guideWeCom}
        </p>
      )}
    </div>
  );
}
