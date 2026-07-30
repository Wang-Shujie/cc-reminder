pub mod resolver;

pub(crate) use resolver::path_leaf;
pub use resolver::{PathPlatform, ProjectMatch, ProjectRegistration, resolve_project};
