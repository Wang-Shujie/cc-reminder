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
        .max_by_key(|(_, root)| root.len())
        .map(|(project, _)| ProjectMatch::Matched {
            project_id: project.id,
            display_name: project.display_name.clone(),
        })
        .unwrap_or(ProjectMatch::Unmatched)
}

fn normalized(path: &Path, platform: PathPlatform) -> Vec<String> {
    let path = path.to_string_lossy();
    let path = match platform {
        PathPlatform::Unix => path.into_owned(),
        PathPlatform::Windows => path.replace('\\', "/").to_ascii_lowercase(),
    };
    path.split('/')
        .filter(|segment| !segment.is_empty() && *segment != ".")
        .map(str::to_owned)
        .collect()
}

fn is_prefix(root: &[String], path: &[String]) -> bool {
    root.len() <= path.len() && root.iter().zip(path).all(|(left, right)| left == right)
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
