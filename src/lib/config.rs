//! Jupytext configuration file loading and parsing.
//!
//! Supports reading configuration from:
//! - `jupytext.toml` / `.jupytext.toml`
//! - `pyproject.toml` (under `[tool.jupytext]`)
//! - `jupytext.yml` / `.jupytext.yml` / `jupytext.yaml` / `.jupytext.yaml`
//! - `jupytext.json` / `.jupytext.json`
//!
//! Configuration files are searched in the notebook directory and all parent
//! directories up to a ceiling directory or filesystem root.

use std::collections::BTreeMap;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Names of recognized Jupytext configuration files, checked in order.
const JUPYTEXT_CONFIG_FILES: &[&str] = &[
    "jupytext",
    "jupytext.toml",
    "jupytext.yml",
    "jupytext.yaml",
    "jupytext.json",
    ".jupytext",
    ".jupytext.toml",
    ".jupytext.yml",
    ".jupytext.yaml",
    ".jupytext.json",
];

/// The pyproject.toml filename.
const PYPROJECT_FILE: &str = "pyproject.toml";

/// Jupytext configuration, mirroring all options from the Python `JupytextConfiguration` class.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct JupytextConfig {
    /// Paired notebook formats, e.g. "ipynb,py:percent".
    /// Can be a single string or semicolon-separated list for multiple pairing groups.
    pub formats: Option<String>,

    /// Deprecated alias for `formats`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_jupytext_formats: Option<String>,

    /// Preferred format when saving notebooks as text, per extension.
    /// e.g. "jl:percent,py:percent,R:percent".
    #[serde(skip_serializing_if = "Option::is_none")]
    pub preferred_jupytext_formats_save: Option<String>,

    /// Preferred format when reading notebooks from text, per extension.
    /// e.g. "py:sphinx".
    #[serde(skip_serializing_if = "Option::is_none")]
    pub preferred_jupytext_formats_read: Option<String>,

    /// Notebook metadata filter.
    /// Examples: "all", "-all", "widgets,nteract", "kernelspec,jupytext-all".
    #[serde(skip_serializing_if = "Option::is_none")]
    pub notebook_metadata_filter: Option<String>,

    /// Deprecated alias for `notebook_metadata_filter`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_notebook_metadata_filter: Option<String>,

    /// Whether notebook metadata should be wrapped in an HTML comment in Markdown format.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hide_notebook_metadata: Option<bool>,

    /// Whether root-level metadata of text documents (e.g. `title`, `author` in R Markdown)
    /// should appear as a raw cell (true) or go to notebook metadata (false).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub root_level_metadata_as_raw_cell: Option<bool>,

    /// Root-level metadata filter.
    /// Examples: "all", "-all", "kernelspec,jupytext".
    #[serde(skip_serializing_if = "Option::is_none")]
    pub root_level_metadata_filter: Option<String>,

    /// Cell metadata filter.
    /// Examples: "all", "hide_input,hide_output".
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cell_metadata_filter: Option<String>,

    /// Deprecated alias for `cell_metadata_filter`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_cell_metadata_filter: Option<String>,

    /// Whether Jupyter magic commands should be commented out in the text representation.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub comment_magics: Option<bool>,

    /// Split markdown cells on headings (Markdown and R Markdown formats only).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub split_at_heading: Option<bool>,

    /// When opening a Sphinx Gallery script, convert the reStructuredText to markdown.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sphinx_convert_rst2md: Option<bool>,

    /// Use DOxygen equation markers.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub doxygen_equation_markers: Option<bool>,

    /// Margin (in seconds) for refusing to overwrite ipynb with older text notebook.
    /// Ignored by the CLI.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub outdated_text_notebook_margin: Option<f64>,

    /// Log level for configuration file messages in the contents manager.
    /// One of: "warning", "info", "info_if_changed", "debug", "none".
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cm_config_log_level: Option<String>,

    /// Start and end cell markers for the light format, comma-separated.
    /// e.g. "{{{,}}}" for Vim folding, "region,endregion" for VSCode/PyCharm.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cell_markers: Option<String>,

    /// Deprecated alias for `cell_markers`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_cell_markers: Option<String>,

    /// List of recognized notebook extensions (overrides the default list).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub notebook_extensions: Option<Vec<String>>,

    /// Comma-separated list of custom cell magics to comment out.
    /// e.g. "configure,local" for Spark magic cell commands.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub custom_cell_magics: Option<String>,
}

impl JupytextConfig {
    /// Apply default format options from this configuration to a format options map.
    ///
    /// The `read` flag controls whether read-specific or write-specific defaults
    /// are applied.
    pub fn set_default_format_options(
        &self,
        format_options: &mut BTreeMap<String, Value>,
        read: bool,
    ) {
        if let Some(ref filter) = self.default_notebook_metadata_filter {
            if !filter.is_empty() {
                format_options
                    .entry("notebook_metadata_filter".to_string())
                    .or_insert_with(|| Value::String(filter.clone()));
            }
        }
        if let Some(ref filter) = self.notebook_metadata_filter {
            if !filter.is_empty() {
                format_options
                    .entry("notebook_metadata_filter".to_string())
                    .or_insert_with(|| Value::String(filter.clone()));
            }
        }
        if let Some(ref filter) = self.default_cell_metadata_filter {
            if !filter.is_empty() {
                format_options
                    .entry("cell_metadata_filter".to_string())
                    .or_insert_with(|| Value::String(filter.clone()));
            }
        }
        if let Some(ref filter) = self.root_level_metadata_filter {
            if !filter.is_empty() {
                format_options
                    .entry("root_level_metadata_filter".to_string())
                    .or_insert_with(|| Value::String(filter.clone()));
            }
        }
        if let Some(ref filter) = self.cell_metadata_filter {
            if !filter.is_empty() {
                format_options
                    .entry("cell_metadata_filter".to_string())
                    .or_insert_with(|| Value::String(filter.clone()));
            }
        }
        if let Some(hide) = self.hide_notebook_metadata {
            format_options
                .entry("hide_notebook_metadata".to_string())
                .or_insert_with(|| Value::Bool(hide));
        }
        if self.root_level_metadata_as_raw_cell == Some(false) {
            format_options
                .entry("root_level_metadata_as_raw_cell".to_string())
                .or_insert_with(|| Value::Bool(false));
        }
        if let Some(cm) = self.comment_magics {
            format_options
                .entry("comment_magics".to_string())
                .or_insert_with(|| Value::Bool(cm));
        }
        if self.split_at_heading == Some(true) {
            format_options
                .entry("split_at_heading".to_string())
                .or_insert_with(|| Value::Bool(true));
        }
        if self.doxygen_equation_markers == Some(true) {
            format_options
                .entry("doxygen_equation_markers".to_string())
                .or_insert_with(|| Value::Bool(true));
        }
        if !read {
            if let Some(ref markers) = self.default_cell_markers {
                if !markers.is_empty() {
                    format_options
                        .entry("cell_markers".to_string())
                        .or_insert_with(|| Value::String(markers.clone()));
                }
            }
            if let Some(ref markers) = self.cell_markers {
                if !markers.is_empty() {
                    format_options
                        .entry("cell_markers".to_string())
                        .or_insert_with(|| Value::String(markers.clone()));
                }
            }
        }
        if read {
            if self.sphinx_convert_rst2md == Some(true) {
                format_options
                    .entry("rst2md".to_string())
                    .or_insert_with(|| Value::Bool(true));
            }
        }
        if let Some(ref magics) = self.custom_cell_magics {
            if !magics.is_empty() {
                format_options
                    .entry("custom_cell_magics".to_string())
                    .or_insert_with(|| Value::String(magics.clone()));
            }
        }
    }

    /// Return the effective formats string, preferring `formats` over the deprecated
    /// `default_jupytext_formats`.
    pub fn effective_formats(&self) -> Option<&str> {
        self.formats
            .as_deref()
            .or(self.default_jupytext_formats.as_deref())
    }

    /// Return the effective cell markers string, preferring `cell_markers` over the deprecated
    /// `default_cell_markers`.
    pub fn effective_cell_markers(&self) -> Option<&str> {
        self.cell_markers
            .as_deref()
            .or(self.default_cell_markers.as_deref())
    }

    /// Return the effective notebook metadata filter, preferring the non-deprecated version.
    pub fn effective_notebook_metadata_filter(&self) -> Option<&str> {
        self.notebook_metadata_filter
            .as_deref()
            .or(self.default_notebook_metadata_filter.as_deref())
    }

    /// Return the effective cell metadata filter, preferring the non-deprecated version.
    pub fn effective_cell_metadata_filter(&self) -> Option<&str> {
        self.cell_metadata_filter
            .as_deref()
            .or(self.default_cell_metadata_filter.as_deref())
    }
}

/// Ceiling directories from the `JUPYTEXT_CEILING_DIRECTORIES` environment variable.
fn ceiling_directories() -> Vec<PathBuf> {
    env::var("JUPYTEXT_CEILING_DIRECTORIES")
        .unwrap_or_default()
        .split(':')
        .filter(|s| !s.is_empty())
        .map(PathBuf::from)
        .filter(|p| p.is_dir())
        .collect()
}

/// Return the global configuration directories to search, in order.
///
/// Follows the XDG Base Directory Specification on Unix-like systems and
/// uses `%USERPROFILE%` / `%ALLUSERSPROFILE%` on Windows.
fn global_jupytext_configuration_directories() -> Vec<PathBuf> {
    let mut dirs = Vec::new();

    if let Ok(xdg) = env::var("XDG_CONFIG_HOME") {
        for part in xdg.split(':') {
            dirs.push(PathBuf::from(part));
        }
    } else if let Ok(userprofile) = env::var("USERPROFILE") {
        dirs.push(PathBuf::from(userprofile));
    } else if let Ok(home) = env::var("HOME") {
        dirs.push(PathBuf::from(&home).join(".config"));
        dirs.push(PathBuf::from(home));
    }

    if let Ok(xdg_dirs) = env::var("XDG_CONFIG_DIRS") {
        for part in xdg_dirs.split(':') {
            dirs.push(PathBuf::from(part));
        }
    } else if let Ok(all) = env::var("ALLUSERSPROFILE") {
        dirs.push(PathBuf::from(all));
    } else {
        dirs.push(PathBuf::from("/usr/local/share/"));
        dirs.push(PathBuf::from("/usr/share/"));
    }

    // For each config dir, check both `<dir>/jupytext/` and `<dir>/`.
    let mut result = Vec::new();
    for d in dirs {
        result.push(d.join("jupytext"));
        result.push(d);
    }
    result
}

/// Find the global Jupytext configuration file, if any.
fn find_global_jupytext_configuration_file() -> Option<PathBuf> {
    for config_dir in global_jupytext_configuration_directories() {
        if let Some(found) = find_jupytext_configuration_file_in_dir(&config_dir) {
            return Some(found);
        }
    }
    None
}

/// Look for a Jupytext configuration file in a single directory (no parent traversal).
fn find_jupytext_configuration_file_in_dir(dir: &Path) -> Option<PathBuf> {
    if !dir.is_dir() {
        return None;
    }

    for filename in JUPYTEXT_CONFIG_FILES {
        let candidate = dir.join(filename);
        if candidate.is_file() {
            return Some(candidate);
        }
    }

    // Check pyproject.toml for a [tool.jupytext] section
    let pyproject = dir.join(PYPROJECT_FILE);
    if pyproject.is_file() {
        if let Ok(contents) = fs::read_to_string(&pyproject) {
            if let Ok(doc) = contents.parse::<toml::Table>() {
                if doc
                    .get("tool")
                    .and_then(|t| t.get("jupytext"))
                    .is_some()
                {
                    return Some(pyproject);
                }
            }
        }
    }

    None
}

/// Search for a Jupytext configuration file starting from `start_path`, traversing parent
/// directories up to the filesystem root or a ceiling directory.
///
/// If `start_path` is a file, the search begins in its parent directory.
///
/// Returns `None` if no configuration file is found.
pub fn find_jupytext_configuration_file(start_path: &Path) -> Option<PathBuf> {
    let abs = if start_path.is_absolute() {
        start_path.to_path_buf()
    } else {
        env::current_dir().ok()?.join(start_path)
    };

    let dir = if abs.is_file() {
        abs.parent()?.to_path_buf()
    } else {
        abs.clone()
    };

    find_jupytext_configuration_file_from_dir(&dir)
}

/// Internal recursive search from a directory.
fn find_jupytext_configuration_file_from_dir(dir: &Path) -> Option<PathBuf> {
    // Check current directory
    if let Some(found) = find_jupytext_configuration_file_in_dir(dir) {
        return Some(found);
    }

    // Are we at a ceiling directory?
    let ceilings = ceiling_directories();
    if ceilings
        .iter()
        .any(|c| c.canonicalize().ok() == dir.canonicalize().ok())
    {
        return None;
    }

    // Traverse to parent
    match dir.parent() {
        Some(parent) if parent != dir => find_jupytext_configuration_file_from_dir(parent),
        _ => find_global_jupytext_configuration_file(),
    }
}

/// Parse a Jupytext configuration file and return a raw TOML/YAML/JSON table.
fn parse_jupytext_configuration_file(config_path: &Path) -> Result<toml::Table> {
    let contents = fs::read_to_string(config_path)
        .with_context(|| format!("Cannot read config file: {}", config_path.display()))?;

    let filename = config_path
        .file_name()
        .and_then(|f| f.to_str())
        .unwrap_or("");

    if filename.ends_with(".toml") || filename == "jupytext" || filename == ".jupytext" {
        let doc: toml::Table = contents.parse().with_context(|| {
            format!(
                "Cannot parse TOML config file: {}",
                config_path.display()
            )
        })?;

        if filename == PYPROJECT_FILE || filename.ends_with("pyproject.toml") {
            // Extract [tool.jupytext]
            let tool = doc
                .get("tool")
                .and_then(|t| t.as_table())
                .ok_or_else(|| {
                    anyhow::anyhow!(
                        "No [tool] section in {}",
                        config_path.display()
                    )
                })?;
            let jupytext = tool
                .get("jupytext")
                .and_then(|j| j.as_table())
                .ok_or_else(|| {
                    anyhow::anyhow!(
                        "No [tool.jupytext] section in {}",
                        config_path.display()
                    )
                })?;
            return Ok(jupytext.clone());
        }

        return Ok(doc);
    }

    if filename.ends_with(".yml") || filename.ends_with(".yaml") {
        let yaml_val: serde_yaml::Value = serde_yaml::from_str(&contents).with_context(|| {
            format!(
                "Cannot parse YAML config file: {}",
                config_path.display()
            )
        })?;
        // Convert YAML -> JSON -> TOML for uniform handling
        let json_val = serde_json::to_string(&yaml_val)?;
        let table: toml::Table = toml::from_str(&json_to_toml_string(&json_val)?)?;
        return Ok(table);
    }

    if filename.ends_with(".json") {
        let json_val: serde_json::Value =
            serde_json::from_str(&contents).with_context(|| {
                format!(
                    "Cannot parse JSON config file: {}",
                    config_path.display()
                )
            })?;
        let toml_str = json_to_toml_string(&serde_json::to_string(&json_val)?)?;
        let table: toml::Table = toml::from_str(&toml_str)?;
        return Ok(table);
    }

    bail!(
        "Unsupported config file format: {}",
        config_path.display()
    );
}

/// Helper: convert a JSON string to a TOML-compatible string by deserializing via serde.
fn json_to_toml_string(json_str: &str) -> Result<String> {
    let val: toml::Value = serde_json::from_str::<serde_json::Value>(json_str)
        .map(|v| json_value_to_toml_value(&v))?;
    Ok(toml::to_string(&val)?)
}

/// Recursively convert a `serde_json::Value` to a `toml::Value`.
fn json_value_to_toml_value(v: &serde_json::Value) -> toml::Value {
    match v {
        serde_json::Value::Null => toml::Value::String(String::new()),
        serde_json::Value::Bool(b) => toml::Value::Boolean(*b),
        serde_json::Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                toml::Value::Integer(i)
            } else if let Some(f) = n.as_f64() {
                toml::Value::Float(f)
            } else {
                toml::Value::String(n.to_string())
            }
        }
        serde_json::Value::String(s) => toml::Value::String(s.clone()),
        serde_json::Value::Array(arr) => {
            toml::Value::Array(arr.iter().map(json_value_to_toml_value).collect())
        }
        serde_json::Value::Object(map) => {
            let mut table = toml::map::Map::new();
            for (k, val) in map {
                table.insert(k.clone(), json_value_to_toml_value(val));
            }
            toml::Value::Table(table)
        }
    }
}

/// Load and validate a Jupytext configuration file, returning a `JupytextConfig`.
pub fn load_jupytext_configuration_file(config_path: &Path) -> Result<JupytextConfig> {
    let table = parse_jupytext_configuration_file(config_path)?;
    let toml_str = toml::to_string(&table)?;
    let config: JupytextConfig = toml::from_str(&toml_str).with_context(|| {
        format!(
            "Invalid Jupytext configuration in {}",
            config_path.display()
        )
    })?;
    Ok(config)
}

/// Load the Jupytext configuration for a given notebook file.
///
/// Searches for a configuration file in the notebook's directory and parent
/// directories. Returns `Ok(None)` if no configuration file is found.
/// Returns `Ok(None)` if the config file IS the notebook file itself.
pub fn load_jupytext_config(nb_file: &Path) -> Result<Option<JupytextConfig>> {
    let config_file = match find_jupytext_configuration_file(nb_file) {
        Some(cf) => cf,
        None => return Ok(None),
    };

    // Don't load the notebook itself as config
    if nb_file.is_file() {
        if let (Ok(a), Ok(b)) = (
            fs::canonicalize(nb_file),
            fs::canonicalize(&config_file),
        ) {
            if a == b {
                return Ok(None);
            }
        }
    }

    let config = load_jupytext_configuration_file(&config_file)?;
    Ok(Some(config))
}

/// Return the notebook formats from notebook metadata, configuration, or defaults.
///
/// Notebook metadata takes precedence over config. Returns a formats string
/// (e.g. "ipynb,py:percent") or `None`.
pub fn notebook_formats(
    nb: &crate::notebook::Notebook,
    config: Option<&JupytextConfig>,
    path: &Path,
) -> Option<String> {
    // Check notebook metadata first
    let from_metadata = nb
        .metadata
        .get("jupytext")
        .and_then(|j| j.get("formats"))
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    if from_metadata.is_some() {
        return from_metadata;
    }

    // Fall back to configuration
    if let Some(cfg) = config {
        if let Some(formats) = cfg.effective_formats() {
            if !formats.is_empty() {
                return Some(formats.to_string());
            }
        }
    }

    // Fall back to the current file extension
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| format!(".{}", e))
        .unwrap_or_default();
    if !ext.is_empty() {
        return Some(ext);
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write as IoWrite;

    #[test]
    fn test_default_config() {
        let config = JupytextConfig::default();
        assert!(config.formats.is_none());
        assert!(config.comment_magics.is_none());
        assert!(config.cell_markers.is_none());
    }

    #[test]
    fn test_parse_toml_config() {
        let dir = tempfile::tempdir().unwrap();
        let config_path = dir.path().join("jupytext.toml");
        let mut f = fs::File::create(&config_path).unwrap();
        writeln!(
            f,
            r#"
formats = "ipynb,py:percent"
notebook_metadata_filter = "-all"
comment_magics = true
cell_markers = "region,endregion"
"#
        )
        .unwrap();

        let config = load_jupytext_configuration_file(&config_path).unwrap();
        assert_eq!(config.formats.as_deref(), Some("ipynb,py:percent"));
        assert_eq!(
            config.notebook_metadata_filter.as_deref(),
            Some("-all")
        );
        assert_eq!(config.comment_magics, Some(true));
        assert_eq!(
            config.cell_markers.as_deref(),
            Some("region,endregion")
        );
    }

    #[test]
    fn test_parse_pyproject_toml() {
        let dir = tempfile::tempdir().unwrap();
        let config_path = dir.path().join("pyproject.toml");
        let mut f = fs::File::create(&config_path).unwrap();
        writeln!(
            f,
            r#"
[tool.jupytext]
formats = "ipynb,py:light"
split_at_heading = true
"#
        )
        .unwrap();

        let config = load_jupytext_configuration_file(&config_path).unwrap();
        assert_eq!(config.formats.as_deref(), Some("ipynb,py:light"));
        assert_eq!(config.split_at_heading, Some(true));
    }

    #[test]
    fn test_find_config_in_dir() {
        let dir = tempfile::tempdir().unwrap();
        let config_path = dir.path().join("jupytext.toml");
        fs::write(&config_path, "formats = \"ipynb,py\"").unwrap();

        let found = find_jupytext_configuration_file_in_dir(dir.path());
        assert!(found.is_some());
        assert_eq!(found.unwrap(), config_path);
    }

    #[test]
    fn test_no_config_in_empty_dir() {
        let dir = tempfile::tempdir().unwrap();
        let found = find_jupytext_configuration_file_in_dir(dir.path());
        assert!(found.is_none());
    }

    #[test]
    fn test_effective_formats() {
        let mut config = JupytextConfig::default();
        assert!(config.effective_formats().is_none());

        config.formats = Some("ipynb,py".to_string());
        assert_eq!(config.effective_formats(), Some("ipynb,py"));

        // formats takes precedence over deprecated field
        config.default_jupytext_formats = Some("ipynb,md".to_string());
        assert_eq!(config.effective_formats(), Some("ipynb,py"));

        // deprecated field used as fallback
        config.formats = None;
        assert_eq!(config.effective_formats(), Some("ipynb,md"));
    }

    #[test]
    fn test_set_default_format_options() {
        let mut config = JupytextConfig::default();
        config.comment_magics = Some(true);
        config.cell_markers = Some("{{{,}}}".to_string());
        config.notebook_metadata_filter = Some("-all".to_string());

        let mut opts = BTreeMap::new();
        config.set_default_format_options(&mut opts, false);

        assert_eq!(opts.get("comment_magics"), Some(&Value::Bool(true)));
        assert_eq!(
            opts.get("cell_markers"),
            Some(&Value::String("{{{,}}}".to_string()))
        );
        assert_eq!(
            opts.get("notebook_metadata_filter"),
            Some(&Value::String("-all".to_string()))
        );
    }

    #[test]
    fn test_set_default_format_options_read_mode() {
        let mut config = JupytextConfig::default();
        config.cell_markers = Some("region,endregion".to_string());
        config.sphinx_convert_rst2md = Some(true);

        let mut opts = BTreeMap::new();
        config.set_default_format_options(&mut opts, true);

        // cell_markers should NOT be set in read mode
        assert!(!opts.contains_key("cell_markers"));
        // rst2md should be set in read mode
        assert_eq!(opts.get("rst2md"), Some(&Value::Bool(true)));
    }
}
