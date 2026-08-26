// 通知规则:规则表 + 项目管理两个页签。两个子页状态机完全隔离
// (设计 §3):规则表自带全局/项目 scope 单选,项目管理独立增删根目录。
import { useState, type ReactNode } from "react";

import type { Backend } from "../lib/backend";
import type { LocaleCode } from "../lib/contracts";
import { dictionary } from "../lib/i18n";
import { HookRulesPage } from "../hooks/HookRulesPage";
import { ProjectsPage } from "../projects/ProjectsPage";
import { TabBar } from "../shell/TabBar";

type RulesTab = "rules" | "projects";

export function RulesPage({
  locale,
  backend: injected,
}: {
  locale: LocaleCode;
  backend?: Backend;
}): ReactNode {
  const t = dictionary(locale);
  const [tab, setTab] = useState<RulesTab>("rules");
  return (
    <section aria-label={t.navRules}>
      <TabBar
        ariaLabel={t.navRules}
        active={tab}
        onSelect={setTab}
        tabs={[
          { id: "rules", label: t.tabRuleTable },
          { id: "projects", label: t.tabProjectManagement },
        ]}
      />
      {tab === "rules" ? (
        <HookRulesPage locale={locale} />
      ) : (
        <ProjectsPage locale={locale} backend={injected} />
      )}
    </section>
  );
}
