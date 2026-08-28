// Settings page (Task 19): native controls persisting exact values via
// save_settings (the Rust side applies autostart), notification pause, and
// the update check/install confirmation flow. The credential-store section
// discloses a secure-store unavailability reported by shared health.
// Task 20 fix round 1: the diagnostics export opens its save dialog inside
// the core (no path crosses the bridge), and a bounded debug-logging window
// (关闭 / 15 分钟 / 60 分钟) is reachable from here.
import { useCallback, useEffect, useRef, useState, type ReactNode } from "react";

import { usePageBackend, type Backend } from "../../lib/backend";
import { ChannelsPage } from "../channels/ChannelsPage";
import { ChannelsSectionTemplate } from "./ChannelsSectionTemplate";
import { errorOf, type PageError } from "../../lib/errors";
import type {
  HealthIssue,
  LocaleCode,
  PauseDurationCode,
  SaveSettingsInput,
  SetDebugLoggingInput,
  SettingsView,
  ThemeCode,
  UpdateCheckResult,
} from "../../lib/contracts";
import { Monitor, Moon, Sun } from "lucide-react";

import { dictionary } from "../../lib/i18n";

const RETENTION_MIN = 1;
const RETENTION_MAX = 365;

function parseRetentionDays(raw: string): number | null {
  if (!/^\d+$/.test(raw.trim())) {
    return null;
  }
  const n = Number.parseInt(raw, 10);
  return n >= RETENTION_MIN && n <= RETENTION_MAX ? n : null;
}

export function SettingsPage({
  locale = "zh_cn",
  backend: injected,
}: {
  locale?: LocaleCode;
  backend?: Backend;
}): ReactNode {
  const backend = usePageBackend(injected);
  const t = dictionary(locale);
  const [clearConfirmOpen, setClearConfirmOpen] = useState(false);
  // Form state starts on safe defaults and hydrates from get_settings; saving
  // stays disabled until hydration lands so onboarding_completed is never
  // regressed by an early save.
  const [hydrated, setHydrated] = useState(false);
  const [autostart, setAutostart] = useState(false);
  const [closeToTray, setCloseToTray] = useState(true);
  const [uiLocale, setUiLocale] = useState<LocaleCode>("zh_cn");
  const [theme, setTheme] = useState<ThemeCode>("system");
  const [eventDays, setEventDays] = useState("30");
  const [logDays, setLogDays] = useState("7");
  const [onboardingCompleted, setOnboardingCompleted] = useState(true);
  /** 全局通知正文模板;空串 = 内建统一默认(用户裁决 2026-08-27)。 */
  const [template, setTemplate] = useState("");

  const [pausedUntil, setPausedUntil] = useState<string | null>(null);
  const [pauseBusy, setPauseBusy] = useState(false);

  /** Bounded debug window selection (0 = off). The window itself lives in the
   *  core; this only mirrors the last value chosen on this page. */
  const [debugMinutes, setDebugMinutes] = useState<0 | 15 | 60>(0);
  const [debugBusy, setDebugBusy] = useState(false);
  const [exportStatus, setExportStatus] = useState<string | null>(null);

  const [savedOk, setSavedOk] = useState(false);
  const [validationError, setValidationError] = useState<string | null>(null);
  const [loadError, setLoadError] = useState<PageError | null>(null);
  const [actionError, setActionError] = useState<PageError | null>(null);

  const [checking, setChecking] = useState(false);
  const [updateResult, setUpdateResult] = useState<UpdateCheckResult | null>(null);
  const [installConfirmOpen, setInstallConfirmOpen] = useState(false);
  const [installing, setInstalling] = useState(false);
  const [installOk, setInstallOk] = useState(false);
  const [storeIssues, setStoreIssues] = useState<HealthIssue[]>([]);
  /** Focus returns to the initiating control after dialogs close. */
  const installTriggerRef = useRef<HTMLButtonElement | null>(null);
  const installConfirmRef = useRef<HTMLButtonElement | null>(null);
  /** Set when a dialog closes so the post-commit effect can restore focus
   *  (restoring synchronously races the dialog's own unmount). */
  const pendingFocusRestore = useRef(false);
  useEffect(() => {
    if (!installConfirmOpen && pendingFocusRestore.current) {
      pendingFocusRestore.current = false;
      installTriggerRef.current?.focus();
    }
  }, [installConfirmOpen]);

  // Focus moves to the confirm button when the install dialog opens.
  useEffect(() => {
    if (installConfirmOpen) {
      installConfirmRef.current?.focus();
    }
  }, [installConfirmOpen]);

  useEffect(() => {
    let cancelled = false;
    backend
      .getSettings()
      .then((view: SettingsView) => {
        if (cancelled) return;
        setAutostart(view.autostart);
        setCloseToTray(view.close_to_tray);
        setUiLocale(view.locale);
        setTheme(view.theme);
        setEventDays(String(view.event_retention_days));
        setLogDays(String(view.log_retention_days));
        setOnboardingCompleted(view.onboarding_completed);
        setPausedUntil(view.paused_until);
        setTemplate(view.notification_template ?? "");
        setHydrated(true);
      })
      .catch((e: unknown) => {
        if (!cancelled) {
          setLoadError(errorOf(e));
        }
      });
    backend
      .getHealthSnapshot()
      .then((snap) => {
        if (!cancelled) {
          setStoreIssues(
            snap.issues.filter(
              (issue) =>
                issue.issue_code.startsWith("credentials.") ||
                issue.issue_code.startsWith("channel."),
            ),
          );
        }
      })
      .catch(() => {});
    return () => {
      cancelled = true;
    };
  }, [backend]);

  // 自动保存(2026-08-28 用户裁决):整页不再有保存按钮。saveNow 永远读
  // ref 里的最新表单值——防抖回调不会拿到过期闭包;勾选/单选/下拉即时存,
  // 数字与模板输入防抖 600ms 合并。序号丢弃过期响应的 UI 反馈。
  const autoFormRef = useRef({
    autostart,
    closeToTray,
    uiLocale,
    theme,
    eventDays,
    logDays,
    template,
    onboardingCompleted,
  });
  autoFormRef.current = {
    autostart,
    closeToTray,
    uiLocale,
    theme,
    eventDays,
    logDays,
    template,
    onboardingCompleted,
  };
  const saveSeqRef = useRef(0);
  const saveTimerRef = useRef<number | null>(null);

  const saveNow = useCallback(async (): Promise<void> => {
    if (!hydrated) {
      return;
    }
    const form = autoFormRef.current;
    const eventRetention = parseRetentionDays(form.eventDays);
    const logRetention = parseRetentionDays(form.logDays);
    if (eventRetention === null || logRetention === null) {
      setSavedOk(false);
      setValidationError(t.retentionBounds);
      return;
    }
    setValidationError(null);
    setActionError(null);
    const seq = ++saveSeqRef.current;
    const input: SaveSettingsInput = {
      autostart: form.autostart,
      close_to_tray: form.closeToTray,
      locale: form.uiLocale,
      theme: form.theme,
      event_retention_days: eventRetention,
      log_retention_days: logRetention,
      onboarding_completed: form.onboardingCompleted,
      notification_template: form.template.trim() === "" ? null : form.template,
    };
    try {
      const view = await backend.saveSettings(input);
      if (seq !== saveSeqRef.current) {
        return;
      }
      setPausedUntil(view.paused_until);
      setOnboardingCompleted(view.onboarding_completed);
      setSavedOk(true);
    } catch (e: unknown) {
      if (seq !== saveSeqRef.current) {
        return;
      }
      setActionError(errorOf(e));
    }
  }, [backend, hydrated, t]);

  function scheduleSave(immediate: boolean, patch?: Partial<SaveSettingsInput>): void {
    if (!hydrated) {
      return;
    }
    // setState 后同步调用时组件尚未重渲染,ref 仍是旧值——调用方必须把
    // 本次的变更作为 patch 显式并入,否则即时保存会把旧值写回(实测竞态)。
    if (patch !== undefined) {
      autoFormRef.current = { ...autoFormRef.current, ...patch };
    }
    if (saveTimerRef.current !== null) {
      window.clearTimeout(saveTimerRef.current);
      saveTimerRef.current = null;
    }
    if (immediate) {
      void saveNow();
      return;
    }
    saveTimerRef.current = window.setTimeout(() => {
      saveTimerRef.current = null;
      void saveNow();
    }, 600);
  }

  useEffect(
    () => () => {
      if (saveTimerRef.current !== null) {
        window.clearTimeout(saveTimerRef.current);
      }
    },
    [],
  );

  /** 保留天数输入:非法值即时提示且不入队保存;合法值防抖自动保存。 */
  function changeRetentionDays(
    setter: (value: string) => void,
    field: "event_retention_days" | "log_retention_days",
  ): (value: string) => void {
    return (value: string) => {
      setter(value);
      if (parseRetentionDays(value) === null) {
        setValidationError(t.retentionBounds);
        return;
      }
      setValidationError(null);
      scheduleSave(false, { [field]: Number.parseInt(value, 10) });
    };
  }

  async function pause(duration: PauseDurationCode): Promise<void> {
    setPauseBusy(true);
    setActionError(null);
    try {
      // getTimezoneOffset is minutes WEST of UTC, so negate it for the core's
      // east-positive offset — required for a correct 暂停至今日 deadline.
      const view = await backend.setNotificationPause({
        duration,
        offset_seconds: -new Date().getTimezoneOffset() * 60,
      });
      setPausedUntil(view.paused_until);
    } catch (e: unknown) {
      setActionError(errorOf(e));
    } finally {
      setPauseBusy(false);
    }
  }

  async function resume(): Promise<void> {
    setPauseBusy(true);
    setActionError(null);
    try {
      const view = await backend.clearNotificationPause();
      setPausedUntil(view.paused_until);
    } catch (e: unknown) {
      setActionError(errorOf(e));
    } finally {
      setPauseBusy(false);
    }
  }

  async function applyDebug(duration_minutes: SetDebugLoggingInput["duration_minutes"]): Promise<void> {
    const previous = debugMinutes;
    setDebugMinutes(duration_minutes);
    setDebugBusy(true);
    setActionError(null);
    try {
      await backend.setDebugLogging({ duration_minutes });
    } catch (e: unknown) {
      setDebugMinutes(previous);
      setActionError(errorOf(e));
    } finally {
      setDebugBusy(false);
    }
  }

  /** The save dialog opens inside the core; this side sends nothing and only
   *  learns whether it was cancelled plus a safe filename. */
  async function exportDiagnostics(): Promise<void> {
    setExportStatus(null);
    setActionError(null);
    try {
      const result = await backend.exportDiagnostics();
      setExportStatus(
        result.status === "saved"
          ? `${t.diagnosticsSavedPrefix}${result.filename}`
          : t.diagnosticsCancelled,
      );
    } catch (e: unknown) {
      setActionError(errorOf(e));
    }
  }

  async function checkUpdates(): Promise<void> {
    setChecking(true);
    setInstallOk(false);
    setUpdateResult(null);
    setActionError(null);
    try {
      setUpdateResult(await backend.checkForUpdates());
    } catch (e: unknown) {
      setActionError(errorOf(e));
    } finally {
      setChecking(false);
    }
  }

  async function install(): Promise<void> {
    setInstalling(true);
    setActionError(null);
    try {
      await backend.installUpdate({ confirmed: true });
      pendingFocusRestore.current = true;
      setInstallConfirmOpen(false);
      setInstallOk(true);
    } catch (e: unknown) {
      pendingFocusRestore.current = true;
      setInstallConfirmOpen(false);
      setActionError(errorOf(e));
    } finally {
      setInstalling(false);
    }
  }

  function applyTheme(next: ThemeCode): void {
    setTheme(next);
    // Live preview; AppShell owns the attribute at boot.
    document.documentElement.dataset.theme = next;
  }

  return (
    <section aria-label={t.navSettings}>
      {loadError !== null && <p role="alert">{t.settingsLoadFailed}</p>}
      {actionError !== null && (
        <p role="alert">
          {actionError.message}
          {actionError.suggested_action !== null && <>（{actionError.suggested_action}）</>}
        </p>
      )}
      {validationError !== null && <p role="alert">{validationError}</p>}
      {/* 静默保存(2026-08-28 用户裁决):视觉不出现"已保存",仅读屏播报。 */}
      <p role="status" className="sr-only">
        {savedOk ? t.savedOk : ""}
      </p>

      {/* CC Switch 式重排(2026-08-28 用户裁决):无卡片框,单列分组,
          每组 = 标题 + 灰色说明 + 控件;选择类为分段胶囊(原生 radio 语义,
          输入盖在胶囊上保持可点/可聚焦,键盘方向键原生可用)。 */}
      <div className="settings-page">
        {/* 启动与窗口:整行开关行(标题 + 说明 + 开关在右)。 */}
        <section className="settings-group" aria-label={t.sectionStartup}>
          <h2>{t.sectionStartup}</h2>
          <div className="switch-row">
            <div className="switch-row-text">
              <span className="switch-row-title">{t.autostartLabel}</span>
              <span className="switch-row-desc">{t.autostartDesc}</span>
            </div>
            <input
              type="checkbox"
              aria-label={t.autostartLabel}
              checked={autostart}
              disabled={!hydrated}
              onChange={(event) => {
                setAutostart(event.target.checked);
                scheduleSave(true, { autostart: event.target.checked });
              }}
            />
          </div>
          <div className="switch-row">
            <div className="switch-row-text">
              <span className="switch-row-title">{t.closeToTrayLabel}</span>
              <span className="switch-row-desc">{t.closeToTrayDesc}</span>
            </div>
            <input
              type="checkbox"
              aria-label={t.closeToTrayLabel}
              checked={closeToTray}
              disabled={!hydrated}
              onChange={(event) => {
                setCloseToTray(event.target.checked);
                scheduleSave(true, { close_to_tray: event.target.checked });
              }}
            />
          </div>
        </section>

        {/* 界面语言:分段胶囊(原生 radio)。 */}
        <section className="settings-group" aria-label={t.sectionLanguage}>
          <h2>{t.sectionLanguage}</h2>
          <p className="settings-hint">{t.languageHint}</p>
          <div className="seg seg-accent" role="radiogroup" aria-label={t.languageLabel}>
            {([["zh_cn", t.langZh], ["en", t.langEn]] as const).map(([code, label]) => (
              <label
                key={code}
                className={`seg-item${uiLocale === code ? " seg-active" : ""}`}
              >
                <input
                  type="radio"
                  name="settings-locale"
                  className="seg-input"
                  aria-label={label}
                  checked={uiLocale === code}
                  disabled={!hydrated}
                  onChange={() => {
                    setUiLocale(code);
                    scheduleSave(true, { locale: code });
                  }}
                />
                <span>{label}</span>
              </label>
            ))}
          </div>
          {/* The applied locale comes from bootstrap; a changed one needs a
              restart, so say so instead of silently doing nothing. */}
          {hydrated && uiLocale !== locale && <p className="muted">{t.localeRestartHint}</p>}
        </section>

        {/* 外观主题:分段胶囊(浅/深/跟随系统,带图标)。 */}
        <section className="settings-group" aria-label={t.themeSectionTitle}>
          <h2>{t.themeSectionTitle}</h2>
          <p className="settings-hint">{t.themeHint}</p>
          <div className="seg seg-accent" role="radiogroup" aria-label={t.themeLabel}>
            {(["light", "dark", "system"] as const).map((code) => (
              <label
                key={code}
                className={`seg-item${theme === code ? " seg-active" : ""}`}
              >
                <input
                  type="radio"
                  name="settings-theme"
                  className="seg-input"
                  aria-label={
                    code === "system" ? t.themeSystem : code === "light" ? t.themeLight : t.themeDark
                  }
                  checked={theme === code}
                  disabled={!hydrated}
                  onChange={() => {
                    applyTheme(code);
                    scheduleSave(true, { theme: code });
                  }}
                />
                {code === "light" ? (
                  <Sun size={14} aria-hidden="true" />
                ) : code === "dark" ? (
                  <Moon size={14} aria-hidden="true" />
                ) : (
                  <Monitor size={14} aria-hidden="true" />
                )}
                <span>
                  {code === "system"
                    ? t.themeSystem
                    : code === "light"
                      ? t.themeLight
                      : t.themeDark}
                </span>
              </label>
            ))}
          </div>
        </section>

        {/* 数据保留:两个并排数字域。 */}
        <section className="settings-group" aria-label={t.retentionSection}>
          <h2>{t.retentionSection}</h2>
          <p className="settings-hint">{t.retentionHint}</p>
          {/* noValidate: bounds are validated in JS so our own alert message
              shows instead of a native bubble blocking submission. */}
          <form
            className="retention-fields"
            noValidate
            onSubmit={(event) => {
              event.preventDefault();
              scheduleSave(true);
            }}
          >
            <label className="retention-field" htmlFor="settings-event-days">
              <span>{t.eventRetentionLabel}</span>
              <input
                id="settings-event-days"
                type="number"
                min={RETENTION_MIN}
                max={RETENTION_MAX}
                value={eventDays}
                disabled={!hydrated}
                onChange={(event) => {
                  setEventDays(event.target.value);
                  changeRetentionDays(setEventDays, "event_retention_days")(event.target.value);
                }}
              />
            </label>
            <label className="retention-field" htmlFor="settings-log-days">
              <span>{t.logRetentionLabel}</span>
              <input
                id="settings-log-days"
                type="number"
                min={RETENTION_MIN}
                max={RETENTION_MAX}
                value={logDays}
                disabled={!hydrated}
                onChange={(event) => {
                  setLogDays(event.target.value);
                  changeRetentionDays(setLogDays, "log_retention_days")(event.target.value);
                }}
              />
            </label>
            {/* 自动保存:无保存按钮;Enter 立即存一次。 */}
          </form>
        </section>

        {/* 通知暂停 */}
        <section className="settings-group" aria-label={t.pauseSection}>
          <h2>{t.pauseSection}</h2>
          <p className="settings-hint">{t.pauseHint}</p>
          {pausedUntil === null ? (
            <p className="muted">{t.notPaused}</p>
          ) : (
            <p>
              {t.pausedUntilPrefix} {new Date(pausedUntil).toLocaleString()}
            </p>
          )}
          <div className="agent-actions">
            <button
              type="button"
              className="cc-focusable"
              disabled={pauseBusy}
              onClick={() => {
                void pause("fifteen_minutes");
              }}
            >
              {t.pause15m}
            </button>
            <button
              type="button"
              className="cc-focusable"
              disabled={pauseBusy}
              onClick={() => {
                void pause("one_hour");
              }}
            >
              {t.pause1h}
            </button>
            <button
              type="button"
              className="cc-focusable"
              disabled={pauseBusy}
              onClick={() => {
                void pause("today");
              }}
            >
              {t.pauseToday}
            </button>
            <button
              type="button"
              className="cc-focusable"
              disabled={pauseBusy || pausedUntil === null}
              onClick={() => {
                void resume();
              }}
            >
              {t.resumeNotifications}
            </button>
          </div>
        </section>

        {/* 凭据存储异常披露(条件组)。 */}
        {storeIssues.length > 0 && (
          <section className="settings-group" aria-label={t.credentialStoreSection}>
            <h2>{t.credentialStoreSection}</h2>
            <ul className="issue-list">
              {storeIssues.map((issue) => (
                <li key={`${issue.issue_code}:${issue.message}`}>
                  <span>{issue.message}</span>
                  {issue.suggested_action !== null && (
                    <span className="muted">{issue.suggested_action}</span>
                  )}
                  {issue.suggested_command !== null && (
                    <code className="inline-code">{issue.suggested_command}</code>
                  )}
                </li>
              ))}
            </ul>
          </section>
        )}

        {/* 调试与更新 */}
        <section className="settings-group" aria-label={t.sectionDebugUpdate}>
          <h2>{t.sectionDebugUpdate}</h2>
          <p className="settings-hint">{t.debugHint}</p>
          <div className="seg seg-accent" role="radiogroup" aria-label={t.debugLoggingLabel}>
            {(
              [
                [0, t.debugOff],
                [15, t.debug15m],
                [60, t.debug60m],
              ] as const
            ).map(([minutes, label]) => (
              <label
                key={minutes}
                className={`seg-item${debugMinutes === minutes ? " seg-active" : ""}`}
              >
                <input
                  type="radio"
                  name="settings-debug"
                  className="seg-input"
                  aria-label={label}
                  checked={debugMinutes === minutes}
                  disabled={debugBusy}
                  onChange={() => {
                    void applyDebug(minutes);
                  }}
                />
                <span>{label}</span>
              </label>
            ))}
          </div>
          <p className="settings-hint">{t.updateHint}</p>
          <div className="agent-actions">
            <button
              type="button"
              ref={installTriggerRef}
              className="cc-focusable"
              disabled={checking || installing}
              onClick={() => {
                void checkUpdates();
              }}
            >
              {checking ? t.checkingUpdates : t.checkUpdates}
            </button>
            {updateResult !== null && updateResult.available && updateResult.installable && (
              <button
                type="button"
                className="cc-focusable"
                disabled={checking || installing}
                onClick={() => setInstallConfirmOpen(true)}
              >
                {t.installUpdateAction}
              </button>
            )}
          </div>
          {updateResult !== null &&
            (updateResult.available ? (
              <p>
                {t.updateAvailablePrefix} <strong>{updateResult.version ?? ""}</strong>
              </p>
            ) : (
              <p className="muted">{t.upToDate}</p>
            ))}
          {updateResult !== null &&
            updateResult.available &&
            updateResult.notes !== null && <p className="muted">{updateResult.notes}</p>}
          {installOk && (
            <p role="status" className="muted">
              {t.updateInstalled}
            </p>
          )}
        </section>

        {/* 添加渠道(v2-issues:集成页「添加渠道」箭头跳转到这里,表单
            必须真实存在——variant=add 只渲染添加表单,不带表格。) */}
        <section className="settings-group" aria-label={t.addChannelAction}>
          <h2>{t.addChannelAction}</h2>
          <ChannelsPage locale={locale} backend={injected} variant="add" showHeading={false} />
        </section>

        {/* 通知模板 */}
        <ChannelsSectionTemplate
          template={template}
          onTemplateChange={(value) => {
            setTemplate(value);
            scheduleSave(false, {
              notification_template: value.trim() === "" ? null : value,
            });
          }}
          locale={locale}
        />

        {/* 诊断/清空:页面末尾安静一行。 */}
        <div className="settings-footer">
          {exportStatus !== null && (
            <p role="status" className="muted">
              {exportStatus}
            </p>
          )}
          <button
            type="button"
            className="cc-focusable"
            onClick={() => {
              void exportDiagnostics();
            }}
          >
            {t.exportDiagnostics}
          </button>
          <button
            type="button"
            className="cc-focusable"
            onClick={() => {
              setClearConfirmOpen(true);
            }}
          >
            {t.clearHistory}
          </button>
        </div>
      </div>

      {installConfirmOpen && (
        <div className="dialog-overlay">
          <div role="dialog" aria-label={t.installConfirmTitle} className="dialog">
            <h2>{t.installConfirmTitle}</h2>
            <p>
              {updateResult?.version ?? ""}
            </p>
            <div className="row-end">
              <button
                type="button"
                className="cc-focusable"
                onClick={() => {
                  setInstallConfirmOpen(false);
                  installTriggerRef.current?.focus();
                }}
              >
                {t.cancel}
              </button>
              <button
                type="button"
                ref={installConfirmRef}
                className="primary cc-focusable"
                disabled={installing}
                onClick={() => {
                  void install();
                }}
              >
                {t.confirmInstall}
              </button>
            </div>
          </div>
        </div>
      )}

      {clearConfirmOpen && (
        <div className="dialog-overlay" role="presentation">
          <div className="dialog" role="dialog" aria-modal="true" aria-label={t.clearHistory}>
            <h2>{t.clearHistory}</h2>
            <p>{t.clearHistoryWarning}</p>
            <div className="row-end">
              <button
                type="button"
                className="cc-focusable"
                onClick={() => {
                  setClearConfirmOpen(false);
                }}
              >
                {t.cancel}
              </button>
              <button
                type="button"
                className="primary cc-focusable"
                onClick={() => {
                  void backend.clearHistory({ preserve_active_jobs: true });
                  setClearConfirmOpen(false);
                }}
              >
                {t.confirmClearHistory}
              </button>
            </div>
          </div>
        </div>
      )}
    </section>
  );
}
