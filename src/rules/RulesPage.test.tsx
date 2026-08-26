import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";

import { BackendProvider } from "../lib/backend";
import { configuredBackend } from "../test/TestApp";
import { RulesPage } from "./RulesPage";

test("defaults to the rules tab and can switch to project management", async () => {
  const user = userEvent.setup();
  render(
    <BackendProvider backend={configuredBackend()}>
      <RulesPage locale="zh_cn" />
    </BackendProvider>,
  );
  expect(
    await screen.findByRole("heading", { name: "通知规则", level: 1 }),
  ).toBeVisible();
  expect(screen.getByRole("heading", { name: "Hook 规则" })).toBeVisible();
  await user.click(screen.getByRole("tab", { name: "项目管理" }));
  expect(screen.getByRole("heading", { name: "项目" })).toBeVisible();
});
