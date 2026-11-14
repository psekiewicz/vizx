use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};

use serde::Deserialize;

/// Name of the file that describes a plugin.
pub const MANIFEST_FILE_NAME: &str = "visualizer.json";
/// Name of the file with a plugin's shader code.
pub const SHADER_FILE_NAME: &str = "shader.wgsl";
/// How many numbers all parameters of one plugin may take up together. A
/// number or a switch takes one, a color takes three.
pub const PARAMETER_SLOTS: usize = 32;

/// Parameter values chosen by the user, by parameter id. A number or a
/// switch has one value, a color has three (red, green, blue).
pub type ParameterValues = BTreeMap<String, Vec<f64>>;

/// The content of `visualizer.json`.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Manifest {
    /// Name shown to the user.
    pub name: String,
    #[serde(default)]
    pub author: String,
    #[serde(default)]
    pub description: String,
    /// What the user can adjust, in the order it is shown.
    #[serde(default)]
    pub parameters: Vec<ParameterSpec>,
}

/// One adjustable value of a plugin.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct ParameterSpec {
    /// Name used in the shader: parameter `glow` is read with `p_glow()`.
    /// Lower-case letters, digits and `_`, starting with a letter.
    pub id: String,
    /// Name shown to the user.
    pub name: String,
    #[serde(flatten)]
    pub kind: ParameterKind,
}

/// The kinds of parameters, chosen with `"kind"` in the manifest.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum ParameterKind {
    /// A slider. In the shader: `f32`.
    Number {
        min: f64,
        max: f64,
        default: f64,
        /// Distance between allowed values, e.g. 1 for whole numbers.
        /// Without it the slider is continuous.
        #[serde(default)]
        step: Option<f64>,
    },
    /// A color picker. In the shader: `vec3f` with red, green and blue from
    /// 0 to 1.
    Color { default: [f64; 3] },
    /// A checkbox. In the shader: `f32`, 1 when on and 0 when off.
    Toggle { default: bool },
}

impl ParameterSpec {
    /// How many numbers this parameter takes up.
    fn slot_count(&self) -> usize {
        match self.kind {
            ParameterKind::Color { .. } => 3,
            ParameterKind::Number { .. } | ParameterKind::Toggle { .. } => 1,
        }
    }

    /// The values used until the user changes them.
    pub fn default_values(&self) -> Vec<f64> {
        match &self.kind {
            ParameterKind::Number { default, .. } => vec![*default],
            ParameterKind::Color { default } => default.to_vec(),
            ParameterKind::Toggle { default } => vec![f64::from(u8::from(*default))],
        }
    }

    /// `values` made valid for this parameter: the right count, within the
    /// allowed range. Anything unusable is replaced by the default.
    pub fn sanitized(&self, values: Option<&Vec<f64>>) -> Vec<f64> {
        let defaults = self.default_values();
        let Some(values) = values.filter(|values| {
            values.len() == defaults.len() && values.iter().all(|value| value.is_finite())
        }) else {
            return defaults;
        };
        match &self.kind {
            ParameterKind::Number { min, max, .. } => {
                vec![values[0].clamp(min.min(*max), max.max(*min))]
            }
            ParameterKind::Color { .. } => {
                values.iter().map(|value| value.clamp(0.0, 1.0)).collect()
            }
            ParameterKind::Toggle { .. } => vec![if values[0] >= 0.5 { 1.0 } else { 0.0 }],
        }
    }
}

/// A plugin read from its folder.
#[derive(Debug, Clone, PartialEq)]
pub struct VisualizerPlugin {
    pub manifest: Manifest,
    /// The plugin author's shader code, exactly as in `shader.wgsl`.
    pub shader_source: String,
    pub folder: PathBuf,
}

impl VisualizerPlugin {
    /// Reads and checks the plugin in `folder`. The error text is meant for
    /// the plugin's author.
    pub fn load(folder: &Path) -> Result<Self, String> {
        let manifest = Self::load_manifest(folder)?;
        let shader_path = folder.join(SHADER_FILE_NAME);
        let shader_source = fs::read_to_string(&shader_path)
            .map_err(|error| format!("could not read {}: {error}", shader_path.display()))?;

        Ok(Self {
            manifest,
            shader_source,
            folder: folder.to_owned(),
        })
    }

    /// Reads and checks only `visualizer.json`, which is all that is needed
    /// to list a plugin and show its parameters.
    pub fn load_manifest(folder: &Path) -> Result<Manifest, String> {
        let manifest_path = folder.join(MANIFEST_FILE_NAME);
        let manifest_text = fs::read_to_string(&manifest_path)
            .map_err(|error| format!("could not read {}: {error}", manifest_path.display()))?;
        let manifest: Manifest = serde_json::from_str(&manifest_text)
            .map_err(|error| format!("{} is not valid: {error}", manifest_path.display()))?;
        check_manifest(&manifest)
            .map_err(|problem| format!("{}: {problem}", manifest_path.display()))?;
        Ok(manifest)
    }

    /// The complete shader handed to the graphics card: the author's code
    /// followed by what VizX adds.
    ///
    /// The author's code comes first so that line numbers in error messages
    /// match the lines of `shader.wgsl`. (WGSL does not care in which order
    /// functions are written.)
    pub fn full_shader_source(&self) -> String {
        let mut source = self.shader_source.clone();
        source.push_str("\n\n// ---- added by VizX ----\n");
        source.push_str(include_str!("prelude.wgsl"));
        source.push_str(&parameter_accessors(&self.manifest));
        source
    }

    /// The parameter values as the numbers the shader reads, in manifest
    /// order. Missing or invalid values fall back to the defaults.
    pub fn pack_parameters(&self, values: &ParameterValues) -> [f32; PARAMETER_SLOTS] {
        let mut packed = [0.0; PARAMETER_SLOTS];
        let mut slot = 0;
        for parameter in &self.manifest.parameters {
            for value in parameter.sanitized(values.get(&parameter.id)) {
                packed[slot] = value as f32;
                slot += 1;
            }
        }
        packed
    }
}

/// Rejects manifests the shader glue could not work with.
fn check_manifest(manifest: &Manifest) -> Result<(), String> {
    if manifest.name.trim().is_empty() {
        return Err("\"name\" must not be empty".to_owned());
    }

    let mut used_slots = 0;
    for (index, parameter) in manifest.parameters.iter().enumerate() {
        let id = &parameter.id;
        let starts_with_letter = id.chars().next().is_some_and(|c| c.is_ascii_lowercase());
        let has_only_allowed_characters = id
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_');
        if !starts_with_letter || !has_only_allowed_characters {
            return Err(format!(
                "parameter id \"{id}\" must start with a lower-case letter and contain only \
                 lower-case letters, digits and _"
            ));
        }
        if manifest.parameters[..index]
            .iter()
            .any(|other| other.id == *id)
        {
            return Err(format!("parameter id \"{id}\" is used twice"));
        }
        if let ParameterKind::Number {
            min,
            max,
            default,
            step,
        } = &parameter.kind
        {
            let all_finite = [min, max, default].iter().all(|value| value.is_finite());
            if !all_finite || min >= max {
                return Err(format!(
                    "parameter \"{id}\" needs \"min\" smaller than \"max\""
                ));
            }
            if step.is_some_and(|step| !step.is_finite() || step <= 0.0) {
                return Err(format!(
                    "parameter \"{id}\": \"step\" must be greater than 0"
                ));
            }
        }
        used_slots += parameter.slot_count();
    }
    if used_slots > PARAMETER_SLOTS {
        return Err(format!(
            "too many parameters: they need {used_slots} numbers, {PARAMETER_SLOTS} are available \
             (a color counts as 3)"
        ));
    }
    Ok(())
}

/// WGSL functions `p_<id>()` that read the parameters by name.
fn parameter_accessors(manifest: &Manifest) -> String {
    // The numbers are stored four to a `vec4f`.
    let slot = |index: usize| format!("vizx.params[{}][{}]", index / 4, index % 4);

    let mut code = String::new();
    let mut next = 0;
    for parameter in &manifest.parameters {
        let id = &parameter.id;
        // Writing to a String cannot fail.
        let _ = match parameter.kind {
            ParameterKind::Color { .. } => writeln!(
                code,
                "fn p_{id}() -> vec3f {{ return vec3f({}, {}, {}); }}",
                slot(next),
                slot(next + 1),
                slot(next + 2)
            ),
            ParameterKind::Number { .. } | ParameterKind::Toggle { .. } => {
                writeln!(code, "fn p_{id}() -> f32 {{ return {}; }}", slot(next))
            }
        };
        next += parameter.slot_count();
    }
    code
}

#[cfg(test)]
mod tests {
    use super::*;

    fn manifest(json: &str) -> Result<Manifest, String> {
        let manifest: Manifest = serde_json::from_str(json).map_err(|error| error.to_string())?;
        check_manifest(&manifest)?;
        Ok(manifest)
    }

    const EXAMPLE: &str = r#"{
        "name": "Example",
        "parameters": [
            { "id": "bars", "name": "Bars", "kind": "number", "min": 8, "max": 64, "default": 32, "step": 1 },
            { "id": "color", "name": "Color", "kind": "color", "default": [1.0, 0.5, 0.0] },
            { "id": "mirror", "name": "Mirror", "kind": "toggle", "default": true }
        ]
    }"#;

    fn example_plugin() -> VisualizerPlugin {
        VisualizerPlugin {
            manifest: manifest(EXAMPLE).unwrap(),
            shader_source: "fn visualize(uv: vec2f) -> vec4f { return vec4f(0.0); }".to_owned(),
            folder: PathBuf::new(),
        }
    }

    #[test]
    fn manifest_lists_the_three_kinds_of_parameters() {
        let manifest = manifest(EXAMPLE).unwrap();

        assert_eq!(manifest.name, "Example");
        assert_eq!(manifest.author, "");
        assert_eq!(manifest.parameters.len(), 3);
        assert_eq!(manifest.parameters[0].default_values(), [32.0]);
        assert_eq!(manifest.parameters[1].default_values(), [1.0, 0.5, 0.0]);
        assert_eq!(manifest.parameters[2].default_values(), [1.0]);
    }

    #[test]
    fn broken_manifests_are_explained() {
        let problem = |json| manifest(json).unwrap_err();

        assert!(problem(r#"{ "name": " " }"#).contains("name"));
        assert!(
            problem(
                r#"{ "name": "A", "parameters": [
                { "id": "Glow", "name": "Glow", "kind": "toggle", "default": true } ] }"#
            )
            .contains("lower-case")
        );
        assert!(
            problem(
                r#"{ "name": "A", "parameters": [
                { "id": "a", "name": "A", "kind": "toggle", "default": true },
                { "id": "a", "name": "A", "kind": "toggle", "default": true } ] }"#
            )
            .contains("twice")
        );
        assert!(
            problem(
                r#"{ "name": "A", "parameters": [
                { "id": "a", "name": "A", "kind": "number", "min": 5, "max": 5, "default": 5 } ] }"#
            )
            .contains("smaller")
        );
        // An unknown kind is refused by the JSON reader itself.
        assert!(
            manifest(
                r#"{ "name": "A", "parameters": [ { "id": "a", "name": "A", "kind": "knob" } ] }"#
            )
            .is_err()
        );
    }

    #[test]
    fn parameters_are_packed_in_manifest_order_with_defaults_and_limits() {
        let plugin = example_plugin();

        let defaults = plugin.pack_parameters(&ParameterValues::new());
        assert_eq!(defaults[..5], [32.0, 1.0, 0.5, 0.0, 1.0]);
        assert!(defaults[5..].iter().all(|value| *value == 0.0));

        let mut chosen = ParameterValues::new();
        chosen.insert("bars".to_owned(), vec![1000.0]); // above the maximum
        chosen.insert("color".to_owned(), vec![0.2, 0.4]); // one value missing
        chosen.insert("mirror".to_owned(), vec![0.0]);
        chosen.insert("gone".to_owned(), vec![7.0]); // not a parameter any more
        let packed = plugin.pack_parameters(&chosen);
        assert_eq!(packed[..5], [64.0, 1.0, 0.5, 0.0, 0.0]);
    }

    #[test]
    fn shader_gets_named_parameter_functions_after_the_authors_code() {
        let source = example_plugin().full_shader_source();

        assert!(source.starts_with("fn visualize"));
        assert!(source.contains("fn p_bars() -> f32 { return vizx.params[0][0]; }"));
        assert!(source.contains(
            "fn p_color() -> vec3f { return vec3f(vizx.params[0][1], vizx.params[0][2], vizx.params[0][3]); }"
        ));
        assert!(source.contains("fn p_mirror() -> f32 { return vizx.params[1][0]; }"));
    }

    #[test]
    fn loading_reports_a_missing_file_by_name() {
        let folder = std::env::temp_dir().join(format!(
            "vizx-visualizer-plugin-test-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&folder);
        fs::create_dir_all(&folder).unwrap();

        assert!(
            VisualizerPlugin::load(&folder)
                .unwrap_err()
                .contains(MANIFEST_FILE_NAME)
        );

        fs::write(folder.join(MANIFEST_FILE_NAME), r#"{ "name": "Test" }"#).unwrap();
        assert!(
            VisualizerPlugin::load(&folder)
                .unwrap_err()
                .contains(SHADER_FILE_NAME)
        );

        fs::write(folder.join(SHADER_FILE_NAME), "fn visualize() {}").unwrap();
        let plugin = VisualizerPlugin::load(&folder).unwrap();
        let _ = fs::remove_dir_all(&folder);
        assert_eq!(plugin.manifest.name, "Test");
        assert_eq!(plugin.shader_source, "fn visualize() {}");
    }
}
