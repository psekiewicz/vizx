//! "Collect All and Save": planning a copy of a project together with every
//! file it uses, in one folder that can be moved, backed up or sent to
//! another computer as a whole.
//!
//! ```text
//! My Song\
//!     My Song.vzx
//!     Media\
//!         song.mp3
//!         cover.png
//!         walk\walk_0001.png ...      (an image sequence keeps its frames together)
//!         ring\visualizer.json ...    (a visualizer plugin keeps its folder)
//! ```
//!
//! This module only decides what goes where; copying the files is left to
//! the caller, so it can show progress and be stopped.

use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

use crate::file::is_file_reference;
use crate::{FrameLayout, MediaContent, Project, sequence_frame_path, split_numbered_path};

/// Name of the folder, next to the collected project file, that holds its
/// files.
pub const COLLECTED_MEDIA_FOLDER: &str = "Media";

/// One file to copy.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileCopy {
    pub from: PathBuf,
    pub to: PathBuf,
    /// Its size, for progress.
    pub bytes: u64,
}

/// What collecting a project into a folder takes.
#[derive(Debug, Clone, PartialEq)]
pub struct CollectionPlan {
    /// The project as it will be saved: every file it uses points into the
    /// media folder.
    pub project: Project,
    /// Where each file the project used now is (old path, new path), for
    /// pointing the open project at the copies afterwards.
    pub moved_paths: Vec<(String, String)>,
    pub copies: Vec<FileCopy>,
    /// Files the project uses that could not be found; they keep their old
    /// path.
    pub missing: Vec<String>,
}

impl CollectionPlan {
    /// How many bytes all the copies take.
    pub fn total_bytes(&self) -> u64 {
        self.copies.iter().map(|copy| copy.bytes).sum()
    }
}

impl Project {
    /// Plans copying every file this project uses into
    /// `project_folder/Media`. Nothing is written.
    pub fn plan_collection(&self, project_folder: &Path) -> CollectionPlan {
        let mut planner = Planner {
            media_folder: project_folder.join(COLLECTED_MEDIA_FOLDER),
            used_names: HashSet::new(),
            placed: HashMap::new(),
            copies: Vec::new(),
            missing: Vec::new(),
        };
        let mut project = self.clone();
        for media in &mut project.media {
            let new_path = match &media.content {
                MediaContent::VisualizerPlugin => planner.folder(&media.path),
                MediaContent::Animation(animation) => match animation.layout {
                    FrameLayout::Sequence {
                        first_number,
                        digits,
                    } => planner.sequence(&media.path, first_number, digits, animation.frame_count),
                    FrameLayout::SpriteSheet { .. } => planner.file(&media.path),
                },
                MediaContent::Text(text) => {
                    let font = planner.file(&text.font_path);
                    if let (Some(font), MediaContent::Text(text)) = (font, &mut media.content) {
                        text.font_path = font;
                    }
                    None
                }
                MediaContent::Audio(_)
                | MediaContent::MilkdropPreset
                | MediaContent::Image(_)
                | MediaContent::Video(_) => planner.file(&media.path),
                MediaContent::EffectsLayer => None,
            };
            if let Some(new_path) = new_path {
                media.path = new_path;
            }
        }
        // The font of a subtitle layer belongs to the layer.
        for track in project.tracks_mut() {
            if let Some(style) = &mut track.subtitle_style
                && let Some(font) = planner.file(&style.font_path)
            {
                style.font_path = font;
            }
        }
        let mut moved_paths: Vec<(String, String)> = planner.placed.into_iter().collect();
        moved_paths.sort();
        CollectionPlan {
            project,
            moved_paths,
            copies: planner.copies,
            missing: planner.missing,
        }
    }
}

struct Planner {
    media_folder: PathBuf,
    /// Names already taken in the media folder, in lower case (Windows does
    /// not tell `Song.mp3` from `song.mp3`).
    used_names: HashSet<String>,
    /// Each file or folder planned so far: old path to new path. A file
    /// used several times is copied once.
    placed: HashMap<String, String>,
    copies: Vec<FileCopy>,
    missing: Vec<String>,
}

impl Planner {
    /// A single file. `None` if it is not a file (a bundled preset, no font)
    /// or cannot be found.
    fn file(&mut self, path: &str) -> Option<String> {
        if let Some(placed) = self.reuse(path)? {
            return Some(placed);
        }
        let source = Path::new(path);
        let Some(metadata) = fs::metadata(source)
            .ok()
            .filter(|metadata| metadata.is_file())
        else {
            self.note_missing(path);
            return None;
        };
        let name = file_name(source, "file");
        let target = self.free_path(&name);
        self.copies.push(FileCopy {
            from: source.to_owned(),
            to: target.clone(),
            bytes: metadata.len(),
        });
        Some(self.place(path, &target))
    }

    /// A folder with everything in it (a visualizer plugin).
    fn folder(&mut self, path: &str) -> Option<String> {
        if let Some(placed) = self.reuse(path)? {
            return Some(placed);
        }
        let source = Path::new(path);
        if !source.is_dir() {
            self.note_missing(path);
            return None;
        }
        let target = self.free_path(&file_name(source, "plugin"));
        let mut files = Vec::new();
        files_in(source, &mut files);
        for file in files {
            let Ok(relative) = file.strip_prefix(source) else {
                continue;
            };
            let bytes = fs::metadata(&file).map_or(0, |metadata| metadata.len());
            self.copies.push(FileCopy {
                to: target.join(relative),
                from: file,
                bytes,
            });
        }
        Some(self.place(path, &target))
    }

    /// The numbered files of an image sequence, kept together in a folder
    /// of their own with their names unchanged. Frames that are missing
    /// are left out.
    fn sequence(
        &mut self,
        first: &str,
        first_number: u64,
        digits: u32,
        frame_count: u32,
    ) -> Option<String> {
        if let Some(placed) = self.reuse(first)? {
            return Some(placed);
        }
        let first_source = Path::new(first);
        if !first_source.is_file() {
            self.note_missing(first);
            return None;
        }
        let folder_name = split_numbered_path(first)
            .map(|(before, ..)| {
                file_name(Path::new(before), "")
                    .trim_end_matches(['_', '-', '.', ' '])
                    .to_owned()
            })
            .filter(|name| !name.is_empty())
            .unwrap_or_else(|| "sequence".to_owned());
        let target = self.free_path(&folder_name);
        for index in 0..frame_count.max(1) {
            let frame = PathBuf::from(sequence_frame_path(first, first_number, digits, index));
            let Ok(metadata) = fs::metadata(&frame) else {
                continue;
            };
            self.copies.push(FileCopy {
                to: target.join(file_name(&frame, "frame")),
                from: frame,
                bytes: metadata.len(),
            });
        }
        let first_target = target.join(file_name(first_source, "frame"));
        Some(self.place(first, &first_target))
    }

    /// `Some(Some(new path))` if `path` was planned already, `Some(None)` if
    /// it still has to be, `None` if it is not a file at all.
    #[allow(clippy::option_option)]
    fn reuse(&self, path: &str) -> Option<Option<String>> {
        if !is_file_reference(path) {
            return None;
        }
        Some(self.placed.get(path).cloned())
    }

    fn place(&mut self, old: &str, new: &Path) -> String {
        let new = new.to_string_lossy().into_owned();
        self.placed.insert(old.to_owned(), new.clone());
        new
    }

    fn note_missing(&mut self, path: &str) {
        if !self.missing.iter().any(|missing| missing == path) {
            self.missing.push(path.to_owned());
        }
    }

    /// `name` in the media folder, or `name (2)`, `name (3)`, ... (before
    /// the extension) if it is taken.
    fn free_path(&mut self, name: &str) -> PathBuf {
        let (stem, extension) = match name.rfind('.') {
            Some(dot) if dot > 0 => name.split_at(dot),
            _ => (name, ""),
        };
        let mut candidate = name.to_owned();
        let mut number = 2;
        while !self.used_names.insert(candidate.to_lowercase()) {
            candidate = format!("{stem} ({number}){extension}");
            number += 1;
        }
        self.media_folder.join(candidate)
    }
}

fn file_name(path: &Path, fallback: &str) -> String {
    path.file_name().map_or_else(
        || fallback.to_owned(),
        |name| name.to_string_lossy().into_owned(),
    )
}

/// Every file in `folder` and its subfolders.
fn files_in(folder: &Path, files: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(folder) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            files_in(&path, files);
        } else {
            files.push(path);
        }
    }
}

/// A folder named `name` inside `parent` that does not exist yet:
/// `name`, or `name (2)`, `name (3)`, ...
pub fn new_collection_folder(parent: &Path, name: &str) -> PathBuf {
    let mut folder = parent.join(name);
    let mut number = 2;
    while folder.exists() {
        folder = parent.join(format!("{name} ({number})"));
        number += 1;
    }
    folder
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{AudioProperties, FrameAnimation, TextProperties};

    #[test]
    fn every_file_the_project_uses_is_planned_into_the_media_folder() {
        let root = std::env::temp_dir().join(format!("vizx-collect-plan-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let (a, b) = (root.join("a"), root.join("b"));
        fs::create_dir_all(a.join("ring")).unwrap();
        fs::create_dir_all(a.join("walk")).unwrap();
        fs::create_dir_all(&b).unwrap();
        fs::write(a.join("song.wav"), b"song").unwrap();
        // The same name in another folder: a different file.
        fs::write(b.join("Song.wav"), b"other song").unwrap();
        fs::write(a.join("ring").join("visualizer.json"), b"{}").unwrap();
        fs::write(a.join("ring").join("shader.wgsl"), b"//").unwrap();
        for frame in 7..10 {
            fs::write(a.join("walk").join(format!("walk_{frame:04}.png")), b"png").unwrap();
        }
        fs::write(a.join("font.ttf"), b"font").unwrap();

        let audio = AudioProperties {
            duration_milliseconds: 1000,
            sample_rate: 44_100,
            channel_count: 2,
        };
        let path = |path: PathBuf| path.to_string_lossy().into_owned();
        let mut project = Project::new();
        project.import_audio(path(a.join("song.wav")), audio);
        project.import_audio(path(b.join("Song.wav")), audio);
        project.import_audio(path(a.join("song.wav")), audio);
        project.import_audio(path(root.join("gone.wav")), audio);
        let mut add = |path: String, content: MediaContent| {
            project.media.push(crate::MediaSource {
                id: crate::MediaId(100 + project.media.len() as u64),
                path,
                relative_path: Some("../old".to_owned()),
                content,
                manual_beats: None,
                tempo_bpm: None,
            });
        };
        add(path(a.join("ring")), MediaContent::VisualizerPlugin);
        let mut walk = FrameAnimation::sprite_sheet((10, 10), 1, 1, 12.0);
        walk.layout = FrameLayout::Sequence {
            first_number: 7,
            digits: 4,
        };
        walk.frame_count = 3;
        add(
            path(a.join("walk").join("walk_0007.png")),
            MediaContent::Animation(walk),
        );
        add(
            String::new(),
            MediaContent::Text(TextProperties {
                font_path: path(a.join("font.ttf")),
                ..TextProperties::default()
            }),
        );
        add(
            "bundled-presets:/pack/a.milk".to_owned(),
            MediaContent::MilkdropPreset,
        );

        let folder = root.join("Collected");
        let plan = project.plan_collection(&folder);
        let media = folder.join(COLLECTED_MEDIA_FOLDER);
        let paths: Vec<&str> = plan.project.media.iter().map(|m| m.path.as_str()).collect();

        assert_eq!(paths[0], path(media.join("song.wav")));
        assert_eq!(paths[1], path(media.join("Song (2).wav")));
        // Used twice, copied once.
        assert_eq!(paths[2], paths[0]);
        assert_eq!(paths[3], path(root.join("gone.wav")));
        assert_eq!(plan.missing, [path(root.join("gone.wav"))]);
        assert_eq!(paths[4], path(media.join("ring")));
        assert_eq!(paths[5], path(media.join("walk").join("walk_0007.png")));
        assert_eq!(
            plan.project.media[6].text().unwrap().font_path,
            path(media.join("font.ttf"))
        );
        assert_eq!(paths[7], "bundled-presets:/pack/a.milk");

        // The open project can be pointed at the copies the same way.
        let mut open = project.clone();
        let new_path = |path: &str| {
            plan.moved_paths
                .iter()
                .find(|(old, _)| old == path)
                .map(|(_, new)| new.clone())
        };
        // 3 songs (not the missing one), the plugin, the sequence, the font.
        assert_eq!(open.repoint_media(new_path), 6);
        assert_eq!(open, plan.project);

        // 2 songs + 2 plugin files + 3 frames + 1 font.
        assert_eq!(plan.copies.len(), 8);
        assert_eq!(plan.total_bytes(), 4 + 10 + 2 + 2 + 3 * 3 + 4);
        assert!(
            plan.copies
                .iter()
                .any(|copy| copy.to == media.join("walk").join("walk_0009.png"))
        );
        assert!(
            plan.moved_paths
                .contains(&(path(a.join("ring")), path(media.join("ring"))))
        );
        // Nothing was written.
        assert!(!folder.exists());

        fs::create_dir_all(&folder).unwrap();
        assert_eq!(
            new_collection_folder(&root, "Collected"),
            root.join("Collected (2)")
        );
        let _ = fs::remove_dir_all(&root);
    }
}
