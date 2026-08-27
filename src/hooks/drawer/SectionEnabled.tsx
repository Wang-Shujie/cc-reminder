// 「启用通知」分区(JSX 原样移出,架构提案 §4)。
import type { ReactNode } from "react";

import type { SectionCtx } from "./fields";

export function SectionEnabled({ t, draft, editable, mutate }: SectionCtx): ReactNode {
  return (
    <>
      <label className="check-row">
        <input
          type="checkbox"
          role="switch"
          aria-label={t.enableNotify}
          checked={draft.enabled}
          disabled={!editable}
          onChange={(event) =>
            mutate("enabled", (d) => {
              d.enabled = event.target.checked;
            })
          }
        />
        <span>{t.enableNotify}</span>
      </label>
    </>
  );
}
