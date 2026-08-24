// First-run onboarding: Detect Agent → Install Hooks → Add Channel →
// Choose Default Rules → Send Test. Completion persists ONLY after a
// successful test send; the flow resumes at the first incomplete step.
import { useEffect, useState, type ReactNode } from "react";

import { useBackend } from "../lib/backend";
import type {
  AgentIntegrationSummary,
  ChannelId,
  ChannelKindCode,
  ChannelSummary,
  LocaleCode,
  SaveSettingsInput,
  ThemeCode,
} from "../lib/contracts";
import { dictionary } from "../lib/i18n";
import { AppShell } from "../shell/AppShell";

type Step = "detect" | "install" | "channel" | "defaults" | "test";

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
  const [channels, setChannels] = useState<ChannelSummary[]>([]);
  const [channelName, setChannelName] = useState("");
  const [webhook, setWebhook] = useState("");
  const [kind, setKind] = useState<ChannelKindCode>("we_com");
  const [selectedChannel, setSelectedChannel] = useState<ChannelId | "">("");

  // Resume: an existing channel means detect/install/channel already succeeded.
  useEffect(() => {
    backend
      .listChannels()
      .then((existing) => {
        if (existing.length > 0) {
          setChannels(existing);
          setSelectedChannel(existing[0]?.id ?? "");
          setStep("defaults");
        }
      })
      .catch(() => {});
  }, [backend]);

  const blockedAgents =
    detections?.filter((agent) => agent.needs_compatible_version_confirmation) ??
    [];

  function runDetection(): void {
    setDetections(null);
    backend
      .detectAgents({ confirm_compatible_version: false })
      .then((results) => setDetections(results))
      .catch(() => setDetections([]));
  }

  useEffect(() => {
    if (step === "detect" && detections === null) {
      runDetection();
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [step]);

  async function installHooks(): Promise<void> {
    const agents = (detections ?? []).filter((agent) => agent.installed);
    for (const agent of agents) {
      await backend.applyHookAction({
        agent: agent.agent as "claude-code" | "codex",
        action: "install",
        expected_health_revision: 0,
        confirm_compatible_version: false,
      });
    }
    setStep("channel");
  }

  async function saveChannel(): Promise<void> {
    const saved = await backend.saveChannel({
      channel_id: null,
      name: channelName,
      credential:
        kind === "ding_talk"
          ? { kind: "ding_talk", webhook }
          : { kind: "we_com", webhook },
    });
    const refreshed = await backend.listChannels();
    setChannels(refreshed.length > 0 ? refreshed : [saved]);
    setSelectedChannel(saved.id);
    setStep("defaults");
  }

  async function sendTest(): Promise<void> {
    if (selectedChannel === "") {
      return;
    }
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
    } satisfies SaveSettingsInput);
    setCompleted(true);
  }

  if (completed) {
    return <AppShell locale={locale} theme={theme} />;
  }

  return (
    <main className="onboarding">
      <ol className="onboarding-steps" aria-label="onboarding steps">
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
                  <span className="trust-item">{t.trustPending}</span>
                )}
              </li>
            ))}
          </ul>
          {blockedAgents.length > 0 && (
            <p className="trust-command">
              <span>{t.trustCommand}</span>
              <button
                type="button"
                className="cc-focusable"
                onClick={() => {
                  void navigator.clipboard?.writeText(t.trustCommand);
                }}
              >
                {t.copyCommand}
              </button>
              <button type="button" className="cc-focusable" onClick={runDetection}>
                {t.recheck}
              </button>
            </p>
          )}
          <div className="row-end">
            <button
              type="button"
              className="primary cc-focusable"
              disabled={detections === null || blockedAgents.length > 0}
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
          <div className="row-end">
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
            <label htmlFor="ob-channel-name">{t.channelName}</label>
            <input
              id="ob-channel-name"
              value={channelName}
              onChange={(event) => setChannelName(event.target.value)}
            />
            <label htmlFor="ob-channel-kind">kind</label>
            <select
              id="ob-channel-kind"
              value={kind}
              onChange={(event) => setKind(event.target.value as ChannelKindCode)}
            >
              <option value="we_com">WeCom</option>
              <option value="ding_talk">DingTalk</option>
            </select>
            <label htmlFor="ob-channel-webhook">{t.webhookUrl}</label>
            <input
              id="ob-channel-webhook"
              value={webhook}
              onChange={(event) => setWebhook(event.target.value)}
            />
            <div className="row-end">
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
            <button type="button" className="primary cc-focusable" onClick={() => setStep("test")}>
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
          <div className="row-end">
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
