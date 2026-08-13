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
}

#[derive(Clone, Copy, Debug, Deserialize)]
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
}

#[derive(Clone, Debug, Serialize)]
pub struct ProjectPathView {
    pub id: String,
    pub kind: String,
    pub canonical_path: String,
}

pub(crate) fn list_projects_impl(state: &CoreState) -> Result<Vec<ProjectView>, AppError> {
    let projects = state.config.list_projects()?;
    let mut views = Vec::with_capacity(projects.len());
    for p in projects {
        let paths = state.config.list_project_paths(p.id)?;
        views.push(ProjectView {
            id: p.id.to_string(),
            name: p.name,
            canonical_root: p.canonical_root.to_string_lossy().into_owned(),
            worktree_mode: worktree_mode_code(p.worktree_mode),
            paths: paths
                .into_iter()
                .map(|pp| ProjectPathView {
                    id: pp.id.to_string(),
                    kind: path_kind_code(pp.kind),
                    canonical_path: pp.canonical_path.to_string_lossy().into_owned(),
                })
                .collect(),
        });
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
    let canonical = canonicalize_root(&input.canonical_root)?;
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
    state.config.save_project(&record)?;
    let paths = state.config.list_project_paths(id)?;
    Ok(ProjectView {
        id: record.id.to_string(),
        name: record.name,
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
    })
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
    state.config.save_project_path(&record)
}

pub(crate) fn remove_project_alias_impl(
    state: &CoreState,
    input: RemoveProjectAliasInput,
) -> Result<(), AppError> {
    let path_id = parse_uuid_input(&input.path_id)?;
    state.config.delete_project_path(path_id)
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
