// Drawer 分区壳:标题 + (项目域的)「已覆盖」标签与恢复继承按钮。
// JSX 自 HookRuleDrawer.sectionHead 原样移出(架构提案 §4)。
import type { ReactNode } from "react";
import { RotateCcw } from "lucide-react";

export function DrawerSection({
  title,
  isProject,
  overridden,
  overriddenTagLabel,
  resetLabel,
  onReset,
  children,
}: {
  title: string;
  isProject: boolean;
  overridden: boolean;
  overriddenTagLabel: string;
  resetLabel: string;
  onReset: () => void;
  children: ReactNode;
}): ReactNode {
  return (
    <section className="drawer-section">
      <div className="drawer-section-head">
        <h3>{title}</h3>
        {isProject && overridden && (
          <>
            <span className="tag tag-overridden">{overriddenTagLabel}</span>
            <button
              type="button"
              className="icon-btn cc-focusable"
              aria-label={resetLabel}
              title={resetLabel}
              onClick={onReset}
            >
              <RotateCcw size={14} aria-hidden="true" />
            </button>
          </>
        )}
      </div>
      {children}
    </section>
  );
}
