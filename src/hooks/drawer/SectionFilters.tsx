// 「过滤条件」分区:五组多选(JSX 原样移出,架构提案 §4)。
import type { ReactNode } from "react";

import type { SectionCtx } from "./fields";

export function SectionFilters({ t, draft, editable, mutate }: SectionCtx): ReactNode {
  return (
    <>
      {(
        [
          ["tool_names", t.filterTools, ["Read", "Edit", "Write", "Bash"]],
          ["event_subtypes", t.filterSubtypes, ["matcher", "timeout"]],
          [
            "permission_modes",
            t.filterModes,
            ["default", "acceptEdits", "plan", "bypassPermissions"],
          ],
          ["models", t.filterModels, ["opus", "sonnet", "haiku"]],
          ["statuses", t.filterStatuses, ["success", "error"]],
        ] as const
      ).map(([key, label, presets]) => {
        const selected = draft.filters[key];
        const options = Array.from(new Set([...presets, ...selected]));
        return (
          <div key={key}>
            <label htmlFor={`filter-${key}`}>{label}</label>
            <select
              id={`filter-${key}`}
              multiple
              size={Math.min(4, options.length)}
              value={selected}
              disabled={!editable}
              aria-label={label}
              onChange={(event) => {
                const chosen = Array.from(event.target.selectedOptions).map((o) => o.value);
                mutate("filters", (d) => {
                  d.filters[key] = chosen;
                });
              }}
            >
              {options.map((value) => (
                <option key={value} value={value}>
                  {value}
                </option>
              ))}
            </select>
          </div>
        );
      })}
    </>
  );
}
