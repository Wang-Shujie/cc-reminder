// 「隐私」分区:敏感字段白名单/正文上限/摘要模式/附加脱敏(JSX 原样移出)。
import type { ReactNode } from "react";

import type { SectionCtx } from "./fields";
import { MAX_PATTERN_CHARS, NumberField, Segmented } from "./fields";

export function SectionPrivacy({
  t,
  draft,
  editable,
  mutate,
  ruleInputFields,
  patternText,
  onPatternChange,
  onPatternBlur,
}: SectionCtx & {
  ruleInputFields: readonly { name: string }[];
  patternText: string;
  onPatternChange: (text: string) => void;
  onPatternBlur: () => void;
}): ReactNode {
  return (
    <>
      <fieldset>
        <legend>{t.allowedFields}</legend>
        {ruleInputFields.map((field) => (
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
        onChange={(event) => onPatternChange(event.target.value)}
        onBlur={onPatternBlur}
      />
    </>
  );
}
