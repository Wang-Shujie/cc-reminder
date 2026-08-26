// src/shell/TabBar.tsx
// Shared in-page tab strip for the 4-destination shell (spec §7). Reuses the
// rules-tabs visual language; deliberate simplification: no aria-controls /
// roving tabindex — every tab stays Tab-reachable, arrows auto-activate.
import { useRef, type KeyboardEvent, type ReactNode } from "react";

export interface TabItem<T extends string> {
  id: T;
  label: string;
}

export function TabBar<T extends string>({
  tabs,
  active,
  onSelect,
  ariaLabel,
}: {
  tabs: readonly TabItem<T>[];
  active: T;
  onSelect: (id: T) => void;
  ariaLabel: string;
}): ReactNode {
  const buttons = useRef<(HTMLButtonElement | null)[]>([]);

  function onKeyDown(event: KeyboardEvent<HTMLButtonElement>, index: number): void {
    if (event.key !== "ArrowLeft" && event.key !== "ArrowRight") {
      return;
    }
    event.preventDefault();
    const delta = event.key === "ArrowLeft" ? -1 : 1;
    const next = (index + delta + tabs.length) % tabs.length;
    onSelect(tabs[next]!.id);
    buttons.current[next]?.focus();
  }

  return (
    <div role="tablist" aria-label={ariaLabel} className="rules-tabs">
      {tabs.map((tab, index) => (
        <button
          key={tab.id}
          ref={(el) => {
            buttons.current[index] = el;
          }}
          type="button"
          role="tab"
          aria-selected={active === tab.id}
          className={`cc-focusable rules-tab${active === tab.id ? " rules-tab-active" : ""}`}
          onClick={() => onSelect(tab.id)}
          onKeyDown={(event) => onKeyDown(event, index)}
        >
          {tab.label}
        </button>
      ))}
    </div>
  );
}
