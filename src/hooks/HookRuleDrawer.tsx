// Right-hand configuration drawer for one Hook rule (Task 17).
//
// Patch semantics:
// - Global scope: every committed change sends the FULL RuleConfig via
//   save_global_rule.
// - Project scope: every committed change sends ONLY the touched top-level
//   field as a Partial<RuleConfig> patch; an explicitly cleared quiet-hours
//   value is sent as `quiet_hours: null`, while the reset button removes the
//   patch key entirely via reset_project_rule_field.
// - Preview reflects the SAVED rule: it is debounced 250 ms, stale responses
//   are dropped by a monotonic request id, and unsaved text edits deliberately
//   do not trigger refetches; only backend-redacted documents are rendered.
import { useEffect, useId, useRef, useState, type ReactNode } from "react";
import { RotateCcw, Send, X } from "lucide-react";

import { useBackend } from "../lib/backend";
import type {
  AgentKindCode,
  ChannelId,
  ChannelSummary,
  HookRuleRow,
  LocaleCode,
  NotificationDocument,
  PatchFieldCode,
  QuietHours,
  RuleConfig,
  SeverityCode,
} from "../lib/contracts";
import { dictionary } from "../lib/i18n";
import type { RulesScope } from "./HookRulesPage";

const PREVIEW_DEBOUNCE_MS = 250;
const MAX_PATTERN_CHARS = 512;

interface SegmentOption<T> {
  value: T;
  label: string;
  disabled?: boolean;
  /** Screen-reader + hover explanation; keeps the accessible NAME untouched. */
  tooltip?: string;
}

function Segmented<T extends string>({
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
function NumberField({
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
function TimeField({
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

const WEEKDAY_LABELS_ZH = ["一", "二", "三", "四", "五", "六", "日"];
const WEEKDAY_LABELS_EN = ["Mo", "Tu", "We", "Th", "Fr", "Sa", "Su"];

export function HookRuleDrawer({
  locale,
  agent,
  rule,
  scope,
  channels,
  onClose,
  onChanged,
}: {
  locale: LocaleCode;
  agent: AgentKindCode;
  rule: HookRuleRow;
  scope: RulesScope;
  channels: ChannelSummary[];
  onClose: () => void;
  onChanged: () => void;
}): ReactNode {
  const backend = useBackend();
  const t = dictionary(locale);
  const isProject = scope.scope === "project";
  const projectId = isProject ? scope.project_id : null;

  // Local draft of the effective config; resyncs whenever the parent reloads
  // and hands us a fresh row object — but NEVER while the user has uncommitted
  // edits (dirty text fields, or focus inside the drawer): a server resync
  // racing mid-typing must not eat input. The next successful commit (whose
  // saved state matches the draft) or a drawer reopen resumes syncing.
  const asideRef = useRef<HTMLElement | null>(null);
  const dirtyRef = useRef(false);
  const [draft, setDraft] = useState<RuleConfig>(() => structuredClone(rule.config));

  const [templateText, setTemplateText] = useState(
    () => rule.config.targets[0]?.template ?? "",
  );
  const [patternText, setPatternText] = useState(() =>
    rule.config.privacy.extra_redaction_patterns.join(", "),
  );
  const [error, setError] = useState<string | null>(null);
  const [sentOk, setSentOk] = useState(false);
  const [sendDialogOpen, setSendDialogOpen] = useState(false);
  const [sendChannelId, setSendChannelId] = useState<ChannelId | "">(
    () => channels[0]?.id ?? "",
  );
  const [previewDoc, setPreviewDoc] = useState<NotificationDocument | null>(null);
  const [previewError, setPreviewError] = useState<string | null>(null);
  /** Field whose override was last removed; surfaces one 继承全局 status. */
  const [resetFieldCode, setResetFieldCode] = useState<PatchFieldCode | null>(null);

  // Resync from the parent's fresh row (see the draft comment above).
  useEffect(() => {
    if (
      dirtyRef.current ||
      (asideRef.current !== null && asideRef.current.contains(document.activeElement))
    ) {
      return;
    }
    setDraft(structuredClone(rule.config));
    setTemplateText(rule.config.targets[0]?.template ?? "");
    setPatternText(rule.config.privacy.extra_redaction_patterns.join(", "));
    // A resync also clears transient UI state from the previous rule view.
    // resetFieldCode is intentionally NOT cleared: the 继承全局 status is set
    // by the very reset that triggers this resync, and the drawer key already
    // remounts (fresh state) whenever agent/event/project change.
    setError(null);
    setSentOk(false);
    setPreviewDoc(null);
    setPreviewError(null);
    setSendChannelId(channels[0]?.id ?? "");
  }, [rule]);

  // Debounced redacted preview: monotonic request id drops stale responses.
  const requestSeq = useRef(0);
  useEffect(() => {
    const id = ++requestSeq.current;
    const timer = setTimeout(() => {
      backend
        .previewNotification({
          agent,
          source_event: rule.source_event,
          project_id: projectId,
        })
        .then((doc) => {
          if (requestSeq.current === id) {
            setPreviewDoc(doc);
            setPreviewError(null);
          }
        })
        .catch((e: unknown) => {
          if (requestSeq.current === id) {
            setPreviewDoc(null);
            setPreviewError(e instanceof Error ? e.message : String(e));
          }
        });
    }, PREVIEW_DEBOUNCE_MS);
    return () => {
      clearTimeout(timer);
    };
  // Deps deliberately exclude templateText/patternText: the preview shows the
  // SAVED config (已保存配置的预览), so typing must not imply a refetch.
  }, [backend, agent, rule.source_event, projectId]);

  function overridden(field: PatchFieldCode): boolean {
    return rule.patched_fields.includes(field);
  }

  /** Commit one top-level field: full config in global scope, single-field
   *  patch in project scope. */
  function commit(field: PatchFieldCode, next: RuleConfig): void {
    setDraft(next);
    const failure = (e: unknown): void => {
      setError(e instanceof Error ? e.message : String(e));
    };
    if (!isProject) {
      backend
        .saveGlobalRule({ agent, source_event: rule.source_event, config: next })
        .then(() => {
          // Committed state matches the server: resume resyncing.
          dirtyRef.current = false;
          onChanged();
        })
        .catch(failure);
      return;
    }
    const patch = {
      [field]: (next as unknown as Record<string, unknown>)[field],
    } as Partial<RuleConfig>;
    backend
      .saveProjectRulePatch({
        project_id: projectId as string,
        agent,
        source_event: rule.source_event,
        patch,
      })
      .then(() => {
        dirtyRef.current = false;
        onChanged();
      })
      .catch(failure);
  }

  function mutate(field: PatchFieldCode, fn: (draft: RuleConfig) => void): void {
    const next = structuredClone(draft);
    fn(next);
    commit(field, next);
  }

  async function resetField(field: PatchFieldCode): Promise<void> {
    try {
      await backend.resetProjectRuleField({
        project_id: projectId as string,
        agent,
        source_event: rule.source_event,
        field,
      });
      setResetFieldCode(field);
      onChanged();
    } catch (e: unknown) {
      setError(e instanceof Error ? e.message : String(e));
    }
  }

  const sectionTitles: Record<PatchFieldCode, string> = {
    enabled: t.sectionEnabled,
    targets: t.sectionTargets,
    filters: t.sectionFilters,
    privacy: t.sectionPrivacy,
    delivery: t.sectionDelivery,
    quiet_hours: t.sectionQuietHours,
  };

  function sectionHead(title: string, field: PatchFieldCode): ReactNode {
    const resetLabel = `${t.resetInheritedPrefix}${title}${t.resetInheritedSuffix}`;
    return (
      <div className="drawer-section-head">
        <h3>{title}</h3>
        {isProject && overridden(field) && (
          <>
            <span className="tag tag-overridden">{t.sourceOverridden}</span>
            <button
              type="button"
              className="icon-btn cc-focusable"
              aria-label={resetLabel}
              title={resetLabel}
              onClick={() => {
                void resetField(field);
              }}
            >
              <RotateCcw size={14} aria-hidden="true" />
            </button>
          </>
        )}
      </div>
    );
  }

  function commitQuietHours(next: QuietHours | null): void {
    mutate("quiet_hours", (d) => {
      d.quiet_hours = next;
    });
  }

  const quiet = draft.quiet_hours;
  const editable = rule.available;
  const aggregateDisabled = rule.source_event === "PermissionRequest";
  const weekdayLabels = locale === "en" ? WEEKDAY_LABELS_EN : WEEKDAY_LABELS_ZH;

  return (
    <aside ref={asideRef} className="drawer" aria-label={rule.source_event}>
      <div className="drawer-head">
        <h2>{rule.source_event}</h2>
        {!rule.available && <span className="badge">{t.unsupportedVersion}</span>}
        <button
          type="button"
          className="icon-btn cc-focusable"
          aria-label={t.drawerClose}
          title={t.drawerClose}
          onClick={onClose}
        >
          <X size={16} aria-hidden="true" />
        </button>
      </div>

      {error !== null && <p role="alert">{error}</p>}
      {isProject && resetFieldCode !== null && (
        <p className="drawer-reset-status" role="status">
          <span>{sectionTitles[resetFieldCode]}</span>{" "}
          <span>{t.sourceInherited}</span>
        </p>
      )}

      <section className="drawer-section">
        {sectionHead(t.sectionEnabled, "enabled")}
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
      </section>

      <section className="drawer-section">
        {sectionHead(t.sectionTargets, "targets")}
        {channels.map((channel) => {
          const checked = draft.targets.some((target) => target.channel_id === channel.id);
          return (
            <label key={channel.id} className="check-row">
              <input
                type="checkbox"
                checked={checked}
                disabled={!editable}
                onChange={(event) =>
                  mutate("targets", (d) => {
                    d.targets = event.target.checked
                      ? [...d.targets, { channel_id: channel.id, template: null }]
                      : d.targets.filter((target) => target.channel_id !== channel.id);
                  })
                }
              />
              <span>{channel.name}</span>
            </label>
          );
        })}
        <label htmlFor="drawer-template">{t.channelTemplate}</label>
        <textarea
          id="drawer-template"
          value={templateText}
          disabled={!editable}
          onChange={(event) => {
            dirtyRef.current = true;
            setTemplateText(event.target.value);
          }}
          onBlur={() => {
            const first = draft.targets[0];
            if (!editable || !first || (first.template ?? "") === templateText) {
              return;
            }
            mutate("targets", (d) => {
              const target = d.targets[0];
              if (target) {
                target.template = templateText === "" ? null : templateText;
              }
            });
          }}
        />
      </section>

      <section className="drawer-section">
        {sectionHead(t.sectionFilters, "filters")}
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
      </section>

      <section className="drawer-section">
        {sectionHead(t.sectionPrivacy, "privacy")}
        <fieldset>
          <legend>{t.allowedFields}</legend>
          {rule.input_fields.map((field) => (
            <label key={field.name} className="check-row">
              <input
                type="checkbox"
                checked={draft.privacy.allowed_sensitive_fields.includes(field.name)}
                disabled={!editable}
                onChange={(event) =>
                  mutate("privacy", (d) => {
                    d.privacy.allowed_sensitive_fields = event.target.checked
                      ? [...d.privacy.allowed_sensitive_fields, field.name]
                      : d.privacy.allowed_sensitive_fields.filter((name) => name !== field.name);
                  })
                }
              />
              <span>{field.name}</span>
            </label>
          ))}
        </fieldset>
        <NumberField
          id="drawer-max-body"
          label={t.maxBodyChars}
          min={0}
          max={4000}
          value={draft.privacy.max_body_chars}
          disabled={!editable}
          onChange={(v) =>
            mutate("privacy", (d) => {
              d.privacy.max_body_chars = v;
            })
          }
        />
        <div>
          <span className="field-label">{t.summaryMode}</span>
          <Segmented
            label={t.summaryMode}
            current={draft.privacy.summary_mode}
            disabled={!editable}
            options={[
              { value: "metadata_only", label: t.metadataOnly },
              { value: "native_summary", label: t.nativeSummary },
            ]}
            onChange={(value) =>
              mutate("privacy", (d) => {
                d.privacy.summary_mode = value;
              })
            }
          />
        </div>
        <label htmlFor="drawer-patterns">{t.extraPatterns}</label>
        <input
          id="drawer-patterns"
          value={patternText}
          disabled={!editable}
          onChange={(event) => {
            dirtyRef.current = true;
            setPatternText(event.target.value);
          }}
          onBlur={() => {
            const patterns = patternText
              .split(",")
              .map((p) => p.trim())
              .filter((p) => p !== "");
            if (patterns.some((p) => p.length > MAX_PATTERN_CHARS)) {
              setError(t.patternTooLong);
              return;
            }
            setError(null);
            if (patterns.join(",") === draft.privacy.extra_redaction_patterns.join(",")) {
              return;
            }
            mutate("privacy", (d) => {
              d.privacy.extra_redaction_patterns = patterns;
            });
          }}
        />
      </section>

      <section className="drawer-section">
        {sectionHead(t.sectionDelivery, "delivery")}
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
      </section>

      <section className="drawer-section">
        {sectionHead(t.sectionQuietHours, "quiet_hours")}
        <label className="check-row">
          <input
            type="checkbox"
            role="switch"
            aria-label={t.quietEnable}
            checked={quiet !== null}
            disabled={!editable}
            onChange={(event) => {
              commitQuietHours(
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
              onCommit={(value) => commitQuietHours({ ...quiet, start_local: value })}
            />
            <TimeField
              id="drawer-quiet-end"
              label={t.quietEnd}
              value={quiet.end_local}
              disabled={!editable}
              onCommit={(value) => commitQuietHours({ ...quiet, end_local: value })}
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
                        commitQuietHours({
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
                commitQuietHours({
                  ...quiet,
                  bypass_at_or_above: (value === "" ? null : value) as SeverityCode | null,
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
      </section>

      <section className="drawer-section">
        <h3>{t.previewTitle}</h3>
        {previewError !== null && <p role="alert">{previewError}</p>}
        {previewDoc !== null && (
          <div className="preview-doc">
            <p className="preview-title">{previewDoc.title}</p>
            <ul>
              {previewDoc.facts.map(([name, value]) => (
                <li key={name}>
                  {name}: {value}
                </li>
              ))}
            </ul>
            <pre>{previewDoc.body}</pre>
            {previewDoc.footer !== null && <p className="muted">{previewDoc.footer}</p>}
          </div>
        )}
        {sentOk && <p className="muted">{t.sentOk}</p>}
        <button
          type="button"
          className="cc-focusable"
          disabled={!editable || channels.length === 0}
          onClick={() => setSendDialogOpen(true)}
        >
          <Send size={14} aria-hidden="true" /> {t.sendTestAction}
        </button>
      </section>

      {sendDialogOpen && sendChannelId !== "" && (
        <div className="dialog-overlay">
          <div
            role="dialog"
            aria-label={`${t.sendConfirmTitle}「${
              channels.find((c) => c.id === sendChannelId)?.name ?? ""
            }」`}
            className="dialog"
          >
            <h2>
              {t.sendConfirmTitle}「{channels.find((c) => c.id === sendChannelId)?.name ?? ""}」
            </h2>
            {channels.map((channel) => (
              <label key={channel.id} className="check-row">
                <input
                  type="radio"
                  name="send-test-channel"
                  checked={sendChannelId === channel.id}
                  onChange={() => setSendChannelId(channel.id)}
                />
                <span>{channel.name}</span>
              </label>
            ))}
            <div className="row-end">
              <button
                type="button"
                className="cc-focusable"
                onClick={() => setSendDialogOpen(false)}
              >
                {t.cancel}
              </button>
              <button
                type="button"
                className="primary cc-focusable"
                onClick={() => {
                  backend
                    .sendRuleTest({
                      agent,
                      source_event: rule.source_event,
                      channel_id: sendChannelId as ChannelId,
                    })
                    .then(() => {
                      setSendDialogOpen(false);
                      setSentOk(true);
                    })
                    .catch((e: unknown) => {
                      setSendDialogOpen(false);
                      setError(e instanceof Error ? e.message : String(e));
                    });
                }}
              >
                {t.confirmSend}
              </button>
            </div>
          </div>
        </div>
      )}
    </aside>
  );
}
