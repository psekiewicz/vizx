use serde::{Deserialize, Serialize};

use crate::{CURRENT_FORMAT_VERSION, ProjectMetadata, ProjectSettings};

/// Root of all data that belongs to one VizX project.
///
/// Everything that is saved into a `.vzx` file hangs off this struct. Future
/// milestones add media sources, timeline tracks, clips, keyframes and
/// effects here, so that every subsystem works on the same shared data.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Project {
    /// Data layout version this project was written with.
    pub format_version: u32,
    pub metadata: ProjectMetadata,
    pub settings: ProjectSettings,
}

impl Project {
    /// Creates an empty project with default settings.
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            format_version: CURRENT_FORMAT_VERSION,
            metadata: ProjectMetadata::new(name),
            settings: ProjectSettings::default(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::FrameRate;

    #[test]
    fn new_project_uses_current_format_and_defaults() {
        let project = Project::new("Demo");

        assert_eq!(project.format_version, CURRENT_FORMAT_VERSION);
        assert_eq!(project.metadata.name, "Demo");
        assert_eq!(project.settings, ProjectSettings::default());
    }

    #[test]
    fn project_survives_a_serialization_round_trip() {
        let mut project = Project::new("Round trip");
        project.metadata.author = "Tester".to_owned();
        project.settings.frame_rate = FrameRate {
            numerator: 30000,
            denominator: 1001,
        };

        let encoded = serde_json::to_string(&project).expect("project should serialize");
        let decoded: Project = serde_json::from_str(&encoded).expect("project should deserialize");

        assert_eq!(decoded, project);
    }

    #[test]
    fn frame_rate_fraction_converts_to_expected_value() {
        let ntsc = FrameRate {
            numerator: 30000,
            denominator: 1001,
        };

        assert!((ntsc.as_f64() - 29.97).abs() < 0.001);
        assert_eq!(FrameRate::FPS_60.as_f64(), 60.0);
    }
}
