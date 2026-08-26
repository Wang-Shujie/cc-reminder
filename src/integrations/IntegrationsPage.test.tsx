import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";

import { BackendProvider } from "../lib/backend";
import { configuredBackend } from "../test/TestApp";
import { IntegrationsPage } from "./IntegrationsPage";

test("defaults to sources and can switch to destinations", async () => {
  const user = userEvent.setup();
  render(
    <BackendProvider backend={configuredBackend()}>
      <IntegrationsPage locale="zh_cn" />
    </BackendProvider>,
  );
  expect(
    await screen.findByRole("heading", { name: "Agent 集成" }),
  ).toBeVisible();
  await user.click(screen.getByRole("tab", { name: "通知去向" }));
  expect(screen.getByRole("heading", { name: "渠道" })).toBeVisible();
});
