// Settings page (Task 19): native controls persisting exact values via
// save_settings (the Rust side applies autostart), notification pause, and
// the update check/install confirmation flow. The credential-store section
// discloses a secure-store unavailability reported by shared health.
// Task 20 fix round 1: the diagnostics export opens its save dialog inside
// the core (no path crosses the bridge), and a bounded debug-logging window
// (关闭 / 15 分钟 / 60 分钟) is reachable from here.
import { useEffect, useRef, useState, type ReactNode } from "react";

import { usePageBackend, type Backend } from "../lib/backend";
import { errorOf, type PageError } from "../lib/errors";
import type {
  HealthIssue,
  LocaleCode,
  PauseDurationCode,
  SaveSettingsInput,
  SetDebugLoggingInput,
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

  const [pausedUntil, setPausedUntil] = useState<string | null>(null);
  const [pauseBusy, setPauseBusy] = useState(false);

  /** Bounded debug window selection (0 = off). The window itself lives in the
   *  core; this only mirrors the last value chosen on this page. */
  const [debugMinutes, setDebugMinutes] = useState<0 | 15 | 60>(0);
  const [debugBusy, setDebugBusy] = useState(false);
  const [exportStatus, setExportStatus] = useState<string | null>(null);

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
  const installConfirmRef = useRef<HTMLButtonElement | null>(null);
  /** Set when a dialog closes so the post-commit effect can restore focus
   *  (restoring synchronously races the dialog's own unmount). */
  const pendingFocusRestore = useRef(false);
  /** Last persisted theme; restores the live preview when leaving the page
   *  without saving. */
  const persistedThemeRef = useRef<ThemeCode>("system");

  useEffect(() => {
    // Unmount cleanup: undo any unsaved theme preview so AppShell's persisted
    // theme applies again.
    return () => {
      document.documentElement.dataset.theme = persistedThemeRef.current;
    };
  }, []);

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
        persistedThemeRef.current = view.theme;
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
      persistedThemeRef.current = view.theme;
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
        {/* The applied locale comes from bootstrap; a changed one needs a
            restart, so say so instead of silently doing nothing. */}
        {hydrated && uiLocale !== locale && <p className="muted">{t.localeRestartHint}</p>}
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

      {/* Debug logging: a bounded window (off / 15 / 60 minutes) applied
          immediately via its own command, like notification pause. */}
      <div className="settings-section">
        <h2>{t.debugSection}</h2>
        <label htmlFor="settings-debug">{t.debugLoggingLabel}</label>
        <select
          id="settings-debug"
          value={debugMinutes}
          disabled={debugBusy}
          onChange={(event) => {
            void applyDebug(Number(event.target.value) as 0 | 15 | 60);
          }}
        >
          <option value={0}>{t.debugOff}</option>
          <option value={15}>{t.debug15m}</option>
          <option value={60}>{t.debug60m}</option>
        </select>
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

      <div className="row-end">
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
