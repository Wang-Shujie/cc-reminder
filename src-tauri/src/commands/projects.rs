//! Project commands: list/save + alias add/remove.

use chrono::Utc;
use serde::{Deserialize, Serialize};
use tauri::State;
use uuid::Uuid;

use super::{CoreState, configuration_error, parse_uuid_input};
use crate::error::AppError;
use crate::model::{ProjectPathKind, ProjectPathRecord, ProjectRecord, WorktreeMode};

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SaveProjectInput {
    pub project_id: Option<String>,
    pub name: String,
    pub canonical_root: String,
    pub worktree_mode: WorktreeModeInput,
    /// The user-selected directory from the native folder picker (Task 18).
    /// When present it wins over `canonical_root` for canonicalization and
    /// worktree detection; the frontend never decides canonical paths.
    pub selected_path: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorktreeModeInput {
    Alias,
    Separate,
}
impl WorktreeModeInput {
    fn into_mode(self) -> WorktreeMode {
        match self {
            Self::Alias => WorktreeMode::Alias,
            Self::Separate => WorktreeMode::Separate,
        }
    }
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AddProjectAliasInput {
    pub project_id: String,
    pub canonical_path: String,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RemoveProjectAliasInput {
    pub path_id: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct ProjectView {
    pub id: String,
    pub name: String,
    pub canonical_root: String,
    pub worktree_mode: String,
    pub paths: Vec<ProjectPathView>,
    /// Git root found by inspecting ONLY the selected directory and its
    /// ancestors; `None` when the selected directory is its own root or no
    /// `.git` was found nearby.
    pub git_root: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
pub struct ProjectPathView {
    pub id: String,
    pub kind: String,
    pub canonical_path: String,
}

pub(crate) fn list_projects_impl(state: &CoreState) -> Result<Vec<ProjectView>, AppError> {
    let projects = state.storage.config.list_projects()?;
    let mut views = Vec::with_capacity(projects.len());
    for p in projects {
        views.push(project_view(state, &p, None)?);
    }
    Ok(views)
}

pub(crate) fn save_project_impl(
    state: &CoreState,
    input: SaveProjectInput,
) -> Result<ProjectView, AppError> {
    if input.name.trim().is_empty() {
        return Err(configuration_error(
            "project_invalid",
            "project name is empty",
        ));
    }
    // The user-selected directory (when supplied) is what the core
    // canonicalizes; it never trusts the frontend's path bookkeeping.
    let source = input
        .selected_path
        .as_deref()
        .filter(|s| !s.is_empty())
        .unwrap_or(&input.canonical_root);
    let canonical = canonicalize_root(source)?;
    let git_root = probe_git_root(&canonical);

    // Worktree-as-alias (default): when the selected directory lives inside a
    // registered repository, register it as a worktree path of THAT project
    // instead of creating an independent registration.
    if input.worktree_mode == WorktreeModeInput::Alias
        && let Some(root) = &git_root
        && root != &canonical
        && let Some(owner) = find_project_owning_root(state, root)?
    {
        state.storage.config.save_project_path(&ProjectPathRecord {
            id: Uuid::now_v7(),
            project_id: owner.id,
            canonical_path: canonical.clone(),
            kind: ProjectPathKind::Worktree,
        })?;
        return project_view(state, &owner, Some(root));
    }

    let id = match input.project_id.as_deref() {
        Some(id) => parse_uuid_input(id)?,
        None => Uuid::now_v7(),
    };
    let now = Utc::now();
    let record = ProjectRecord {
        id,
        name: input.name,
        canonical_root: canonical,
        worktree_mode: input.worktree_mode.into_mode(),
        created_at: now,
        updated_at: now,
    };
    state.storage.config.save_project(&record)?;
    project_view(state, &record, git_root.as_deref())
}

pub(crate) fn add_project_alias_impl(
    state: &CoreState,
    input: AddProjectAliasInput,
) -> Result<(), AppError> {
    let project_id = parse_uuid_input(&input.project_id)?;
    let canonical = canonicalize_root(&input.canonical_path)?;
    let record = ProjectPathRecord {
        id: Uuid::now_v7(),
        project_id,
        canonical_path: canonical,
        kind: ProjectPathKind::Alias,
    };
    state.storage.config.save_project_path(&record)
}

pub(crate) fn remove_project_alias_impl(
    state: &CoreState,
    input: RemoveProjectAliasInput,
) -> Result<(), AppError> {
    let path_id = parse_uuid_input(&input.path_id)?;
    state.storage.config.delete_project_path(path_id)
}

/// Canonicalize a project root/path input in the core, never trusting the
/// frontend-supplied string verbatim. We do not require the path to exist
/// (projects may be registered ahead of `git clone`), but we normalize it.
fn canonicalize_root(input: &str) -> Result<std::path::PathBuf, AppError> {
    let path = std::path::PathBuf::from(input);
    if path.as_os_str().is_empty() {
        return Err(configuration_error("path_invalid", "project path is empty"));
    }
    match std::fs::canonicalize(&path) {
        Ok(canon) => Ok(canon),
        Err(_) => {
            // ponytail: fs::canonicalize requires the path to exist; we accept
            // a non-existent but lexically-normalized path so users can
            // pre-register a worktree. The repo layer enforces size limits.
            let normalized = normalize_lexical(&path);
            Ok(normalized)
        }
    }
}

fn normalize_lexical(path: &std::path::Path) -> std::path::PathBuf {
    let mut out = std::path::PathBuf::new();
    for component in path.components() {
        match component {
            std::path::Component::ParentDir => {
                out.pop();
            }
            std::path::Component::CurDir => {}
            other => out.push(other.as_os_str()),
        }
    }
    out
}

fn worktree_mode_code(mode: WorktreeMode) -> String {
    match mode {
        WorktreeMode::Alias => "alias".into(),
        WorktreeMode::Separate => "separate".into(),
    }
}

fn path_kind_code(kind: ProjectPathKind) -> String {
    match kind {
        ProjectPathKind::Root => "root".into(),
        ProjectPathKind::Alias => "alias".into(),
        ProjectPathKind::Worktree => "worktree".into(),
    }
}

/// Walk from `start` upward looking for a `.git` entry, inspecting ONLY the
/// selected directory and its ancestors — never siblings or unrelated
/// directories. ponytail: nesting deeper than 32 levels silently degrades to
/// "no git root found" (independent project); raise the bound if that ever
/// bites a real tree. A real linked worktree has `.git` as a FILE pointing at
/// `<owner>/.git/worktrees/<name>`; such gitfiles resolve to the owning
/// repository's working tree so alias-mode saves can join it.
fn probe_git_root(start: &std::path::Path) -> Option<std::path::PathBuf> {
    let mut current = Some(start.to_path_buf());
    for _ in 0..32 {
        let dir = current?;
        let dot_git = dir.join(".git");
        if dot_git.exists() {
            if dot_git.is_file()
                && let Some(owner) = resolve_worktree_gitfile(&dir, &dot_git)
            {
                return Some(owner);
            }
            return Some(dir);
        }
        current = dir.parent().map(std::path::Path::to_path_buf);
    }
    None
}

/// Parse a `.git` gitfile of the form `gitdir: <path>` written by real
/// `git worktree` checkouts. When the target contains `/.git/worktrees/`,
/// everything from that marker onward is stripped: the remainder is the
/// owning repository's working tree, canonicalized (or lexically normalized
/// when it no longer exists). Anything else — submodule-style pointers,
/// unreadable files — yields `None` so the caller treats `dir` as its own
/// root, unchanged.
fn resolve_worktree_gitfile(
    dir: &std::path::Path,
    gitfile: &std::path::Path,
) -> Option<std::path::PathBuf> {
    let contents = std::fs::read_to_string(gitfile).ok()?;
    let pointer = contents.strip_prefix("gitdir: ")?.trim_end();
    let gitdir = std::path::PathBuf::from(pointer);
    let gitdir = if gitdir.is_absolute() {
        gitdir
    } else {
        dir.join(gitdir)
    };
    let text = gitdir.to_string_lossy();
    let marker = "/.git/worktrees/";
    let index = text.find(marker)?;
    let owner = std::path::PathBuf::from(&text[..index]);
    if owner.as_os_str().is_empty() {
        return None;
    }
    match std::fs::canonicalize(&owner) {
        Ok(canon) => Some(canon),
        Err(_) => Some(normalize_lexical(&owner)),
    }
}

/// Find the registered project that owns `root` as its canonical root (or as
/// one of its registered paths).
fn find_project_owning_root(
    state: &CoreState,
    root: &std::path::Path,
) -> Result<Option<ProjectRecord>, AppError> {
    for project in state.storage.config.list_projects()? {
        if project.canonical_root == root {
            return Ok(Some(project));
        }
        for path in state.storage.config.list_project_paths(project.id)? {
            if path.canonical_path == root {
                return Ok(Some(project));
            }
        }
    }
    Ok(None)
}

fn project_view(
    state: &CoreState,
    record: &ProjectRecord,
    git_root: Option<&std::path::Path>,
) -> Result<ProjectView, AppError> {
    let paths = state.storage.config.list_project_paths(record.id)?;
    Ok(ProjectView {
        id: record.id.to_string(),
        name: record.name.clone(),
        canonical_root: record.canonical_root.to_string_lossy().into_owned(),
        worktree_mode: worktree_mode_code(record.worktree_mode),
        paths: paths
            .into_iter()
            .map(|pp| ProjectPathView {
                id: pp.id.to_string(),
                kind: path_kind_code(pp.kind),
                canonical_path: pp.canonical_path.to_string_lossy().into_owned(),
            })
            .collect(),
        git_root: git_root.map(|p| p.to_string_lossy().into_owned()),
    })
}

#[tauri::command]
pub async fn list_projects(state: State<'_, CoreState>) -> Result<Vec<ProjectView>, AppError> {
    let state = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || list_projects_impl(&state))
        .await
        .map_err(|_| configuration_error("join_failed", "command join failed"))?
}

#[tauri::command]
pub async fn save_project(
    state: State<'_, CoreState>,
    input: SaveProjectInput,
) -> Result<ProjectView, AppError> {
    let state = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || save_project_impl(&state, input))
        .await
        .map_err(|_| configuration_error("join_failed", "command join failed"))?
}

#[tauri::command]
pub async fn add_project_alias(
    state: State<'_, CoreState>,
    input: AddProjectAliasInput,
) -> Result<(), AppError> {
    let state = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || add_project_alias_impl(&state, input))
        .await
        .map_err(|_| configuration_error("join_failed", "command join failed"))?
}

#[tauri::command]
pub async fn remove_project_alias(
    state: State<'_, CoreState>,
    input: RemoveProjectAliasInput,
) -> Result<(), AppError> {
    let state = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || remove_project_alias_impl(&state, input))
        .await
        .map_err(|_| configuration_error("join_failed", "command join failed"))?
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::CoreState;
    use crate::security::credentials::CredentialStore;
    use crate::security::crypto::FieldCipher;
    use crate::storage::config::ConfigRepository;
    use crate::storage::db::Database;
    use crate::storage::events::EventRepository;
    use crate::storage::integrations::IntegrationRepository;
    use crate::storage::queue::QueueRepository;

    fn command_state(database_path: &std::path::Path) -> CoreState {
        let database = Database::open(database_path).unwrap();
        let config = ConfigRepository::new(database.clone());
        let events = EventRepository::new(database.clone());
        let queue = QueueRepository::new(database.clone());
        let integrations = IntegrationRepository::new(database.clone());
        let credentials = CredentialStore::memory_for_test();
        let cipher = std::sync::Arc::new(FieldCipher::from_key([7u8; 32]));
        let logs_dir = database_path.parent().unwrap().join("logs");
        let diagnostics = std::sync::Arc::new(crate::diagnostics::Diagnostics::test(
            &logs_dir,
            1024 * 1024,
            3,
        ));
        CoreState::new(
            config,
            events,
            queue,
            integrations,
            credentials,
            cipher,
            diagnostics,
        )
    }

    /// Temp project tree: `<root>/repo` (git root) containing `wt`.
    /// ponytail: leak the TempDir so its on-disk DB outlives the helper —
    /// same rationale as the channels command tests.
    fn worktree_fixture() -> (std::path::PathBuf, std::path::PathBuf, CoreState) {
        let root = tempfile::tempdir().unwrap();
        let repo = root.path().join("repo");
        std::fs::create_dir_all(repo.join(".git")).unwrap();
        std::fs::create_dir_all(repo.join("wt")).unwrap();
        // The DB lives OUTSIDE the repo so probe_git_root never sees it, and
        // under the app directory name the storage layer requires.
        let db_dir = root.path().join("com.ccreminder.app");
        std::fs::create_dir_all(&db_dir).unwrap();
        let state = command_state(&db_dir.join("cc-reminder.sqlite3"));
        let repo_path = repo.canonicalize().unwrap();
        let wt_path = repo.join("wt").canonicalize().unwrap();
        std::mem::forget(root);
        (repo_path, wt_path, state)
    }

    #[test]
    fn probe_git_root_inspects_only_the_selected_chain() {
        let (repo, wt, _state) = worktree_fixture();
        assert_eq!(probe_git_root(&wt).as_deref(), Some(repo.as_path()));
        assert_eq!(probe_git_root(&repo).as_deref(), Some(repo.as_path()));
    }

    #[test]
    fn selected_worktree_with_alias_mode_joins_the_existing_project() {
        let (repo, wt, state) = worktree_fixture();
        save_project_impl(
            &state,
            SaveProjectInput {
                project_id: None,
                name: "主仓库".into(),
                canonical_root: repo.to_string_lossy().into_owned(),
                worktree_mode: WorktreeModeInput::Separate,
                selected_path: None,
            },
        )
        .unwrap();

        let view = save_project_impl(
            &state,
            SaveProjectInput {
                project_id: None,
                name: "wt".into(),
                canonical_root: wt.to_string_lossy().into_owned(),
                worktree_mode: WorktreeModeInput::Alias,
                selected_path: Some(wt.to_string_lossy().into_owned()),
            },
        )
        .unwrap();

        assert_eq!(view.name, "主仓库");
        assert_eq!(view.git_root.as_deref(), Some(repo.to_str().unwrap()));
        assert!(
            view.paths
                .iter()
                .any(|p| p.kind == "worktree" && p.canonical_path == wt.to_string_lossy())
        );
    }

    /// REAL linked-worktree shape: `<root>/repo` keeps `.git` as a DIRECTORY
    /// while the sibling checkout `<root>/wt` has `.git` as a regular FILE
    /// containing `gitdir: <repo>/.git/worktrees/wt`.
    fn real_worktree_fixture() -> (std::path::PathBuf, std::path::PathBuf, CoreState) {
        let root = tempfile::tempdir().unwrap();
        let repo = root.path().join("repo");
        std::fs::create_dir_all(repo.join(".git")).unwrap();
        let wt = root.path().join("wt");
        std::fs::create_dir_all(&wt).unwrap();
        std::fs::write(
            wt.join(".git"),
            format!("gitdir: {}\n", repo.join(".git/worktrees/wt").display()),
        )
        .unwrap();
        // The DB lives OUTSIDE both trees so probe_git_root never sees it.
        let db_dir = root.path().join("com.ccreminder.app");
        std::fs::create_dir_all(&db_dir).unwrap();
        let state = command_state(&db_dir.join("cc-reminder.sqlite3"));
        let repo_path = repo.canonicalize().unwrap();
        let wt_path = wt.canonicalize().unwrap();
        std::mem::forget(root);
        (repo_path, wt_path, state)
    }

    #[test]
    fn real_worktree_gitfile_resolves_to_the_owning_repo() {
        let (repo, wt, _state) = real_worktree_fixture();
        assert_eq!(probe_git_root(&wt).as_deref(), Some(repo.as_path()));
    }

    #[test]
    fn real_linked_worktree_with_alias_mode_joins_the_existing_project() {
        let (repo, wt, state) = real_worktree_fixture();
        save_project_impl(
            &state,
            SaveProjectInput {
                project_id: None,
                name: "主仓库".into(),
                canonical_root: repo.to_string_lossy().into_owned(),
                worktree_mode: WorktreeModeInput::Separate,
                selected_path: None,
            },
        )
        .unwrap();

        let view = save_project_impl(
            &state,
            SaveProjectInput {
                project_id: None,
                name: "wt".into(),
                canonical_root: wt.to_string_lossy().into_owned(),
                worktree_mode: WorktreeModeInput::Alias,
                selected_path: Some(wt.to_string_lossy().into_owned()),
            },
        )
        .unwrap();

        assert_eq!(view.name, "主仓库");
        assert_eq!(view.git_root.as_deref(), Some(repo.to_str().unwrap()));
        assert!(
            view.paths
                .iter()
                .any(|p| p.kind == "worktree" && p.canonical_path == wt.to_string_lossy())
        );
    }

    #[test]
    fn separate_mode_creates_an_independent_project_at_the_selected_path() {
        let (repo, wt, state) = worktree_fixture();
        save_project_impl(
            &state,
            SaveProjectInput {
                project_id: None,
                name: "主仓库".into(),
                canonical_root: repo.to_string_lossy().into_owned(),
                worktree_mode: WorktreeModeInput::Separate,
                selected_path: None,
            },
        )
        .unwrap();

        let view = save_project_impl(
            &state,
            SaveProjectInput {
                project_id: None,
                name: "独立".into(),
                canonical_root: wt.to_string_lossy().into_owned(),
                worktree_mode: WorktreeModeInput::Separate,
                selected_path: Some(wt.to_string_lossy().into_owned()),
            },
        )
        .unwrap();

        assert_eq!(view.name, "独立");
        assert_eq!(view.canonical_root, wt.to_string_lossy().into_owned());
        assert_eq!(view.git_root.as_deref(), Some(repo.to_str().unwrap()));
    }
}
