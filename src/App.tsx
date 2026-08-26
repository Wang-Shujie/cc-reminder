// Production entry point. The default export owns the real TauriBackend;
// tests render `AppRoot` through a BackendProvider with the in-memory fake.
import { useEffect, useState, type ReactNode } from "react";

import { BackendProvider, TauriBackend, useBackend, type Backend } from "./lib/backend";
import type { BootstrapState } from "./lib/contracts";
import { dictionary } from "./lib/i18n";
import { Onboarding } from "./onboarding/Onboarding";
import { AppShell } from "./shell/AppShell";

/** Bootstrapped root: picks onboarding or the shell from the core state. */
export function AppRoot(): ReactNode {
  const backend = useBackend();
  const [boot, setBoot] = useState<BootstrapState | null>(null);

  useEffect(() => {
    let cancelled = false;
    // getTimezoneOffset is minutes WEST of UTC, so negate it for the core's
    // east-positive seconds. Reported at first paint of every session so the
    // core can persist it and evaluate quiet hours in local time (same
    // frontend-reported pattern as the notification pause).
    backend
      .getBootstrapState(-new Date().getTimezoneOffset() * 60)
      .then((state) => {
        if (!cancelled) {
          setBoot(state);
        }
      })
      .catch(() => {
        /* no Tauri runtime (tests) or transient failure: stay on loading */
      });
    return () => {
      cancelled = true;
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
