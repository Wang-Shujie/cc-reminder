use std::io;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::model::ProjectId;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PathPlatform {
    Unix,
    Windows,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ProjectRegistration {
    pub id: ProjectId,
    pub display_name: String,
    pub canonical_root: PathBuf,
    pub aliases: Vec<PathBuf>,
}

impl ProjectRegistration {
    pub fn new(
        id: ProjectId,
        display_name: String,
        root: PathBuf,
        aliases: Vec<PathBuf>,
    ) -> io::Result<Self> {
        Ok(Self {
            id,
            display_name,
            canonical_root: root.canonicalize()?,
            aliases: aliases
                .into_iter()
                .map(|alias| alias.canonicalize())
                .collect::<io::Result<_>>()?,
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ProjectMatch {
    Matched {
        project_id: ProjectId,
        display_name: String,
    },
    Unmatched,
}

pub fn resolve_project(
    cwd: &Path,
    projects: &[ProjectRegistration],
    platform: PathPlatform,
) -> ProjectMatch {
    let cwd = normalized(cwd, platform);
    projects
        .iter()
        .flat_map(|project| {
            std::iter::once(&project.canonical_root)
                .chain(project.aliases.iter())
                .map(move |root| (project, normalized(root, platform)))
        })
        .filter(|(_, root)| is_prefix(root, &cwd))
        .max_by_key(|(_, root)| root.segments.len())
        .map(|(project, _)| ProjectMatch::Matched {
            project_id: project.id,
            display_name: project.display_name.clone(),
        })
        .unwrap_or(ProjectMatch::Unmatched)
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct NormalizedPath {
    anchor: PathAnchor,
    segments: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum PathAnchor {
    Relative,
    UnixRoot,
    WindowsRoot,
    WindowsDrive { drive: String, absolute: bool },
    WindowsUnc { server: String, share: String },
}

pub(crate) fn path_leaf(path: &Path, platform: PathPlatform) -> Option<String> {
    match platform {
        PathPlatform::Unix => normalized(path, platform).segments.last().cloned(),
        PathPlatform::Windows => normalized_windows(path, false).segments.last().cloned(),
    }
}

fn normalized(path: &Path, platform: PathPlatform) -> NormalizedPath {
    match platform {
        PathPlatform::Unix => {
            let path = path.to_string_lossy();
            let anchor = if path.starts_with('/') {
                PathAnchor::UnixRoot
            } else {
                PathAnchor::Relative
            };
            NormalizedPath {
                segments: normalized_segments(&path, &anchor),
                anchor,
            }
        }
        PathPlatform::Windows => normalized_windows(path, true),
    }
}

fn normalized_windows(path: &Path, fold_case: bool) -> NormalizedPath {
    let path = path.to_string_lossy().replace('\\', "/");
    let mut path = if fold_case { path.to_lowercase() } else { path };
    if let Some(rest) = path.strip_prefix("//?/unc/") {
        path = format!("//{rest}");
    } else if let Some(rest) = path.strip_prefix("//?/") {
        path = rest.to_owned();
    }

    let (anchor, remainder) = if let Some(rest) = path.strip_prefix("//") {
        let mut components = rest.split('/').filter(|segment| !segment.is_empty());
        let server = components.next().unwrap_or_default().to_owned();
        let share = components.next().unwrap_or_default().to_owned();
        let consumed = server.len() + share.len() + usize::from(!server.is_empty()) + 2;
        (
            PathAnchor::WindowsUnc { server, share },
            path.get(consumed..).unwrap_or_default(),
        )
    } else if path.as_bytes().get(1) == Some(&b':') {
        let drive = path[..2].to_owned();
        let remainder = &path[2..];
        let absolute = remainder.starts_with('/');
        (
            PathAnchor::WindowsDrive { drive, absolute },
            remainder.trim_start_matches('/'),
        )
    } else if let Some(rest) = path.strip_prefix('/') {
        (PathAnchor::WindowsRoot, rest)
    } else {
        (PathAnchor::Relative, path.as_str())
    };

    NormalizedPath {
        segments: normalized_segments(remainder, &anchor),
        anchor,
    }
}

fn normalized_segments(path: &str, anchor: &PathAnchor) -> Vec<String> {
    let mut segments = Vec::new();
    for segment in path.split('/') {
        match segment {
            "" | "." => {}
            ".." if segments.last().is_some_and(|last| last != "..") => {
                segments.pop();
            }
            ".." if matches!(
                anchor,
                PathAnchor::Relative
                    | PathAnchor::WindowsDrive {
                        absolute: false,
                        ..
                    }
            ) =>
            {
                segments.push(segment.to_owned())
            }
            ".." => {}
            _ => segments.push(segment.to_owned()),
        }
    }
    segments
}

fn is_prefix(root: &NormalizedPath, path: &NormalizedPath) -> bool {
    root.anchor == path.anchor
        && root.segments.len() <= path.segments.len()
        && root
            .segments
            .iter()
            .zip(&path.segments)
            .all(|(left, right)| left == right)
}

#[cfg(test)]
mod tests {
    #[cfg(unix)]
    use std::fs;
    use std::path::Path;

    use uuid::Uuid;

    use super::{PathPlatform, ProjectMatch, ProjectRegistration, resolve_project};

    #[test]
    fn unix_paths_normalize_separators_and_dot_segments() {
        let project = registration("/repos/app", &[]);

        assert_match(
            resolve_project(
                Path::new("/repos/app/./src"),
                std::slice::from_ref(&project),
                PathPlatform::Unix,
            ),
            project.id,
        );
    }

    #[test]
    fn windows_paths_fold_drive_and_case() {
        let project = registration("C:\\Repos\\App", &[]);

        assert_match(
            resolve_project(
                Path::new("c:\\repos\\app\\src"),
                std::slice::from_ref(&project),
                PathPlatform::Windows,
            ),
            project.id,
        );
    }

    #[test]
    fn longest_whole_segment_prefix_wins() {
        let repo = registration("/repo", &[]);
        let repo_one = registration("/repo-one", &[]);

        assert_match(
            resolve_project(
                Path::new("/repo-one/src"),
                &[repo.clone(), repo_one.clone()],
                PathPlatform::Unix,
            ),
            repo_one.id,
        );
        assert!(matches!(
            resolve_project(
                Path::new("/repository/src"),
                &[repo, repo_one],
                PathPlatform::Unix,
            ),
            ProjectMatch::Unmatched
        ));
    }

    #[cfg(unix)]
    #[test]
    fn registered_symlink_root_is_canonicalized() {
        let temporary =
            std::env::temp_dir().join(format!("cc-reminder-resolver-{}", Uuid::now_v7()));
        let root = temporary.join("root");
        let link = temporary.join("link");
        fs::create_dir_all(&temporary).unwrap();
        fs::create_dir(&root).unwrap();
        std::os::unix::fs::symlink(&root, &link).unwrap();
        let project =
            ProjectRegistration::new(Uuid::now_v7(), "app".into(), link, Vec::new()).unwrap();

        assert_eq!(project.canonical_root, fs::canonicalize(&root).unwrap());
        assert_match(
            resolve_project(
                &project.canonical_root.join("src"),
                std::slice::from_ref(&project),
                PathPlatform::Unix,
            ),
            project.id,
        );
        fs::remove_dir_all(temporary).unwrap();
    }

    #[test]
    fn worktree_alias_resolves_to_parent_project() {
        let project = registration("/repos/app", &["/repos/app-worktree"]);

        assert_match(
            resolve_project(
                Path::new("/repos/app-worktree/src"),
                std::slice::from_ref(&project),
                PathPlatform::Unix,
            ),
            project.id,
        );
    }

    #[test]
    fn parent_segments_cannot_escape_into_a_registered_root() {
        let project = registration("/repo/app", &[]);

        assert!(matches!(
            resolve_project(
                Path::new("/repo/app/../secret"),
                &[project],
                PathPlatform::Unix,
            ),
            ProjectMatch::Unmatched
        ));
    }

    #[test]
    fn absolute_and_relative_paths_do_not_share_an_anchor() {
        let project = registration("/repo/app", &[]);

        assert!(matches!(
            resolve_project(Path::new("repo/app/src"), &[project], PathPlatform::Unix),
            ProjectMatch::Unmatched
        ));
    }

    #[test]
    fn windows_verbatim_drive_matches_an_ordinary_drive_path() {
        let project = registration(r"\\?\C:\Repos\App", &[]);

        assert_match(
            resolve_project(
                Path::new(r"c:\repos\app\src"),
                std::slice::from_ref(&project),
                PathPlatform::Windows,
            ),
            project.id,
        );
    }

    #[test]
    fn windows_verbatim_unc_matches_an_ordinary_unc_path() {
        let project = registration(r"\\?\UNC\server\share\app", &[]);

        assert_match(
            resolve_project(
                Path::new(r"\\server\share\app\src"),
                std::slice::from_ref(&project),
                PathPlatform::Windows,
            ),
            project.id,
        );
    }

    #[cfg(windows)]
    #[test]
    fn registered_windows_root_matches_non_verbatim_hook_path() {
        let root = std::env::temp_dir().join(format!("cc-reminder-resolver-{}", Uuid::now_v7()));
        std::fs::create_dir_all(&root).unwrap();
        let project =
            ProjectRegistration::new(Uuid::now_v7(), "app".into(), root.clone(), Vec::new())
                .unwrap();

        assert_match(
            resolve_project(
                &root.join("src"),
                std::slice::from_ref(&project),
                PathPlatform::Windows,
            ),
            project.id,
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    fn registration(root: &str, aliases: &[&str]) -> ProjectRegistration {
        ProjectRegistration {
            id: Uuid::now_v7(),
            display_name: "app".into(),
            canonical_root: root.into(),
            aliases: aliases.iter().map(Into::into).collect(),
        }
    }

    fn assert_match(result: ProjectMatch, id: Uuid) {
        assert!(matches!(result, ProjectMatch::Matched { project_id, .. } if project_id == id));
    }
}
