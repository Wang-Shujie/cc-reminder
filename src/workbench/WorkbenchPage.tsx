// 工作台:单页(用户裁决合并页签)——状态概览在上,通知记录以固定高度
// 小窗滚动区收在页面底部。历史下钻在此承接:概览的 onOpenHistory 设
// 筛选并滚动到记录区,key 变化强制重挂载使筛选立即生效。
import { useRef, useState, type ReactNode } from "react";

import type { Backend } from "../lib/backend";
import type { DeliveryStatusCode, LocaleCode } from "../lib/contracts";
import { dictionary } from "../lib/i18n";
import { HistoryPage } from "../history/HistoryPage";
import { OverviewPage } from "../overview/OverviewPage";
import type { PageId } from "../shell/AppShell";

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
  const [historyFilter, setHistoryFilter] = useState<DeliveryStatusCode | null>(
    null,
  );
  const logPane = useRef<HTMLDivElement>(null);

  function openHistory(deliveryStatus?: DeliveryStatusCode): void {
    setHistoryFilter(deliveryStatus ?? null);
    logPane.current?.scrollIntoView({ block: "end" });
  }

  return (
    <section aria-label={t.navWorkbench}>
      <OverviewPage
        locale={locale}
        backend={injected}
        onNavigate={onNavigate}
        onOpenHistory={openHistory}
      />
      <div className="log-pane" ref={logPane}>
        <HistoryPage
          key={historyFilter ?? "all"}
          locale={locale}
          backend={injected}
          initialDeliveryStatus={historyFilter}
        />
      </div>
    </section>
  );
}
