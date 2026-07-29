import { render, screen } from "@testing-library/react";
import App from "./App";

test("renders the product shell in Chinese", () => {
  render(<App />);
  expect(screen.getByRole("application", { name: "CC Reminder" })).toBeVisible();
  expect(screen.getByRole("heading", { name: "Hook 规则" })).toBeVisible();
});
