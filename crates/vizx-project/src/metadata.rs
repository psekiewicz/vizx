use serde::{Deserialize, Serialize};

/// Descriptive information about a project. None of it affects rendering.
///
/// There is deliberately no "name" here. A project is named by its file,
/// as documents are in other editors: a second name stored inside the file
/// could disagree with the file name, and then nobody would know which one
/// is the project's name.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectMetadata {
    pub author: String,
    pub description: String,
    /// VizX version that created the project, e.g. `"0.1.0"`.
    pub created_with_version: String,
}

impl Default for ProjectMetadata {
    fn default() -> Self {
        Self {
            author: String::new(),
            description: String::new(),
            created_with_version: env!("CARGO_PKG_VERSION").to_owned(),
        }
    }
}
