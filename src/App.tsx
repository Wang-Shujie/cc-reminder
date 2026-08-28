// Production entry point. The default export owns the real TauriBackend;
// tests render `AppRoot` through a BackendProvider with the in-memory fake.
import { useEffect, useState, type ReactNode } from "react";

import { BackendProvider, TauriBackend, useBackend, type Backend } from "./lib/backend";
import type { BootstrapState } from "./lib/contracts";
import { dictionary } from "./lib/i18n";
import { Onboarding } from "./pages/onboarding/Onboarding";
import { AppShell } from "./shell/AppShell";

/** Bootstrapped root: picks onboarding or the shell from the core state. */
export function AppRoot(): ReactNode {
  const backend = useBackend();
  const [boot, setBoot] = useState<BootstrapState | null>(null);

  useEffect(() => {
    let cancelled = false;
    let timer: number | null = null;
    // getTimezoneOffset is minutes WEST of UTC, so negate it for the core's
    // east-positive seconds. Reported at first paint of every session so the
    // core can persist it and evaluate quiet hours in local time (same
    // frontend-reported pattern as the notification pause).
    //
    // v2-issues:core 初始化已后台化——钥匙串加载期间命令可能报
    // core_starting,这里轮询重试直到 bootstrap 成功,而不是永远停在
    // loading 屏。
    const offset = -new Date().getTimezoneOffset() * 60;
    const load = () => {
      backend
        .getBootstrapState(offset)
        .then((state) => {
          if (!cancelled) {
            setBoot(state);
          }
        })
        .catch(() => {
          // no Tauri runtime (tests) or core still starting: retry shortly
          if (!cancelled) {
            timer = window.setTimeout(load, 400);
          }
        });
    };
    load();
    return () => {
      cancelled = true;
      if (timer !== null) {
        window.clearTimeout(timer);
      }
    };
  }, [backend]);

  if (!boot) {
    // Locale is unknown before bootstrap resolves; zh-CN is the authoritative
    // default per the i18n dictionary.
    return (
      <main className="app-loading">
        <p>CC Reminder</p>
        <p>{dictionary("zh_cn").loading}</p>
      </main>
    );
  }

  if (!boot.onboarding_completed) {
    return <Onboarding locale={boot.locale} theme={boot.theme} />;
  }

  return <AppShell locale={boot.locale} theme={boot.theme} />;
}

function LoadingScreen(): ReactNode {
  return (
    <main className="app-loading">
      <p>CC Reminder</p>
      <p>{dictionary("zh_cn").loading}</p>
    </main>
  );
}

async function resolveBackend(): Promise<Backend> {
  // Compile-time selection ONLY: Vite statically replaces
  // VITE_CC_REMINDER_TEST_BACKEND, so when the var is unset (every production
  // build) this branch is constant-false, the dynamic import is tree-shaken,
  // and src/test/browser-backend never ships in dist. CI greps dist for that
  // module's marker string to keep this guarantee honest.
  if (import.meta.env.VITE_CC_REMINDER_TEST_BACKEND === "1") {
    const { createBrowserTestBackend } = await import("./test/browser-backend");
    return createBrowserTestBackend();
  }
  return new TauriBackend();
}

export default function App(): ReactNode {
  const [backend, setBackend] = useState<Backend | null>(null);

  useEffect(() => {
    let cancelled = false;
    void resolveBackend().then((resolved) => {
      if (!cancelled) {
        setBackend(resolved);
      }
    });
    return () => {
      cancelled = true;
    };
  }, []);

  if (backend === null) {
    return <LoadingScreen />;
  }

  return (
    <BackendProvider backend={backend}>
      <AppRoot />
    </BackendProvider>
  );
}
