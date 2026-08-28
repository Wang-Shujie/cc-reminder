// Wire-format regression tests for TauriBackend's invoke payloads (real-core
// contract): the FakeBackend used by page tests bypasses invoke, so argument
// wrapping drift like the list_history pagination bug is invisible to them.
import { beforeEach, describe, expect, it, vi } from "vitest";

const invoke = vi.fn();

vi.mock("@tauri-apps/api/core", () => ({ invoke: (...args: unknown[]) => invoke(...args) }));

import { TauriBackend } from "./backend";

describe("TauriBackend wire payloads", () => {
  beforeEach(() => {
    invoke.mockReset();
    invoke.mockResolvedValue({ items: [], next_offset: null });
  });

  it("list_history keeps pagination out of the deny_unknown_fields filter", async () => {
    const backend = new TauriBackend();
    await backend.listHistory({
      delivery_status: "failed",
      source: "codex",
      source_event: "Stop",
      offset: 100,
      limit: 50,
    });
    expect(invoke).toHaveBeenCalledWith("list_history", {
      // offset/limit MUST travel only in `page`; HistoryFilterInput on the
      // Rust side rejects unknown fields.
      filter: { delivery_status: "failed", source: "codex", source_event: "Stop" },
      page: { offset: 100, limit: 50 },
    });

    await backend.listHistory();
    expect(invoke).toHaveBeenLastCalledWith("list_history", {
      filter: {},
      page: { offset: 0, limit: 50 },
    });
  });

  it("single-input commands wrap their payload under `input`", async () => {
    const backend = new TauriBackend();
    await backend.getHistoryDetail({ event_id: "evt-1" });
    expect(invoke).toHaveBeenCalledWith("get_history_detail", {
      input: { event_id: "evt-1" },
    });
  });
});
