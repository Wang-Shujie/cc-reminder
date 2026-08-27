// 「静默时段」分区(JSX 原样移出,架构提案 §4)。
import type { ReactNode } from "react";

import type { QuietHours } from "../../../lib/contracts";
import type { Dictionary } from "../../../lib/i18n";
import { TimeField, WEEKDAY_LABELS_EN, WEEKDAY_LABELS_ZH } from "./fields";

export function SectionQuietHours({
  t,
  quiet,
  editable,
  locale,
  onQuietChange,
}: {
  t: Dictionary;
  quiet: QuietHours | null;
  editable: boolean;
  locale: string;
  onQuietChange: (next: QuietHours | null) => void;
}): ReactNode {
  const weekdayLabels = locale === "en" ? WEEKDAY_LABELS_EN : WEEKDAY_LABELS_ZH;
  return (
    <>
      <label className="check-row">
        <input
          type="checkbox"
          role="switch"
          aria-label={t.quietEnable}
          checked={quiet !== null}
          disabled={!editable}
          onChange={(event) => {
            onQuietChange(
              event.target.checked
                ? (quiet ?? {
                    start_local: "22:00",
                    end_local: "08:00",
                    weekdays: [1, 2, 3, 4, 5],
                    bypass_at_or_above: null,
                  })
                : null,
            );
          }}
        />
        <span>{t.quietEnable}</span>
      </label>
      {quiet !== null && (
        <>
          <TimeField
            id="drawer-quiet-start"
            label={t.quietStart}
            value={quiet.start_local}
            disabled={!editable}
            onCommit={(value) => onQuietChange({ ...quiet, start_local: value })}
          />
          <TimeField
            id="drawer-quiet-end"
            label={t.quietEnd}
            value={quiet.end_local}
            disabled={!editable}
            onCommit={(value) => onQuietChange({ ...quiet, end_local: value })}
          />
          <fieldset>
            <legend>{t.quietWeekdays}</legend>
            {weekdayLabels.map((dayLabel, index) => {
              const weekday = index + 1;
              return (
                <label key={weekday} className="weekday">
                  <input
                    type="checkbox"
                    checked={quiet.weekdays.includes(weekday)}
                    disabled={!editable}
                    onChange={(event) =>
                      onQuietChange({
                        ...quiet,
                        weekdays: event.target.checked
                          ? [...quiet.weekdays, weekday].sort((a, b) => a - b)
                          : quiet.weekdays.filter((day) => day !== weekday),
                      })
                    }
                  />
                  <span>{dayLabel}</span>
                </label>
              );
            })}
          </fieldset>
          <label htmlFor="drawer-bypass">{t.bypassSeverity}</label>
          <select
            id="drawer-bypass"
            value={quiet.bypass_at_or_above ?? ""}
            disabled={!editable}
            onChange={(event) => {
              const value = event.target.value;
              onQuietChange({
                ...quiet,
                bypass_at_or_above: (value === "" ? null : value) as QuietHours["bypass_at_or_above"],
              });
            }}
          >
            <option value="">{t.bypassNone}</option>
            <option value="info">info</option>
            <option value="warning">warning</option>
            <option value="error">error</option>
            <option value="critical">critical</option>
          </select>
        </>
      )}
    </>
  );
}
