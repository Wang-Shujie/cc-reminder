// 「投递」分区:模式/聚合窗/冷却/上限/统计窗/TTL/尝试数/静默行为(JSX 原样移出)。
import type { ReactNode } from "react";

import type { SectionCtx } from "./fields";
import { NumberField, Segmented } from "./fields";

export function SectionDelivery({
  t,
  draft,
  editable,
  mutate,
  aggregateDisabled,
}: SectionCtx & { aggregateDisabled: boolean }): ReactNode {
  return (
    <>
      <div>
        <span className="field-label">{t.deliveryMode}</span>
        <Segmented
          label={t.deliveryMode}
          current={draft.delivery.mode.mode}
          disabled={!editable}
          options={[
            { value: "immediate", label: t.immediate },
              {
                value: "aggregate",
                label: t.aggregate,
                disabled: aggregateDisabled,
                tooltip: aggregateDisabled ? t.aggregateDisabledTooltip : undefined,
              },
          ]}
          onChange={(value) =>
            mutate("delivery", (d) => {
              d.delivery.mode =
                value === "immediate"
                  ? { mode: "immediate" }
                  : {
                      mode: "aggregate",
                      window_seconds:
                        d.delivery.mode.mode === "aggregate" ? d.delivery.mode.window_seconds : 60,
                    };
            })
          }
        />
      </div>
      {draft.delivery.mode.mode === "aggregate" && (
        <NumberField
          id="drawer-aggregate-window"
          label={t.aggregateWindow}
          min={10}
          max={3600}
          value={
            draft.delivery.mode.mode === "aggregate" ? draft.delivery.mode.window_seconds : 60
          }
          disabled={!editable}
          onChange={(v) =>
            mutate("delivery", (d) => {
              if (d.delivery.mode.mode === "aggregate") {
                d.delivery.mode.window_seconds = v;
              }
            })
          }
        />
      )}
      <NumberField
        id="drawer-cooldown"
        label={t.cooldown}
        min={0}
        max={86_400}
        value={draft.delivery.cooldown_seconds}
        disabled={!editable}
        onChange={(v) =>
          mutate("delivery", (d) => {
            d.delivery.cooldown_seconds = v;
          })
        }
      />
      <NumberField
        id="drawer-window-cap"
        label={t.windowCap}
        min={1}
        max={100}
        value={draft.delivery.max_per_window}
        disabled={!editable}
        onChange={(v) =>
          mutate("delivery", (d) => {
            d.delivery.max_per_window = v;
          })
        }
      />
      <NumberField
        id="drawer-stat-window"
        label={t.statWindow}
        min={1}
        max={86_400}
        value={draft.delivery.window_seconds}
        disabled={!editable}
        onChange={(v) =>
          mutate("delivery", (d) => {
            d.delivery.window_seconds = v;
          })
        }
      />
      <NumberField
        id="drawer-ttl"
        label={t.ttl}
        min={1}
        max={86_400}
        value={draft.delivery.ttl_seconds}
        disabled={!editable}
        onChange={(v) =>
          mutate("delivery", (d) => {
            d.delivery.ttl_seconds = v;
          })
        }
      />
      <NumberField
        id="drawer-max-attempts"
        label={t.maxAttempts}
        min={1}
        max={10}
        value={draft.delivery.max_attempts}
        disabled={!editable}
        onChange={(v) =>
          mutate("delivery", (d) => {
            d.delivery.max_attempts = v;
          })
        }
      />
      <div>
        <span className="field-label">{t.quietBehavior}</span>
        <Segmented
          label={t.quietBehavior}
          current={draft.delivery.quiet_behavior}
          disabled={!editable}
          options={[
            { value: "suppress", label: t.suppress },
            { value: "defer", label: t.defer },
          ]}
          onChange={(value) =>
            mutate("delivery", (d) => {
              d.delivery.quiet_behavior = value;
            })
          }
        />
      </div>
    </>
  );
}
