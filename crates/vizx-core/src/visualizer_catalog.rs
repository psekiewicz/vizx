use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime};

use vizx_visualizer::{MANIFEST_FILE_NAME, Manifest, SHADER_FILE_NAME, VisualizerPlugin};

use crate::preset_library::{find_bundled_folder, resolve_bundled_path};

/// Start of the path stored in a project for a visualizer plugin that ships
/// with VizX, e.g. `bundled-visualizers:/spectrum-bars`. As with bundled
/// presets, this keeps projects independent of where VizX is installed.
pub const BUNDLED_VISUALIZER_PREFIX: &str = "bundled-visualizers:/";

/// A plugin's files are looked at again at most this often.
const RECHECK_INTERVAL: Duration = Duration::from_millis(500);

/// One plugin that ships with VizX.
#[derive(Debug, Clone)]
pub struct BundledVisualizer {
    /// The path to store in a project for this plugin.
    pub reference: String,
    pub manifest: Manifest,
}

/// What is known about one plugin used by the project.
#[derive(Debug, Clone)]
pub struct PluginState {
    /// The plugin's folder on this computer. `None` for a bundled plugin
    /// when this installation has no bundled plugins.
    pub folder: Option<PathBuf>,
    /// The plugin's description, or why it could not be read.
    pub manifest: Result<Manifest, String>,
    /// Changes whenever one of the plugin's files is saved. Whoever shows
    /// or renders the plugin compares it to notice that it must start over.
    pub version: u64,
    checked_at: Instant,
}

/// The visualizer plugins VizX knows about: the ones that ship with it, and
/// the ones the open project uses.
///
/// It watches the files of the plugins in use. When a plugin's author saves
/// `shader.wgsl` or `visualizer.json`, the plugin's `version` changes, and
/// the preview renders it again. That makes writing a plugin a matter of
/// "save and look".
pub struct VisualizerCatalog {
    bundled_root: Option<PathBuf>,
    bundled: Vec<BundledVisualizer>,
    /// By the path stored in the project.
    plugins: HashMap<String, PluginState>,
}

impl VisualizerCatalog {
    /// Finds the plugins that ship with VizX and reads their descriptions.
    pub fn new() -> Self {
        Self::with_bundled_root(find_bundled_folder("visualizers"))
    }

    /// Like [`VisualizerCatalog::new`], with the bundled plugins in a given
    /// folder (each plugin in a folder of its own inside it).
    pub fn with_bundled_root(bundled_root: Option<PathBuf>) -> Self {
        let mut bundled = Vec::new();
        let folders = bundled_root
            .as_deref()
            .and_then(|root| fs::read_dir(root).ok())
            .into_iter()
            .flatten()
            .flatten();
        for folder in folders {
            let folder_name = folder.file_name().to_string_lossy().into_owned();
            // A folder that is not a (working) plugin is simply not offered.
            if let Ok(manifest) = VisualizerPlugin::load_manifest(&folder.path()) {
                bundled.push(BundledVisualizer {
                    reference: format!("{BUNDLED_VISUALIZER_PREFIX}{folder_name}"),
                    manifest,
                });
            }
        }
        bundled.sort_by_key(|plugin| plugin.manifest.name.to_lowercase());

        Self {
            bundled_root,
            bundled,
            plugins: HashMap::new(),
        }
    }

    /// The plugins that ship with VizX, by name.
    pub fn bundled(&self) -> &[BundledVisualizer] {
        &self.bundled
    }

    /// The folder on this computer of the plugin stored as `stored_path`.
    pub fn folder(&self, stored_path: &str) -> Option<PathBuf> {
        resolve_bundled_path(
            stored_path,
            BUNDLED_VISUALIZER_PREFIX,
            self.bundled_root.as_deref(),
        )
    }

    /// The current state of a plugin. Its files are checked for changes if
    /// that has not been done for a moment.
    pub fn plugin(&mut self, stored_path: &str) -> &PluginState {
        let is_fresh = self
            .plugins
            .get(stored_path)
            .is_some_and(|state| state.checked_at.elapsed() < RECHECK_INTERVAL);
        if !is_fresh {
            let folder = self.folder(stored_path);
            let version = folder.as_deref().map_or(0, files_version);
            let previous = self.plugins.get(stored_path);
            // Read the description again only if a file changed.
            let manifest = match previous {
                Some(previous) if previous.version == version && previous.folder == folder => {
                    previous.manifest.clone()
                }
                _ => match &folder {
                    Some(folder) => VisualizerPlugin::load_manifest(folder),
                    None => Err("Bundled visualizers not found.".to_owned()),
                },
            };
            self.plugins.insert(
                stored_path.to_owned(),
                PluginState {
                    folder,
                    manifest,
                    version,
                    checked_at: Instant::now(),
                },
            );
        }
        &self.plugins[stored_path]
    }
}

impl Default for VisualizerCatalog {
    fn default() -> Self {
        Self::new()
    }
}

/// A number that changes when one of the plugin's two files is saved:
/// derived from their modification times.
fn files_version(folder: &Path) -> u64 {
    [MANIFEST_FILE_NAME, SHADER_FILE_NAME]
        .iter()
        .map(|file_name| {
            fs::metadata(folder.join(file_name))
                .and_then(|metadata| metadata.modified())
                .ok()
                .and_then(|modified| modified.duration_since(SystemTime::UNIX_EPOCH).ok())
                // A missing file counts as 0; it "changes" when it appears.
                .map_or(0, |since_epoch| since_epoch.as_nanos() as u64)
        })
        .fold(0, |version, file_time| {
            version.wrapping_mul(31).wrapping_add(file_time)
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_folder(name: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!(
            "vizx-visualizer-catalog-{name}-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        root
    }

    fn write_plugin(folder: &Path, name: &str) {
        fs::create_dir_all(folder).unwrap();
        fs::write(
            folder.join(MANIFEST_FILE_NAME),
            format!(r#"{{ "name": "{name}" }}"#),
        )
        .unwrap();
        fs::write(folder.join(SHADER_FILE_NAME), "// shader").unwrap();
    }

    #[test]
    fn bundled_plugins_are_listed_by_name_and_broken_folders_are_left_out() {
        let root = test_folder("bundled");
        write_plugin(&root.join("waves"), "Waves");
        write_plugin(&root.join("bars"), "bars");
        fs::create_dir_all(root.join("not-a-plugin")).unwrap();

        let catalog = VisualizerCatalog::with_bundled_root(Some(root.clone()));

        let listed: Vec<(&str, &str)> = catalog
            .bundled()
            .iter()
            .map(|plugin| (plugin.manifest.name.as_str(), plugin.reference.as_str()))
            .collect();
        assert_eq!(
            listed,
            [
                ("bars", "bundled-visualizers:/bars"),
                ("Waves", "bundled-visualizers:/waves")
            ]
        );
        assert_eq!(
            catalog.folder("bundled-visualizers:/bars"),
            Some(root.join("bars"))
        );
        // A plugin folder of the user's own is used as it is.
        assert_eq!(
            catalog.folder("D:/my plugins/glow"),
            Some(PathBuf::from("D:/my plugins/glow"))
        );
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn a_missing_bundled_plugin_is_explained() {
        let mut catalog = VisualizerCatalog::with_bundled_root(None);

        let state = catalog.plugin("bundled-visualizers:/bars");

        assert!(state.folder.is_none());
        assert!(
            state
                .manifest
                .as_ref()
                .unwrap_err()
                .contains("Bundled visualizers")
        );
    }

    #[test]
    fn version_follows_the_plugin_files() {
        let root = test_folder("version");
        let folder = root.join("glow");
        write_plugin(&folder, "Glow");

        let before = files_version(&folder);
        assert_eq!(before, files_version(&folder));

        // A different modification time, as after saving in an editor.
        let later = SystemTime::now() + Duration::from_secs(5);
        fs::File::options()
            .write(true)
            .open(folder.join(SHADER_FILE_NAME))
            .unwrap()
            .set_modified(later)
            .unwrap();

        assert_ne!(files_version(&folder), before);
        assert_eq!(files_version(&root.join("missing")), 0);
        let _ = fs::remove_dir_all(&root);
    }
}
