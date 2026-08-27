// 集成:单页——通知来源在上,通知去向 = 完整渠道管理表(统计与操作,
// 用户裁决保留图1形态)在下;添加渠道表单在设置页,箭头链接跳转。
import { type ReactNode } from "react";
import { ArrowRight } from "lucide-react";

import type { Backend } from "../../lib/backend";
import type { LocaleCode } from "../../lib/contracts";
import { dictionary } from "../../lib/i18n";
import { AgentsPage } from "../agents/AgentsPage";
import { ChannelsPage } from "../channels/ChannelsPage";
import type { PageId } from "../../shell/AppShell";

export function IntegrationsPage({
  locale,
  backend: injected,
  onNavigate,
}: {
  locale: LocaleCode;
  backend?: Backend;
  onNavigate?: (page: PageId) => void;
}): ReactNode {
  const t = dictionary(locale);

  return (
    <section aria-label={t.navIntegrations}>
      <h2>{t.tabSources}</h2>
      <AgentsPage locale={locale} backend={injected} />

      <h2 className="integrations-gap">{t.tabDestinations}</h2>
      <ChannelsPage locale={locale} backend={injected} variant="manage" />
      <button
        type="button"
        className="cc-focusable link-arrow"
        onClick={() => onNavigate?.("settings")}
      >
        {t.addChannelAction}
        <ArrowRight size={14} aria-hidden="true" />
      </button>
    </section>
  );
}
