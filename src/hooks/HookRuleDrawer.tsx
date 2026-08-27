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
//
// 架构提案 §4:本文件只保留抽屉壳(草稿/提交/重同步/错误与继承状态),
// 六个分区与预览-测试分区在 ./drawer/ 下各自成组件,JSX 与行为原样移出。
import { useEffect, useRef, useState, type ReactNode } from "react";
import { X } from "lucide-react";

import { useBackend } from "../lib/backend";
import type {
  AgentKindCode,
  ChannelSummary,
  HookRuleRow,
  LocaleCode,
  PatchFieldCode,
  QuietHours,
  RuleConfig,
} from "../lib/contracts";
import { dictionary } from "../lib/i18n";
import type { RulesScope } from "./HookRulesPage";
import { DrawerSection } from "./drawer/DrawerSection";
import { SectionDelivery } from "./drawer/SectionDelivery";
import { SectionEnabled } from "./drawer/SectionEnabled";
import { SectionFilters } from "./drawer/SectionFilters";
import { SectionPreview } from "./drawer/SectionPreview";
import { SectionPrivacy } from "./drawer/SectionPrivacy";
import { SectionQuietHours } from "./drawer/SectionQuietHours";
import { SectionTargets } from "./drawer/SectionTargets";

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
  /** Field whose override was last removed; surfaces one 继承全局 status. */
  const [resetFieldCode, setResetFieldCode] = useState<PatchFieldCode | null>(null);
  /** 每次重同步 +1:让预览/回执分区复位到新规则视图(原语义)。 */
  const [syncTick, setSyncTick] = useState(0);

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
    setSyncTick((tick) => tick + 1);
  }, [rule]);

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

  function section(field: PatchFieldCode, children: ReactNode): ReactNode {
    return (
      <DrawerSection
        title={sectionTitles[field]}
        isProject={isProject}
        overridden={overridden(field)}
        overriddenTagLabel={t.sourceOverridden}
        resetLabel={`${t.resetInheritedPrefix}${sectionTitles[field]}${t.resetInheritedSuffix}`}
        onReset={() => {
          void resetField(field);
        }}
      >
        {children}
      </DrawerSection>
    );
  }

  function commitQuietHours(next: QuietHours | null): void {
    mutate("quiet_hours", (d) => {
      d.quiet_hours = next;
    });
  }

  const sectionCtx = { t, draft, editable: rule.available, mutate };

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

      {section("enabled", <SectionEnabled {...sectionCtx} />)}

      {section(
        "targets",
        <SectionTargets
          {...sectionCtx}
          channels={channels}
          templateText={templateText}
          onTemplateChange={(text) => {
            dirtyRef.current = true;
            setTemplateText(text);
          }}
          onTemplateBlur={() => {
            const first = draft.targets[0];
            if (!rule.available || !first || (first.template ?? "") === templateText) {
              return;
            }
            mutate("targets", (d) => {
              const target = d.targets[0];
              if (target) {
                target.template = templateText === "" ? null : templateText;
              }
            });
          }}
        />,
      )}

      {section("filters", <SectionFilters {...sectionCtx} />)}

      {section(
        "privacy",
        <SectionPrivacy
          {...sectionCtx}
          ruleInputFields={rule.input_fields}
          patternText={patternText}
          onPatternChange={(text) => {
            dirtyRef.current = true;
            setPatternText(text);
          }}
          onPatternBlur={() => {
            const patterns = patternText
              .split(",")
              .map((p) => p.trim())
              .filter((p) => p !== "");
            if (patterns.some((p) => p.length > 512)) {
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
        />,
      )}

      {section(
        "delivery",
        <SectionDelivery
          {...sectionCtx}
          aggregateDisabled={rule.source_event === "PermissionRequest"}
        />,
      )}

      {section(
        "quiet_hours",
        <SectionQuietHours
          t={t}
          quiet={draft.quiet_hours}
          editable={rule.available}
          locale={locale}
          onQuietChange={commitQuietHours}
        />,
      )}

      <section className="drawer-section">
        <h3>{t.previewTitle}</h3>
        <SectionPreview
          t={t}
          agent={agent}
          sourceEvent={rule.source_event}
          projectId={projectId}
          editable={rule.available}
          channels={channels}
          resetTick={syncTick}
          onError={setError}
        />
      </section>
    </aside>
  );
}
