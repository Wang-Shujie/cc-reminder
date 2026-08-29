// First-run onboarding: Detect Agent → Install Hooks → Add Channel →
// Choose Default Rules → Send Test. Completion persists ONLY after a
// successful test send; the flow resumes at the first incomplete step.
import { useEffect, useState, type ReactNode } from "react";

import { useBackend } from "../../lib/backend";
import type {
  AgentIntegrationSummary,
  ChannelId,
  ChannelKindCode,
  ChannelSummary,
  LocaleCode,
  SaveSettingsInput,
  ThemeCode,
} from "../../lib/contracts";
import { dictionary } from "../../lib/i18n";
import { errorOf, type PageError } from "../../lib/errors";
import { ChannelGuide } from "../channels/ChannelGuide";
import { AppShell } from "../../shell/AppShell";

type Step = "detect" | "install" | "channel" | "defaults" | "test";

/** True when detection shows a missing agent. A version newer than the
 * embedded catalog (`needs_compatible_version_confirmation`) does NOT gate
 * navigation: it is confirmed at install time, exactly like the Agent
 * integration page — blocking here deadlocked the wizard for every agent
 * version above the catalog (e.g. claude-code 2.1.247 > 2.1.218). */
function trustGateBlocked(agents: AgentIntegrationSummary[]): boolean {
  return agents.some((agent) => !agent.installed);
}

export function Onboarding({
  locale,
  theme,
}: {
  locale: LocaleCode;
  theme: ThemeCode;
}): ReactNode {
  const backend = useBackend();
  const t = dictionary(locale);
  const [completed, setCompleted] = useState(false);
  const [step, setStep] = useState<Step>("detect");
  const [detections, setDetections] = useState<AgentIntegrationSummary[] | null>(
    null,
  );
  const [detectError, setDetectError] = useState(false);
  const [error, setError] = useState<PageError | null>(null);
  const [channels, setChannels] = useState<ChannelSummary[]>([]);
  const [channelName, setChannelName] = useState("");
  const [webhook, setWebhook] = useState("");
  const [signingSecret, setSigningSecret] = useState("");
  const [keywordPrefix, setKeywordPrefix] = useState("");
  const [kind, setKind] = useState<ChannelKindCode>("we_com");
  const [selectedChannel, setSelectedChannel] = useState<ChannelId | "">("");

  // Resume: load channels AND run detection once on mount, then compute the
  // true starting step. A pending Codex trust confirmation (or a missing
  // agent) must block resume at the checklist instead of jumping straight to
  // defaults — completion would otherwise persist unconfirmed hooks.
  useEffect(() => {
    let cancelled = false;
    type DetectionOutcome =
      | { ok: true; results: AgentIntegrationSummary[] }
      | { ok: false; results: [] };
    const channelsPromise: Promise<ChannelSummary[]> = backend
      .listChannels()
      .catch(() => []);
    const detectionPromise: Promise<DetectionOutcome> = backend
      .detectAgents({ confirm_compatible_version: false })
      .then(
        (results): DetectionOutcome => ({ ok: true, results }),
        (): DetectionOutcome => ({ ok: false, results: [] }),
      );
    void Promise.all([channelsPromise, detectionPromise]).then(
      ([existing, detected]) => {
        if (cancelled) {
          return;
        }
        if (existing.length > 0) {
          setChannels(existing);
          setSelectedChannel(existing[0]?.id ?? "");
        }
        if (!detected.ok) {
          setDetectError(true);
          return;
        }
        setDetections(detected.results);
        if (existing.length > 0 && !trustGateBlocked(detected.results)) {
          setStep("defaults");
        }
      },
    );
    return () => {
      cancelled = true;
    };
  }, [backend]);

  const blockedAgents =
    detections?.filter((agent) => agent.needs_compatible_version_confirmation) ??
    [];
  // Installing IS the acknowledgment: the backend rejects unconfirmed applies
  // for catalog-unverified versions (agent_confirmation_required), and this
  // disclosure is what the user sees before that click (Agent integration
  // page parity, minus the extra round trip).
  const needsVersionConsent = blockedAgents.some((agent) => agent.installed);

  function runDetection(): void {
    setDetections(null);
    setDetectError(false);
    backend
      .detectAgents({ confirm_compatible_version: false })
      .then((results) => {
        setDetections(results);
        // Same rule as the mount-time resume computation: once the trust gate
        // clears and a channel already exists, resume at the defaults step.
        if (channels.length > 0 && !trustGateBlocked(results)) {
          setStep("defaults");
        }
      })
      .catch(() => {
        setDetections(null);
        setDetectError(true);
      });
  }

  async function installHooks(): Promise<void> {
    setError(null);
    try {
      const agents = (detections ?? []).filter((agent) => agent.installed);
      for (const agent of agents) {
        await backend.applyHookAction({
          agent: agent.agent as "claude-code" | "codex",
          action: "install",
          expected_health_revision: 0,
          confirm_compatible_version: needsVersionConsent,
        });
      }
      setStep("channel");
    } catch (e: unknown) {
      setError(errorOf(e));
    }
  }

  async function saveChannel(): Promise<void> {
    setError(null);
    try {
      const saved = await backend.saveChannel({
        channel_id: null,
        name: channelName,
        keyword_prefix:
          kind === "ding_talk" && keywordPrefix !== "" ? keywordPrefix : null,
        credential:
          kind === "ding_talk"
            ? {
                kind: "ding_talk",
                webhook,
                signing_secret: signingSecret === "" ? null : signingSecret,
              }
            : { kind: "we_com", webhook },
      });
      const refreshed = await backend.listChannels();
      setChannels(refreshed.length > 0 ? refreshed : [saved]);
      setSelectedChannel(saved.id);
      setStep("defaults");
    } catch (e: unknown) {
      setError(errorOf(e));
    }
  }

  /** v2-issues: 默认步骤把已保存渠道写入所有"启用且无目标"的默认规则,
   *  否则引导完成后事件被捕获但无处投递(收不到通知)。 */
  async function applyDefaults(): Promise<void> {
    if (channels.length > 0) {
      setError(null);
      try {
        for (const agent of ["claude-code", "codex"] as const) {
          const rows = await backend.listHookRules({ agent, project_id: null });
          for (const row of rows) {
            if (row.enabled && row.config.targets.length === 0) {
              await backend.saveGlobalRule({
                agent,
                source_event: row.source_event,
                config: {
                  ...row.config,
                  targets: channels.map((channel) => ({
                    channel_id: channel.id,
                    template: null,
                  })),
                },
              });
            }
          }
        }
      } catch (e: unknown) {
        setError(errorOf(e));
        return;
      }
    }
    setStep("test");
  }

  async function sendTest(): Promise<void> {
    if (selectedChannel === "") {
      return;
    }
    setError(null);
    try {
      await backend.sendRuleTest({
        agent: "codex",
        source_event: "Stop",
        channel_id: selectedChannel as ChannelId,
      });
      // Completion is persisted only after the successful test send.
      const settings = await backend.getSettings();
      await backend.saveSettings({
        autostart: settings.autostart,
        close_to_tray: settings.close_to_tray,
        locale: settings.locale,
        theme: settings.theme,
        event_retention_days: settings.event_retention_days,
        log_retention_days: settings.log_retention_days,
        onboarding_completed: true,
          notification_template: null,
      } satisfies SaveSettingsInput);
      setCompleted(true);
    } catch (e: unknown) {
      setError(errorOf(e));
    }
  }

  const errorLine =
    error === null ? null : (
      <p role="alert">
        {error.message}
        {error.suggested_action !== null && <>（{error.suggested_action}）</>}
      </p>
    );

  if (completed) {
    return <AppShell locale={locale} theme={theme} />;
  }

  return (
    <main className="onboarding">
      <ol className="onboarding-steps" aria-label={t.onboardingSteps}>
        <li aria-current={step === "detect" ? "step" : undefined}>
          {t.onboardingDetect}
        </li>
        <li aria-current={step === "install" ? "step" : undefined}>
          {t.onboardingInstall}
        </li>
        <li aria-current={step === "channel" ? "step" : undefined}>
          {t.onboardingChannel}
        </li>
        <li aria-current={step === "defaults" ? "step" : undefined}>
          {t.onboardingDefaults}
        </li>
        <li aria-current={step === "test" ? "step" : undefined}>
          {t.onboardingTest}
        </li>
      </ol>

      {step === "detect" && (
        <section>
          <h1>{t.onboardingDetect}</h1>
          <h2>{t.detectedAgents}</h2>
          <ul className="detect-list">
            {(detections ?? []).map((agent) => (
              <li key={agent.agent}>
                <span>{agent.agent}</span>
                <span>
                  {agent.version ?? "—"} · {agent.health}
                </span>
                {agent.needs_compatible_version_confirmation && (
                  <span className="trust-item">{t.versionUnverified}</span>
                )}
              </li>
            ))}
          </ul>
          {detectError && (
            <p className="trust-command" role="alert">
              <span>{t.detectFailed}</span>
              <button type="button" className="cc-focusable" onClick={runDetection}>
                {t.recheck}
              </button>
            </p>
          )}
          <div className="row-end">
            <button
              type="button"
              className="primary cc-focusable"
              disabled={detections === null || detections.length === 0}
              onClick={() => setStep("install")}
            >
              {t.next}
            </button>
          </div>
        </section>
      )}

      {step === "install" && (
        <section>
          <h1>{t.onboardingInstall}</h1>
          {needsVersionConsent && (
            <p className="muted field-hint" role="note">
              {t.versionConsentHint}
            </p>
          )}
          {errorLine}
          <div className="row-end">
            <button
              type="button"
              className="cc-focusable"
              onClick={() => setStep("detect")}
            >
              {t.onboardingBack}
            </button>
            <button type="button" className="primary cc-focusable" onClick={installHooks}>
              {t.installHook}
            </button>
          </div>
        </section>
      )}

      {step === "channel" && (
        <section>
          <h1>{t.onboardingChannel}</h1>
          <form
            onSubmit={(event) => {
              event.preventDefault();
              void saveChannel();
            }}
          >
            {/* v2-issues:分步指引随平台切换,与 docs/operations.md §5 同源。 */}
            <ChannelGuide locale={locale} kind={kind} />
            <label htmlFor="ob-channel-name">{t.channelName}</label>
            <input
              id="ob-channel-name"
              value={channelName}
              onChange={(event) => setChannelName(event.target.value)}
            />
            <label htmlFor="ob-channel-kind">{t.channelKind}</label>
            <select
              id="ob-channel-kind"
              value={kind}
              onChange={(event) => setKind(event.target.value as ChannelKindCode)}
            >
              <option value="we_com">{t.kindWeCom}</option>
              <option value="ding_talk">{t.kindDingTalk}</option>
            </select>
            <label htmlFor="ob-channel-webhook">{t.webhookUrl}</label>
            <input
              id="ob-channel-webhook"
              value={webhook}
              onChange={(event) => setWebhook(event.target.value)}
            />
            {kind === "ding_talk" && (
              <>
                <label htmlFor="ob-channel-secret">{t.signingSecret}</label>
                <input
                  id="ob-channel-secret"
                  type="password"
                  autoComplete="new-password"
                  value={signingSecret}
                  onChange={(event) => setSigningSecret(event.target.value)}
                />
                <p className="muted field-hint">{t.secretHint}</p>
                <label htmlFor="ob-channel-prefix">{t.keywordPrefixField}</label>
                <input
                  id="ob-channel-prefix"
                  value={keywordPrefix}
                  onChange={(event) => setKeywordPrefix(event.target.value)}
                />
                <p className="muted field-hint">{t.keywordHint}</p>
              </>
            )}
            {errorLine}
            <div className="row-end">
              <button
                type="button"
                className="cc-focusable"
                onClick={() => setStep("install")}
              >
                {t.onboardingBack}
              </button>
              <button type="submit" className="primary cc-focusable">
                {t.saveChannel}
              </button>
            </div>
          </form>
        </section>
      )}

      {step === "defaults" && (
        <section>
          <h1>{t.onboardingDefaults}</h1>
          <p className="muted">{t.useDefaults}</p>
          <div className="row-end">
            <button
              type="button"
              className="cc-focusable"
              onClick={() => setStep("channel")}
            >
              {t.onboardingBack}
            </button>
            <button type="button" className="primary cc-focusable" onClick={() => { void applyDefaults(); }}>
              {t.useDefaults}
            </button>
          </div>
        </section>
      )}

      {step === "test" && (
        <section>
          <h1>{t.onboardingTest}</h1>
          <label htmlFor="ob-test-channel">{t.selectChannel}</label>
          <select
            id="ob-test-channel"
            value={selectedChannel}
            onChange={(event) => setSelectedChannel(event.target.value as ChannelId)}
          >
            {channels.map((channel) => (
              <option key={channel.id} value={channel.id}>
                {channel.name}
              </option>
            ))}
          </select>
          {errorLine}
          <div className="row-end">
            <button
              type="button"
              className="cc-focusable"
              onClick={() => setStep("defaults")}
            >
              {t.onboardingBack}
            </button>
            <button
              type="button"
              className="primary cc-focusable"
              disabled={selectedChannel === ""}
              onClick={() => {
                void sendTest();
              }}
            >
              {t.sendTest}
            </button>
          </div>
        </section>
      )}
    </main>
  );
}
