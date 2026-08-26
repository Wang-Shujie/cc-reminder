import { act, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { TestApp, configuredBackend, type FakeBackend } from "../test/TestApp";

// vitest's jsdom rewrites import.meta.url to http://localhost — resolve the
// stylesheet from the project root instead.
const appCss = readFileSync(resolve(process.cwd(), "src/app.css"), "utf8");

async function renderShell(backend: FakeBackend) {
  render(<TestApp backend={backend} />);
  // Locale-independent: the zh and en shells use different headings.
  await screen.findByRole("heading", { level: 1 });
}

test("navigation is keyboard accessible and remembers the selected page", async () => {
  const user = userEvent.setup();
  const backend = configuredBackend();
  await renderShell(backend);
  await user.click(screen.getByRole("button", { name: "通知规则" }));
  expect(
    screen.getByRole("heading", { name: "通知规则", level: 1 }),
  ).toBeVisible();
  expect(localStorage.getItem("cc-reminder:last-page")).toBe("rules");
  expect(screen.getByRole("button", { name: "通知规则" })).toHaveAttribute(
    "aria-current",
    "page",
  );
});

test("selected page survives a remount via localStorage", async () => {
  const user = userEvent.setup();
  const { unmount } = render(<TestApp backend={configuredBackend()} />);
  await screen.findByRole("heading", { name: "工作台" });
  await user.click(screen.getByRole("button", { name: "设置" }));
  expect(screen.getByRole("heading", { name: "设置", level: 1 })).toBeVisible();
  unmount();
  render(<TestApp backend={configuredBackend()} />);
  expect(await screen.findByRole("heading", { name: "设置" })).toBeVisible();
});

test("legacy v1 page ids migrate to the new destinations", async () => {
  for (const [legacy, heading] of [
    ["overview", "工作台"],
    ["history", "工作台"],
    ["hooks", "通知规则"],
    ["projects", "通知规则"],
    ["agents", "集成"],
    ["channels", "集成"],
    ["settings", "设置"],
  ] as const) {
    localStorage.setItem("cc-reminder:last-page", legacy);
    const { unmount } = render(<TestApp backend={configuredBackend()} />);
    expect(
      await screen.findByRole("heading", { name: heading, level: 1 }),
    ).toBeVisible();
    unmount();
  }
});

test("unknown legacy value falls back to the workbench", async () => {
  localStorage.setItem("cc-reminder:last-page", "nonsense");
  render(<TestApp backend={configuredBackend()} />);
  expect(
    await screen.findByRole("heading", { name: "工作台", level: 1 }),
  ).toBeVisible();
});

// The workbench's overview panel also fetches health on mount and on these
// events, so exact call counts are owned by the page — assert relative growth.
test("core revision events refresh health instead of trusting payload data", async () => {
  const backend = configuredBackend();
  const calls = () => vi.mocked(backend.getHealthSnapshot).mock.calls.length;
  render(<TestApp backend={backend} />);
  await screen.findByRole("heading", { name: "工作台" });
  await waitFor(() => expect(calls()).toBeGreaterThanOrEqual(2));
  const before = calls();
  act(() => {
    backend.emit("core://health-changed", { revision: 4, overall: "forged" });
  });
  await waitFor(() => expect(calls()).toBeGreaterThan(before));
  expect(screen.queryByText("forged")).not.toBeInTheDocument();
});

test("queue revision events also trigger a refetch", async () => {
  const backend = configuredBackend();
  const calls = () => vi.mocked(backend.getHealthSnapshot).mock.calls.length;
  render(<TestApp backend={backend} />);
  await screen.findByRole("heading", { name: "工作台" });
  await waitFor(() => expect(calls()).toBeGreaterThanOrEqual(2));
  const before = calls();
  act(() => {
    backend.emit("core://queue-changed", { revision: 7 });
  });
  await waitFor(() => expect(calls()).toBeGreaterThan(before));
});

test("labels default to Chinese", async () => {
  await renderShell(configuredBackend());
  expect(screen.getByRole("button", { name: "工作台" })).toBeVisible();
  expect(screen.queryByRole("button", { name: "Workbench" })).not.toBeInTheDocument();
});

test("locale can follow the saved setting (English)", async () => {
  await renderShell(configuredBackend({ locale: "en" }));
  expect(screen.getByRole("button", { name: "Workbench" })).toBeVisible();
  expect(screen.queryByRole("button", { name: "工作台" })).not.toBeInTheDocument();
});

test("theme follows the saved setting; system resolves to the system attribute", async () => {
  await renderShell(configuredBackend({ theme: "system" }));
  expect(document.documentElement.dataset.theme).toBe("system");
});

test("explicit dark theme is applied verbatim for CSS to consume", async () => {
  await renderShell(configuredBackend({ theme: "dark" }));
  expect(document.documentElement.dataset.theme).toBe("dark");
});

test("focus is visible on navigation controls", async () => {
  const user = userEvent.setup();
  await renderShell(configuredBackend());
  await user.tab();
  expect(screen.getByRole("button", { name: "工作台" })).toHaveFocus();
  expect(screen.getByRole("button", { name: "工作台" })).toHaveClass("cc-focusable");
  // The stylesheet must define the visible focus treatment.
  expect(appCss).toContain(":focus-visible");
  expect(appCss).toContain("outline");
});

test("all four navigation targets are present", async () => {
  await renderShell(configuredBackend());
  for (const label of ["工作台", "通知规则", "集成", "设置"]) {
    expect(screen.getByRole("button", { name: label })).toBeVisible();
  }
});

test("configured startup defaults to the workbench", async () => {
  await renderShell(configuredBackend());
  expect(screen.getByRole("button", { name: "工作台" })).toHaveAttribute(
    "aria-current",
    "page",
  );
  // The default is not persisted as if the user had chosen it.
  expect(localStorage.getItem("cc-reminder:last-page")).toBeNull();
});

test("960 x 640 fixture: rail, header, content stay structurally separate", async () => {
  window.innerWidth = 960;
  window.innerHeight = 640;
  try {
    await renderShell(configuredBackend());
    const shell = document.querySelector(".shell-root");
    const header = document.querySelector(".shell-header");
    const nav = document.querySelector(".shell-nav");
    const content = document.querySelector(".shell-content");
    expect(shell).not.toBeNull();
    expect(header).not.toBeNull();
    expect(nav).not.toBeNull();
    expect(content).not.toBeNull();
    // Non-overlap contract: one grid root owns three disjoint regions.
    expect(header!.parentElement).toBe(shell);
    expect(nav!.parentElement).toBe(shell);
    expect(content!.parentElement).toBe(shell);
    expect(nav!.contains(header!)).toBe(false);
    expect(header!.contains(content!)).toBe(false);
    // Fixed rail width / header height / minimum window come from CSS
    // (jsdom performs no layout, so the geometry lives in the stylesheet).
    expect(appCss).toContain("grid-template-columns: 184px 1fr");
    expect(appCss).toMatch(/grid-template-rows:\s*48px 1fr/);
    expect(appCss).toMatch(/min-width:\s*960px/);
    expect(appCss).toMatch(/min-height:\s*640px/);
    // Quiet aesthetic: no viewport-scaled fonts, letter spacing pinned to 0,
    // radii capped at 8px, health-only color accents.
    expect(appCss).not.toMatch(/\d+vmin|\d+vw/);
    expect(appCss).toContain("letter-spacing: 0");
    for (const radius of appCss.matchAll(/border-radius:\s*([^;]+);/g)) {
      const values = radius[1]?.match(/\d+/g) ?? [];
      for (const v of values) {
        expect(Number(v)).toBeLessThanOrEqual(8);
      }
    }
  } finally {
    window.innerWidth = 1024;
    window.innerHeight = 768;
  }
});
