//! Export notebook cells as text
//!
//! This module provides cell exporters that convert Jupyter notebook cells
//! into various text formats (Markdown, R Markdown, light scripts,
//! percent scripts, etc.).

use regex::Regex;
use serde_json::Value;
use std::collections::BTreeMap;

use crate::cell_metadata::{
    is_active, metadata_to_double_percent_options, metadata_to_rmd_options, metadata_to_text,
    IGNORE_CELL_METADATA,
};
use crate::languages::{cell_language, comment_lines, same_language, SCRIPT_EXTENSIONS};
use crate::magics::{comment_magic, escape_code_start, need_explicit_marker};
use crate::metadata_filter::filter_metadata;
use crate::notebook::{Cell, CellType};
use crate::pep8::pep8_lines_between_cells;

// ---------------------------------------------------------------------------
// Utility functions
// ---------------------------------------------------------------------------

/// Return the source of the current cell as a vector of lines.
pub fn cell_source(cell: &Cell) -> Vec<String> {
    if cell.source.is_empty() {
        return vec![String::new()];
    }
    let mut lines: Vec<String> = cell.source.lines().map(|l| l.to_string()).collect();
    if cell.source.ends_with('\n') {
        lines.push(String::new());
    }
    lines
}

/// Return a string with enough backticks to safely encapsulate the given
/// lines in a Markdown fenced code block.
/// cf. <https://github.com/mwouts/jupytext/issues/712>
pub fn three_backticks_or_more(lines: &[String]) -> String {
    let mut code_cell_delimiter = "```".to_string();
    for line in lines {
        if !line.starts_with(&code_cell_delimiter) {
            continue;
        }
        // Count how many consecutive backticks the line has beyond the delimiter
        for ch in line[code_cell_delimiter.len()..].chars() {
            if ch != '`' {
                break;
            }
            code_cell_delimiter.push('`');
        }
        code_cell_delimiter.push('`');
    }
    code_cell_delimiter
}

/// Compute the end-of-cell marker that does not conflict with the source.
/// Issues #31 #38: does the cell contain a blank line? In that case we add
/// an end-of-cell marker.
pub fn endofcell_marker(source: &[String], comment: &str) -> String {
    let mut endofcell = "-".to_string();
    loop {
        let pattern = format!(
            r"^{}\s+{}\s*$",
            regex::escape(comment),
            regex::escape(&endofcell)
        );
        let re = Regex::new(&pattern).unwrap();
        if source.iter().any(|line| re.is_match(line)) {
            endofcell.push('-');
        } else {
            return endofcell;
        }
    }
}

// ---------------------------------------------------------------------------
// Helper: extract an Option<bool> from a format map
// ---------------------------------------------------------------------------

fn fmt_bool(fmt: &BTreeMap<String, Value>, key: &str) -> Option<bool> {
    fmt.get(key).and_then(|v| v.as_bool())
}

fn fmt_str<'a>(fmt: &'a BTreeMap<String, Value>, key: &str) -> Option<&'a str> {
    fmt.get(key).and_then(|v| v.as_str())
}

fn fmt_string(fmt: &BTreeMap<String, Value>, key: &str) -> Option<String> {
    fmt_str(fmt, key).map(|s| s.to_string())
}

// ---------------------------------------------------------------------------
// CellExporter trait
// ---------------------------------------------------------------------------

/// Trait for exporting a single cell as text lines.
pub trait CellExporter {
    /// Return the text representation of the cell.
    fn cell_to_text(&mut self) -> Vec<String>;

    /// Optionally remove the end-of-cell marker when it is redundant with the
    /// start marker of the next cell.
    fn remove_eoc_marker(&mut self, text: Vec<String>, next_text: &[String]) -> Vec<String> {
        let _ = next_text;
        text
    }

    /// Number of blank lines to insert before the next cell (if overridden
    /// by cell metadata).
    fn lines_to_next_cell(&self) -> Option<usize>;
}

// ---------------------------------------------------------------------------
// BaseCellExporter – common state shared by all exporters
// ---------------------------------------------------------------------------

/// Common data carried by every cell exporter.
#[derive(Debug, Clone)]
pub struct BaseCellData {
    pub fmt: BTreeMap<String, Value>,
    pub ext: String,
    pub cell_type: CellType,
    pub source: Vec<String>,
    pub unfiltered_metadata: BTreeMap<String, Value>,
    pub metadata: BTreeMap<String, Value>,
    pub language: String,
    pub default_language: String,
    pub comment: String,
    pub comment_suffix: String,
    pub comment_magics: bool,
    pub cell_metadata_json: bool,
    pub use_runtools: bool,
    pub doxygen_equation_markers: bool,
    pub lines_to_next_cell: Option<usize>,
    pub lines_to_end_of_cell_marker: Option<usize>,
}

impl BaseCellData {
    /// Construct common cell data from a notebook cell.
    ///
    /// `parse_cell_language` – whether to detect cell-magic language from
    /// the first source line (most exporters set this to `true`).
    ///
    /// `default_comment_magics` – the exporter-specific default for
    /// `comment_magics`. `None` means "do not escape magics" (maps to
    /// `false` in the Rust bool).
    pub fn new(
        cell: &Cell,
        default_language: &str,
        fmt: &BTreeMap<String, Value>,
        parse_cell_language: bool,
        default_comment_magics: Option<bool>,
    ) -> Self {
        let ext = fmt_string(fmt, "extension").unwrap_or_default();

        let mut source = cell_source(cell);

        // Filter metadata
        let cell_metadata_filter = fmt_str(fmt, "cell_metadata_filter").unwrap_or("");
        let metadata = filter_metadata(&cell.metadata, cell_metadata_filter, IGNORE_CELL_METADATA);
        let mut metadata = metadata;

        // Detect cell language from magic commands
        let mut language: Option<String> = None;
        if parse_cell_language {
            let custom_cell_magics_str = fmt_str(fmt, "custom_cell_magics").unwrap_or("");
            let custom_cell_magics: Vec<String> = custom_cell_magics_str
                .split(',')
                .filter(|s| !s.is_empty())
                .map(|s| s.to_string())
                .collect();

            let (lang, magic_args) =
                cell_language(&mut source, default_language, &custom_cell_magics);
            language = lang;

            if let Some(ref args) = magic_args {
                metadata.insert(
                    "magic_args".to_string(),
                    Value::String(args.clone()),
                );
            }
        }

        // Store language in metadata for non-Rmd formats
        if language.is_some() && !ext.ends_with(".Rmd") {
            metadata.insert(
                "language".to_string(),
                Value::String(language.as_ref().unwrap().clone()),
            );
        }

        // Fall back to cell metadata language, then default
        let language = language
            .or_else(|| {
                cell.metadata
                    .get("language")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string())
            })
            .unwrap_or_else(|| default_language.to_string());

        // Comment characters from script extension
        let comment = SCRIPT_EXTENSIONS
            .get(ext.as_str())
            .map(|sl| sl.comment.to_string())
            .unwrap_or_else(|| "#".to_string());
        let comment_suffix = SCRIPT_EXTENSIONS
            .get(ext.as_str())
            .map(|sl| sl.comment_suffix.to_string())
            .unwrap_or_default();

        // comment_magics: use fmt override, then exporter default, then false
        let comment_magics = fmt_bool(fmt, "comment_magics")
            .or(default_comment_magics)
            .unwrap_or(false);

        let cell_metadata_json = fmt_bool(fmt, "cell_metadata_json").unwrap_or(false);
        let use_runtools = fmt_bool(fmt, "use_runtools").unwrap_or(false);
        let doxygen_equation_markers = fmt_bool(fmt, "doxygen_equation_markers").unwrap_or(false);

        let lines_to_next_cell = cell
            .metadata
            .get("lines_to_next_cell")
            .and_then(|v| v.as_u64())
            .map(|n| n as usize);
        let lines_to_end_of_cell_marker = cell
            .metadata
            .get("lines_to_end_of_cell_marker")
            .and_then(|v| v.as_u64())
            .map(|n| n as usize);

        // Raw cells without explicit "active" metadata get active=""
        let cell_type = cell.cell_type.clone();
        if cell_type == CellType::Raw
            && !metadata.contains_key("active")
            && !has_active_tag(&metadata)
        {
            metadata.insert("active".to_string(), Value::String(String::new()));
        }

        BaseCellData {
            fmt: fmt.clone(),
            ext,
            cell_type,
            source,
            unfiltered_metadata: cell.metadata.clone(),
            metadata,
            language,
            default_language: default_language.to_string(),
            comment,
            comment_suffix,
            comment_magics,
            cell_metadata_json,
            use_runtools,
            doxygen_equation_markers,
            lines_to_next_cell,
            lines_to_end_of_cell_marker,
        }
    }

    /// Is this cell a code cell (or a raw cell with "active" metadata)?
    pub fn is_code(&self) -> bool {
        if self.cell_type == CellType::Code {
            return true;
        }
        if self.cell_type == CellType::Raw
            && (self.metadata.contains_key("active") || has_active_tag(&self.metadata))
        {
            return true;
        }
        false
    }

    /// Should this markdown cell use triple quotes?
    pub fn use_triple_quotes(&self) -> bool {
        let cell_marker = match self.unfiltered_metadata.get("cell_marker") {
            Some(Value::String(s)) => s.clone(),
            _ => return false,
        };
        if cell_marker == "\"\"\"" || cell_marker == "'''" {
            return true;
        }
        if !cell_marker.contains(',') {
            return false;
        }
        let parts: Vec<&str> = cell_marker.splitn(2, ',').collect();
        if parts.len() != 2 {
            return false;
        }
        let left = parts[0];
        let right = parts[1];
        if left.len() < 3 || right.len() < 3 {
            return false;
        }
        let left3 = &left[..3];
        let right3 = &right[right.len() - 3..];
        left3 == right3 && (left3 == "\"\"\"" || left3 == "'''")
    }

    /// Base cell_to_text logic shared by most exporters (except those that
    /// override entirely).
    pub fn base_cell_to_text<F>(&mut self, code_to_text: F) -> Vec<String>
    where
        F: FnOnce(&mut Self) -> Vec<String>,
    {
        // Trigger cell marker in case we are using multiline quotes
        if self.cell_type != CellType::Code && self.metadata.is_empty() && self.use_triple_quotes()
        {
            self.metadata.insert(
                "cell_type".to_string(),
                Value::String(self.cell_type.to_string()),
            );
        }

        // Go notebooks have '%%' or '%% -' magic commands that need to be escaped
        if self.default_language == "go" && self.language == "go" {
            let re = Regex::new(r"^(//\s*)*(%%\s*$|%%\s+-.*$)").unwrap();
            self.source = self
                .source
                .iter()
                .map(|line| {
                    if re.is_match(line) {
                        re.replace(line, "${1}//gonb:${2}").to_string()
                    } else {
                        line.clone()
                    }
                })
                .collect();
        }

        if self.is_code() {
            return code_to_text(self);
        }

        let mut source = self.source.clone();
        if self.comment.is_empty() {
            escape_code_start(&mut source, &self.ext, "");
        }
        self.markdown_to_text(&source)
    }

    /// Convert a markdown cell source into commented text, handling triple
    /// quote cell markers and comment_magics.
    pub fn markdown_to_text(&self, source: &[String]) -> Vec<String> {
        let cell_markers = self
            .unfiltered_metadata
            .get("cell_marker")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
            .or_else(|| fmt_string(&self.fmt, "cell_markers"));

        if let Some(ref markers) = cell_markers {
            if markers.contains(',') {
                let parts: Vec<&str> = markers.splitn(2, ',').collect();
                let left = parts[0].to_string();
                let right = parts[1].to_string();

                let left3 = if left.len() >= 3 { &left[..3] } else { "" };
                let right3 = if right.len() >= 3 {
                    &right[right.len() - 3..]
                } else {
                    ""
                };
                let left_r = if left.len() >= 4 && (left.starts_with('r') || left.starts_with('R'))
                {
                    &left[1..4]
                } else {
                    ""
                };

                if (left3 == right3 || left_r == right3)
                    && (right3 == "\"\"\"" || right3 == "'''")
                {
                    let mut left = left;
                    // Markdown cells that contain a backslash should be encoded as raw strings
                    if !left.starts_with('r')
                        && !left.starts_with('R')
                        && source.iter().any(|l| l.contains('\\'))
                        && fmt_str(&self.fmt, "format_name") == Some("percent")
                    {
                        left = format!("r{}", left);
                    }

                    let mut out = source.to_vec();
                    out[0] = format!("{}{}", left, out[0]);
                    let last = out.len() - 1;
                    out[last] = format!("{}{}", out[last], right);
                    return out;
                }
            } else {
                // Single marker
                let mut left = format!("{}\n", markers);
                let unmarked = if markers.starts_with('r') || markers.starts_with('R') {
                    &markers[1..]
                } else {
                    markers
                };
                let right = format!("\n{}", unmarked);

                let left3 = if left.len() >= 3 { &left[..3] } else { "" };
                let right3 = if right.len() >= 3 {
                    &right[right.len() - 3..]
                } else {
                    ""
                };
                let left_r = if left.len() >= 4 && (left.starts_with('r') || left.starts_with('R'))
                {
                    &left[1..4]
                } else {
                    ""
                };

                if (left3 == right3 || left_r == right3)
                    && (right3 == "\"\"\"" || right3 == "'''")
                {
                    if !left.starts_with('r')
                        && !left.starts_with('R')
                        && source.iter().any(|l| l.contains('\\'))
                        && fmt_str(&self.fmt, "format_name") == Some("percent")
                    {
                        left = format!("r{}", left);
                    }

                    let mut out = source.to_vec();
                    out[0] = format!("{}{}", left, out[0]);
                    let last = out.len() - 1;
                    out[last] = format!("{}{}", out[last], right);
                    return out;
                }
            }
        }

        // Commented markdown
        if !self.comment.is_empty()
            && self.comment != "#'"
            && is_active(&self.ext, &self.metadata, true)
            && fmt_str(&self.fmt, "format_name") != Some("percent")
            && fmt_str(&self.fmt, "format_name") != Some("hydrogen")
        {
            let mut src = source.to_vec();
            comment_magic(
                &mut src,
                &self.language,
                self.comment_magics,
                self.cell_type == CellType::Code,
            );
            return comment_lines(&src, &self.comment, &self.comment_suffix);
        }

        comment_lines(source, &self.comment, &self.comment_suffix)
    }
}

/// Check whether the cell has any tag starting with "active-"
fn has_active_tag(metadata: &BTreeMap<String, Value>) -> bool {
    if let Some(Value::Array(tags)) = metadata.get("tags") {
        for tag in tags {
            if let Some(s) = tag.as_str() {
                if s.starts_with("active-") {
                    return true;
                }
            }
        }
    }
    false
}

// ===========================================================================
// MarkdownCellExporter
// ===========================================================================

/// Exports notebook cells as Markdown with backtick fences.
pub struct MarkdownCellExporter {
    pub data: BaseCellData,
}

impl MarkdownCellExporter {
    pub fn new(
        cell: &Cell,
        default_language: &str,
        fmt: &BTreeMap<String, Value>,
    ) -> Self {
        let mut data = BaseCellData::new(cell, default_language, fmt, true, Some(false));
        data.comment = String::new();
        MarkdownCellExporter { data }
    }

    /// Protect a Markdown or Raw cell with HTML comments.
    fn html_comment(&self, metadata: &BTreeMap<String, Value>, code: &str) -> Vec<String> {
        let region_start = if !metadata.is_empty() {
            let meta_text = metadata_to_text(None, metadata, self.data.cell_metadata_json);
            format!("<!-- #{} {} -->", code, meta_text)
        } else {
            format!("<!-- #{} -->", code)
        };

        let mut lines = vec![region_start];
        lines.extend(self.data.source.iter().cloned());
        lines.push(format!("<!-- #end{} -->", code));
        lines
    }
}

impl CellExporter for MarkdownCellExporter {
    fn cell_to_text(&mut self) -> Vec<String> {
        if self.data.cell_type == CellType::Markdown {
            // Is an explicit region required?
            let protect = if !self.data.metadata.is_empty() {
                true
            } else {
                // In the Python version this delegates to MarkdownCellReader
                // to check if the source would be parsed back to the same
                // cell. For now we conservatively protect if the source
                // contains markers that could confuse the reader.
                source_needs_protection(&self.data.source, &self.data.cell_type)
            };

            if protect {
                let region_name = self
                    .data
                    .metadata
                    .remove("region_name")
                    .and_then(|v| match v {
                        Value::String(s) => Some(s),
                        _ => None,
                    })
                    .unwrap_or_else(|| "region".to_string());
                return self.html_comment(&self.data.metadata, &region_name);
            }
            return self.data.source.clone();
        }

        // Code or raw cell
        self.code_to_text()
    }

    fn lines_to_next_cell(&self) -> Option<usize> {
        self.data.lines_to_next_cell
    }
}

impl MarkdownCellExporter {
    fn code_to_text(&mut self) -> Vec<String> {
        let mut source = self.data.source.clone();
        comment_magic(
            &mut source,
            &self.data.language,
            self.data.comment_magics,
            true,
        );

        // Remove empty "active" metadata (raw cell default)
        if self.data.metadata.get("active") == Some(&Value::String(String::new())) {
            self.data.metadata.remove("active");
        }

        // Pop language from metadata
        let language = self
            .data
            .metadata
            .remove("language")
            .and_then(|v| match v {
                Value::String(s) => Some(s),
                _ => None,
            })
            .unwrap_or_else(|| self.data.language.clone());

        // If raw cell is not active, wrap as HTML comment
        if self.data.cell_type == CellType::Raw
            && !is_active(&self.data.ext, &self.data.metadata, false)
        {
            return self.html_comment(&self.data.metadata, "raw");
        }

        let options = metadata_to_text(
            Some(&language),
            &self.data.metadata,
            self.data.cell_metadata_json,
        );
        let delimiter = three_backticks_or_more(&self.data.source);
        let mut lines = vec![format!("{}{}", delimiter, options)];
        lines.extend(source);
        lines.push(delimiter);
        lines
    }
}

/// Heuristic: does the source require HTML-comment protection in Markdown
/// format? This is a simplified check; the full check in Python delegates
/// to `MarkdownCellReader.read()`.
fn source_needs_protection(source: &[String], cell_type: &CellType) -> bool {
    if *cell_type != CellType::Markdown {
        return false;
    }
    for line in source {
        // Fenced code blocks or HTML comments would be misread
        if line.starts_with("```") || line.starts_with("<!-- #") {
            return true;
        }
    }
    false
}

// ===========================================================================
// RMarkdownCellExporter
// ===========================================================================

/// Exports notebook cells as R Markdown with ```{language} fences.
pub struct RMarkdownCellExporter {
    pub data: BaseCellData,
}

impl RMarkdownCellExporter {
    pub fn new(
        cell: &Cell,
        default_language: &str,
        fmt: &BTreeMap<String, Value>,
    ) -> Self {
        let mut data = BaseCellData::new(cell, default_language, fmt, true, Some(true));
        data.ext = ".Rmd".to_string();
        data.comment = String::new();
        RMarkdownCellExporter { data }
    }
}

impl CellExporter for RMarkdownCellExporter {
    fn cell_to_text(&mut self) -> Vec<String> {
        if self.data.cell_type == CellType::Markdown {
            let protect = if !self.data.metadata.is_empty() {
                true
            } else {
                source_needs_protection(&self.data.source, &self.data.cell_type)
            };

            if protect {
                let region_name = self
                    .data
                    .metadata
                    .remove("region_name")
                    .and_then(|v| match v {
                        Value::String(s) => Some(s),
                        _ => None,
                    })
                    .unwrap_or_else(|| "region".to_string());
                // Re-use the HTML comment helper from Markdown
                let meta = &self.data.metadata;
                let region_start = if !meta.is_empty() {
                    let meta_text =
                        metadata_to_text(None, meta, self.data.cell_metadata_json);
                    format!("<!-- #{} {} -->", region_name, meta_text)
                } else {
                    format!("<!-- #{} -->", region_name)
                };
                let mut lines = vec![region_start];
                lines.extend(self.data.source.iter().cloned());
                lines.push(format!("<!-- #end{} -->", region_name));
                return lines;
            }
            return self.data.source.clone();
        }

        self.code_to_text()
    }

    fn lines_to_next_cell(&self) -> Option<usize> {
        self.data.lines_to_next_cell
    }
}

impl RMarkdownCellExporter {
    fn code_to_text(&mut self) -> Vec<String> {
        let active = is_active(&self.data.ext, &self.data.metadata, true);
        let mut source = self.data.source.clone();

        if active {
            comment_magic(
                &mut source,
                &self.data.language,
                self.data.comment_magics,
                true,
            );
        }

        let mut lines = Vec::new();
        if !is_active(&self.data.ext, &self.data.metadata, true) {
            self.data
                .metadata
                .insert("eval".to_string(), Value::Bool(false));
        }
        let options = metadata_to_rmd_options(
            Some(&self.data.language),
            &self.data.metadata,
            self.data.use_runtools,
        );
        lines.push(format!("```{{{}}}", options));
        lines.extend(source);
        lines.push("```".to_string());
        lines
    }
}

// ===========================================================================
// LightScriptCellExporter
// ===========================================================================

/// Exports notebook cells in "light" script format with `# +` / `# -`
/// cell markers.
pub struct LightScriptCellExporter {
    pub data: BaseCellData,
    pub use_cell_markers: bool,
    pub cell_marker_start: Option<String>,
    pub cell_marker_end: Option<String>,
}

impl LightScriptCellExporter {
    pub fn new(
        cell: &Cell,
        default_language: &str,
        fmt: &BTreeMap<String, Value>,
    ) -> Self {
        Self::new_inner(cell, default_language, fmt, true)
    }

    fn new_inner(
        cell: &Cell,
        default_language: &str,
        fmt: &BTreeMap<String, Value>,
        use_cell_markers: bool,
    ) -> Self {
        let mut data = BaseCellData::new(cell, default_language, fmt, true, Some(true));

        let mut cell_marker_start: Option<String> = None;
        let mut cell_marker_end: Option<String> = None;

        if let Some(Value::String(markers)) = fmt.get("cell_markers") {
            if !markers.contains(',') {
                // warn and ignore
            } else if markers != "+,-" {
                let parts: Vec<&str> = markers.splitn(2, ',').collect();
                cell_marker_start = Some(parts[0].to_string());
                cell_marker_end = Some(parts[1].to_string());
            }
        }

        // Preserve endofcell from unfiltered metadata
        if let Some(v) = data.unfiltered_metadata.get("endofcell").cloned() {
            data.metadata.insert("endofcell".to_string(), v);
        }

        LightScriptCellExporter {
            data,
            use_cell_markers,
            cell_marker_start,
            cell_marker_end,
        }
    }

    /// Check if this cell is code (with additional light-format logic for
    /// markdown cells that have metadata).
    fn is_code_light(&mut self) -> bool {
        if (self.data.cell_type == CellType::Markdown && !self.data.metadata.is_empty())
            || self.data.use_triple_quotes()
        {
            if is_active(&self.data.ext, &self.data.metadata, true) {
                self.data.metadata.insert(
                    "cell_type".to_string(),
                    Value::String(self.data.cell_type.to_string()),
                );
                let source = self.data.source.clone();
                self.data.source = self.data.markdown_to_text(&source);
                self.data.cell_type = CellType::Code;
                self.data
                    .unfiltered_metadata
                    .remove("cell_marker");
            }
            return true;
        }
        self.data.is_code()
    }

    /// Does the representation of this cell require an explicit start marker?
    fn explicit_start_marker(&self, source: &[String]) -> bool {
        if !self.use_cell_markers {
            return false;
        }
        if !self.data.metadata.is_empty() {
            return true;
        }
        if self.cell_marker_start.is_some() {
            // Custom markers: check if the source already starts with them
            let start_pat = format!(
                r"^{}\s*{}\s*(.*)$",
                regex::escape(&self.data.comment),
                regex::escape(self.cell_marker_start.as_ref().unwrap())
            );
            let end_pat = format!(
                r"^{}\s*{}\s*$",
                regex::escape(&self.data.comment),
                regex::escape(self.cell_marker_end.as_ref().unwrap())
            );
            if let (Ok(start_re), Ok(end_re)) = (Regex::new(&start_pat), Regex::new(&end_pat)) {
                if !source.is_empty()
                    && (start_re.is_match(&source[0]) || end_re.is_match(&source[0]))
                {
                    return false;
                }
            }
        }

        // All lines are comments -> needs explicit marker
        if self
            .data
            .source
            .iter()
            .all(|line| line.starts_with(&self.data.comment))
        {
            return true;
        }

        // Would LightScriptCellReader read fewer lines? (simplified check)
        // In the full implementation this delegates to LightScriptCellReader.
        // Here we use a heuristic: if the source contains blank lines that
        // would split it into multiple cells, we need a marker.
        let has_blank = source.iter().any(|l| l.trim().is_empty());
        if has_blank && source.len() > 1 {
            // Check if the blank line is in the middle of code
            for (i, line) in source.iter().enumerate() {
                if i > 0 && i < source.len() - 1 && line.trim().is_empty() {
                    return true;
                }
            }
        }

        false
    }
}

impl CellExporter for LightScriptCellExporter {
    fn cell_to_text(&mut self) -> Vec<String> {
        // Trigger cell marker for multiline quotes
        if self.data.cell_type != CellType::Code
            && self.data.metadata.is_empty()
            && self.data.use_triple_quotes()
        {
            self.data.metadata.insert(
                "cell_type".to_string(),
                Value::String(self.data.cell_type.to_string()),
            );
        }

        // Go escape
        if self.data.default_language == "go" && self.data.language == "go" {
            let re = Regex::new(r"^(//\s*)*(%%\s*$|%%\s+-.*$)").unwrap();
            self.data.source = self
                .data
                .source
                .iter()
                .map(|line| {
                    if re.is_match(line) {
                        re.replace(line, "${1}//gonb:${2}").to_string()
                    } else {
                        line.clone()
                    }
                })
                .collect();
        }

        if self.is_code_light() {
            return self.code_to_text();
        }

        let mut source = self.data.source.clone();
        if self.data.comment.is_empty() {
            escape_code_start(&mut source, &self.data.ext, "");
        }
        self.data.markdown_to_text(&source)
    }

    fn remove_eoc_marker(&mut self, mut text: Vec<String>, next_text: &[String]) -> Vec<String> {
        if self.cell_marker_start.is_some() {
            return text;
        }

        let eoc_marker = format!("{} -", self.data.comment);
        if self.data.is_code() && text.last().map(|s| s.as_str()) == Some(&eoc_marker) {
            let next_start = format!("{} +", self.data.comment);
            if next_text.is_empty()
                || next_text
                    .first()
                    .map(|s| s.starts_with(&next_start))
                    .unwrap_or(false)
            {
                text.pop();
                // When we do not need the end of cell marker, number of blank
                // lines is the max between that required at the end of the
                // cell and that required before the next cell.
                if let Some(eoc_lines) = self.data.lines_to_end_of_cell_marker {
                    if self.data.lines_to_next_cell.is_none()
                        || eoc_lines > self.data.lines_to_next_cell.unwrap_or(0)
                    {
                        self.data.lines_to_next_cell = Some(eoc_lines);
                    }
                }
            } else {
                // Insert blank lines at the end of the cell
                let blank_lines = match self.data.lines_to_end_of_cell_marker {
                    Some(n) => n,
                    None => {
                        let bl = pep8_lines_between_cells(
                            &text[..text.len() - 1],
                            next_text,
                            &self.data.ext,
                        );
                        if bl < 2 { 0 } else { 2 }
                    }
                };
                let marker = text.pop().unwrap();
                for _ in 0..blank_lines {
                    text.push(String::new());
                }
                text.push(marker);
            }
        }

        text
    }

    fn lines_to_next_cell(&self) -> Option<usize> {
        self.data.lines_to_next_cell
    }
}

impl LightScriptCellExporter {
    fn code_to_text(&mut self) -> Vec<String> {
        let active = is_active(
            &self.data.ext,
            &self.data.metadata,
            same_language(&self.data.language, &self.data.default_language),
        );
        let mut source = self.data.source.clone();
        escape_code_start(&mut source, &self.data.ext, &self.data.language);

        let comment_questions = self
            .data
            .metadata
            .remove("comment_questions")
            .and_then(|v| v.as_bool())
            .unwrap_or(true);

        if active {
            comment_magic(
                &mut source,
                &self.data.language,
                self.data.comment_magics,
                comment_questions,
            );
        } else {
            let md_source = source.clone();
            source = self.data.markdown_to_text(&md_source);
        }

        // Determine if we need an endofcell marker
        if (active
            && comment_questions
            && need_explicit_marker(
                &self.data.source,
                &self.data.language,
                self.data.comment_magics,
            ))
            || self.explicit_start_marker(&source)
        {
            let eoc = self
                .cell_marker_end
                .clone()
                .unwrap_or_else(|| endofcell_marker(&source, &self.data.comment));
            self.data
                .metadata
                .insert("endofcell".to_string(), Value::String(eoc));
        }

        if self.data.metadata.is_empty() || !self.use_cell_markers {
            return source;
        }

        let mut lines = Vec::new();
        let endofcell = self
            .data
            .metadata
            .get("endofcell")
            .and_then(|v| v.as_str())
            .unwrap_or("-")
            .to_string();

        if endofcell == "-" || self.cell_marker_end.is_some() {
            self.data.metadata.remove("endofcell");
        }

        let marker_start = self
            .cell_marker_start
            .as_deref()
            .unwrap_or("+");
        let mut cell_start = vec![
            self.data.comment.clone(),
            marker_start.to_string(),
        ];
        let options =
            metadata_to_double_percent_options(&mut self.data.metadata, self.data.cell_metadata_json);
        if !options.is_empty() {
            cell_start.push(options);
        }
        lines.push(cell_start.join(" "));
        lines.extend(source);
        lines.push(format!("{} {}", self.data.comment, endofcell));
        lines
    }
}

// ===========================================================================
// BareScriptCellExporter
// ===========================================================================

/// Like `LightScriptCellExporter` but without any cell markers.
pub struct BareScriptCellExporter {
    inner: LightScriptCellExporter,
}

impl BareScriptCellExporter {
    pub fn new(
        cell: &Cell,
        default_language: &str,
        fmt: &BTreeMap<String, Value>,
    ) -> Self {
        let inner = LightScriptCellExporter::new_inner(cell, default_language, fmt, false);
        BareScriptCellExporter { inner }
    }
}

impl CellExporter for BareScriptCellExporter {
    fn cell_to_text(&mut self) -> Vec<String> {
        self.inner.cell_to_text()
    }

    fn remove_eoc_marker(&mut self, text: Vec<String>, next_text: &[String]) -> Vec<String> {
        self.inner.remove_eoc_marker(text, next_text)
    }

    fn lines_to_next_cell(&self) -> Option<usize> {
        self.inner.data.lines_to_next_cell
    }
}

// ===========================================================================
// DoublePercentCellExporter
// ===========================================================================

/// Exports notebook cells in Spyder/VSCode percent format (`# %%`).
pub struct DoublePercentCellExporter {
    pub data: BaseCellData,
    pub cell_markers: Option<String>,
}

impl DoublePercentCellExporter {
    pub fn new(
        cell: &Cell,
        default_language: &str,
        fmt: &BTreeMap<String, Value>,
    ) -> Self {
        Self::new_inner(cell, default_language, fmt, true, Some(true))
    }

    fn new_inner(
        cell: &Cell,
        default_language: &str,
        fmt: &BTreeMap<String, Value>,
        parse_cell_language: bool,
        default_comment_magics: Option<bool>,
    ) -> Self {
        let data = BaseCellData::new(
            cell,
            default_language,
            fmt,
            parse_cell_language,
            default_comment_magics,
        );
        let cell_markers = fmt_string(fmt, "cell_markers");
        DoublePercentCellExporter {
            data,
            cell_markers,
        }
    }
}

impl CellExporter for DoublePercentCellExporter {
    fn cell_to_text(&mut self) -> Vec<String> {
        // Go notebooks: escape '%%' or '%% -' magic commands
        if self.data.default_language == "go" && self.data.language == "go" {
            let re = Regex::new(r"^(//\s*)*(%%\s*$|%%\s+-.*$)").unwrap();
            self.data.source = self
                .data
                .source
                .iter()
                .map(|line| {
                    if re.is_match(line) {
                        re.replace(line, "${1}//gonb:${2}").to_string()
                    } else {
                        line.clone()
                    }
                })
                .collect();
        }

        let active = is_active(
            &self.data.ext,
            &self.data.metadata,
            same_language(&self.data.language, &self.data.default_language),
        );

        // Raw cells: remove empty "active" metadata
        if self.data.cell_type == CellType::Raw {
            if self.data.metadata.get("active") == Some(&Value::String(String::new())) {
                self.data.metadata.remove("active");
            }
        }

        // Non-code cells get cell_type in metadata
        if !self.data.is_code() {
            self.data.metadata.insert(
                "cell_type".to_string(),
                Value::String(self.data.cell_type.to_string()),
            );
        }

        let options =
            metadata_to_double_percent_options(&mut self.data.metadata, self.data.cell_metadata_json);

        // Detect indentation from the first non-empty source line
        let indent = if self.data.is_code() && active && !self.data.source.is_empty() {
            let first_line = &self.data.source[0];
            if !first_line.trim().is_empty() {
                let trimmed = first_line.trim_start();
                first_line[..first_line.len() - trimmed.len()].to_string()
            } else {
                String::new()
            }
        } else {
            String::new()
        };

        let marker_line = if options.starts_with('%') || options.is_empty() {
            format!("%%{}", options)
        } else {
            format!("%% {}", options)
        };
        let lines = comment_lines(
            &[marker_line],
            &format!("{}{}", indent, self.data.comment),
            &self.data.comment_suffix,
        );

        if self.data.is_code() && active {
            let mut source = self.data.source.clone();
            comment_magic(
                &mut source,
                &self.data.language,
                self.data.comment_magics,
                true,
            );
            if source == vec![String::new()] {
                return lines;
            }
            let mut result = lines;
            result.extend(source);
            return result;
        }

        let md = self.data.markdown_to_text(&self.data.source.clone());
        let mut result = lines;
        result.extend(md);
        result
    }

    fn lines_to_next_cell(&self) -> Option<usize> {
        self.data.lines_to_next_cell
    }
}

// ===========================================================================
// HydrogenCellExporter
// ===========================================================================

/// Like `DoublePercentCellExporter` but magics are not commented.
pub struct HydrogenCellExporter {
    inner: DoublePercentCellExporter,
}

impl HydrogenCellExporter {
    pub fn new(
        cell: &Cell,
        default_language: &str,
        fmt: &BTreeMap<String, Value>,
    ) -> Self {
        let inner =
            DoublePercentCellExporter::new_inner(cell, default_language, fmt, false, Some(false));
        HydrogenCellExporter { inner }
    }
}

impl CellExporter for HydrogenCellExporter {
    fn cell_to_text(&mut self) -> Vec<String> {
        self.inner.cell_to_text()
    }

    fn lines_to_next_cell(&self) -> Option<usize> {
        self.inner.data.lines_to_next_cell
    }
}

// ===========================================================================
// SphinxGalleryCellExporter
// ===========================================================================

/// Exports notebook cells as Sphinx Gallery scripts.
pub struct SphinxGalleryCellExporter {
    pub data: BaseCellData,
    pub default_cell_marker: String,
}

impl SphinxGalleryCellExporter {
    pub fn new(
        cell: &Cell,
        default_language: &str,
        fmt: &BTreeMap<String, Value>,
    ) -> Self {
        let mut data = BaseCellData::new(cell, default_language, fmt, true, Some(true));
        data.comment = "#".to_string();

        // Preserve cell_marker from unfiltered metadata
        if let Some(v) = data.unfiltered_metadata.get("cell_marker").cloned() {
            data.metadata.insert("cell_marker".to_string(), v);
        }

        if fmt_bool(fmt, "rst2md") == Some(true) {
            panic!(
                "The 'rst2md' option is a read only option. The reverse conversion is not \
                 implemented. Please either deactivate the option, or save to another format."
            );
        }

        SphinxGalleryCellExporter {
            data,
            default_cell_marker: "#".repeat(79),
        }
    }
}

impl CellExporter for SphinxGalleryCellExporter {
    fn cell_to_text(&mut self) -> Vec<String> {
        if self.data.cell_type == CellType::Code {
            let mut source = self.data.source.clone();
            comment_magic(
                &mut source,
                &self.data.language,
                self.data.comment_magics,
                true,
            );
            return source;
        }

        let cell_marker = self
            .data
            .metadata
            .remove("cell_marker")
            .and_then(|v| match v {
                Value::String(s) => Some(s),
                _ => None,
            })
            .unwrap_or_else(|| self.default_cell_marker.clone());

        if self.data.source == vec![String::new()] {
            return if cell_marker == "\"\"" || cell_marker == "''" {
                vec![cell_marker]
            } else {
                vec!["\"\"".to_string()]
            };
        }

        if cell_marker == "\"\"\"" || cell_marker == "'''" {
            let mut lines = vec![cell_marker.clone()];
            lines.extend(self.data.source.iter().cloned());
            lines.push(cell_marker);
            return lines;
        }

        let marker = if cell_marker.starts_with(&"#".repeat(20)) {
            cell_marker
        } else {
            self.default_cell_marker.clone()
        };

        let mut lines = vec![marker];
        lines.extend(comment_lines(
            &self.data.source,
            &self.data.comment,
            &self.data.comment_suffix,
        ));
        lines
    }

    fn lines_to_next_cell(&self) -> Option<usize> {
        self.data.lines_to_next_cell
    }
}

// ===========================================================================
// RScriptCellExporter
// ===========================================================================

/// Exports notebook cells in R knitr spin format (`#'` comments for
/// markdown, `#+` for chunk options).
pub struct RScriptCellExporter {
    pub data: BaseCellData,
}

impl RScriptCellExporter {
    pub fn new(
        cell: &Cell,
        default_language: &str,
        fmt: &BTreeMap<String, Value>,
    ) -> Self {
        let mut data = BaseCellData::new(cell, default_language, fmt, true, Some(true));
        data.comment = "#'".to_string();
        RScriptCellExporter { data }
    }
}

impl CellExporter for RScriptCellExporter {
    fn cell_to_text(&mut self) -> Vec<String> {
        self.data.base_cell_to_text(|d| {
            let active = is_active(&d.ext, &d.metadata, true);
            let mut source = d.source.clone();
            escape_code_start(&mut source, &d.ext, &d.language);

            if active {
                comment_magic(&mut source, &d.language, d.comment_magics, true);
            }

            if !active {
                source = source
                    .iter()
                    .map(|line| {
                        if line.is_empty() {
                            "#".to_string()
                        } else {
                            format!("# {}", line)
                        }
                    })
                    .collect();
            }

            let mut lines = Vec::new();
            if !is_active(&d.ext, &d.metadata, true) {
                d.metadata
                    .insert("eval".to_string(), Value::Bool(false));
            }
            let options =
                metadata_to_rmd_options(None, &d.metadata, d.use_runtools);
            if !options.is_empty() {
                lines.push(format!("#+ {}", options));
            }
            lines.extend(source);
            lines
        })
    }

    fn lines_to_next_cell(&self) -> Option<usize> {
        self.data.lines_to_next_cell
    }
}

// ===========================================================================
// Tests
// ===========================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cell_source_empty() {
        let cell = Cell::new_code("");
        assert_eq!(cell_source(&cell), vec![""]);
    }

    #[test]
    fn test_cell_source_single_line() {
        let cell = Cell::new_code("x = 1");
        assert_eq!(cell_source(&cell), vec!["x = 1"]);
    }

    #[test]
    fn test_cell_source_trailing_newline() {
        let cell = Cell::new_code("x = 1\n");
        assert_eq!(cell_source(&cell), vec!["x = 1", ""]);
    }

    #[test]
    fn test_cell_source_multi_line() {
        let cell = Cell::new_code("x = 1\ny = 2");
        assert_eq!(cell_source(&cell), vec!["x = 1", "y = 2"]);
    }

    #[test]
    fn test_three_backticks_basic() {
        let lines = vec!["some code".to_string()];
        assert_eq!(three_backticks_or_more(&lines), "```");
    }

    #[test]
    fn test_three_backticks_with_fence() {
        let lines = vec!["```python".to_string(), "print('hello')".to_string()];
        assert_eq!(three_backticks_or_more(&lines), "````");
    }

    #[test]
    fn test_three_backticks_nested() {
        let lines = vec!["````".to_string()];
        assert_eq!(three_backticks_or_more(&lines), "`````");
    }

    #[test]
    fn test_endofcell_marker_basic() {
        let source = vec!["x = 1".to_string(), "".to_string(), "y = 2".to_string()];
        assert_eq!(endofcell_marker(&source, "#"), "-");
    }

    #[test]
    fn test_endofcell_marker_conflict() {
        let source = vec!["# -".to_string()];
        assert_eq!(endofcell_marker(&source, "#"), "--");
    }

    #[test]
    fn test_markdown_exporter_code_cell() {
        let cell = Cell::new_code("print('hello')");
        let fmt = {
            let mut m = BTreeMap::new();
            m.insert(
                "extension".to_string(),
                Value::String(".md".to_string()),
            );
            m
        };
        let mut exporter = MarkdownCellExporter::new(&cell, "python", &fmt);
        let text = exporter.cell_to_text();
        assert_eq!(text[0], "```python");
        assert_eq!(text[1], "print('hello')");
        assert_eq!(text[2], "```");
    }

    #[test]
    fn test_markdown_exporter_markdown_cell() {
        let cell = Cell::new_markdown("# Hello");
        let fmt = {
            let mut m = BTreeMap::new();
            m.insert(
                "extension".to_string(),
                Value::String(".md".to_string()),
            );
            m
        };
        let mut exporter = MarkdownCellExporter::new(&cell, "python", &fmt);
        let text = exporter.cell_to_text();
        assert_eq!(text, vec!["# Hello"]);
    }

    #[test]
    fn test_rmarkdown_exporter_code_cell() {
        let cell = Cell::new_code("1 + 1");
        let fmt = {
            let mut m = BTreeMap::new();
            m.insert(
                "extension".to_string(),
                Value::String(".Rmd".to_string()),
            );
            m
        };
        let mut exporter = RMarkdownCellExporter::new(&cell, "R", &fmt);
        let text = exporter.cell_to_text();
        assert_eq!(text[0], "```{r}");
        assert_eq!(text[1], "1 + 1");
        assert_eq!(text[2], "```");
    }

    #[test]
    fn test_double_percent_exporter_code_cell() {
        let cell = Cell::new_code("x = 1");
        let fmt = {
            let mut m = BTreeMap::new();
            m.insert(
                "extension".to_string(),
                Value::String(".py".to_string()),
            );
            m
        };
        let mut exporter = DoublePercentCellExporter::new(&cell, "python", &fmt);
        let text = exporter.cell_to_text();
        assert_eq!(text[0], "# %%");
        assert_eq!(text[1], "x = 1");
    }

    #[test]
    fn test_double_percent_exporter_markdown_cell() {
        let cell = Cell::new_markdown("A short *paragraph*.");
        let fmt = {
            let mut m = BTreeMap::new();
            m.insert(
                "extension".to_string(),
                Value::String(".py".to_string()),
            );
            m
        };
        let mut exporter = DoublePercentCellExporter::new(&cell, "python", &fmt);
        let text = exporter.cell_to_text();
        assert_eq!(text[0], "# %% [markdown]");
        assert_eq!(text[1], "# A short *paragraph*.");
    }

    #[test]
    fn test_sphinx_gallery_code_cell() {
        let cell = Cell::new_code("x = 1");
        let fmt = {
            let mut m = BTreeMap::new();
            m.insert(
                "extension".to_string(),
                Value::String(".py".to_string()),
            );
            m
        };
        let mut exporter = SphinxGalleryCellExporter::new(&cell, "python", &fmt);
        let text = exporter.cell_to_text();
        assert_eq!(text, vec!["x = 1"]);
    }

    #[test]
    fn test_sphinx_gallery_markdown_cell() {
        let cell = Cell::new_markdown("Hello world");
        let fmt = {
            let mut m = BTreeMap::new();
            m.insert(
                "extension".to_string(),
                Value::String(".py".to_string()),
            );
            m
        };
        let mut exporter = SphinxGalleryCellExporter::new(&cell, "python", &fmt);
        let text = exporter.cell_to_text();
        assert_eq!(text[0], "#".repeat(79));
        assert_eq!(text[1], "# Hello world");
    }

    #[test]
    fn test_light_script_simple_code() {
        let cell = Cell::new_code("x = 1");
        let fmt = {
            let mut m = BTreeMap::new();
            m.insert(
                "extension".to_string(),
                Value::String(".py".to_string()),
            );
            m
        };
        let mut exporter = LightScriptCellExporter::new(&cell, "python", &fmt);
        let text = exporter.cell_to_text();
        assert_eq!(text, vec!["x = 1"]);
    }

    #[test]
    fn test_bare_script_simple_code() {
        let cell = Cell::new_code("x = 1");
        let fmt = {
            let mut m = BTreeMap::new();
            m.insert(
                "extension".to_string(),
                Value::String(".py".to_string()),
            );
            m
        };
        let mut exporter = BareScriptCellExporter::new(&cell, "python", &fmt);
        let text = exporter.cell_to_text();
        assert_eq!(text, vec!["x = 1"]);
    }

    #[test]
    fn test_rscript_exporter_code_cell() {
        let cell = Cell::new_code("1 + 1");
        let fmt = {
            let mut m = BTreeMap::new();
            m.insert(
                "extension".to_string(),
                Value::String(".R".to_string()),
            );
            m
        };
        let mut exporter = RScriptCellExporter::new(&cell, "R", &fmt);
        let text = exporter.cell_to_text();
        // RScript format includes a chunk header line before code
        assert_eq!(text, vec!["#+ r", "1 + 1"]);
    }

    #[test]
    fn test_rscript_exporter_markdown_cell() {
        let cell = Cell::new_markdown("Some text");
        let fmt = {
            let mut m = BTreeMap::new();
            m.insert(
                "extension".to_string(),
                Value::String(".R".to_string()),
            );
            m
        };
        let mut exporter = RScriptCellExporter::new(&cell, "R", &fmt);
        let text = exporter.cell_to_text();
        assert_eq!(text, vec!["#' Some text"]);
    }
}
