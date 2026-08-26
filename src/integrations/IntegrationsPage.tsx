// 集成:单页(用户裁决合并页签)——通知来源在上,通知去向摘要 + 跳设置
// 按钮在下。渠道的添加/凭据/删除整体移入设置页,这里只读概览。
import { useEffect, useState, type ReactNode } from "react";
import { ArrowRight } from "lucide-react";

import { usePageBackend, type Backend } from "../lib/backend";
import type { ChannelKindCode, ChannelSummary, LocaleCode } from "../lib/contracts";
import { dictionary } from "../lib/i18n";
import { AgentsPage } from "../agents/AgentsPage";
import type { PageId } from "../shell/AppShell";

export function IntegrationsPage({
  locale,
  backend: injected,
  onNavigate,
}: {
  locale: LocaleCode;
  backend?: Backend;
  onNavigate?: (page: PageId) => void;
}): ReactNode {
  const backend = usePageBackend(injected);
  const t = dictionary(locale);
  const [channels, setChannels] = useState<ChannelSummary[] | null>(null);

  useEffect(() => {
    let cancelled = false;
    backend
      .listChannels()
      .then((list) => {
        if (!cancelled) {
          setChannels(list);
        }
      })
      .catch(() => {
        if (!cancelled) {
          setChannels([]);
        }
      });
    return () => {
      cancelled = true;
    };
  }, [backend]);

  const kindLabel = (kind: ChannelKindCode): string =>
    kind === "ding_talk" ? t.kindDingTalk : t.kindWeCom;

  return (
    <section aria-label={t.navIntegrations}>
      <h2>{t.tabSources}</h2>
      <AgentsPage locale={locale} backend={injected} />

      <h2 className="integrations-gap">{t.tabDestinations}</h2>
      {channels === null ? null : channels.length === 0 ? (
        <p className="muted">{t.noChannelsHint}</p>
      ) : (
        <ul className="channel-summary">
          {channels.map((channel) => (
            <li key={channel.id}>
              <span className="channel-name">{channel.name}</span>
              <span className="muted">{kindLabel(channel.kind)}</span>
            </li>
          ))}
        </ul>
      )}
      <button
        type="button"
        className="cc-focusable link-arrow"
        onClick={() => onNavigate?.("settings")}
      >
        {t.manageChannels}
        <ArrowRight size={14} aria-hidden="true" />
      </button>
    </section>
  );
}
