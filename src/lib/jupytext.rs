//! Read and write Jupyter notebooks as text files
//!
//! This module provides the main `reads`/`writes`/`read`/`write` API for
//! converting between `.ipynb` JSON and text-based notebook formats (Markdown,
//! percent scripts, light scripts, R Markdown, etc.).

use serde_json::Value;
use std::collections::{BTreeMap, HashSet};
use std::path::Path;

use crate::cell_metadata::IGNORE_CELL_METADATA;
use crate::cell_reader::{create_cell_reader, FormatOptions};
use crate::cell_to_text::{
    BareScriptCellExporter, CellExporter, DoublePercentCellExporter, HydrogenCellExporter,
    LightScriptCellExporter, MarkdownCellExporter, RMarkdownCellExporter, RScriptCellExporter,
    SphinxGalleryCellExporter,
};
use crate::formats::{
    divine_format, format_name_for_ext, get_format_implementation, guess_format,
    long_form_one_format, read_format_from_metadata,
    rearrange_jupytext_metadata, update_jupytext_formats_metadata, ExporterType,
    JupytextFormatError, NotebookFormatDescription, ReaderType, MYST_FORMAT_NAME,
    VALID_FORMAT_OPTIONS,
};
use crate::header::{
    encoding_and_executable, header_to_metadata_and_cell, insert_or_test_version_number,
    metadata_and_cell_to_header,
};
use crate::languages::{
    default_language_from_metadata_and_ext, set_main_and_cell_language, SCRIPT_EXTENSIONS,
};
use crate::metadata_filter::{filter_metadata, update_metadata_filters};
use crate::notebook::{reads_ipynb, writes_ipynb, Cell, CellType, Notebook};
use crate::pep8::pep8_lines_between_cells;

// ---------------------------------------------------------------------------
// Version
// ---------------------------------------------------------------------------

/// The version of this Rust jupytext implementation
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

// ---------------------------------------------------------------------------
// Error types
// ---------------------------------------------------------------------------

/// Errors that can occur during notebook read/write
#[derive(Debug, thiserror::Error)]
pub enum JupytextError {
    #[error("Format error: {0}")]
    Format(#[from] JupytextFormatError),

    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Unsupported nbformat version {major}.{minor}")]
    UnsupportedNbFormat { major: u32, minor: u32 },

    #[error("{0}")]
    Other(String),
}

// ---------------------------------------------------------------------------
// TextNotebookConverter
// ---------------------------------------------------------------------------

/// A converter that can read or write a Jupyter notebook as text.
///
/// It holds the long-form format specification, a reference to the matching
/// `NotebookFormatDescription`, and an optional format-options map.
pub struct TextNotebookConverter {
    /// Long-form format options (extension, format_name, cell_markers, ...)
    pub fmt: BTreeMap<String, Value>,
    /// Resolved format implementation (reader/exporter types, version, etc.)
    pub implementation: &'static NotebookFormatDescription,
    /// The file extension (e.g. `.py`, `.md`)
    pub ext: String,
}

impl TextNotebookConverter {
    /// Create a new converter from a long-form format specification.
    pub fn new(fmt: BTreeMap<String, Value>) -> Result<Self, JupytextError> {
        let ext = fmt
            .get("extension")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let format_name = fmt
            .get("format_name")
            .and_then(|v| v.as_str());
        let implementation = get_format_implementation(&ext, format_name)?;
        Ok(TextNotebookConverter {
            fmt,
            implementation,
            ext,
        })
    }

    // ----- update_fmt_with_notebook_options ---------------------------------

    /// Merge format options from the notebook metadata into `self.fmt`, and
    /// record those options back into the metadata.
    pub fn update_fmt_with_notebook_options(
        &mut self,
        metadata: &mut serde_json::Map<String, Value>,
        _read: bool,
    ) {
        // Use format options from the notebook metadata if not already set
        if let Some(jupytext) = metadata.get("jupytext").and_then(|v| v.as_object()) {
            for opt in VALID_FORMAT_OPTIONS {
                if let Some(val) = jupytext.get(*opt) {
                    self.fmt
                        .entry(opt.to_string())
                        .or_insert_with(|| val.clone());
                }
            }
        }

        // Save options back into notebook metadata
        for opt in VALID_FORMAT_OPTIONS {
            if let Some(val) = self.fmt.get(*opt) {
                let jupytext = metadata
                    .entry("jupytext".to_string())
                    .or_insert_with(|| Value::Object(serde_json::Map::new()));
                if let Some(obj) = jupytext.as_object_mut() {
                    obj.insert(opt.to_string(), val.clone());
                }
            }
        }

        // If the format matches the text_representation in the file, copy
        // version info into self.fmt
        if let Some(file_fmt) = metadata
            .get("jupytext")
            .and_then(|j| j.get("text_representation"))
            .and_then(|v| v.as_object())
        {
            let file_ext = file_fmt
                .get("extension")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let file_name = file_fmt
                .get("format_name")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let my_ext = self
                .fmt
                .get("extension")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let my_name = self
                .fmt
                .get("format_name")
                .and_then(|v| v.as_str())
                .unwrap_or("");

            if my_ext == file_ext && my_name == file_name {
                for (k, v) in file_fmt {
                    self.fmt.entry(k.clone()).or_insert_with(|| v.clone());
                }
            }
        }

        // rst2md should only fire once
        if let Some(jupytext) = metadata.get_mut("jupytext") {
            if let Some(obj) = jupytext.as_object_mut() {
                if obj.get("rst2md") == Some(&Value::Bool(true)) {
                    obj.insert("rst2md".to_string(), Value::Bool(false));
                }
            }
        }
    }

    // ----- reads -----------------------------------------------------------

    /// Read a notebook from a text string.
    pub fn reads(&mut self, text: &str) -> Result<Notebook, JupytextError> {
        // External formats that bypass the cell-reader pipeline
        match self.fmt.get("format_name").and_then(|v| v.as_str()) {
            Some("pandoc") | Some("quarto") | Some("marimo") => {
                return Err(JupytextError::Other(format!(
                    "The '{}' format requires an external converter not yet implemented in Rust.",
                    self.fmt
                        .get("format_name")
                        .and_then(|v| v.as_str())
                        .unwrap_or("unknown")
                )));
            }
            Some(name) if name == MYST_FORMAT_NAME => {
                return crate::myst::myst_to_notebook(text)
                    .map_err(JupytextError::Other);
            }
            _ => {}
        }

        let lines: Vec<String> = text.lines().map(|l| l.to_string()).collect();

        let root_level_metadata_as_raw_cell = self
            .fmt
            .get("root_level_metadata_as_raw_cell")
            .and_then(|v| v.as_bool())
            .unwrap_or(true);

        let header_result = header_to_metadata_and_cell(
            &lines,
            self.implementation.header_prefix,
            self.implementation.header_suffix,
            self.implementation.extension,
            root_level_metadata_as_raw_cell,
        );

        let mut metadata = header_result.metadata;
        let has_jupyter_md = header_result.has_jupyter_md;
        let header_cell = header_result.header_cell;
        let pos = header_result.next_line;

        let _default_language =
            default_language_from_metadata_and_ext(&metadata, self.implementation.extension, false);
        self.update_fmt_with_notebook_options(&mut metadata, true);

        let mut cells: Vec<Cell> = Vec::new();

        if let Some(hc) = header_cell {
            cells.push(hc);
        }

        let remaining_lines: Vec<String> = lines[pos..].to_vec();

        // For Sphinx gallery, prepend a %matplotlib inline cell
        if self
            .implementation
            .format_name
            .starts_with("sphinx")
        {
            cells.push(Cell::new_code("%matplotlib inline"));
        }

        // Cell reading loop using the cell_reader module
        if self.implementation.reader_type != ReaderType::None && !remaining_lines.is_empty() {
            let fmt_opts = FormatOptions {
                extension: Some(self.ext.clone()),
                format_name: self.fmt.get("format_name").and_then(|v| v.as_str()).map(|s| s.to_string()),
                format_version: self.fmt.get("format_version").and_then(|v| v.as_str()).map(|s| s.to_string()),
                comment_magics: self.fmt.get("comment_magics").and_then(|v| v.as_bool()),
                cell_metadata_json: self.fmt.get("cell_metadata_json").and_then(|v| v.as_bool()).unwrap_or(false),
                use_runtools: self.fmt.get("use_runtools").and_then(|v| v.as_bool()).unwrap_or(false),
                split_at_heading: self.fmt.get("split_at_heading").and_then(|v| v.as_bool()).unwrap_or(false),
                cell_markers: self.fmt.get("cell_markers").and_then(|v| v.as_str()).map(|s| s.to_string()),
                rst2md: self.fmt.get("rst2md").and_then(|v| v.as_bool()).unwrap_or(false),
                doxygen_equation_markers: self.fmt.get("doxygen_equation_markers").and_then(|v| v.as_bool()).unwrap_or(false),
            };
            let default_lang = _default_language.as_deref();

            let mut pos_in_remaining = 0;
            while pos_in_remaining < remaining_lines.len() {
                let mut reader = create_cell_reader(&fmt_opts, default_lang);
                let result = reader.read(&remaining_lines[pos_in_remaining..]);
                cells.push(result.cell);
                if result.next_position == 0 {
                    // Safety: avoid infinite loop if reader returns 0
                    break;
                }
                pos_in_remaining += result.next_position;
            }
        }

        // Set main and cell language
        let custom_cell_magics: Vec<String> = self
            .fmt
            .get("custom_cell_magics")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .split(',')
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string())
            .collect();

        set_main_and_cell_language(
            &mut metadata,
            &mut cells,
            self.implementation.extension,
            &custom_cell_magics,
        );

        // Collect cell metadata keys
        let mut cell_metadata_keys: HashSet<String> = HashSet::new();
        for cell in &cells {
            for key in cell.metadata.keys() {
                cell_metadata_keys.insert(key.clone());
            }
        }
        update_metadata_filters(&mut metadata, has_jupyter_md, &cell_metadata_keys);

        // Sphinx gallery: filter empty cells between non-markdown cells
        if self
            .implementation
            .format_name
            .starts_with("sphinx")
        {
            let mut filtered = Vec::new();
            for (i, cell) in cells.iter().enumerate() {
                if cell.source.is_empty()
                    && i > 0
                    && i + 1 < cells.len()
                    && cells[i - 1].cell_type != CellType::Markdown
                    && cells[i + 1].cell_type != CellType::Markdown
                {
                    continue;
                }
                filtered.push(cell.clone());
            }
            cells = filtered;
        }

        // Build notebook
        let nb_metadata: BTreeMap<String, Value> =
            metadata.into_iter().collect();

        Ok(Notebook {
            nbformat: 4,
            nbformat_minor: 5,
            metadata: nb_metadata,
            cells,
        })
    }

    // ----- filter_notebook -------------------------------------------------

    /// Return a filtered copy of the notebook suitable for writing to text.
    ///
    /// This drops outputs, execution counts, and applies metadata filters.
    pub fn filter_notebook(
        &mut self,
        nb: &Notebook,
    ) -> Notebook {
        // Convert BTreeMap to serde_json::Map for update_fmt_with_notebook_options
        let mut meta_map: serde_json::Map<String, Value> = nb
            .metadata
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();

        self.update_fmt_with_notebook_options(&mut meta_map, false);

        // Filter notebook-level metadata
        let notebook_metadata_filter = self
            .fmt
            .get("notebook_metadata_filter")
            .and_then(|v| v.as_str())
            .unwrap_or("");

        let filtered_meta: BTreeMap<String, Value> = meta_map.into_iter().collect();
        let filtered_meta = filter_metadata(
            &filtered_meta,
            notebook_metadata_filter,
            crate::metadata_filter::DEFAULT_NOTEBOOK_METADATA,
        );
        // Sort metadata for consistent output
        let sorted_meta: BTreeMap<String, Value> = filtered_meta.into_iter().collect();

        // Filter cell metadata, drop outputs
        let cell_metadata_filter = self
            .fmt
            .get("cell_metadata_filter")
            .and_then(|v| v.as_str())
            .unwrap_or("");

        let cells: Vec<Cell> = nb
            .cells
            .iter()
            .map(|cell| {
                let filtered_cell_meta = filter_metadata(
                    &cell.metadata,
                    cell_metadata_filter,
                    IGNORE_CELL_METADATA,
                );
                Cell {
                    cell_type: cell.cell_type.clone(),
                    source: cell.source.clone(),
                    metadata: filtered_cell_meta,
                    execution_count: if cell.cell_type == CellType::Code {
                        Some(Value::Null)
                    } else {
                        None
                    },
                    outputs: if cell.cell_type == CellType::Code {
                        Some(Vec::new())
                    } else {
                        None
                    },
                    id: cell.id.clone(),
                }
            })
            .collect();

        Notebook {
            nbformat: nb.nbformat,
            nbformat_minor: nb.nbformat_minor,
            metadata: sorted_meta,
            cells,
        }
    }

    // ----- writes ----------------------------------------------------------

    /// Return the text representation of the notebook.
    pub fn writes(
        &mut self,
        nb: &Notebook,
        metadata_override: Option<&BTreeMap<String, Value>>,
    ) -> Result<String, JupytextError> {
        // External formats
        match self.fmt.get("format_name").and_then(|v| v.as_str()) {
            Some("pandoc") | Some("quarto") | Some("marimo") => {
                return Err(JupytextError::Other(format!(
                    "The '{}' format requires an external converter not yet implemented in Rust.",
                    self.fmt
                        .get("format_name")
                        .and_then(|v| v.as_str())
                        .unwrap_or("unknown")
                )));
            }
            Some(name) if name == MYST_FORMAT_NAME => {
                let filtered = self.filter_notebook(nb);
                return Ok(crate::myst::notebook_to_myst(&filtered));
            }
            _ => {}
        }

        // Build working metadata
        let metadata = metadata_override.cloned().unwrap_or_else(|| nb.metadata.clone());

        let mut meta_map: serde_json::Map<String, Value> =
            metadata.iter().map(|(k, v)| (k.clone(), v.clone())).collect();

        let _default_language =
            default_language_from_metadata_and_ext(&meta_map, self.implementation.extension, true)
                .unwrap_or_else(|| "python".to_string());

        self.update_fmt_with_notebook_options(&mut meta_map, false);

        // Detect use_runtools from cell metadata if not already set
        if !self.fmt.contains_key("use_runtools") {
            for cell in &nb.cells {
                if cell.metadata.get("hide_input") == Some(&Value::Bool(true))
                    || cell.metadata.get("hide_output") == Some(&Value::Bool(true))
                {
                    self.fmt
                        .insert("use_runtools".to_string(), Value::Bool(true));
                    break;
                }
            }
        }

        // Build the header (encoding, executable, YAML front matter)
        let mut header = encoding_and_executable(&mut meta_map, &self.ext);

        let fmt_btree: BTreeMap<String, Value> = self.fmt.clone();
        let (header_content, header_lines_to_next_cell) = metadata_and_cell_to_header(
            &meta_map,
            &fmt_btree,
            self.implementation.header_prefix,
            self.implementation.header_suffix,
        );
        header.extend(header_content.clone());

        // Build cell texts using the appropriate cell exporter
        let is_sphinx = self.implementation.format_name.starts_with("sphinx");
        let split_at_heading = self
            .fmt
            .get("split_at_heading")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        // Collect cell texts
        let mut cell_texts: Vec<Vec<String>> = Vec::new();
        let mut cell_is_code: Vec<bool> = Vec::new();

        for (_i, cell) in nb.cells.iter().enumerate() {
            let text = match self.implementation.exporter_type {
                ExporterType::Markdown => {
                    let mut exp = MarkdownCellExporter::new(cell, &_default_language, &self.fmt);
                    exp.cell_to_text()
                }
                ExporterType::RMarkdown => {
                    let mut exp = RMarkdownCellExporter::new(cell, &_default_language, &self.fmt);
                    exp.cell_to_text()
                }
                ExporterType::LightScript => {
                    let mut exp = LightScriptCellExporter::new(cell, &_default_language, &self.fmt);
                    exp.cell_to_text()
                }
                ExporterType::BareScript => {
                    let mut exp = BareScriptCellExporter::new(cell, &_default_language, &self.fmt);
                    exp.cell_to_text()
                }
                ExporterType::DoublePercent => {
                    let mut exp = DoublePercentCellExporter::new(cell, &_default_language, &self.fmt);
                    exp.cell_to_text()
                }
                ExporterType::Hydrogen => {
                    let mut exp = HydrogenCellExporter::new(cell, &_default_language, &self.fmt);
                    exp.cell_to_text()
                }
                ExporterType::SphinxGallery => {
                    let mut exp = SphinxGalleryCellExporter::new(cell, &_default_language, &self.fmt);
                    exp.cell_to_text()
                }
                ExporterType::RScript => {
                    let mut exp = RScriptCellExporter::new(cell, &_default_language, &self.fmt);
                    exp.cell_to_text()
                }
                ExporterType::None => {
                    // Fallback: dump source lines directly
                    let lines: Vec<String> = cell.source.lines().map(|l| l.to_string()).collect();
                    if lines.is_empty() { vec![String::new()] } else { lines }
                }
            };

            cell_is_code.push(cell.cell_type == CellType::Code);
            cell_texts.push(text);
        }

        // Concatenate cells in reverse order (to compute PEP8 spacing)
        let mut lines: Vec<String> = Vec::new();

        for i in (0..cell_texts.len()).rev() {
            let mut text = cell_texts[i].clone();

            // Skip %matplotlib inline for sphinx
            if i == 0
                && is_sphinx
                && (text == vec!["%matplotlib inline".to_string()]
                    || text == vec!["# %matplotlib inline".to_string()])
            {
                continue;
            }

            // Compute blank lines to next cell
            let lines_to_next_cell = nb.cells[i]
                .metadata
                .get("lines_to_next_cell")
                .and_then(|v| v.as_u64())
                .map(|n| n as usize);

            let blank_lines = lines_to_next_cell.unwrap_or_else(|| {
                pep8_lines_between_cells(&text, &lines, self.implementation.extension)
            });

            // Add blank lines
            for _ in 0..blank_lines {
                text.push(String::new());
            }

            // Two blank lines between adjacent markdown cells in .md/.Rmd
            // when they don't have explicit region markers
            if matches!(self.ext.as_str(), ".md" | ".markdown" | ".Rmd")
                && !cell_is_code[i]
            {
                if i + 1 < cell_texts.len()
                    && !cell_is_code[i + 1]
                    && !cell_texts[i][0].starts_with("<!-- #")
                    && !cell_texts[i + 1][0].starts_with("<!-- #")
                    && (!split_at_heading
                        || cell_texts[i + 1].is_empty()
                        || !cell_texts[i + 1][0].starts_with('#'))
                {
                    text.push(String::new());
                }
            }

            // Sphinx: empty `""` between consecutive code cells
            if is_sphinx && cell_is_code[i] {
                if i + 1 < cell_texts.len() && cell_is_code[i + 1] {
                    text.push("\"\"".to_string());
                }
            }

            // Prepend text to the accumulated lines
            text.extend(lines);
            lines = text;
        }

        // Header spacing
        let header_blank_lines = match header_lines_to_next_cell {
            Some(n) => n,
            None => pep8_lines_between_cells(
                &header_content,
                &lines,
                self.implementation.extension,
            ),
        };

        for _ in 0..header_blank_lines {
            header.push(String::new());
        }

        header.extend(lines);

        Ok(header.join("\n"))
    }
}

// ---------------------------------------------------------------------------
// Public API: reads / writes / read / write
// ---------------------------------------------------------------------------

/// Read a notebook from a string.
///
/// If `fmt` is `None` the format is detected automatically via `divine_format`.
pub fn reads(
    text: &str,
    fmt: Option<&str>,
) -> Result<Notebook, JupytextError> {
    let format_str = match fmt {
        Some(f) => f.to_string(),
        None => divine_format(text),
    };

    let mut fmt_map = long_form_one_format(&format_str, None, None, true)?;
    let ext = fmt_map
        .get("extension")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    // .ipynb: direct JSON parse
    if ext == ".ipynb" {
        let nb = reads_ipynb(text)?;
        if nb.nbformat != 4 {
            eprintln!(
                "Warning: Notebooks in nbformat version {}.{} may not be fully supported.",
                nb.nbformat, nb.nbformat_minor
            );
        }
        return Ok(nb);
    }

    // Detect format from metadata or content
    let format_name = read_format_from_metadata(text, &ext)
        .or_else(|| {
            fmt_map
                .get("format_name")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string())
        });

    let (final_format_name, format_options) = match format_name {
        Some(name) => (Some(name), BTreeMap::new()),
        None => {
            let (name, opts) = guess_format(text, &ext);
            (Some(name), opts)
        }
    };

    if let Some(ref name) = final_format_name {
        fmt_map.insert(
            "format_name".to_string(),
            Value::String(name.clone()),
        );
    }
    for (k, v) in &format_options {
        fmt_map.insert(k.clone(), v.clone());
    }

    let mut converter = TextNotebookConverter::new(fmt_map)?;
    let mut notebook = converter.reads(text)?;

    // Rearrange legacy metadata
    let mut meta_map: serde_json::Map<String, Value> = notebook
        .metadata
        .iter()
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect();
    rearrange_jupytext_metadata(&mut meta_map);
    notebook.metadata = meta_map.into_iter().collect();

    // Record text representation in metadata
    if final_format_name.is_some() && insert_or_test_version_number() {
        let jupytext = notebook
            .metadata
            .entry("jupytext".to_string())
            .or_insert_with(|| Value::Object(serde_json::Map::new()));
        if let Some(obj) = jupytext.as_object_mut() {
            let tr = obj
                .entry("text_representation".to_string())
                .or_insert_with(|| Value::Object(serde_json::Map::new()));
            if let Some(tr_obj) = tr.as_object_mut() {
                tr_obj.insert(
                    "extension".to_string(),
                    Value::String(ext.clone()),
                );
                if let Some(ref name) = final_format_name {
                    tr_obj.insert(
                        "format_name".to_string(),
                        Value::String(name.clone()),
                    );
                }
            }
        }
    }

    Ok(notebook)
}

/// Return the text representation of the notebook.
///
/// `fmt` is a format string like `"md"`, `"py:percent"`, etc.
pub fn writes(
    notebook: &Notebook,
    fmt: &str,
) -> Result<String, JupytextError> {
    if notebook.nbformat < 4 {
        return Err(JupytextError::UnsupportedNbFormat {
            major: notebook.nbformat,
            minor: notebook.nbformat_minor,
        });
    }
    if notebook.nbformat > 4 || (notebook.nbformat == 4 && notebook.nbformat_minor > 5) {
        eprintln!(
            "Warning: Notebooks in nbformat version {}.{} have not been tested.",
            notebook.nbformat, notebook.nbformat_minor
        );
    }

    let mut metadata: BTreeMap<String, Value> = notebook.metadata.clone();

    // Rearrange legacy metadata
    let mut meta_map: serde_json::Map<String, Value> =
        metadata.iter().map(|(k, v)| (k.clone(), v.clone())).collect();
    rearrange_jupytext_metadata(&mut meta_map);
    metadata = meta_map.into_iter().collect();

    let fmt_map = long_form_one_format(
        fmt,
        Some(
            &metadata
                .iter()
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect(),
        ),
        None,
        true,
    )?;
    let ext = fmt_map
        .get("extension")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let mut format_name = fmt_map
        .get("format_name")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    // .ipynb: drop text representation metadata and serialise as JSON
    if ext == ".ipynb" {
        return write_ipynb(notebook, &metadata);
    }

    // Resolve format name if not explicitly given
    if format_name.is_none() {
        let meta_serde: serde_json::Map<String, Value> =
            metadata.iter().map(|(k, v)| (k.clone(), v.clone())).collect();
        format_name = format_name_for_ext(&meta_serde, &ext, None, false);
    }

    // Since Jupytext >= 1.17, default format for scripts is percent
    if format_name.is_none()
        && !fmt_map.contains_key("cell_markers")
        && SCRIPT_EXTENSIONS.contains_key(ext.as_str())
    {
        format_name = Some("percent".to_string());
    }

    let mut fmt_map = fmt_map;
    if let Some(ref name) = format_name {
        fmt_map.insert(
            "format_name".to_string(),
            Value::String(name.clone()),
        );
        let mut meta_serde: serde_json::Map<String, Value> =
            metadata.iter().map(|(k, v)| (k.clone(), v.clone())).collect();
        update_jupytext_formats_metadata(&mut meta_serde, &fmt_map);
        metadata = meta_serde.into_iter().collect();
    }

    let mut converter = TextNotebookConverter::new(fmt_map)?;
    converter.writes(notebook, Some(&metadata))
}

/// Write a notebook as `.ipynb` JSON, dropping text representation metadata.
fn write_ipynb(
    notebook: &Notebook,
    metadata: &BTreeMap<String, Value>,
) -> Result<String, JupytextError> {
    let mut clean_meta = metadata.clone();

    // Remove text_representation from jupytext metadata
    if let Some(jupytext) = clean_meta.get_mut("jupytext") {
        if let Some(obj) = jupytext.as_object_mut() {
            obj.remove("text_representation");
            if obj.is_empty() {
                clean_meta.remove("jupytext");
            }
        }
    }

    let nb = Notebook {
        nbformat: notebook.nbformat,
        nbformat_minor: notebook.nbformat_minor,
        metadata: clean_meta,
        cells: notebook.cells.clone(),
    };

    let json = writes_ipynb(&nb)?;
    Ok(json)
}

/// Read a notebook from a file path.
///
/// The format is inferred from the file extension unless `fmt` is given.
pub fn read(
    path: &str,
    fmt: Option<&str>,
) -> Result<Notebook, JupytextError> {
    let p = Path::new(path);
    let ext = p
        .extension()
        .map(|e| format!(".{}", e.to_string_lossy()))
        .unwrap_or_default();

    let text = std::fs::read_to_string(path)?;

    // Build format string
    let format_str = match fmt {
        Some(f) => f.to_string(),
        None => {
            // If extension alone is sufficient, use it
            if ext == ".ipynb" {
                "ipynb".to_string()
            } else {
                // Let reads auto-detect, but pass extension
                let ext_no_dot = ext.trim_start_matches('.');
                ext_no_dot.to_string()
            }
        }
    };

    reads(&text, Some(&format_str))
}

/// Write a notebook to a file path.
///
/// The format is inferred from the file extension unless `fmt` is given.
pub fn write(
    notebook: &Notebook,
    path: &str,
    fmt: Option<&str>,
) -> Result<(), JupytextError> {
    let p = Path::new(path);
    let ext = p
        .extension()
        .map(|e| format!(".{}", e.to_string_lossy()))
        .unwrap_or_default();

    let format_str = match fmt {
        Some(f) => f.to_string(),
        None => {
            let ext_no_dot = ext.trim_start_matches('.');
            ext_no_dot.to_string()
        }
    };

    let content = writes(notebook, &format_str)?;

    // Create parent directories if a prefix format is used
    if let Some(parent) = p.parent() {
        if !parent.exists() {
            std::fs::create_dir_all(parent)?;
        }
    }

    let mut final_content = content;
    if !final_content.ends_with('\n') {
        final_content.push('\n');
    }

    std::fs::write(path, final_content)?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Compatibility wrappers for CLI module
// ---------------------------------------------------------------------------

/// Convert a `BTreeMap<String, String>` format dict to the canonical
/// `BTreeMap<String, Value>` form used internally.
#[allow(dead_code)]
fn string_fmt_to_value_fmt(fmt: &BTreeMap<String, String>) -> BTreeMap<String, Value> {
    fmt.iter()
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

/// Build a format string from a `BTreeMap<String, String>` dict.
fn fmt_dict_to_string(fmt: &BTreeMap<String, String>) -> String {
    let mut s = String::new();
    if let Some(ext) = fmt.get("extension") {
        let ext_no_dot = ext.trim_start_matches('.');
        s.push_str(ext_no_dot);
    }
    if let Some(name) = fmt.get("format_name") {
        if !name.is_empty() {
            if !s.is_empty() {
                s.push(':');
            }
            s.push_str(name);
        }
    }
    if s.is_empty() {
        s.push_str("ipynb");
    }
    s
}

/// Read a notebook from text using a `BTreeMap<String, String>` format specification.
///
/// This is the entry point used by the CLI module.
pub fn reads_notebook(
    text: &str,
    fmt: &BTreeMap<String, String>,
) -> Result<Notebook, anyhow::Error> {
    let fmt_str = fmt_dict_to_string(fmt);
    Ok(reads(text, Some(&fmt_str))?)
}

/// Write a notebook to text using a `BTreeMap<String, String>` format specification.
///
/// This is the entry point used by the CLI module.
pub fn writes_notebook(
    notebook: &Notebook,
    fmt: &BTreeMap<String, String>,
) -> Result<String, anyhow::Error> {
    let fmt_str = fmt_dict_to_string(fmt);
    Ok(writes(notebook, &fmt_str)?)
}

/// Read a notebook from a file path using a `BTreeMap<String, String>` format specification.
///
/// This is the entry point used by the CLI module (not typically used; the CLI
/// reads files itself, but provided for completeness).
pub fn read_notebook(
    path: &std::path::Path,
    fmt: &BTreeMap<String, String>,
) -> Result<Notebook, anyhow::Error> {
    let fmt_str = fmt_dict_to_string(fmt);
    Ok(read(path.to_str().unwrap_or(""), Some(&fmt_str))?)
}

/// Write a notebook to a file path using a `BTreeMap<String, String>` format specification.
///
/// This is the entry point used by the CLI module.
pub fn write_notebook(
    notebook: &Notebook,
    path: &std::path::Path,
    fmt: &BTreeMap<String, String>,
) -> Result<(), anyhow::Error> {
    let fmt_str = fmt_dict_to_string(fmt);
    Ok(write(notebook, path.to_str().unwrap_or(""), Some(&fmt_str))?)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::notebook::Notebook;

    #[test]
    fn test_reads_ipynb() {
        let text = r#"{
            "nbformat": 4,
            "nbformat_minor": 5,
            "metadata": {},
            "cells": [
                {
                    "cell_type": "code",
                    "source": "x = 1",
                    "metadata": {},
                    "execution_count": null,
                    "outputs": []
                }
            ]
        }"#;
        let nb = reads(text, Some("ipynb")).unwrap();
        assert_eq!(nb.nbformat, 4);
        assert_eq!(nb.cells.len(), 1);
        assert_eq!(nb.cells[0].source, "x = 1");
    }

    #[test]
    fn test_writes_ipynb() {
        let mut nb = Notebook::new();
        nb.cells.push(Cell::new_code("x = 1"));
        let text = writes(&nb, "ipynb").unwrap();
        assert!(text.contains("\"nbformat\": 4"));
        assert!(text.contains("x = 1"));
    }

    #[test]
    fn test_writes_ipynb_drops_text_representation() {
        let mut nb = Notebook::new();
        let mut jupytext = serde_json::Map::new();
        let mut tr = serde_json::Map::new();
        tr.insert(
            "extension".to_string(),
            Value::String(".py".to_string()),
        );
        tr.insert(
            "format_name".to_string(),
            Value::String("percent".to_string()),
        );
        jupytext.insert(
            "text_representation".to_string(),
            Value::Object(tr),
        );
        nb.metadata.insert(
            "jupytext".to_string(),
            Value::Object(jupytext),
        );
        nb.cells.push(Cell::new_code("x = 1"));

        let text = writes(&nb, "ipynb").unwrap();
        // text_representation should be stripped for .ipynb output
        assert!(!text.contains("text_representation"));
    }

    #[test]
    fn test_reads_auto_detect_json() {
        let text = r#"{"nbformat": 4, "nbformat_minor": 5, "metadata": {}, "cells": []}"#;
        let nb = reads(text, None).unwrap();
        assert_eq!(nb.nbformat, 4);
    }

    #[test]
    fn test_round_trip_ipynb() {
        let original = r#"{
  "nbformat": 4,
  "nbformat_minor": 5,
  "metadata": {},
  "cells": [
    {
      "cell_type": "code",
      "source": "print('hello')",
      "metadata": {},
      "execution_count": null,
      "outputs": []
    }
  ]
}"#;
        let nb = reads(original, Some("ipynb")).unwrap();
        let written = writes(&nb, "ipynb").unwrap();
        let nb2 = reads(&written, Some("ipynb")).unwrap();
        assert_eq!(nb2.cells.len(), 1);
        assert_eq!(nb2.cells[0].source, "print('hello')");
    }

    #[test]
    fn test_unsupported_nbformat() {
        let nb = Notebook {
            nbformat: 3,
            nbformat_minor: 0,
            metadata: BTreeMap::new(),
            cells: Vec::new(),
        };
        let result = writes(&nb, "ipynb");
        assert!(result.is_err());
    }

    #[test]
    fn test_write_and_read_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.ipynb");
        let path_str = path.to_str().unwrap();

        let mut nb = Notebook::new();
        nb.cells.push(Cell::new_code("x = 42"));

        write(&nb, path_str, Some("ipynb")).unwrap();
        let nb2 = read(path_str, Some("ipynb")).unwrap();
        assert_eq!(nb2.cells.len(), 1);
        assert_eq!(nb2.cells[0].source, "x = 42");
    }

    #[test]
    fn test_converter_reads_simple_py() {
        // A very simple Python text notebook (just code, no markers)
        let text = "x = 1\ny = 2\n";
        let mut fmt = BTreeMap::new();
        fmt.insert(
            "extension".to_string(),
            Value::String(".py".to_string()),
        );
        fmt.insert(
            "format_name".to_string(),
            Value::String("light".to_string()),
        );
        let mut converter = TextNotebookConverter::new(fmt).unwrap();
        let nb = converter.reads(text).unwrap();
        assert!(!nb.cells.is_empty());
    }

    #[test]
    fn test_converter_writes_simple() {
        let mut nb = Notebook::new();
        nb.cells.push(Cell::new_code("x = 1"));

        let mut fmt = BTreeMap::new();
        fmt.insert(
            "extension".to_string(),
            Value::String(".py".to_string()),
        );
        fmt.insert(
            "format_name".to_string(),
            Value::String("light".to_string()),
        );
        let mut converter = TextNotebookConverter::new(fmt).unwrap();
        let text = converter.writes(&nb, None).unwrap();
        assert!(text.contains("x = 1"));
    }

    #[test]
    fn test_version_constant() {
        assert!(!VERSION.is_empty());
    }
}
