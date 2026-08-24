// Settings page (Task 19): native controls persisting exact values via
// save_settings (the Rust side applies autostart), notification pause, and
// the update check/install confirmation flow. The credential-store section
// discloses a secure-store unavailability reported by shared health.
// Clear-history / diagnostics export / debug logging arrive in Task 20.
import { useEffect, useRef, useState, type ReactNode } from "react";

import { usePageBackend, type Backend } from "../lib/backend";
import { errorOf, type PageError } from "../lib/errors";
import type {
  HealthIssue,
  LocaleCode,
  PauseDurationCode,
  SaveSettingsInput,
  SettingsView,
  ThemeCode,
  UpdateCheckResult,
} from "../lib/contracts";
import { dictionary } from "../lib/i18n";

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

  const [pausedUntil, setPausedUntil] = useState<string | null>(null);
  const [pauseBusy, setPauseBusy] = useState(false);

  const [saving, setSaving] = useState(false);
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
  /** Set when a dialog closes so the post-commit effect can restore focus
   *  (restoring synchronously races the dialog's own unmount). */
  const pendingFocusRestore = useRef(false);

  useEffect(() => {
    if (!installConfirmOpen && pendingFocusRestore.current) {
      pendingFocusRestore.current = false;
      installTriggerRef.current?.focus();
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

  async function save(): Promise<void> {
    const eventRetention = parseRetentionDays(eventDays);
    const logRetention = parseRetentionDays(logDays);
    if (eventRetention === null || logRetention === null) {
      setSavedOk(false);
      setValidationError(t.retentionBounds);
      return;
    }
    setSaving(true);
    setSavedOk(false);
    setValidationError(null);
    setActionError(null);
    const input: SaveSettingsInput = {
      autostart,
      close_to_tray: closeToTray,
      locale: uiLocale,
      theme,
      event_retention_days: eventRetention,
      log_retention_days: logRetention,
      onboarding_completed: onboardingCompleted,
    };
    try {
      const view = await backend.saveSettings(input);
      setPausedUntil(view.paused_until);
      setOnboardingCompleted(view.onboarding_completed);
      setSavedOk(true);
    } catch (e: unknown) {
      setActionError(errorOf(e));
    } finally {
      setSaving(false);
    }
  }

  async function pause(duration: PauseDurationCode): Promise<void> {
    setPauseBusy(true);
    setActionError(null);
    try {
      const view = await backend.setNotificationPause({ duration });
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
      <h1>{t.navSettings}</h1>

      {loadError !== null && <p role="alert">{t.settingsLoadFailed}</p>}
      {actionError !== null && (
        <p role="alert">
          {actionError.message}
          {actionError.suggested_action !== null && <>（{actionError.suggested_action}）</>}
        </p>
      )}
      {validationError !== null && <p role="alert">{validationError}</p>}
      {savedOk && (
        <p role="status" className="muted">
          {t.savedOk}
        </p>
      )}

      {/* Startup + window */}
      <div className="settings-section">
        <h2>{t.sectionStartup}</h2>
        <label className="check-row">
          <input
            type="checkbox"
            checked={autostart}
            disabled={!hydrated}
            onChange={(event) => setAutostart(event.target.checked)}
          />
          <span>{t.autostartLabel}</span>
        </label>
        <label className="check-row">
          <input
            type="checkbox"
            checked={closeToTray}
            disabled={!hydrated}
            onChange={(event) => setCloseToTray(event.target.checked)}
          />
          <span>{t.closeToTrayLabel}</span>
        </label>
      </div>

      {/* Language */}
      <div className="settings-section">
        <h2>{t.languageLabel}</h2>
        <label htmlFor="settings-language">{t.languageLabel}</label>
        <select
          id="settings-language"
          value={uiLocale}
          disabled={!hydrated}
          onChange={(event) => setUiLocale(event.target.value as LocaleCode)}
        >
          <option value="zh_cn">{t.langZh}</option>
          <option value="en">{t.langEn}</option>
        </select>
      </div>

      {/* Theme */}
      <div className="settings-section" role="radiogroup" aria-label={t.themeLabel}>
        <h2>{t.themeLabel}</h2>
        {(["system", "light", "dark"] as const).map((code) => (
          <label key={code} className="check-row">
            <input
              type="radio"
              name="settings-theme"
              aria-label={
                code === "system" ? t.themeSystem : code === "light" ? t.themeLight : t.themeDark
              }
              checked={theme === code}
              disabled={!hydrated}
              onChange={() => applyTheme(code)}
            />
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

      {/* Retention */}
      {/* noValidate: bounds are validated in JS so our own alert message
          shows instead of a native bubble blocking submission. */}
      <form
        className="settings-section"
        noValidate
        onSubmit={(event) => {
          event.preventDefault();
          void save();
        }}
      >
        <h2>{t.retentionSection}</h2>
        <label htmlFor="settings-event-days">{t.eventRetentionLabel}</label>
        <input
          id="settings-event-days"
          type="number"
          min={RETENTION_MIN}
          max={RETENTION_MAX}
          value={eventDays}
          disabled={!hydrated}
          onChange={(event) => setEventDays(event.target.value)}
        />
        <label htmlFor="settings-log-days">{t.logRetentionLabel}</label>
        <input
          id="settings-log-days"
          type="number"
          min={RETENTION_MIN}
          max={RETENTION_MAX}
          value={logDays}
          disabled={!hydrated}
          onChange={(event) => setLogDays(event.target.value)}
        />
        <div className="row-end">
          <button
            type="submit"
            className="primary cc-focusable"
            disabled={saving || !hydrated}
          >
            {t.saveBtn}
          </button>
        </div>
      </form>

      {/* Notification pause */}
      <div className="settings-section">
        <h2>{t.pauseSection}</h2>
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
      </div>

      {/* Updates */}
      <div className="settings-section">
        <h2>{t.updatesSection}</h2>
        {updateResult !== null &&
          (updateResult.available ? (
            <p>
              {t.updateAvailablePrefix}{" "}
              <strong>{updateResult.version ?? ""}</strong>
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
        </button>{" "}
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

      {/* Credential store disclosure */}
      {storeIssues.length > 0 && (
        <div className="settings-section">
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
        </div>
      )}

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
    </section>
  );
}
