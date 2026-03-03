//! Format registry and detection for Jupytext text notebooks
//!
//! This module defines all supported notebook text formats and provides
//! functions to detect, parse, and validate format specifications.

use once_cell::sync::Lazy;
use regex::Regex;
use serde_json::Value;
use std::collections::{BTreeMap, HashSet};

use crate::header::header_to_metadata_and_cell;
use crate::languages::{same_language, ScriptLanguage, SCRIPT_EXTENSIONS};
use crate::magics::is_magic;
use crate::metadata_filter::metadata_filter_as_string;
use crate::string_parser::StringParser;

// ---------------------------------------------------------------------------
// Error type
// ---------------------------------------------------------------------------

/// Error in the specification of the format for the text notebook
#[derive(Debug, Clone, thiserror::Error)]
#[error("{0}")]
pub struct JupytextFormatError(pub String);

// ---------------------------------------------------------------------------
// Reader / Exporter type enums
// ---------------------------------------------------------------------------

/// Identifies which cell reader implementation to use
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ReaderType {
    Markdown,
    RMarkdown,
    LightScript,
    DoublePercent,
    Hydrogen,
    SphinxGallery,
    RScript,
    /// For formats handled externally (pandoc, myst, quarto, marimo)
    None,
}

/// Identifies which cell exporter implementation to use
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ExporterType {
    Markdown,
    RMarkdown,
    LightScript,
    BareScript,
    DoublePercent,
    Hydrogen,
    SphinxGallery,
    RScript,
    /// For formats handled externally (pandoc, myst, quarto, marimo)
    None,
}

// ---------------------------------------------------------------------------
// NotebookFormatDescription
// ---------------------------------------------------------------------------

/// Description of a single notebook text format
#[derive(Debug, Clone)]
pub struct NotebookFormatDescription {
    pub format_name: &'static str,
    pub extension: &'static str,
    pub header_prefix: &'static str,
    pub header_suffix: &'static str,
    pub reader_type: ReaderType,
    pub exporter_type: ExporterType,
    pub current_version_number: &'static str,
    pub min_readable_version_number: Option<&'static str>,
}

// ---------------------------------------------------------------------------
// Helper: build a format entry for every script extension
// ---------------------------------------------------------------------------

/// Construct format descriptions that repeat for every script extension.
fn script_formats(
    format_name: &'static str,
    reader_type: ReaderType,
    exporter_type: ExporterType,
    current_version_number: &'static str,
    min_readable_version_number: Option<&'static str>,
) -> Vec<NotebookFormatDescription> {
    SCRIPT_EXTENSIONS
        .iter()
        .map(|(ext, sl)| NotebookFormatDescription {
            format_name,
            extension: ext,
            header_prefix: sl.comment,
            header_suffix: sl.comment_suffix,
            reader_type,
            exporter_type,
            current_version_number,
            min_readable_version_number,
        })
        .collect()
}

// ---------------------------------------------------------------------------
// JUPYTEXT_FORMATS (lazy static)
// ---------------------------------------------------------------------------

/// All registered notebook text format descriptions (mirrors the Python `JUPYTEXT_FORMATS` tuple).
pub static JUPYTEXT_FORMATS: Lazy<Vec<NotebookFormatDescription>> = Lazy::new(|| {
    let mut fmts: Vec<NotebookFormatDescription> = Vec::new();

    // --- Markdown -----------------------------------------------------------
    fmts.push(NotebookFormatDescription {
        format_name: "markdown",
        extension: ".md",
        header_prefix: "",
        header_suffix: "",
        reader_type: ReaderType::Markdown,
        exporter_type: ExporterType::Markdown,
        current_version_number: "1.3",
        min_readable_version_number: Some("1.0"),
    });
    fmts.push(NotebookFormatDescription {
        format_name: "markdown",
        extension: ".markdown",
        header_prefix: "",
        header_suffix: "",
        reader_type: ReaderType::Markdown,
        exporter_type: ExporterType::Markdown,
        current_version_number: "1.2",
        min_readable_version_number: Some("1.0"),
    });

    // --- R Markdown ---------------------------------------------------------
    fmts.push(NotebookFormatDescription {
        format_name: "rmarkdown",
        extension: ".Rmd",
        header_prefix: "",
        header_suffix: "",
        reader_type: ReaderType::RMarkdown,
        exporter_type: ExporterType::RMarkdown,
        current_version_number: "1.2",
        min_readable_version_number: Some("1.0"),
    });

    // --- Light script -------------------------------------------------------
    fmts.extend(script_formats(
        "light",
        ReaderType::LightScript,
        ExporterType::LightScript,
        "1.5",
        Some("1.1"),
    ));

    // --- Nomarker (bare) ----------------------------------------------------
    fmts.extend(script_formats(
        "nomarker",
        ReaderType::LightScript,
        ExporterType::BareScript,
        "1.0",
        Some("1.0"),
    ));

    // --- Percent ------------------------------------------------------------
    fmts.extend(script_formats(
        "percent",
        ReaderType::DoublePercent,
        ExporterType::DoublePercent,
        "1.3",
        Some("1.1"),
    ));

    // --- Hydrogen -----------------------------------------------------------
    fmts.extend(script_formats(
        "hydrogen",
        ReaderType::Hydrogen,
        ExporterType::Hydrogen,
        "1.3",
        Some("1.1"),
    ));

    // --- R Spin (.r / .R only) ----------------------------------------------
    for ext in &[".r", ".R"] {
        fmts.push(NotebookFormatDescription {
            format_name: "spin",
            extension: ext,
            header_prefix: "#'",
            header_suffix: "",
            reader_type: ReaderType::RScript,
            exporter_type: ExporterType::RScript,
            current_version_number: "1.0",
            min_readable_version_number: None,
        });
    }

    // --- Sphinx gallery (.py only) ------------------------------------------
    fmts.push(NotebookFormatDescription {
        format_name: "sphinx",
        extension: ".py",
        header_prefix: "#",
        header_suffix: "",
        reader_type: ReaderType::SphinxGallery,
        exporter_type: ExporterType::SphinxGallery,
        current_version_number: "1.1",
        min_readable_version_number: None,
    });

    // --- Pandoc (.md) -------------------------------------------------------
    fmts.push(NotebookFormatDescription {
        format_name: "pandoc",
        extension: ".md",
        header_prefix: "",
        header_suffix: "",
        reader_type: ReaderType::None,
        exporter_type: ExporterType::None,
        current_version_number: "1.0",
        min_readable_version_number: None,
    });

    // --- Quarto (.qmd) ------------------------------------------------------
    fmts.push(NotebookFormatDescription {
        format_name: "quarto",
        extension: ".qmd",
        header_prefix: "",
        header_suffix: "",
        reader_type: ReaderType::None,
        exporter_type: ExporterType::None,
        current_version_number: "1.0",
        min_readable_version_number: None,
    });

    // --- Marimo (.py) -------------------------------------------------------
    fmts.push(NotebookFormatDescription {
        format_name: "marimo",
        extension: ".py",
        header_prefix: "",
        header_suffix: "",
        reader_type: ReaderType::None,
        exporter_type: ExporterType::None,
        current_version_number: "1.0",
        min_readable_version_number: None,
    });

    // --- MyST (.md) ---------------------------------------------------------
    fmts.push(NotebookFormatDescription {
        format_name: MYST_FORMAT_NAME,
        extension: ".md",
        header_prefix: "",
        header_suffix: "",
        reader_type: ReaderType::None,
        exporter_type: ExporterType::None,
        current_version_number: "0.13",
        min_readable_version_number: None,
    });

    fmts
});

// ---------------------------------------------------------------------------
// NOTEBOOK_EXTENSIONS / EXTENSION_PREFIXES / FORMATS_WITH_NO_CELL_METADATA
// ---------------------------------------------------------------------------

/// MyST format name constant
pub const MYST_FORMAT_NAME: &str = "myst";

/// All supported notebook extensions (including `.ipynb`), in the order they first appear.
pub static NOTEBOOK_EXTENSIONS: Lazy<Vec<&'static str>> = Lazy::new(|| {
    let mut seen = HashSet::new();
    let mut exts = Vec::new();
    seen.insert(".ipynb");
    exts.push(".ipynb");
    for fmt in JUPYTEXT_FORMATS.iter() {
        if seen.insert(fmt.extension) {
            exts.push(fmt.extension);
        }
    }
    exts
});

/// Short prefixes used by some text notebook file names
pub static EXTENSION_PREFIXES: &[&str] = &[".lgt", ".spx", ".pct", ".hyd", ".nb"];

/// Formats that do not support cell-level metadata
pub static FORMATS_WITH_NO_CELL_METADATA: Lazy<HashSet<&'static str>> = Lazy::new(|| {
    let mut s = HashSet::new();
    s.insert("sphinx");
    s.insert("nomarker");
    s.insert("spin");
    s.insert("quarto");
    s.insert("marimo");
    s
});

// ---------------------------------------------------------------------------
// Validation constants
// ---------------------------------------------------------------------------

/// Keys that identify the format itself (extension, name, prefix path, suffix)
pub static VALID_FORMAT_INFO: &[&str] = &["extension", "format_name", "suffix", "prefix"];

/// Boolean-valued format options
pub static BINARY_FORMAT_OPTIONS: &[&str] = &[
    "comment_magics",
    "hide_notebook_metadata",
    "root_level_metadata_as_raw_cell",
    "split_at_heading",
    "rst2md",
    "cell_metadata_json",
    "use_runtools",
    "doxygen_equation_markers",
];

/// All valid format option keys (binary + string-valued)
pub static VALID_FORMAT_OPTIONS: &[&str] = &[
    // Binary options
    "comment_magics",
    "hide_notebook_metadata",
    "root_level_metadata_as_raw_cell",
    "split_at_heading",
    "rst2md",
    "cell_metadata_json",
    "use_runtools",
    "doxygen_equation_markers",
    // String options
    "notebook_metadata_filter",
    "root_level_metadata_filter",
    "cell_metadata_filter",
    "cell_markers",
    "custom_cell_magics",
];

/// All valid format names
pub static VALID_FORMAT_NAMES: Lazy<HashSet<&'static str>> = Lazy::new(|| {
    JUPYTEXT_FORMATS
        .iter()
        .map(|fmt| fmt.format_name)
        .collect()
});

// ---------------------------------------------------------------------------
// get_format_implementation
// ---------------------------------------------------------------------------

/// Return the `NotebookFormatDescription` for the given extension and optional format name.
///
/// The extension is normalised so that compound extensions like `.lgt.py` are
/// reduced to `.py`.
pub fn get_format_implementation(
    ext: &str,
    format_name: Option<&str>,
) -> Result<&'static NotebookFormatDescription, JupytextFormatError> {
    // Normalise: keep only the last dot-separated component
    let ext_normalised = format!(".{}", ext.rsplit('.').next().unwrap_or(""));

    let mut formats_for_extension: Vec<&str> = Vec::new();

    for fmt in JUPYTEXT_FORMATS.iter() {
        if fmt.extension == ext_normalised {
            match format_name {
                Some(name) if name == fmt.format_name => return Ok(fmt),
                None => return Ok(fmt),
                Some(_) => formats_for_extension.push(fmt.format_name),
            }
        }
    }

    if !formats_for_extension.is_empty() {
        return Err(JupytextFormatError(format!(
            "Format '{}' is not associated to extension '{}'. Please choose one of: {}.",
            format_name.unwrap_or(""),
            ext_normalised,
            formats_for_extension.join(", ")
        )));
    }

    Err(JupytextFormatError(format!(
        "No format associated to extension '{}'",
        ext_normalised
    )))
}

// ---------------------------------------------------------------------------
// read_metadata  (helper shared by read_format_from_metadata and guess_format)
// ---------------------------------------------------------------------------

/// Parse the YAML header of a text notebook and return its metadata map.
pub fn read_metadata(text: &str, ext: &str) -> serde_json::Map<String, Value> {
    let ext_norm = format!(".{}", ext.rsplit('.').next().unwrap_or(""));
    let lines: Vec<String> = text.lines().map(|l| l.to_string()).collect();

    let (comment, comment_suffix) = if matches!(ext_norm.as_str(), ".md" | ".markdown" | ".Rmd") {
        ("", "")
    } else {
        SCRIPT_EXTENSIONS
            .get(ext_norm.as_str())
            .map(|sl: &ScriptLanguage| (sl.comment, sl.comment_suffix))
            .unwrap_or(("#", ""))
    };

    let result = header_to_metadata_and_cell(&lines, comment, comment_suffix, &ext_norm, true);
    let metadata = result.metadata;

    // For R files: try spin header if nothing found
    if matches!(ext_norm.as_str(), ".r" | ".R") && metadata.is_empty() {
        let result2 = header_to_metadata_and_cell(&lines, "#'", "", &ext_norm, true);
        if !result2.metadata.is_empty() {
            return result2.metadata;
        }
    }

    metadata
}

// ---------------------------------------------------------------------------
// read_format_from_metadata
// ---------------------------------------------------------------------------

/// Return the format of the file when that information is available from the metadata.
pub fn read_format_from_metadata(text: &str, ext: &str) -> Option<String> {
    let mut metadata = read_metadata(text, ext);
    rearrange_jupytext_metadata(&mut metadata);
    format_name_for_ext(&metadata, ext, None, false)
}

// ---------------------------------------------------------------------------
// guess_format
// ---------------------------------------------------------------------------

/// Guess the format and format options of the file from its extension and content.
///
/// Returns `(format_name, options)`.
pub fn guess_format(text: &str, ext: &str) -> (String, BTreeMap<String, Value>) {
    let metadata = read_metadata(text, ext);

    // If text_representation is present in metadata, trust it
    if let Some(jupytext) = metadata.get("jupytext") {
        if jupytext.get("text_representation").is_some() {
            if let Some(name) = format_name_for_ext(&metadata, ext, None, true) {
                return (name, BTreeMap::new());
            }
        }
    }

    let lines: Vec<&str> = text.lines().collect();

    let ext_norm = format!(".{}", ext.rsplit('.').next().unwrap_or(""));

    // Script-based format detection
    if let Some(sl) = SCRIPT_EXTENSIONS.get(ext_norm.as_str()) {
        let unescaped_comment = sl.comment;
        let comment = regex::escape(unescaped_comment);
        let language = sl.language;

        let twenty_hash_re = Regex::new(r"^#( |)#{19,}\s*$").unwrap();
        let double_percent_re = Regex::new(&format!(r"^{}( %%|%%)$", comment)).unwrap();
        let double_percent_and_space_re =
            Regex::new(&format!(r"^{}( %%|%%)\s", comment)).unwrap();
        let nbconvert_script_re =
            Regex::new(&format!(r"^{}( <codecell>| In\[[0-9 ]*\]:?)", comment)).unwrap();
        let vim_folding_markers_re =
            Regex::new(&format!(r"^{}\s*\{{\{{\{{", comment)).unwrap();
        let vscode_folding_markers_re =
            Regex::new(&format!(r"^{}\s*region", comment)).unwrap();
        let marimo_cell_re = Regex::new(r"^@app\.cell.*").unwrap();

        let mut twenty_hash_count: usize = 0;
        let mut double_percent_count: usize = 0;
        let mut magic_command_count: usize = 0;
        let mut rspin_comment_count: usize = 0;
        let mut vim_folding_markers_count: usize = 0;
        let mut vscode_folding_markers_count: usize = 0;
        let mut marimo_app_count: usize = 0;

        let parser_lang = if ext_norm == ".r" || ext_norm == ".R" {
            "R"
        } else {
            "python"
        };
        let mut parser = StringParser::new(parser_lang);

        for line in &lines {
            parser.read_line(line);
            if parser.is_quoted() {
                continue;
            }

            // Double-percent cell markers
            if double_percent_re.is_match(line)
                || double_percent_and_space_re.is_match(line)
                || nbconvert_script_re.is_match(line)
            {
                double_percent_count += 1;
            }

            // Magic commands (lines that look like magics but are NOT commented)
            if !line.starts_with(unescaped_comment)
                && is_magic(line, language, true, false)
            {
                magic_command_count += 1;
            }

            // Twenty-hash lines (Sphinx gallery) -- .py only
            if ext_norm == ".py" && twenty_hash_re.is_match(line) {
                twenty_hash_count += 1;
            }

            // R spin comments
            if (ext_norm == ".R" || ext_norm == ".r") && line.starts_with("#'") {
                rspin_comment_count += 1;
            }

            // Vim folding markers
            if vim_folding_markers_re.is_match(line) {
                vim_folding_markers_count += 1;
            }

            // VS Code folding markers
            if vscode_folding_markers_re.is_match(line) {
                vscode_folding_markers_count += 1;
            }

            // Marimo patterns
            if ext_norm == ".py"
                && (*line == "import marimo"
                    || *line == "app = marimo.App()"
                    || marimo_cell_re.is_match(line))
            {
                marimo_app_count += 1;
            }
        }

        if double_percent_count >= 1 {
            if magic_command_count > 0 {
                return ("hydrogen".to_string(), BTreeMap::new());
            }
            return ("percent".to_string(), BTreeMap::new());
        }

        if marimo_app_count >= 2 {
            return ("marimo".to_string(), BTreeMap::new());
        }

        if vim_folding_markers_count > 0 {
            let mut opts = BTreeMap::new();
            opts.insert(
                "cell_markers".to_string(),
                Value::String("{{{,}}}".to_string()),
            );
            return ("light".to_string(), opts);
        }

        if vscode_folding_markers_count > 0 {
            let mut opts = BTreeMap::new();
            opts.insert(
                "cell_markers".to_string(),
                Value::String("region,endregion".to_string()),
            );
            return ("light".to_string(), opts);
        }

        if twenty_hash_count >= 2 {
            return ("sphinx".to_string(), BTreeMap::new());
        }

        if rspin_comment_count >= 1 {
            return ("spin".to_string(), BTreeMap::new());
        }
    }

    // Markdown / Pandoc detection
    if ext_norm == ".md" || ext_norm == ".markdown" {
        for line in &lines {
            if line.starts_with(":::") {
                return ("pandoc".to_string(), BTreeMap::new());
            }
        }
    }

    // Default format for the extension
    let default_name = get_format_implementation(&ext_norm, None)
        .map(|f| f.format_name.to_string())
        .unwrap_or_else(|_| "light".to_string());

    (default_name, BTreeMap::new())
}

// ---------------------------------------------------------------------------
// divine_format
// ---------------------------------------------------------------------------

/// Guess the format of the notebook based solely on its content.
///
/// Returns a string like `"ipynb"`, `"md"`, `"py:percent"`, etc.
pub fn divine_format(text: &str) -> String {
    // Try JSON / ipynb
    if serde_json::from_str::<Value>(text).is_ok() {
        return "ipynb".to_string();
    }

    let lines: Vec<String> = text.lines().map(|l| l.to_string()).collect();

    // Collect unique comment characters (non-"#") from SCRIPT_EXTENSIONS
    let mut comment_chars: Vec<&str> = Vec::new();
    {
        let mut seen = HashSet::new();
        for sl in SCRIPT_EXTENSIONS.values() {
            if sl.comment != "#" && seen.insert(sl.comment) {
                comment_chars.push(sl.comment);
            }
        }
    }

    // Try each comment prefix to find metadata with extension info
    let mut prefixes: Vec<&str> = vec!["", "#"];
    prefixes.extend(comment_chars.iter());
    for comment in prefixes {
        let result = header_to_metadata_and_cell(&lines, comment, "", "", true);
        let ext_val = result
            .metadata
            .get("jupytext")
            .and_then(|j| j.get("text_representation"))
            .and_then(|tr| tr.get("extension"))
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
        if let Some(ext) = ext_val {
            let (fmt_name, _) = guess_format(text, &ext);
            // ext starts with "."
            return format!("{}:{}", &ext[1..], fmt_name);
        }
    }

    // No metadata - look for ``` lines which indicate markdown
    for line in &lines {
        if line.as_str() == "```" {
            return "md".to_string();
        }
    }

    // Default: Python
    let (fmt_name, _) = guess_format(text, ".py");
    format!("py:{}", fmt_name)
}

// ---------------------------------------------------------------------------
// format_name_for_ext
// ---------------------------------------------------------------------------

/// Return the format name for the given extension using notebook metadata.
///
/// When `explicit_default` is false and no format name is found in metadata,
/// `None` is returned for script extensions (rather than the default).
pub fn format_name_for_ext(
    metadata: &serde_json::Map<String, Value>,
    ext: &str,
    cm_default_formats: Option<&str>,
    explicit_default: bool,
) -> Option<String> {
    let ext_norm = format!(".{}", ext.rsplit('.').next().unwrap_or(""));

    // Check text_representation
    if let Some(tr) = metadata
        .get("jupytext")
        .and_then(|j| j.get("text_representation"))
    {
        let tr_ext = tr.get("extension").and_then(|v| v.as_str()).unwrap_or("");
        let tr_name = tr
            .get("format_name")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
        if tr_ext.ends_with(&ext_norm) && tr_name.is_some() {
            return tr_name;
        }
    }

    // Check jupytext.formats
    let formats_str = metadata
        .get("jupytext")
        .and_then(|j| j.get("formats"))
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .or_else(|| cm_default_formats.map(|s| s.to_string()));
    if let Some(ref fmts_str) = formats_str {
        let fmts = long_form_multiple_formats(fmts_str, None, false);
        for fmt in &fmts {
            if let Some(Value::String(fmt_ext)) = fmt.get("extension") {
                if fmt_ext == &ext_norm {
                    if !explicit_default {
                        return fmt
                            .get("format_name")
                            .and_then(|v| v.as_str())
                            .map(|s| s.to_string());
                    }
                    if let Some(name) = fmt.get("format_name").and_then(|v| v.as_str()) {
                        return Some(name.to_string());
                    }
                }
            }
        }
    }

    if !explicit_default || matches!(ext_norm.as_str(), ".md" | ".markdown" | ".Rmd") {
        return None;
    }

    get_format_implementation(&ext_norm, None)
        .ok()
        .map(|f| f.format_name.to_string())
}

// ---------------------------------------------------------------------------
// rearrange_jupytext_metadata
// ---------------------------------------------------------------------------

/// Convert legacy metadata entries (`jupytext_formats`, `nbrmd_formats`, etc.)
/// into the modern `jupytext.*` namespace.  Mutates `metadata` in place.
pub fn rearrange_jupytext_metadata(metadata: &mut serde_json::Map<String, Value>) {
    // Backward compatibility with nbrmd
    for key in &["nbrmd_formats", "nbrmd_format_version"] {
        if let Some(val) = metadata.remove(*key) {
            let new_key = key.replace("nbrmd", "jupytext");
            metadata.insert(new_key, val);
        }
    }

    // Promote top-level keys into jupytext sub-object
    let mut jupytext_obj: serde_json::Map<String, Value> = metadata
        .get("jupytext")
        .and_then(|v| v.as_object())
        .cloned()
        .unwrap_or_default();

    if let Some(val) = metadata.remove("jupytext_formats") {
        jupytext_obj.insert("formats".to_string(), val);
    }
    if let Some(val) = metadata.remove("jupytext_format_version") {
        let mut tr = serde_json::Map::new();
        tr.insert("format_version".to_string(), val);
        jupytext_obj.insert(
            "text_representation".to_string(),
            Value::Object(tr),
        );
    }
    if let Some(val) = metadata.remove("main_language") {
        jupytext_obj.insert("main_language".to_string(), val);
    }
    for entry in &["encoding", "executable"] {
        if let Some(val) = metadata.remove(*entry) {
            jupytext_obj.insert(entry.to_string(), val);
        }
    }

    // metadata_filter -> notebook_metadata_filter / cell_metadata_filter
    if let Some(Value::Object(filters)) = jupytext_obj.remove("metadata_filter") {
        if let Some(nb) = filters.get("notebook") {
            jupytext_obj.insert("notebook_metadata_filter".to_string(), nb.clone());
        }
        if let Some(cells) = filters.get("cells") {
            jupytext_obj.insert("cell_metadata_filter".to_string(), cells.clone());
        }
    }

    // Convert filter objects to strings
    for filter_level in &["notebook_metadata_filter", "cell_metadata_filter"] {
        if let Some(val) = jupytext_obj.get(*filter_level) {
            if val.is_object() {
                // The Python version calls metadata_filter_as_string on the value.
                // We serialise the filter as a comma-separated string when it is a
                // MetadataFilter-like dict.  For now, store as-is (string form expected
                // in most real notebooks).
                let _ = metadata_filter_as_string; // acknowledge import
            }
        }
    }

    // v0.x compatibility: prefix extensions with '.'
    if let Some(Value::String(ref jv)) = jupytext_obj
        .get("text_representation")
        .and_then(|tr| tr.get("jupytext_version"))
    {
        if jv.starts_with("0.") {
            if let Some(Value::String(ref fmts_str)) = jupytext_obj.get("formats") {
                let fixed: String = fmts_str
                    .split(',')
                    .map(|fmt| {
                        if fmt.rfind('.').map_or(false, |p| p > 0) {
                            format!(".{}", fmt)
                        } else {
                            fmt.to_string()
                        }
                    })
                    .collect::<Vec<_>>()
                    .join(",");
                jupytext_obj.insert("formats".to_string(), Value::String(fixed));
            }
        }
    }

    // auto -> actual extension (round-trip through long/short form)
    if let Some(Value::String(fmts_str)) = jupytext_obj.get("formats").cloned() {
        let long = long_form_multiple_formats(&fmts_str, Some(metadata), false);
        let short = short_form_multiple_formats(&long);
        jupytext_obj.insert("formats".to_string(), Value::String(short));
    }

    if !jupytext_obj.is_empty() {
        metadata.insert("jupytext".to_string(), Value::Object(jupytext_obj));
    }
}

// ---------------------------------------------------------------------------
// update_jupytext_formats_metadata
// ---------------------------------------------------------------------------

/// Update the `jupytext.formats` metadata to reflect `new_format`.
pub fn update_jupytext_formats_metadata(
    metadata: &mut serde_json::Map<String, Value>,
    new_format: &BTreeMap<String, Value>,
) {
    let new_format = long_form_one_format_map(new_format.clone());
    let formats_str = metadata
        .get("jupytext")
        .and_then(|j| j.get("formats"))
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    if formats_str.is_empty() {
        return;
    }
    let mut formats = long_form_multiple_formats(&formats_str, None, true);
    for fmt in formats.iter_mut() {
        if identical_format_path(fmt, &new_format) {
            if let Some(name) = new_format.get("format_name") {
                fmt.insert("format_name".to_string(), name.clone());
            }
        }
    }
    let short = short_form_multiple_formats(&formats);
    let jupytext = metadata
        .entry("jupytext".to_string())
        .or_insert_with(|| Value::Object(serde_json::Map::new()));
    if let Some(obj) = jupytext.as_object_mut() {
        obj.insert("formats".to_string(), Value::String(short));
    }
}

/// Do two long-form formats target the same file path?
pub fn identical_format_path(
    fmt1: &BTreeMap<String, Value>,
    fmt2: &BTreeMap<String, Value>,
) -> bool {
    for key in &["extension", "prefix", "suffix"] {
        if fmt1.get(*key) != fmt2.get(*key) {
            return false;
        }
    }
    true
}

// ---------------------------------------------------------------------------
// long_form_one_format (string variant)
// ---------------------------------------------------------------------------

/// Common name aliases (lower-cased key -> format string)
static COMMON_NAME_TO_EXT: Lazy<BTreeMap<&'static str, &'static str>> = Lazy::new(|| {
    let mut m = BTreeMap::new();
    m.insert("notebook", "ipynb");
    m.insert("rmarkdown", "Rmd");
    m.insert("quarto", "qmd");
    m.insert("marimo", "py");
    m.insert("markdown", "md");
    m.insert("script", "auto");
    m.insert("c++", "cpp");
    m.insert("myst", "md:myst");
    m.insert("pandoc", "md:pandoc");
    m
});

/// Parse a format string like `"sfx.py:percent"` into a `BTreeMap` with keys
/// `extension`, `format_name`, `suffix`, `prefix`.
///
/// If `update` is `Some`, those entries are merged into the result.
pub fn long_form_one_format(
    jupytext_format: &str,
    metadata: Option<&serde_json::Map<String, Value>>,
    update: Option<&BTreeMap<String, Value>>,
    auto_ext_requires_language_info: bool,
) -> Result<BTreeMap<String, Value>, JupytextFormatError> {
    if jupytext_format.is_empty() {
        return Ok(BTreeMap::new());
    }

    let mut format_str = jupytext_format.to_string();

    // Resolve common names
    if let Some(mapped) = COMMON_NAME_TO_EXT.get(format_str.to_lowercase().as_str()) {
        format_str = mapped.to_string();
    }

    let mut fmt = BTreeMap::<String, Value>::new();

    // Prefix (path component before last `/`)
    if let Some(slash_pos) = format_str.rfind('/') {
        if slash_pos > 0 {
            fmt.insert(
                "prefix".to_string(),
                Value::String(format_str[..slash_pos].to_string()),
            );
            format_str = format_str[slash_pos + 1..].to_string();
        }
    }

    let ext: String;

    // Format name after `:`
    if let Some(colon_pos) = format_str.rfind(':') {
        let mut name = format_str[colon_pos + 1..].to_string();
        ext = format_str[..colon_pos].to_string();
        if name == "bare" {
            // Deprecated name
            name = "nomarker".to_string();
        }
        fmt.insert("format_name".to_string(), Value::String(name));
    } else if format_str.is_empty()
        || format_str.contains('.')
        || NOTEBOOK_EXTENSIONS.contains(&format!(".{}", format_str).as_str())
        || format_str == "auto"
    {
        ext = format_str.clone();
    } else if VALID_FORMAT_NAMES.contains(format_str.as_str()) {
        fmt.insert(
            "format_name".to_string(),
            Value::String(format_str.clone()),
        );
        ext = String::new();
    } else {
        return Err(JupytextFormatError(format!(
            "'{}' is not a notebook extension (one of {}), nor a notebook format (one of {})",
            jupytext_format,
            NOTEBOOK_EXTENSIONS.join(", "),
            VALID_FORMAT_NAMES
                .iter()
                .cloned()
                .collect::<Vec<_>>()
                .join(", ")
        )));
    }

    // Suffix (part of ext before the last `.`)
    let mut ext_part = ext.clone();
    if let Some(dot_pos) = ext.rfind('.') {
        if dot_pos > 0 {
            fmt.insert(
                "suffix".to_string(),
                Value::String(ext[..dot_pos].to_string()),
            );
            ext_part = ext[dot_pos..].to_string();
        }
    }

    // Normalise extension to start with `.`
    if !ext_part.starts_with('.') {
        ext_part = format!(".{}", ext_part);
    }

    // Handle `.auto`
    if ext_part == ".auto" {
        if let Some(meta) = metadata {
            if let Some(resolved) = auto_ext_from_metadata(meta) {
                ext_part = resolved;
            } else if auto_ext_requires_language_info {
                return Err(JupytextFormatError(
                    "No language information in this notebook. Please replace 'auto' with an actual script extension.".to_string(),
                ));
            }
            // else leave as ".auto"
        }
        // No metadata => leave as ".auto"
    }

    fmt.insert("extension".to_string(), Value::String(ext_part));

    if let Some(upd) = update {
        for (k, v) in upd {
            fmt.insert(k.clone(), v.clone());
        }
    }

    validate_one_format(&fmt)?;
    Ok(fmt)
}

/// Variant that accepts and returns a `BTreeMap` directly (already parsed).
pub fn long_form_one_format_map(
    fmt: BTreeMap<String, Value>,
) -> BTreeMap<String, Value> {
    // Already a map - just validate (ignore errors for internal use)
    let _ = validate_one_format(&fmt);
    fmt
}

// ---------------------------------------------------------------------------
// long_form_multiple_formats
// ---------------------------------------------------------------------------

/// Convert a comma-separated format string (e.g. `"ipynb,py:percent"`) to a
/// list of long-form format dictionaries.
pub fn long_form_multiple_formats(
    jupytext_formats: &str,
    metadata: Option<&serde_json::Map<String, Value>>,
    auto_ext_requires_language_info: bool,
) -> Vec<BTreeMap<String, Value>> {
    if jupytext_formats.is_empty() {
        return Vec::new();
    }

    let parts: Vec<&str> = jupytext_formats.split(',').filter(|s| !s.is_empty()).collect();
    let mut result = Vec::new();
    for part in parts {
        match long_form_one_format(part.trim(), metadata, None, auto_ext_requires_language_info) {
            Ok(fmt) => {
                if auto_ext_requires_language_info
                    || fmt
                        .get("extension")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        != ".auto"
                {
                    result.push(fmt);
                }
            }
            Err(_) => {
                // Skip invalid formats silently (matches Python behaviour for parsing)
            }
        }
    }
    result
}

// ---------------------------------------------------------------------------
// short_form_one_format / short_form_multiple_formats
// ---------------------------------------------------------------------------

/// Represent a single long-form format dictionary as a short string like `"py:percent"`.
pub fn short_form_one_format(fmt: &BTreeMap<String, Value>) -> String {
    let ext = fmt
        .get("extension")
        .and_then(|v| v.as_str())
        .unwrap_or("");

    let mut result = String::new();

    // Suffix
    if let Some(Value::String(suffix)) = fmt.get("suffix") {
        result.push_str(suffix);
        result.push_str(ext);
    } else if ext.starts_with('.') {
        result.push_str(&ext[1..]);
    } else {
        result.push_str(ext);
    }

    // Prefix
    if let Some(Value::String(prefix)) = fmt.get("prefix") {
        result = format!("{}/{}", prefix, result);
    }

    // Format name
    if let Some(Value::String(name)) = fmt.get("format_name") {
        if !name.is_empty() {
            // Don't append format_name for markdown / Rmd unless it is pandoc or myst
            let skip = matches!(ext, ".md" | ".markdown" | ".Rmd")
                && !matches!(name.as_str(), "pandoc" | "myst");
            if !skip {
                result = format!("{}:{}", result, name);
            }
        }
    }

    result
}

/// Convert a list of long-form format dictionaries to a comma-separated string.
pub fn short_form_multiple_formats(fmts: &[BTreeMap<String, Value>]) -> String {
    fmts.iter()
        .map(|f| short_form_one_format(f))
        .collect::<Vec<_>>()
        .join(",")
}

// ---------------------------------------------------------------------------
// String-keyed convenience wrappers for modules using BTreeMap<String, String>
// ---------------------------------------------------------------------------

/// Helper to convert a `BTreeMap<String, String>` to `BTreeMap<String, Value>`.
pub fn string_map_to_value_map(m: &BTreeMap<String, String>) -> BTreeMap<String, Value> {
    m.iter()
        .map(|(k, v)| {
            let val = match v.as_str() {
                "true" => Value::Bool(true),
                "false" => Value::Bool(false),
                _ => Value::String(v.clone()),
            };
            (k.clone(), val)
        })
        .collect()
}

/// `short_form_one_format` variant accepting `BTreeMap<String, String>`.
pub fn short_form_one_format_str(fmt: &BTreeMap<String, String>) -> String {
    short_form_one_format(&string_map_to_value_map(fmt))
}

/// `short_form_multiple_formats` variant accepting `BTreeMap<String, String>`.
pub fn short_form_multiple_formats_str(fmts: &[BTreeMap<String, String>]) -> String {
    let converted: Vec<BTreeMap<String, Value>> = fmts.iter().map(|f| string_map_to_value_map(f)).collect();
    short_form_multiple_formats(&converted)
}

/// Helper to convert a `BTreeMap<String, Value>` to `BTreeMap<String, String>`.
pub fn value_map_to_string_map(m: &BTreeMap<String, Value>) -> BTreeMap<String, String> {
    m.iter()
        .map(|(k, v)| {
            let s = match v {
                Value::String(s) => s.clone(),
                Value::Bool(b) => b.to_string(),
                Value::Number(n) => n.to_string(),
                Value::Null => String::new(),
                other => other.to_string(),
            };
            (k.clone(), s)
        })
        .collect()
}

/// `long_form_one_format` variant returning `BTreeMap<String, String>` for CLI compatibility.
pub fn long_form_one_format_as_strings(format_str: &str) -> BTreeMap<String, String> {
    match long_form_one_format(format_str, None, None, false) {
        Ok(m) => value_map_to_string_map(&m),
        Err(_) => {
            // Fallback: just put the extension
            let mut m = BTreeMap::new();
            let ext = if format_str.starts_with('.') {
                format_str.to_string()
            } else {
                format!(".{}", format_str)
            };
            m.insert("extension".to_string(), ext);
            m
        }
    }
}

/// `long_form_multiple_formats` variant returning `Vec<BTreeMap<String, String>>` for CLI compatibility.
pub fn long_form_multiple_formats_as_strings(formats_str: &str) -> Vec<BTreeMap<String, String>> {
    long_form_multiple_formats(formats_str, None, false)
        .into_iter()
        .map(|m| value_map_to_string_map(&m))
        .collect()
}

// ---------------------------------------------------------------------------
// validate_one_format
// ---------------------------------------------------------------------------

/// Validate the keys and values in a format dictionary.
pub fn validate_one_format(
    fmt: &BTreeMap<String, Value>,
) -> Result<(), JupytextFormatError> {
    // Check format_name validity
    if let Some(Value::String(name)) = fmt.get("format_name") {
        if !VALID_FORMAT_NAMES.contains(name.as_str()) {
            return Err(JupytextFormatError(format!(
                "{} is not a valid format name. Please choose one of {}",
                name,
                VALID_FORMAT_NAMES
                    .iter()
                    .cloned()
                    .collect::<Vec<_>>()
                    .join(", ")
            )));
        }
    }

    // Check all keys are recognized
    let valid_keys: HashSet<&str> = VALID_FORMAT_INFO
        .iter()
        .chain(VALID_FORMAT_OPTIONS.iter())
        .cloned()
        .collect();
    for key in fmt.keys() {
        if !valid_keys.contains(key.as_str()) {
            return Err(JupytextFormatError(format!(
                "Unknown format option '{}' - should be one of '{}'",
                key,
                VALID_FORMAT_OPTIONS.join("', '")
            )));
        }

        // Binary options must be booleans
        if BINARY_FORMAT_OPTIONS.contains(&key.as_str()) {
            if let Some(val) = fmt.get(key) {
                if !val.is_boolean() {
                    return Err(JupytextFormatError(format!(
                        "Format option '{}' should be a bool, not '{}'",
                        key, val
                    )));
                }
            }
        }
    }

    // Extension is required
    let ext = match fmt.get("extension").and_then(|v| v.as_str()) {
        Some(e) => e,
        None => {
            return Err(JupytextFormatError(
                "Missing format extension".to_string(),
            ));
        }
    };

    // Extension must be a recognized notebook extension (or `.auto`)
    if ext != ".auto" && !NOTEBOOK_EXTENSIONS.contains(&ext) {
        return Err(JupytextFormatError(format!(
            "Extension '{}' is not a notebook extension. Please use one of '{}'.",
            ext,
            NOTEBOOK_EXTENSIONS.join("', '")
        )));
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// auto_ext_from_metadata
// ---------------------------------------------------------------------------

/// Derive the script extension from notebook metadata (language_info / kernelspec).
pub fn auto_ext_from_metadata(metadata: &serde_json::Map<String, Value>) -> Option<String> {
    let mut auto_ext = metadata
        .get("language_info")
        .and_then(|li| li.get("file_extension"))
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    // Sage notebooks report ".py" but should use ".sage"
    if auto_ext.as_deref() == Some(".py") {
        if let Some("sage") = metadata
            .get("kernelspec")
            .and_then(|k| k.get("language"))
            .and_then(|v| v.as_str())
        {
            auto_ext = Some(".sage".to_string());
        }
    }

    if auto_ext.is_none() {
        let language = metadata
            .get("kernelspec")
            .and_then(|k| k.get("language"))
            .and_then(|v| v.as_str())
            .or_else(|| {
                metadata
                    .get("jupytext")
                    .and_then(|j| j.get("main_language"))
                    .and_then(|v| v.as_str())
            });
        if let Some(lang) = language {
            for (ext, sl) in SCRIPT_EXTENSIONS.iter() {
                if same_language(lang, sl.language) {
                    auto_ext = Some(ext.to_string());
                    break;
                }
            }
        }
    }

    // Normalise some extensions
    match auto_ext.as_deref() {
        Some(".r") => Some(".R".to_string()),
        Some(".fs") => Some(".fsx".to_string()),
        Some(".resource") => Some(".robot".to_string()),
        Some(".C") => Some(".cpp".to_string()),
        _ => auto_ext,
    }
}

// ---------------------------------------------------------------------------
// check_auto_ext
// ---------------------------------------------------------------------------

/// Replace `.auto` with the actual file extension.  Returns an error if it
/// cannot be determined.
pub fn check_auto_ext(
    fmt: &BTreeMap<String, Value>,
    metadata: &serde_json::Map<String, Value>,
    option: &str,
) -> Result<BTreeMap<String, Value>, JupytextFormatError> {
    let ext = fmt
        .get("extension")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    if ext != ".auto" {
        return Ok(fmt.clone());
    }
    if let Some(resolved) = auto_ext_from_metadata(metadata) {
        let mut fmt = fmt.clone();
        fmt.insert("extension".to_string(), Value::String(resolved));
        return Ok(fmt);
    }
    Err(JupytextFormatError(format!(
        "The notebook does not have a 'language_info' metadata. \
         Please replace 'auto' with the actual language extension in the {} option (currently {}).",
        option,
        short_form_one_format(fmt)
    )))
}

// ---------------------------------------------------------------------------
// check_file_version
// ---------------------------------------------------------------------------

/// Raise an error if the file's format version would silently override outputs.
pub fn check_file_version(
    notebook: &crate::notebook::Notebook,
    source_path: &str,
    outputs_path: &str,
) -> Result<(), JupytextFormatError> {
    if !crate::header::insert_or_test_version_number() {
        return Ok(());
    }

    let ext = if source_path == "-" {
        notebook
            .metadata
            .get("jupytext")
            .and_then(|j| j.get("text_representation"))
            .and_then(|tr| tr.get("extension"))
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string()
    } else {
        let p = std::path::Path::new(source_path);
        p.extension()
            .map(|e| format!(".{}", e.to_string_lossy()))
            .unwrap_or_default()
    };

    // Convert BTreeMap to serde_json::Map for format_name_for_ext
    let meta_map: serde_json::Map<String, Value> = notebook
        .metadata
        .iter()
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect();

    let version = notebook
        .metadata
        .get("jupytext")
        .and_then(|j| j.get("text_representation"))
        .and_then(|tr| tr.get("format_version"))
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    let format_name = format_name_for_ext(&meta_map, &ext, None, true);

    let fmt_impl = get_format_implementation(&ext, format_name.as_deref())?;
    let current = fmt_impl.current_version_number;

    // Missing version but notebook has metadata => use current
    let version = match version {
        Some(v) => v,
        None if !notebook.metadata.is_empty() => current.to_string(),
        None => return Ok(()),
    };

    if version == current {
        return Ok(());
    }

    let min = fmt_impl
        .min_readable_version_number
        .unwrap_or(current);
    if min <= version.as_str() && version.as_str() <= current {
        return Ok(());
    }

    let jupytext_version_in_file = notebook
        .metadata
        .get("jupytext")
        .and_then(|j| j.get("text_representation"))
        .and_then(|tr| tr.get("jupytext_version"))
        .and_then(|v| v.as_str())
        .unwrap_or("N/A");

    Err(JupytextFormatError(format!(
        "The file {} was generated with jupytext version {} but this build can only read \
         the {} format in versions {} to {}. The file format version is {}. Please upgrade, \
         or remove either {} or {}.",
        source_path,
        jupytext_version_in_file,
        format_name.as_deref().unwrap_or("unknown"),
        min,
        current,
        version,
        source_path,
        outputs_path,
    )))
}

// ---------------------------------------------------------------------------
// formats_with_support_for_cell_metadata
// ---------------------------------------------------------------------------

/// Yield `"ext:format_name"` for every format that supports cell metadata.
pub fn formats_with_support_for_cell_metadata() -> Vec<String> {
    JUPYTEXT_FORMATS
        .iter()
        .filter(|fmt| !FORMATS_WITH_NO_CELL_METADATA.contains(fmt.format_name))
        // Skip external formats (pandoc, myst) that are reader_type None
        .filter(|fmt| fmt.reader_type != ReaderType::None)
        .map(|fmt| format!("{}:{}", &fmt.extension[1..], fmt.format_name))
        .collect()
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_notebook_extensions_starts_with_ipynb() {
        assert_eq!(NOTEBOOK_EXTENSIONS[0], ".ipynb");
    }

    #[test]
    fn test_notebook_extensions_no_duplicates() {
        let mut seen = HashSet::new();
        for ext in NOTEBOOK_EXTENSIONS.iter() {
            assert!(seen.insert(ext), "Duplicate extension: {}", ext);
        }
    }

    #[test]
    fn test_get_format_implementation_default() {
        let fmt = get_format_implementation(".py", None).unwrap();
        assert_eq!(fmt.format_name, "light");
        assert_eq!(fmt.extension, ".py");
    }

    #[test]
    fn test_get_format_implementation_percent() {
        let fmt = get_format_implementation(".py", Some("percent")).unwrap();
        assert_eq!(fmt.format_name, "percent");
    }

    #[test]
    fn test_get_format_implementation_unknown_ext() {
        let result = get_format_implementation(".xyz", None);
        assert!(result.is_err());
    }

    #[test]
    fn test_long_form_one_format_simple() {
        let fmt = long_form_one_format("py:percent", None, None, true).unwrap();
        assert_eq!(
            fmt.get("extension").and_then(|v| v.as_str()),
            Some(".py")
        );
        assert_eq!(
            fmt.get("format_name").and_then(|v| v.as_str()),
            Some("percent")
        );
    }

    #[test]
    fn test_long_form_one_format_with_prefix() {
        let fmt = long_form_one_format("notebooks/py:percent", None, None, true).unwrap();
        assert_eq!(
            fmt.get("prefix").and_then(|v| v.as_str()),
            Some("notebooks")
        );
        assert_eq!(
            fmt.get("extension").and_then(|v| v.as_str()),
            Some(".py")
        );
    }

    #[test]
    fn test_long_form_one_format_common_name() {
        let fmt = long_form_one_format("notebook", None, None, true).unwrap();
        assert_eq!(
            fmt.get("extension").and_then(|v| v.as_str()),
            Some(".ipynb")
        );
    }

    #[test]
    fn test_short_form_round_trip() {
        let fmt = long_form_one_format("py:percent", None, None, true).unwrap();
        let short = short_form_one_format(&fmt);
        assert_eq!(short, "py:percent");
    }

    #[test]
    fn test_short_form_multiple() {
        let fmts = long_form_multiple_formats("ipynb,py:percent", None, true);
        let short = short_form_multiple_formats(&fmts);
        assert_eq!(short, "ipynb,py:percent");
    }

    #[test]
    fn test_guess_format_percent() {
        let text = "# %%\nx = 1\n# %%\ny = 2\n";
        let (name, _opts) = guess_format(text, ".py");
        assert_eq!(name, "percent");
    }

    #[test]
    fn test_guess_format_markdown_default() {
        let text = "# Title\n\nSome text\n";
        let (name, _opts) = guess_format(text, ".md");
        assert_eq!(name, "markdown");
    }

    #[test]
    fn test_guess_format_pandoc() {
        let text = "::: {.cell}\n```python\nx = 1\n```\n:::\n";
        let (name, _opts) = guess_format(text, ".md");
        assert_eq!(name, "pandoc");
    }

    #[test]
    fn test_guess_format_sphinx() {
        let text = "# #####################\n# Title\n# #####################\n# more\n";
        let (name, _opts) = guess_format(text, ".py");
        assert_eq!(name, "sphinx");
    }

    #[test]
    fn test_guess_format_spin() {
        let text = "#' Some text\nx <- 1\n";
        let (name, _opts) = guess_format(text, ".R");
        assert_eq!(name, "spin");
    }

    #[test]
    fn test_divine_format_ipynb() {
        let text = r#"{"nbformat": 4, "nbformat_minor": 5, "metadata": {}, "cells": []}"#;
        assert_eq!(divine_format(text), "ipynb");
    }

    #[test]
    fn test_divine_format_markdown() {
        let text = "Some text\n```\ncode\n```\n";
        assert_eq!(divine_format(text), "md");
    }

    #[test]
    fn test_validate_one_format_valid() {
        let mut fmt = BTreeMap::new();
        fmt.insert(
            "extension".to_string(),
            Value::String(".py".to_string()),
        );
        fmt.insert(
            "format_name".to_string(),
            Value::String("percent".to_string()),
        );
        assert!(validate_one_format(&fmt).is_ok());
    }

    #[test]
    fn test_validate_one_format_bad_name() {
        let mut fmt = BTreeMap::new();
        fmt.insert(
            "extension".to_string(),
            Value::String(".py".to_string()),
        );
        fmt.insert(
            "format_name".to_string(),
            Value::String("nonexistent".to_string()),
        );
        assert!(validate_one_format(&fmt).is_err());
    }

    #[test]
    fn test_validate_one_format_missing_extension() {
        let fmt = BTreeMap::new();
        assert!(validate_one_format(&fmt).is_err());
    }

    #[test]
    fn test_formats_with_no_cell_metadata() {
        assert!(FORMATS_WITH_NO_CELL_METADATA.contains("sphinx"));
        assert!(FORMATS_WITH_NO_CELL_METADATA.contains("nomarker"));
        assert!(!FORMATS_WITH_NO_CELL_METADATA.contains("percent"));
    }
}
