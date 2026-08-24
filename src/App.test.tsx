import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { TestApp, configuredBackend } from "./test/TestApp";
import App from "./App";

test("renders the onboarding first screen when bootstrap is incomplete", async () => {
  render(<TestApp backend={configuredBackend({ onboardingCompleted: false })} />);
  expect(await screen.findByRole("heading", { name: "检测 Agent" })).toBeVisible();
});

test("renders the app shell at the saved page when bootstrap is complete", async () => {
  localStorage.setItem("cc-reminder:last-page", "channels");
  render(<TestApp backend={configuredBackend()} />);
  expect(await screen.findByRole("heading", { name: "渠道" })).toBeVisible();
  localStorage.removeItem("cc-reminder:last-page");
});

test("production entry point mounts without crashing", () => {
  // The default export wires TauriBackend itself; here it must at least render
  // its loading state without a Tauri runtime (no invoke happens until effect).
  const { unmount } = render(<App />);
  expect(document.querySelector("main")).not.toBeNull();
  unmount();
});
