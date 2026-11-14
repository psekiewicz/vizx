use std::fs;
use std::io;
use std::path::{Path, PathBuf};

/// Start of the path stored in a project for a preset that ships with
/// VizX, e.g. `bundled-presets:/cream-of-the-crop/Fractal/Example.milk`.
///
/// Bundled presets are stored by their place inside the preset collection
/// rather than by a location on one particular computer. A project that
/// uses them therefore opens correctly on any machine and operating system
/// with VizX installed, wherever it is installed.
pub const BUNDLED_PRESET_PREFIX: &str = "bundled-presets:/";

/// One preset of the bundled collection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PresetEntry {
    /// File name without the `.milk` extension.
    pub name: String,
    /// Top-level folder inside its pack, e.g. `Fractal`. Empty if the file
    /// lies directly in the pack folder.
    pub category: String,
    /// Folders between the category and the file, e.g. `Blobby`.
    pub subfolder: String,
    /// Path below the presets folder, with `/` between the parts.
    relative_path: String,
    /// Lower-case text that searches are matched against.
    search_text: String,
}

/// The presets that ship with VizX, indexed for browsing and searching.
#[derive(Debug, Clone)]
pub struct PresetLibrary {
    root: PathBuf,
    entries: Vec<PresetEntry>,
    categories: Vec<String>,
}

impl PresetLibrary {
    /// Indexes every `.milk` file below `root`. `root` holds one folder per
    /// preset pack; a pack's top-level folders are its categories.
    pub fn scan(root: &Path) -> io::Result<Self> {
        let mut entries = Vec::new();
        for pack in fs::read_dir(root)? {
            let pack = pack?.path();
            if pack.is_dir() {
                let pack_name = pack.file_name().unwrap_or_default().to_string_lossy();
                collect_presets(&pack, &pack_name, &mut Vec::new(), &mut entries);
            }
        }
        entries.sort_by(|a, b| {
            (&a.category, &a.subfolder, a.name.to_lowercase()).cmp(&(
                &b.category,
                &b.subfolder,
                b.name.to_lowercase(),
            ))
        });

        let mut categories: Vec<String> =
            entries.iter().map(|entry| entry.category.clone()).collect();
        categories.dedup(); // entries are sorted by category

        Ok(Self {
            root: root.to_owned(),
            entries,
            categories,
        })
    }

    pub fn entries(&self) -> &[PresetEntry] {
        &self.entries
    }

    /// Category names in alphabetical order.
    pub fn categories(&self) -> &[String] {
        &self.categories
    }

    /// Presets in `category` (all categories if `None`) whose name or
    /// folder contains every word of `query`, ignoring upper/lower case.
    pub fn search(&self, category: Option<&str>, query: &str) -> Vec<&PresetEntry> {
        let query = query.to_lowercase();
        let words: Vec<&str> = query.split_whitespace().collect();

        self.entries
            .iter()
            .filter(|entry| category.is_none_or(|category| entry.category == category))
            .filter(|entry| words.iter().all(|word| entry.search_text.contains(word)))
            .collect()
    }

    /// The file of a preset on this computer.
    pub fn file_path(&self, entry: &PresetEntry) -> PathBuf {
        self.root.join(&entry.relative_path)
    }

    /// The text stored in a project to refer to a bundled preset.
    pub fn project_reference(entry: &PresetEntry) -> String {
        format!("{BUNDLED_PRESET_PREFIX}{}", entry.relative_path)
    }
}

/// Turns a preset path stored in a project into a file on this computer.
///
/// A bundled preset is looked up in the bundled presets folder; any other
/// path is a file the user picked, and is used as it is. Returns `None` for
/// a bundled preset when this installation has no bundled presets, and for
/// a bundled reference that tries to leave the presets folder with `..`
/// (project files can come from other people, so they are not trusted).
pub fn resolve_preset_path(stored_path: &str, bundled_root: Option<&Path>) -> Option<PathBuf> {
    resolve_bundled_path(stored_path, BUNDLED_PRESET_PREFIX, bundled_root)
}

/// Turns a path stored in a project into a path on this computer, for
/// anything that ships with VizX (presets, visualizer plugins).
///
/// A stored path starting with `prefix` names something inside
/// `bundled_root`; any other stored path is used as it is.
pub fn resolve_bundled_path(
    stored_path: &str,
    prefix: &str,
    bundled_root: Option<&Path>,
) -> Option<PathBuf> {
    match stored_path.strip_prefix(prefix) {
        Some(relative_path) => {
            let stays_inside = Path::new(relative_path)
                .components()
                .all(|part| matches!(part, std::path::Component::Normal(_)));
            if !stays_inside {
                return None;
            }
            Some(bundled_root?.join(relative_path))
        }
        None => Some(PathBuf::from(stored_path)),
    }
}

/// Finds the folder with the presets that ship with VizX.
pub fn find_bundled_presets() -> Option<PathBuf> {
    find_bundled_folder("presets")
}

/// Finds a folder of things that ship with VizX, by its name.
///
/// An installed VizX has the folder next to its executable. When VizX is
/// run from its source code, the folder of that name in the repository's
/// `assets` folder is used instead.
pub fn find_bundled_folder(name: &str) -> Option<PathBuf> {
    let next_to_executable = std::env::current_exe()
        .ok()
        .and_then(|path| Some(path.parent()?.join(name)));
    // `CARGO_MANIFEST_DIR` is this crate's folder on the machine VizX was
    // built on; the repository root is two levels up.
    let in_repository = Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .map(|repository| repository.join("assets").join(name));

    next_to_executable
        .into_iter()
        .chain(in_repository)
        .find(|folder| folder.is_dir())
}
/// Adds every `.milk` file below `folder` to `entries`. `folders` holds the
/// names of the folders walked through so far, below the pack.
fn collect_presets(
    folder: &Path,
    pack_name: &str,
    folders: &mut Vec<String>,
    entries: &mut Vec<PresetEntry>,
) {
    // An unreadable folder is skipped rather than failing the whole scan.
    let Ok(children) = fs::read_dir(folder) else {
        return;
    };
    for child in children.flatten() {
        let path = child.path();
        let file_name = child.file_name().to_string_lossy().into_owned();

        if path.is_dir() {
            // Skip hidden folders such as `.git`.
            if !file_name.starts_with('.') {
                folders.push(file_name);
                collect_presets(&path, pack_name, folders, entries);
                folders.pop();
            }
            continue;
        }

        let is_preset = path
            .extension()
            .is_some_and(|extension| extension.eq_ignore_ascii_case("milk"));
        if !is_preset {
            continue;
        }
        let name = path
            .file_stem()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned();
        let category = folders.first().cloned().unwrap_or_default();
        let subfolder = folders.get(1..).unwrap_or_default().join("/");

        let mut relative_parts = vec![pack_name.to_owned()];
        relative_parts.extend(folders.iter().cloned());
        relative_parts.push(file_name);

        entries.push(PresetEntry {
            search_text: format!("{name} {category} {subfolder}").to_lowercase(),
            name,
            category,
            subfolder,
            relative_path: relative_parts.join("/"),
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Creates a small preset collection in the temp folder.
    fn test_collection(name: &str) -> PathBuf {
        let root =
            std::env::temp_dir().join(format!("vizx-preset-library-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        for file in [
            "pack/Fractal/Blobby/Green Swirl.milk",
            "pack/Fractal/Blobby/red storm.milk",
            "pack/Fractal/Deep Dive.MILK",
            "pack/Waveform/Wire/Thin Lines.milk",
            "pack/Loose Preset.milk",
            "pack/Fractal/notes.txt",
            "pack/.git/ignored.milk",
        ] {
            let path = root.join(file);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, "[preset00]").unwrap();
        }
        root
    }

    #[test]
    fn scan_indexes_presets_with_category_and_subfolder() {
        let root = test_collection("scan");
        let library = PresetLibrary::scan(&root).unwrap();
        let _ = fs::remove_dir_all(&root);

        // Text files and hidden folders are left out.
        assert_eq!(library.entries().len(), 5);
        assert_eq!(library.categories(), ["", "Fractal", "Waveform"]);

        let swirl = library
            .entries()
            .iter()
            .find(|entry| entry.name == "Green Swirl")
            .unwrap();
        assert_eq!(swirl.category, "Fractal");
        assert_eq!(swirl.subfolder, "Blobby");
        assert_eq!(
            PresetLibrary::project_reference(swirl),
            "bundled-presets:/pack/Fractal/Blobby/Green Swirl.milk"
        );
        assert_eq!(
            library.file_path(swirl),
            root.join("pack/Fractal/Blobby/Green Swirl.milk")
        );
    }

    #[test]
    fn search_matches_all_words_in_name_or_folder_ignoring_case() {
        let root = test_collection("search");
        let library = PresetLibrary::scan(&root).unwrap();
        let _ = fs::remove_dir_all(&root);
        let names = |category, query| -> Vec<String> {
            library
                .search(category, query)
                .iter()
                .map(|entry| entry.name.clone())
                .collect()
        };

        assert_eq!(names(None, "").len(), 5);
        assert_eq!(names(None, "SWIRL"), ["Green Swirl"]);
        // A folder name finds everything inside that folder.
        assert_eq!(names(None, "blobby"), ["Green Swirl", "red storm"]);
        // Every word must match, in any order.
        assert_eq!(names(None, "storm blobby"), ["red storm"]);
        assert!(names(None, "storm wire").is_empty());
        assert_eq!(names(Some("Waveform"), ""), ["Thin Lines"]);
        assert!(names(Some("Waveform"), "swirl").is_empty());
    }

    #[test]
    fn stored_paths_resolve_to_files_on_this_computer() {
        let root = Path::new("/opt/vizx/presets");

        assert_eq!(
            resolve_preset_path("bundled-presets:/pack/Fractal/A.milk", Some(root)),
            Some(root.join("pack/Fractal/A.milk"))
        );
        // Without bundled presets, a bundled reference cannot be resolved.
        assert_eq!(
            resolve_preset_path("bundled-presets:/pack/Fractal/A.milk", None),
            None
        );
        // A bundled reference must not reach outside the presets folder.
        assert_eq!(
            resolve_preset_path("bundled-presets:/../../secret.milk", Some(root)),
            None
        );
        assert_eq!(
            resolve_preset_path("bundled-presets:/C:/secret.milk", Some(root)),
            None
        );
        // A file the user picked is used as it is, with or without them.
        assert_eq!(
            resolve_preset_path("D:/my presets/B.milk", None),
            Some(PathBuf::from("D:/my presets/B.milk"))
        );
    }

    #[test]
    fn scanning_a_missing_folder_is_an_error() {
        let missing = std::env::temp_dir().join("vizx-preset-library-does-not-exist");

        assert!(PresetLibrary::scan(&missing).is_err());
    }
}
