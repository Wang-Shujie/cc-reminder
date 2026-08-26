// src/shell/TabBar.test.tsx
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { useState, type ReactNode } from "react";

import { TabBar } from "./TabBar";

const TABS = [
  { id: "alpha", label: "甲" },
  { id: "beta", label: "乙" },
] as const;

function Harness(): ReactNode {
  const [active, setActive] = useState<"alpha" | "beta">("alpha");
  return (
    <TabBar
      tabs={TABS}
      active={active}
      onSelect={setActive}
      ariaLabel="演示标签组"
    />
  );
}

test("renders a tablist with one selected tab", () => {
  render(<Harness />);
  expect(screen.getByRole("tablist", { name: "演示标签组" })).toBeVisible();
  expect(screen.getByRole("tab", { name: "甲" })).toHaveAttribute(
    "aria-selected",
    "true",
  );
  expect(screen.getByRole("tab", { name: "乙" })).toHaveAttribute(
    "aria-selected",
    "false",
  );
});

test("click selects a tab", async () => {
  const user = userEvent.setup();
  render(<Harness />);
  await user.click(screen.getByRole("tab", { name: "乙" }));
  expect(screen.getByRole("tab", { name: "乙" })).toHaveAttribute(
    "aria-selected",
    "true",
  );
});

test("arrow keys move selection with focus (automatic activation)", async () => {
  const user = userEvent.setup();
  render(<Harness />);
  const first = screen.getByRole("tab", { name: "甲" });
  await user.click(first);
  await user.keyboard("{ArrowRight}");
  expect(screen.getByRole("tab", { name: "乙" })).toHaveAttribute(
    "aria-selected",
    "true",
  );
  expect(screen.getByRole("tab", { name: "乙" })).toHaveFocus();
  await user.keyboard("{ArrowLeft}");
  expect(screen.getByRole("tab", { name: "甲" })).toHaveAttribute(
    "aria-selected",
    "true",
  );
  expect(screen.getByRole("tab", { name: "甲" })).toHaveFocus();
});
