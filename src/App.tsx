// Production entry point. The default export owns the real TauriBackend;
// tests render `AppRoot` through a BackendProvider with the in-memory fake.
import { useEffect, useState, type ReactNode } from "react";

import { BackendProvider, TauriBackend, useBackend } from "./lib/backend";
import type { BootstrapState } from "./lib/contracts";
import { Onboarding } from "./onboarding/Onboarding";
import { AppShell } from "./shell/AppShell";

/** Bootstrapped root: picks onboarding or the shell from the core state. */
export function AppRoot(): ReactNode {
  const backend = useBackend();
  const [boot, setBoot] = useState<BootstrapState | null>(null);

  useEffect(() => {
    let cancelled = false;
    backend
      .getBootstrapState()
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
    return (
      <main className="app-loading">
        <p>CC Reminder</p>
      </main>
    );
  }

  if (!boot.onboarding_completed) {
    return <Onboarding locale={boot.locale} theme={boot.theme} />;
  }

  return <AppShell locale={boot.locale} theme={boot.theme} />;
}

export default function App(): ReactNode {
  return (
    <BackendProvider backend={new TauriBackend()}>
      <AppRoot />
    </BackendProvider>
  );
}
