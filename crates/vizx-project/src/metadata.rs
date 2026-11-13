use serde::{Deserialize, Serialize};

/// Descriptive information about a project. None of it affects rendering.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectMetadata {
    /// Human-readable project title shown in the UI.
    pub name: String,
    pub author: String,
    pub description: String,
    /// VizX version that created the project, e.g. `"0.1.0"`.
    pub created_with_version: String,
}

impl ProjectMetadata {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            author: String::new(),
            description: String::new(),
            created_with_version: env!("CARGO_PKG_VERSION").to_owned(),
        }
    }
}
