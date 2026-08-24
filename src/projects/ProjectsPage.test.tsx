// Task 18 contract tests for the Projects page. The plan's Step 3 block is
// authoritative: adding a project is only possible through the user-selected
// directory from an injected folder picker, with the alias/separate choice.
import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";

import {
  PROJECT_ID,
  claudeRulesFixtures,
  codexRulesFixtures,
  configuredBackend,
  type FakeBackend,
} from "../test/TestApp";
import { ProjectsPage } from "./ProjectsPage";

import type { ProjectId, ProjectSummary } from "../lib/contracts";

function projectsBackend(options?: Parameters<typeof configuredBackend>[0]): FakeBackend {
  const project: ProjectSummary = {
    id: PROJECT_ID as ProjectId,
    name: "主仓库",
    canonical_root: "/work/main",
    worktree_mode: "alias",
    paths: [
      { id: "path-root", kind: "root", canonical_path: "/work/main" },
      { id: "path-9", kind: "alias", canonical_path: "/work/main-wt" },
    ],
    override_count: 1,
  };
  return configuredBackend({ projects: [project], ...options });
}

function dialogReturning(path: string | null): () => Promise<string | null> {
  return async () => path;
}

test("adds only a user-selected directory and chooses worktree behavior", async () => {
  const backend = projectsBackend();
  const user = userEvent.setup();
  render(<ProjectsPage backend={backend} dialog={dialogReturning("/work/client")} />);
  await user.click(screen.getByRole("button", { name: "添加项目" }));
  await user.click(screen.getByLabelText("作为现有项目的路径别名"));
  await user.click(screen.getByRole("button", { name: "保存" }));
  expect(backend.saveProject).toHaveBeenCalledWith(
    expect.objectContaining({ selected_path: "/work/client" }),
  );
});

test("alias is the default worktree choice; separate is explicit", async () => {
  const backend = projectsBackend();
  const user = userEvent.setup();
  render(<ProjectsPage backend={backend} dialog={dialogReturning("/work/client")} />);
  await user.click(screen.getByRole("button", { name: "添加项目" }));

  // Default: worktree joins the existing project as a path alias.
  expect(screen.getByLabelText("作为现有项目的路径别名")).toBeChecked();

  await user.click(screen.getByLabelText("作为独立项目添加"));
  await user.click(screen.getByRole("button", { name: "保存" }));
  await waitFor(() =>
    expect(backend.saveProject).toHaveBeenCalledWith(
      expect.objectContaining({
        selected_path: "/work/client",
        worktree_mode: "separate",
      }),
    ),
  );
});

test("cancelling the folder picker opens nothing", async () => {
  const backend = projectsBackend();
  const user = userEvent.setup();
  render(<ProjectsPage backend={backend} dialog={dialogReturning(null)} />);
  await user.click(screen.getByRole("button", { name: "添加项目" }));
  expect(screen.queryByLabelText("作为现有项目的路径别名")).not.toBeInTheDocument();
  expect(backend.saveProject).not.toHaveBeenCalled();
});

test("duplicate or overlapping paths surface the backend conflict error", async () => {
  const backend = projectsBackend({ projectConflict: true });
  const user = userEvent.setup();
  render(<ProjectsPage backend={backend} dialog={dialogReturning("/work/main")} />);
  await user.click(screen.getByRole("button", { name: "添加项目" }));
  await user.click(screen.getByRole("button", { name: "保存" }));
  expect(await screen.findByRole("alert")).toHaveTextContent("project path is already registered");
});

test("canonical root and aliases are listed; alias removal confirms first", async () => {
  const user = userEvent.setup();
  const backend = projectsBackend();
  render(<ProjectsPage backend={backend} />);
  // Canonical root display.
  expect(await screen.findByText("/work/main")).toBeVisible();
  // Alias row with its removal confirmation targeting the exact path id.
  await user.click(screen.getByRole("button", { name: /移除别名/ }));
  expect(screen.getByRole("dialog")).toBeVisible();
  await user.click(screen.getByRole("button", { name: "确认移除" }));
  await waitFor(() =>
    expect(backend.removeProjectAlias).toHaveBeenCalledWith({ path_id: "path-9" }),
  );
});

test("override counts follow the selected agent", async () => {
  const user = userEvent.setup();
  // Base rows + one Claude Code override (Stop) via a project patch.
  const backend = projectsBackend({
    rules: [...claudeRulesFixtures(), ...codexRulesFixtures()],
    projectPatches: { [`${PROJECT_ID}:claude-code:Stop`]: { enabled: true } },
  });
  render(<ProjectsPage backend={backend} />);

  const counts = await screen.findAllByText(/覆盖/);
  expect(counts.length).toBeGreaterThan(0);
  // Default 全部 shows the single Claude Code override…
  expect(screen.getByLabelText("选择 Agent")).toHaveValue("all");
  expect(screen.getByText("1")).toBeVisible();
  // …switching to Codex drops it to zero.
  await user.selectOptions(screen.getByLabelText("选择 Agent"), "codex");
  expect(screen.getByText("0")).toBeVisible();
});

test("no whole-disk scan action exists", async () => {
  render(<ProjectsPage backend={projectsBackend()} />);
  await screen.findByRole("button", { name: "添加项目" });
  expect(screen.queryByRole("button", { name: /扫描|全盘/ })).toBeNull();
  // The page states the boundary explicitly.
  expect(screen.getByText(/不会.*整个磁盘|不扫描整个磁盘/)).toBeVisible();
});
