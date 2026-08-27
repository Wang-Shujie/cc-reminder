// 请求层契约测试:三态、事件重取、轻通知、deps 变化重取、卸载防漏写。
import { act, render, screen, waitFor } from "@testing-library/react";
import { useState, type ReactNode } from "react";

import { BackendProvider } from "./backend";
import { configuredBackend, type FakeBackend } from "../test/TestApp";
import { useCoreQuery } from "./useCoreQuery";

function Probe({
  backend,
  failing,
}: {
  backend: FakeBackend;
  failing?: boolean;
}): ReactNode {
  const [bump, setBump] = useState(0);
  const query = useCoreQuery(
    async () => {
      if (failing) {
        throw new Error("boom");
      }
      return `data-${bump}`;
    },
    [bump],
    ["core://queue-changed"],
  );
  return (
    <div>
      <span data-testid="data">{query.data ?? "null"}</span>
      <span data-testid="failed">{String(query.failed)}</span>
      <span data-testid="notice">{query.noticeRevision ?? "none"}</span>
      <span role="status">{query.noticeRevision !== null ? `刷新(#${query.noticeRevision})` : ""}</span>
      <button type="button" onClick={() => setBump(bump + 1)}>
        bump
      </button>
    </div>
  );
}

function renderProbe(backend: FakeBackend, failing = false) {
  render(
    <BackendProvider backend={backend}>
      <Probe backend={backend} failing={failing} />
    </BackendProvider>,
  );
}

test("loads data on mount", async () => {
  renderProbe(configuredBackend());
  expect(await screen.findByTestId("data")).toHaveTextContent("data-0");
  expect(screen.getByTestId("failed")).toHaveTextContent("false");
});

test("a failing fetcher surfaces failed without data", async () => {
  renderProbe(configuredBackend(), true);
  expect(await screen.findByTestId("failed")).toHaveTextContent("true");
  expect(screen.getByTestId("data")).toHaveTextContent("null");
});

test("core events refetch and surface the revision notice", async () => {
  const backend = configuredBackend();
  renderProbe(backend);
  await screen.findByText("data-0", { selector: "[data-testid=data]" });
  act(() => {
    backend.emit("core://queue-changed", { revision: 7 });
  });
  await waitFor(() =>
    expect(screen.getByTestId("notice")).toHaveTextContent("7"),
  );
  expect(screen.getByRole("status")).toHaveTextContent("刷新(#7)");
});

test("changing deps refetches with the new identity", async () => {
  const { user } = { user: (await import("@testing-library/user-event")).default.setup() };
  renderProbe(configuredBackend());
  await screen.findByText("data-0", { selector: "[data-testid=data]" });
  await user.click(screen.getByRole("button", { name: "bump" }));
  await waitFor(() =>
    expect(screen.getByTestId("data")).toHaveTextContent("data-1"),
  );
});
