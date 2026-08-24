// Per-agent Hook drift derivation shared by the Hook Rules page (Task 17) and
// the Agent Integration page (Task 18). An event is REQUIRED when it is
// enabled globally OR enabled by any project patch, while installed state
// comes from the agent itself; drift exists exactly when the two disagree.
import type { HookRuleRow } from "./contracts";

export interface DriftBasisRow {
  source_event: string;
  installed: boolean;
  enabledAnywhere: boolean;
  available: boolean;
}

/** One merged entry per source_event across global + project-scope rows. */
export function mergeRowsForDrift(rows: HookRuleRow[]): DriftBasisRow[] {
  const byEvent = new Map<string, DriftBasisRow>();
  for (const row of rows) {
    const prev = byEvent.get(row.source_event);
    byEvent.set(row.source_event, {
      source_event: row.source_event,
      installed: prev?.installed ?? row.installed,
      enabledAnywhere: (prev?.enabledAnywhere ?? false) || row.enabled,
      available: prev?.available ?? row.available,
    });
  }
  return [...byEvent.values()];
}

export function driftEvents(basis: DriftBasisRow[]): {
  added: string[];
  removed: string[];
} {
  return {
    added: basis
      .filter((row) => row.available && row.enabledAnywhere && !row.installed)
      .map((row) => row.source_event),
    removed: basis
      .filter((row) => row.installed && !row.enabledAnywhere)
      .map((row) => row.source_event),
  };
}
