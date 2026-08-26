// 集成:通知来源(Agent)与通知去向(渠道)——同一条流水线的两端。
import { useState, type ReactNode } from "react";

import type { Backend } from "../lib/backend";
import type { LocaleCode } from "../lib/contracts";
import { dictionary } from "../lib/i18n";
import { AgentsPage } from "../agents/AgentsPage";
import { ChannelsPage } from "../channels/ChannelsPage";
import { TabBar } from "../shell/TabBar";

type IntegrationsTab = "sources" | "destinations";

export function IntegrationsPage({
  locale,
  backend: injected,
}: {
  locale: LocaleCode;
  backend?: Backend;
}): ReactNode {
  const t = dictionary(locale);
  const [tab, setTab] = useState<IntegrationsTab>("sources");
  return (
    <section aria-label={t.navIntegrations}>
      <h1>{t.navIntegrations}</h1>
      <TabBar
        ariaLabel={t.navIntegrations}
        active={tab}
        onSelect={setTab}
        tabs={[
          { id: "sources", label: t.tabSources },
          { id: "destinations", label: t.tabDestinations },
        ]}
      />
      {tab === "sources" ? (
        <AgentsPage locale={locale} backend={injected} />
      ) : (
        <ChannelsPage locale={locale} backend={injected} />
      )}
    </section>
  );
}
