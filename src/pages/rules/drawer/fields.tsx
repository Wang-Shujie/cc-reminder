// Drawer 共享字段组件与常量(架构提案 §4 拆分,自 HookRuleDrawer 原样移出)。
import { useEffect, useId, useState, type ReactNode } from "react";

import type { PatchFieldCode, RuleConfig } from "../../../lib/contracts";
import type { Dictionary } from "../../../lib/i18n";

export const MAX_PATTERN_CHARS = 512;

export const WEEKDAY_LABELS_ZH = ["一", "二", "三", "四", "五", "六", "日"];
export const WEEKDAY_LABELS_EN = ["Mo", "Tu", "We", "Th", "Fr", "Sa", "Su"];

/** 每个 Section 共享的上下文:字典、当前草稿、可编辑位与统一提交入口。 */
export interface SectionCtx {
  t: Dictionary;
  draft: RuleConfig;
  editable: boolean;
  mutate: (field: PatchFieldCode, fn: (draft: RuleConfig) => void) => void;
}

interface SegmentOption<T> {
  value: T;
  label: string;
  disabled?: boolean;
  /** Screen-reader + hover explanation; keeps the accessible NAME untouched. */
  tooltip?: string;
}

export function Segmented<T extends string>({
  label,
  options,
  current,
  disabled,
  onChange,
}: {
  label: string;
  options: SegmentOption<T>[];
  current: T;
  disabled?: boolean;
  onChange: (value: T) => void;
}): ReactNode {
  const descPrefix = useId();
  return (
    <div className="seg" role="radiogroup" aria-label={label}>
      {options.map((option) => (
        <button
          key={option.value}
          type="button"
          role="radio"
          aria-checked={option.value === current}
          disabled={disabled || option.disabled}
          aria-label={option.label}
          aria-describedby={option.tooltip ? `${descPrefix}-${option.value}` : undefined}
          title={option.tooltip}
          className={`cc-focusable seg-item${option.value === current ? " seg-active" : ""}`}
          onClick={() => onChange(option.value)}
        >
          {option.label}
        </button>
      ))}
      {options
        .filter((option) => option.tooltip)
        .map((option) => (
          <span key={option.value} id={`${descPrefix}-${option.value}`} className="sr-only">
            {option.tooltip}
          </span>
        ))}
    </div>
  );
}

/** Bounded numeric input that commits ONCE on blur (or Enter) instead of per
 *  keystroke, so partial input like "3" never reaches the backend mid-typing.
 *  The typed text stays local until commit; an empty or unparseable edit
 *  reverts to the current value without saving. */
export function NumberField({
  id,
  label,
  value,
  min,
  max,
  disabled,
  onChange,
}: {
  id: string;
  label: string;
  value: number;
  min: number;
  max: number;
  disabled: boolean;
  onChange: (value: number) => void;
}): ReactNode {
  const [text, setText] = useState(() => String(value));
  useEffect(() => {
    setText(String(value));
  }, [value]);
  function commit(): void {
    if (text.trim() === "") {
      setText(String(value));
      return;
    }
    const parsed = Number(text);
    if (Number.isNaN(parsed)) {
      setText(String(value));
      return;
    }
    const next = Math.max(min, Math.min(max, Math.trunc(parsed)));
    if (next !== value) {
      onChange(next);
    } else {
      setText(String(value));
    }
  }
  return (
    <div>
      <label htmlFor={id}>{label}</label>
      <input
        id={id}
        type="number"
        min={min}
        max={max}
        value={text}
        disabled={disabled}
        onChange={(event) => setText(event.target.value)}
        onBlur={commit}
        onKeyDown={(event) => {
          if (event.key === "Enter") {
            event.currentTarget.blur();
          }
        }}
      />
    </div>
  );
}

/** Quiet-hours time input; commits only a complete HH:MM value on blur. */
export function TimeField({
  id,
  label,
  value,
  disabled,
  onCommit,
}: {
  id: string;
  label: string;
  value: string;
  disabled: boolean;
  onCommit: (value: string) => void;
}): ReactNode {
  const [text, setText] = useState(value);
  useEffect(() => {
    setText(value);
  }, [value]);
  return (
    <div>
      <label htmlFor={id}>{label}</label>
      <input
        id={id}
        type="time"
        value={text}
        disabled={disabled}
        onChange={(event) => setText(event.target.value)}
        onBlur={() => {
          // Partial ("22:0") or cleared values are reverted, never saved —
          // the backend rejects them and would raise spurious alerts.
          if (/^\d{2}:\d{2}$/.test(text)) {
            if (text !== value) {
              onCommit(text);
            }
          } else {
            setText(value);
          }
        }}
        onKeyDown={(event) => {
          if (event.key === "Enter") {
            event.currentTarget.blur();
          }
        }}
      />
    </div>
  );
}
