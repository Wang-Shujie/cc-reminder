// 「目标渠道」分区:渠道勾选 + 模板 textarea(JSX 原样移出)。
// templateText 状态留在抽屉壳(重同步由壳统一管),经 props 传入。
import type { ReactNode } from "react";

import type { ChannelSummary } from "../../lib/contracts";
import type { SectionCtx } from "./fields";

export function SectionTargets({
  t,
  draft,
  editable,
  mutate,
  channels,
  templateText,
  onTemplateChange,
  onTemplateBlur,
}: SectionCtx & {
  channels: ChannelSummary[];
  templateText: string;
  onTemplateChange: (text: string) => void;
  onTemplateBlur: () => void;
}): ReactNode {
  return (
    <>
      {channels.map((channel) => {
        const checked = draft.targets.some((target) => target.channel_id === channel.id);
        return (
          <label key={channel.id} className="check-row">
            <input
              type="checkbox"
              checked={checked}
              disabled={!editable}
              onChange={(event) =>
                mutate("targets", (d) => {
                  d.targets = event.target.checked
                    ? [...d.targets, { channel_id: channel.id, template: null }]
                    : d.targets.filter((target) => target.channel_id !== channel.id);
                })
              }
            />
            <span>{channel.name}</span>
          </label>
        );
      })}
      <label htmlFor="drawer-template">{t.channelTemplate}</label>
      <textarea
        id="drawer-template"
        value={templateText}
        disabled={!editable}
        onChange={(event) => onTemplateChange(event.target.value)}
        onBlur={onTemplateBlur}
      />
    </>
  );
}
