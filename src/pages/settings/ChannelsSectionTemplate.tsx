// 设置页「通知模板」编辑区(用户裁决 2026-08-27):全局正文模板,
// 留空 = 内建统一默认。变量语法与规则级模板一致。
import type { ReactNode } from "react";
import { RotateCcw } from "lucide-react";

import type { LocaleCode } from "../../lib/contracts";
import { dictionary } from "../../lib/i18n";

export function ChannelsSectionTemplate({
  template,
  onTemplateChange,
  locale,
}: {
  template: string;
  onTemplateChange: (value: string) => void;
  locale: LocaleCode;
}): ReactNode {
  const t = dictionary(locale);
  return (
    <div className="settings-section">
      <h2>{t.templateLabel}</h2>
      <label className="field-label" htmlFor="settings-template">
        {t.templateHelp}
      </label>
      <textarea
        id="settings-template"
        value={template}
        rows={5}
        placeholder={t.templatePlaceholder}
        onChange={(event) => onTemplateChange(event.target.value)}
      />
      <button
        type="button"
        className="cc-focusable link-arrow"
        onClick={() => onTemplateChange("")}
      >
        <RotateCcw size={14} aria-hidden="true" /> {t.templateReset}
      </button>
    </div>
  );
}
