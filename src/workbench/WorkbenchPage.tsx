// 工作台:状态概览 + 通知记录两个页签。历史下钻(seed 下沉)在此承接:
// OverviewPage 的 onOpenHistory 切页签并作为 HistoryPage 的初始筛选,
// key 变化强制重挂载使筛选立即生效。页签选择不持久化——进页回到概览。
import { useState, type ReactNode } from "react";

import type { Backend } from "../lib/backend";
import type { DeliveryStatusCode, LocaleCode } from "../lib/contracts";
import { dictionary } from "../lib/i18n";
import { HistoryPage } from "../history/HistoryPage";
import { OverviewPage } from "../overview/OverviewPage";
import { TabBar } from "../shell/TabBar";
import type { PageId } from "../shell/AppShell";

type WorkbenchTab = "overview" | "history";

export function WorkbenchPage({
  locale,
  backend: injected,
  onNavigate,
}: {
  locale: LocaleCode;
  backend?: Backend;
  onNavigate?: (page: PageId) => void;
}): ReactNode {
  const t = dictionary(locale);
  const [tab, setTab] = useState<WorkbenchTab>("overview");
  const [historyFilter, setHistoryFilter] = useState<DeliveryStatusCode | null>(
    null,
  );

  function openHistory(deliveryStatus?: DeliveryStatusCode): void {
    setHistoryFilter(deliveryStatus ?? null);
    setTab("history");
  }

  return (
    <section aria-label={t.navWorkbench}>
      <TabBar
        ariaLabel={t.navWorkbench}
        active={tab}
        onSelect={setTab}
        tabs={[
          { id: "overview", label: t.tabStatusOverview },
          { id: "history", label: t.tabNotificationLog },
        ]}
      />
      {tab === "overview" ? (
        <OverviewPage
          locale={locale}
          backend={injected}
          onNavigate={onNavigate}
          onOpenHistory={openHistory}
        />
      ) : (
        <HistoryPage
          key={historyFilter ?? "all"}
          locale={locale}
          backend={injected}
          initialDeliveryStatus={historyFilter}
        />
      )}
    </section>
  );
}
