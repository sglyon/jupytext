//! Read notebook cells from their text representation
//!
//! This module provides cell readers for all supported text formats:
//! - Markdown (.md)
//! - R Markdown (.Rmd)
//! - Light scripts (.py, .jl, etc.)
//! - Double percent / Spyder / VS Code scripts (# %%)
//! - Hydrogen scripts
//! - Sphinx Gallery scripts
//! - R knitr spin scripts

use once_cell::sync::Lazy;
use regex::Regex;
use serde_json::Value;
use std::collections::BTreeMap;

use crate::cell_metadata::{is_active, is_json_metadata, rmd_options_to_metadata, text_to_metadata};
use crate::languages::{
    uncomment_lines, JUPYTER_LANGUAGES_LOWER_AND_UPPER, SCRIPT_EXTENSIONS,
};
use crate::magics::{is_magic, need_explicit_marker, uncomment_magic, unescape_code_start};
use crate::pep8::pep8_lines_between_cells;
use crate::string_parser::StringParser;

// ---------------------------------------------------------------------------
// Shared regex patterns
// ---------------------------------------------------------------------------

static BLANK_LINE: Lazy<Regex> = Lazy::new(|| Regex::new(r"^\s*$").unwrap());
static PY_INDENTED: Lazy<Regex> = Lazy::new(|| Regex::new(r"^\s").unwrap());

// ---------------------------------------------------------------------------
// Helper: uncomment lines with prefix/suffix
// ---------------------------------------------------------------------------

/// Remove the comment prefix (and optionally suffix) from each line.
/// If the prefix followed by a space is present it is removed; otherwise just the
/// bare prefix is stripped.
fn uncomment(lines: &[String], prefix: &str, suffix: &str) -> Vec<String> {
    uncomment_lines(lines, prefix, suffix)
}

// ---------------------------------------------------------------------------
// Free-standing helpers (mirrors of the Python module-level functions)
// ---------------------------------------------------------------------------

/// Is every line in the paragraph commented (or blank after at least one comment)?
fn paragraph_is_fully_commented(lines: &[String], comment: &str, main_language: &str) -> bool {
    for (i, line) in lines.iter().enumerate() {
        if line.starts_with(comment) {
            let rest = &line[comment.len()..];
            if rest.trim_start().starts_with(comment) {
                continue;
            }
            if is_magic(line, main_language, true, false) {
                return false;
            }
            continue;
        }
        return i > 0 && BLANK_LINE.is_match(line);
    }
    true
}

/// Is the next non-blank line indented?
fn next_code_is_indented(lines: &[String]) -> bool {
    for line in lines {
        if BLANK_LINE.is_match(line) {
            continue;
        }
        return PY_INDENTED.is_match(line);
    }
    false
}

/// How many blank lines between end-of-cell marker and next cell?
fn count_lines_to_next_cell(
    cell_end_marker: usize,
    next_cell_start: usize,
    total: usize,
    explicit_eoc: bool,
) -> usize {
    if cell_end_marker < total {
        let mut lines = if next_cell_start >= cell_end_marker {
            next_cell_start - cell_end_marker
        } else {
            0
        };
        if explicit_eoc && lines > 0 {
            lines -= 1;
        }
        if next_cell_start >= total {
            lines += 1;
        }
        return lines;
    }
    1
}

/// Are the last two lines blank, and the third-last NOT blank?
fn last_two_lines_blank(source: &[String]) -> bool {
    if source.len() < 3 {
        return false;
    }
    let n = source.len();
    !BLANK_LINE.is_match(&source[n - 3])
        && BLANK_LINE.is_match(&source[n - 2])
        && BLANK_LINE.is_match(&source[n - 1])
}

// ---------------------------------------------------------------------------
// CellReadResult
// ---------------------------------------------------------------------------

/// The result of reading a single cell from lines.
#[derive(Debug)]
pub struct CellReadResult {
    pub cell: crate::notebook::Cell,
    pub next_position: usize,
}

// ---------------------------------------------------------------------------
// Format description carried through the readers
// ---------------------------------------------------------------------------

/// Configuration extracted from the jupytext format description (a `HashMap`/`BTreeMap`).
#[derive(Debug, Clone, Default)]
pub struct FormatOptions {
    pub extension: Option<String>,
    pub format_name: Option<String>,
    pub format_version: Option<String>,
    pub comment_magics: Option<bool>,
    pub cell_metadata_json: bool,
    pub use_runtools: bool,
    pub split_at_heading: bool,
    pub cell_markers: Option<String>,
    pub rst2md: bool,
    pub doxygen_equation_markers: bool,
}

impl FormatOptions {
    pub fn ext(&self) -> &str {
        self.extension.as_deref().unwrap_or("")
    }
}

// ---------------------------------------------------------------------------
// BaseCellReader – common state and logic
// ---------------------------------------------------------------------------

/// Internal state shared by every reader variant.
struct ReaderState {
    ext: String,
    default_language: String,
    comment_magics: Option<bool>,
    use_runtools: bool,
    format_version: Option<String>,
    cell_metadata_json: bool,
    #[allow(dead_code)]
    doxygen_equation_markers: bool,

    metadata: Option<BTreeMap<String, Value>>,
    org_content: Vec<String>,
    content: Vec<String>,
    explicit_soc: bool,
    explicit_eoc: bool,
    cell_type: Option<CellKind>,
    language: Option<String>,
    lines_to_next_cell: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CellKind {
    Code,
    Markdown,
    Raw,
}

impl CellKind {
    fn to_cell_type(self) -> crate::notebook::CellType {
        match self {
            CellKind::Code => crate::notebook::CellType::Code,
            CellKind::Markdown => crate::notebook::CellType::Markdown,
            CellKind::Raw => crate::notebook::CellType::Raw,
        }
    }
}

impl ReaderState {
    fn new(fmt: &FormatOptions, default_language: Option<&str>) -> Self {
        let ext = fmt.extension.clone().unwrap_or_default();
        let default_lang = default_language
            .map(|s| s.to_string())
            .unwrap_or_else(|| {
                SCRIPT_EXTENSIONS
                    .get(ext.as_str())
                    .map(|sl| sl.language.to_string())
                    .unwrap_or_else(|| "python".to_string())
            });
        ReaderState {
            ext,
            default_language: default_lang,
            comment_magics: fmt.comment_magics,
            use_runtools: fmt.use_runtools,
            format_version: fmt.format_version.clone(),
            cell_metadata_json: fmt.cell_metadata_json,
            doxygen_equation_markers: fmt.doxygen_equation_markers,
            metadata: None,
            org_content: Vec::new(),
            content: Vec::new(),
            explicit_soc: false,
            explicit_eoc: false,
            cell_type: None,
            language: None,
            lines_to_next_cell: 1,
        }
    }

    fn effective_language(&self) -> &str {
        self.language.as_deref().unwrap_or(&self.default_language)
    }

    fn build_cell(&self) -> crate::notebook::Cell {
        let ct = self
            .cell_type
            .unwrap_or(CellKind::Code)
            .to_cell_type();
        let source = self.content.join("\n");
        let mut cell = crate::notebook::Cell::new_with_type(ct, &source);
        if let Some(ref m) = self.metadata {
            cell.metadata = m.clone();
        }
        cell
    }
}

// ---------------------------------------------------------------------------
// Trait: CellReader
// ---------------------------------------------------------------------------

/// A type that can read a single notebook cell from a slice of lines.
pub trait CellReader {
    /// Read one cell from `lines` and return the cell together with the
    /// position (index) of the first line that belongs to the **next** cell.
    fn read(&mut self, lines: &[String]) -> CellReadResult;
}

// =========================================================================
// MarkdownCellReader
// =========================================================================

pub struct MarkdownCellReader {
    state: ReaderState,
    start_code_re: Regex,
    non_jupyter_code_re: Regex,
    end_code_re: Regex,
    start_region_re: Regex,
    end_region_re: Option<Regex>,
    split_at_heading: bool,
    in_region: bool,
    #[allow(dead_code)]
    in_raw: bool,
}

impl MarkdownCellReader {
    pub fn new(fmt: &FormatOptions, default_language: Option<&str>) -> Self {
        let state = ReaderState::new(fmt, default_language);
        let mut reader = MarkdownCellReader {
            state,
            start_code_re: build_markdown_start_code_re(),
            non_jupyter_code_re: Regex::new(r"^```").unwrap(),
            end_code_re: Regex::new(r"^```\s*$").unwrap(),
            start_region_re: Regex::new(
                r"^<!--\s*#(region|markdown|md|raw)(.*)-->\s*$",
            )
            .unwrap(),
            end_region_re: None,
            split_at_heading: fmt.split_at_heading,
            in_region: false,
            in_raw: false,
        };

        if matches!(
            fmt.format_version.as_deref(),
            Some("1.0") | Some("1.1")
        ) && fmt.ext() != ".Rmd"
        {
            reader.start_code_re = Regex::new(r"^```(.*)").unwrap();
            reader.non_jupyter_code_re = Regex::new(r"^```\{").unwrap();
        }

        reader
    }

    // -- option parsing -----------------------------------------------------

    fn metadata_and_language_from_option_line(&mut self, line: &str) {
        if let Some(caps) = self.start_region_re.captures(line) {
            self.in_region = true;
            let region_name = caps.get(1).unwrap().as_str().to_string();
            let rest = caps.get(2).map(|m| m.as_str()).unwrap_or("");

            let end_pat = format!(r"^<!--\s*#end{}\s*-->\s*$", regex::escape(&region_name));
            self.end_region_re = Some(Regex::new(&end_pat).unwrap());

            self.state.cell_metadata_json =
                self.state.cell_metadata_json || is_json_metadata(rest);
            let (title, metadata) = text_to_metadata(rest, true);

            if region_name == "raw" {
                self.state.cell_type = Some(CellKind::Raw);
            } else {
                self.state.cell_type = Some(CellKind::Markdown);
            }
            let mut meta = metadata;
            if !title.is_empty() {
                meta.insert("title".to_string(), Value::String(title));
            }
            if region_name == "markdown" || region_name == "md" {
                meta.insert(
                    "region_name".to_string(),
                    Value::String(region_name),
                );
            }
            self.state.metadata = Some(meta);
        } else if self.start_code_re.is_match(line) {
            let captures = self.start_code_re.captures(line).unwrap();
            let full_match = captures.get(0).unwrap().as_str();
            let options = self.extract_options_from_captures(&captures);
            let (lang, meta) = self.options_to_metadata(&options);
            self.state.language = lang;
            self.state.metadata = Some(meta);

            // Cells with a .noeval attribute are markdown cells #347
            if let Some(ref m) = self.state.metadata {
                if m.get(".noeval").map(|v| v.is_null()).unwrap_or(false) {
                    self.state.cell_type = Some(CellKind::Markdown);
                    self.state.metadata = Some(BTreeMap::new());
                    self.state.language = None;
                }
            }
            let _ = full_match; // suppress warning
        }
    }

    fn extract_options_from_captures(&self, caps: &regex::Captures) -> MarkdownOptions {
        // The start_code_re for Markdown >= 1.2 is:
        //   ^```(`*)(\s*)({languages})($|\s.*$)
        // which has groups: (extra_backticks, ws, language, rest)
        //
        // For Markdown <= 1.1 the pattern is: ^```(.*)
        // which has just one group.
        let groups: Vec<&str> = (1..=caps.len().saturating_sub(1))
            .filter_map(|i| caps.get(i).map(|m| m.as_str()))
            .collect();

        if groups.len() >= 3 {
            // Markdown >= 1.2
            MarkdownOptions::Tuple {
                extra_backticks: groups[0].to_string(),
                rest: groups[1..].iter().map(|s| s.to_string()).collect(),
            }
        } else if groups.len() == 1 {
            MarkdownOptions::Simple(groups[0].to_string())
        } else {
            MarkdownOptions::Simple(String::new())
        }
    }

    fn options_to_metadata(
        &mut self,
        options: &MarkdownOptions,
    ) -> (Option<String>, BTreeMap<String, Value>) {
        match options {
            MarkdownOptions::Tuple {
                extra_backticks,
                rest,
            } => {
                let end_pat = format!("```{}", regex::escape(extra_backticks));
                self.end_code_re = Regex::new(&end_pat).unwrap();
                let joined = rest.join(" ");
                self.state.cell_metadata_json =
                    self.state.cell_metadata_json || is_json_metadata(&joined);
                let (lang, meta) = text_to_metadata(&joined, false);
                let language = if lang.is_empty() { None } else { Some(lang) };
                (language, meta)
            }
            MarkdownOptions::Simple(s) => {
                self.end_code_re = Regex::new(r"^```\s*$").unwrap();
                self.state.cell_metadata_json =
                    self.state.cell_metadata_json || is_json_metadata(s);
                let (lang, meta) = text_to_metadata(s, false);
                let language = if lang.is_empty() { None } else { Some(lang) };
                (language, meta)
            }
        }
    }

    // -- find cell end ------------------------------------------------------

    /// Returns `(cell_end_marker, next_cell_start, explicit_eoc)`.
    fn find_cell_end(&mut self, lines: &[String]) -> (usize, usize, bool) {
        if self.in_region {
            if let Some(ref re) = self.end_region_re {
                for (i, line) in lines.iter().enumerate() {
                    if re.is_match(line) {
                        return (i, i + 1, true);
                    }
                }
            }
        } else if self.state.metadata.is_none() {
            // Default markdown paragraph
            self.state.cell_type = Some(CellKind::Markdown);
            let mut prev_blank: usize = 0;
            let mut in_explicit_code_block = false;
            let mut in_indented_code_block = false;

            for (i, line) in lines.iter().enumerate() {
                if in_explicit_code_block && self.end_code_re.is_match(line) {
                    in_explicit_code_block = false;
                    continue;
                }

                if prev_blank > 0
                    && line.starts_with("    ")
                    && !BLANK_LINE.is_match(line)
                {
                    in_indented_code_block = true;
                    prev_blank = 0;
                    continue;
                }

                if in_indented_code_block
                    && !BLANK_LINE.is_match(line)
                    && !line.starts_with("    ")
                {
                    in_indented_code_block = false;
                }

                if in_indented_code_block || in_explicit_code_block {
                    continue;
                }

                if self.start_region_re.is_match(line) {
                    if i > 1 && prev_blank > 0 {
                        return (i - 1, i, false);
                    }
                    return (i, i, false);
                }

                if self.start_code_re.is_match(line) {
                    let captures = self.start_code_re.captures(line).unwrap();
                    let opts = self.extract_options_from_captures(&captures);
                    let (language, metadata) = self.options_to_metadata(&opts);

                    let lang_str = language.as_deref().unwrap_or("");
                    if !JUPYTER_LANGUAGES_LOWER_AND_UPPER.contains(lang_str)
                        || metadata
                            .get(".noeval")
                            .map(|v| v.is_null())
                            .unwrap_or(false)
                    {
                        in_explicit_code_block = true;
                        prev_blank = 0;
                        continue;
                    }

                    if i > 1 && prev_blank > 0 {
                        return (i - 1, i, false);
                    }
                    return (i, i, false);
                } else if line.starts_with("```{") {
                    // Non-code blocks but we still need to look for their end
                    in_explicit_code_block = true;
                    prev_blank = 0;
                    continue;
                }

                if self.non_jupyter_code_re.is_match(line) {
                    if prev_blank >= 2 {
                        return (i - 2, i, true);
                    }
                    in_explicit_code_block = true;
                    prev_blank = 0;
                    continue;
                }

                if self.split_at_heading && line.starts_with('#') && prev_blank >= 1 {
                    return (i - 1, i, false);
                }

                if BLANK_LINE.is_match(line) {
                    prev_blank += 1;
                } else if prev_blank >= 2 {
                    return (i - 2, i, true);
                } else {
                    prev_blank = 0;
                }
            }
        } else {
            // Code cell
            self.state.cell_type = Some(CellKind::Code);
            let lang = self.state.effective_language().to_string();
            let mut parser = StringParser::new(&lang);
            for (i, line) in lines.iter().enumerate() {
                if i == 0 {
                    continue;
                }
                if parser.is_quoted() {
                    parser.read_line(line);
                    continue;
                }
                parser.read_line(line);
                if self.end_code_re.is_match(line) {
                    return (i, i + 1, true);
                }
            }
        }

        // End not found
        (lines.len(), lines.len(), false)
    }

    // -- uncomment ----------------------------------------------------------

    fn uncomment_code_and_magics(&self, lines: &mut Vec<String>) {
        if self.state.cell_type == Some(CellKind::Code)
            && self.state.comment_magics.unwrap_or(false)
        {
            let lang = self.state.effective_language().to_string();
            uncomment_magic(lines, &lang, true, false);
        }
        // doxygen_to_markdown would go here if we supported it
    }

    // -- extract content (shared base logic) --------------------------------

    fn extract_content(&mut self, lines: Vec<String>, comment: &str) -> Vec<String> {
        let meta = self.state.metadata.clone().unwrap_or_default();

        // Code cells with just a multiline string become Markdown cells (Python)
        if self.state.ext == ".py"
            && !is_active(
                &self.state.ext,
                &meta,
                self.state.cell_type == Some(CellKind::Code),
            )
        {
            let content_str = lines.join("\n");
            let trimmed = content_str.trim().to_string();
            let prefixes: Vec<&str> = if self.state.ext == ".py" {
                vec!["", "r", "R"]
            } else {
                vec![""]
            };
            for prefix in &prefixes {
                for triple_quote in &["\"\"\"", "'''"] {
                    let left = format!("{}{}", prefix, triple_quote);
                    let right = triple_quote.to_string();
                    if trimmed.starts_with(&left)
                        && trimmed.ends_with(&right)
                        && trimmed.len() >= left.len() + right.len()
                    {
                        let mut inner =
                            trimmed[left.len()..trimmed.len() - right.len()].to_string();
                        let mut actual_left = left.clone();
                        let mut actual_right = right.clone();
                        if inner.starts_with('\n') {
                            inner = inner[1..].to_string();
                            actual_left = format!("{}\n", actual_left);
                        }
                        if inner.ends_with('\n') {
                            inner = inner[..inner.len() - 1].to_string();
                            actual_right = format!("\n{}", actual_right);
                        }

                        let m = self.state.metadata.get_or_insert_with(BTreeMap::new);
                        if prefix.is_empty() {
                            if actual_left.len() == actual_right.len()
                                && actual_left.len() == 4
                            {
                                m.insert(
                                    "cell_marker".to_string(),
                                    Value::String(actual_left[..3].to_string()),
                                );
                            }
                        } else if actual_left.len() == 4
                            && actual_right.len() == 4
                        {
                            m.insert(
                                "cell_marker".to_string(),
                                Value::String(actual_left[..4].to_string()),
                            );
                        } else {
                            m.insert(
                                "cell_marker".to_string(),
                                Value::String(format!("{},{}", actual_left, actual_right)),
                            );
                        }

                        return inner.lines().map(|l| l.to_string()).collect();
                    }
                }
            }
        }

        let ext_is_r = self.state.ext == ".r" || self.state.ext == ".R";
        if !is_active(&self.state.ext, &meta, true)
            || (meta.get("active").is_none()
                && self.state.language.is_some()
                && self.state.language.as_deref() != Some(&self.state.default_language))
        {
            let prefix = if ext_is_r { "#" } else { comment };
            return uncomment(&lines, prefix, "");
        }

        let mut result = lines;
        self.uncomment_code_and_magics(&mut result);
        result
    }

    // -- find_cell_content (base logic) -------------------------------------

    fn find_cell_content(&mut self, lines: &[String]) -> usize {
        let (cell_end_marker, mut next_cell_start, explicit_eoc) =
            self.find_cell_end(lines);
        self.state.explicit_eoc = explicit_eoc;

        // Metadata to dict
        let cell_start = if self.state.metadata.is_none() {
            self.state.metadata = Some(BTreeMap::new());
            0
        } else {
            1
        };

        // Cell content
        let source: Vec<String> = lines
            [cell_start..cell_end_marker.min(lines.len())]
            .to_vec();
        self.state.org_content = source.clone();

        // Exactly two empty lines at the end of cell (caused by PEP8)?
        if self.state.ext == ".py" && explicit_eoc {
            let lines_to_end = if last_two_lines_blank(&source) {
                2
            } else {
                0
            };
            let trimmed_source = if lines_to_end == 2 {
                &source[..source.len() - 2]
            } else {
                &source[..]
            };
            let pep8 = pep8_lines_between_cells(
                trimmed_source,
                &lines[cell_end_marker..],
                &self.state.ext,
            );
            let expected = if pep8 == 1 { 0 } else { 2 };
            if lines_to_end != expected {
                let m = self.state.metadata.get_or_insert_with(BTreeMap::new);
                m.insert(
                    "lines_to_end_of_cell_marker".to_string(),
                    Value::from(lines_to_end as i64),
                );
            }
        }

        // Uncomment content
        self.state.explicit_soc = cell_start > 0;
        let source_for_extract = lines
            [cell_start..cell_end_marker.min(lines.len())]
            .to_vec();
        self.state.content = self.extract_content(source_for_extract, "");

        // Is this an inactive cell?
        if self.state.cell_type == Some(CellKind::Code) {
            let meta = self.state.metadata.clone().unwrap_or_default();
            if !is_active(".ipynb", &meta, true) {
                if meta.get("active").and_then(|v| v.as_str()) == Some("") {
                    if let Some(ref mut m) = self.state.metadata {
                        m.remove("active");
                    }
                }
                self.state.cell_type = Some(CellKind::Raw);
            } else if (self.state.ext == ".md" || self.state.ext == ".markdown")
                && self.state.language.is_none()
            {
                if !matches!(
                    self.state.format_version.as_deref(),
                    Some("1.0") | Some("1.1")
                ) {
                    self.state.cell_type = Some(CellKind::Markdown);
                    self.state.explicit_eoc = false;
                    let new_end = cell_end_marker + 1;
                    self.state.content =
                        lines[..new_end.min(lines.len())].to_vec();
                } else {
                    self.state.cell_type = Some(CellKind::Raw);
                }
            }
        }

        // Explicit end of cell marker? Advance past blank lines.
        if next_cell_start + 1 < lines.len()
            && BLANK_LINE.is_match(&lines[next_cell_start])
            && !BLANK_LINE.is_match(&lines[next_cell_start + 1])
        {
            next_cell_start += 1;
        } else if self.state.explicit_eoc
            && next_cell_start + 2 < lines.len()
            && BLANK_LINE.is_match(&lines[next_cell_start])
            && BLANK_LINE.is_match(&lines[next_cell_start + 1])
            && !BLANK_LINE.is_match(&lines[next_cell_start + 2])
        {
            next_cell_start += 2;
        }

        self.state.lines_to_next_cell = count_lines_to_next_cell(
            cell_end_marker,
            next_cell_start,
            lines.len(),
            self.state.explicit_eoc,
        );

        next_cell_start
    }
}

/// Helper enum for markdown option styles.
enum MarkdownOptions {
    /// `(extra_backticks, [ws, language, rest...])`
    Tuple {
        extra_backticks: String,
        rest: Vec<String>,
    },
    /// Single string (for Markdown <= 1.1)
    Simple(String),
}

/// Build the start_code_re used for Markdown >= 1.2
fn build_markdown_start_code_re() -> Regex {
    let langs: Vec<String> = JUPYTER_LANGUAGES_LOWER_AND_UPPER
        .iter()
        .map(|l| regex::escape(l))
        .collect();
    let langs_pattern = langs.join("|");
    let pattern = format!(r"^```(`*)(\s*)({})(|\s.*)$", langs_pattern);
    Regex::new(&pattern).unwrap()
}

impl CellReader for MarkdownCellReader {
    fn read(&mut self, lines: &[String]) -> CellReadResult {
        // Parse the option / start marker on line 0
        self.metadata_and_language_from_option_line(&lines[0]);

        if let Some(ref mut m) = self.state.metadata {
            if let Some(lang) = m.remove("language") {
                if let Some(s) = lang.as_str() {
                    self.state.language = Some(s.to_string());
                }
            }
        }

        let pos_next_cell = self.find_cell_content(lines);

        if let Some(ref mut m) = self.state.metadata {
            if m.is_empty() {
                // keep empty
            }
        } else {
            self.state.metadata = Some(BTreeMap::new());
        }

        let empty_fallback = vec!["".to_string()];
        let expected_blank_lines = if self.state.ext == ".py" {
            let org = if self.state.org_content.is_empty() {
                &empty_fallback
            } else {
                &self.state.org_content
            };
            pep8_lines_between_cells(
                org,
                &lines[pos_next_cell..],
                &self.state.ext,
            )
        } else {
            1
        };

        if self.state.lines_to_next_cell != expected_blank_lines {
            let m = self.state.metadata.get_or_insert_with(BTreeMap::new);
            m.insert(
                "lines_to_next_cell".to_string(),
                Value::from(self.state.lines_to_next_cell as i64),
            );
        }

        if let Some(ref lang) = self.state.language {
            let m = self.state.metadata.get_or_insert_with(BTreeMap::new);
            m.insert("language".to_string(), Value::String(lang.clone()));
        }

        let cell = self.state.build_cell();
        CellReadResult {
            cell,
            next_position: pos_next_cell,
        }
    }
}

// =========================================================================
// RMarkdownCellReader
// =========================================================================

pub struct RMarkdownCellReader {
    inner: MarkdownCellReader,
}

impl RMarkdownCellReader {
    pub fn new(fmt: &FormatOptions, default_language: Option<&str>) -> Self {
        let dl = default_language.unwrap_or("R");
        let mut inner = MarkdownCellReader::new(fmt, Some(dl));
        inner.state.comment_magics = fmt.comment_magics.or(Some(true));
        inner.start_code_re = Regex::new(r"^```\{(.*)\}\s*$").unwrap();
        inner.non_jupyter_code_re = Regex::new(r"^```([^\{]|\s*$)").unwrap();
        RMarkdownCellReader { inner }
    }
}

impl CellReader for RMarkdownCellReader {
    fn read(&mut self, lines: &[String]) -> CellReadResult {
        // Override options_to_metadata by intercepting before/after
        // Parse option line
        let line0 = &lines[0];
        if self.inner.start_code_re.is_match(line0) {
            if let Some(caps) = self.inner.start_code_re.captures(line0) {
                let options = caps.get(1).map(|m| m.as_str()).unwrap_or("");
                let (lang, meta) =
                    rmd_options_to_metadata(options, self.inner.state.use_runtools);
                self.inner.state.language = Some(lang);
                self.inner.state.metadata = Some(meta);
            }
        } else {
            // Re-use parent's region/other logic
            self.inner.metadata_and_language_from_option_line(line0);
        }

        if let Some(ref mut m) = self.inner.state.metadata {
            if let Some(lang) = m.remove("language") {
                if let Some(s) = lang.as_str() {
                    self.inner.state.language = Some(s.to_string());
                }
            }
        }

        let pos_next_cell = self.inner.find_cell_content(lines);

        if self.inner.state.metadata.is_none() {
            self.inner.state.metadata = Some(BTreeMap::new());
        }

        let expected_blank_lines = 1;
        if self.inner.state.lines_to_next_cell != expected_blank_lines {
            let m = self
                .inner
                .state
                .metadata
                .get_or_insert_with(BTreeMap::new);
            m.insert(
                "lines_to_next_cell".to_string(),
                Value::from(self.inner.state.lines_to_next_cell as i64),
            );
        }

        if let Some(ref lang) = self.inner.state.language {
            let m = self
                .inner
                .state
                .metadata
                .get_or_insert_with(BTreeMap::new);
            m.insert("language".to_string(), Value::String(lang.clone()));
        }

        let cell = self.inner.state.build_cell();
        CellReadResult {
            cell,
            next_position: pos_next_cell,
        }
    }
}

// =========================================================================
// ScriptCellReader – shared base for script-format readers
// =========================================================================

/// Shared uncomment-code-and-magics for script readers.
fn script_uncomment_code_and_magics(
    lines: &mut Vec<String>,
    state: &mut ReaderState,
    comment: &str,
    comment_suffix: &str,
    markdown_prefix: Option<&str>,
) {
    let cell_type = state.cell_type.unwrap_or(CellKind::Code);
    let lang = state.effective_language().to_string();

    if cell_type == CellKind::Code || markdown_prefix != Some("#'") {
        let esc_flag = state.comment_magics.unwrap_or(true);
        if esc_flag {
            if is_active(&state.ext, &state.metadata.clone().unwrap_or_default(), true) {
                uncomment_magic(lines, &lang, true, state.explicit_soc);
                if cell_type == CellKind::Code
                    && !state.explicit_soc
                    && need_explicit_marker(lines, &lang, true)
                {
                    let m = state.metadata.get_or_insert_with(BTreeMap::new);
                    m.insert(
                        "comment_questions".to_string(),
                        Value::Bool(false),
                    );
                }
            } else {
                *lines = uncomment(lines, comment, "");
            }
        }
    }

    // Go notebook double-percent handling
    if state.default_language == "go" && state.language.is_none() {
        let re = Regex::new(r"^((//\s*)*)(//\s*gonb:%%)")
            .unwrap();
        for line in lines.iter_mut() {
            if re.is_match(line) {
                *line = re.replace(line, "${1}%%").to_string();
            }
        }
    }

    if cell_type == CellKind::Code {
        unescape_code_start(
            lines,
            &state.ext,
            &lang,
        );
    } else {
        let prefix = markdown_prefix.unwrap_or(comment);
        *lines = uncomment(lines, prefix, comment_suffix);
    }
}

/// Shared extract_content for script readers.
fn script_extract_content(
    lines: Vec<String>,
    state: &mut ReaderState,
    comment: &str,
    comment_suffix: &str,
    markdown_prefix: Option<&str>,
) -> Vec<String> {
    let meta = state.metadata.clone().unwrap_or_default();

    // Code cells with just a multiline string become Markdown cells (Python)
    if state.ext == ".py"
        && !is_active(
            &state.ext,
            &meta,
            state.cell_type == Some(CellKind::Code),
        )
    {
        let content_str = lines.join("\n");
        let trimmed = content_str.trim().to_string();
        let prefixes: Vec<&str> = vec!["", "r", "R"];
        for prefix in &prefixes {
            for triple_quote in &["\"\"\"", "'''"] {
                let left = format!("{}{}", prefix, triple_quote);
                let right = triple_quote.to_string();
                if trimmed.starts_with(&left)
                    && trimmed.ends_with(&right)
                    && trimmed.len() >= left.len() + right.len()
                {
                    let mut inner =
                        trimmed[left.len()..trimmed.len() - right.len()].to_string();
                    let mut actual_left = left.clone();
                    let mut actual_right = right.clone();
                    if inner.starts_with('\n') {
                        inner = inner[1..].to_string();
                        actual_left = format!("{}\n", actual_left);
                    }
                    if inner.ends_with('\n') {
                        inner = inner[..inner.len() - 1].to_string();
                        actual_right = format!("\n{}", actual_right);
                    }

                    let m = state.metadata.get_or_insert_with(BTreeMap::new);
                    if prefix.is_empty() {
                        if actual_left.len() == actual_right.len() && actual_left.len() == 4 {
                            m.insert(
                                "cell_marker".to_string(),
                                Value::String(actual_left[..3].to_string()),
                            );
                        }
                    } else if actual_left[1..].len() == 3 && actual_right.len() == 4 {
                        m.insert(
                            "cell_marker".to_string(),
                            Value::String(actual_left[..4].to_string()),
                        );
                    } else {
                        m.insert(
                            "cell_marker".to_string(),
                            Value::String(format!("{},{}", actual_left, actual_right)),
                        );
                    }

                    return inner.lines().map(|l| l.to_string()).collect();
                }
            }
        }
    }

    let ext_is_r = state.ext == ".r" || state.ext == ".R";
    if !is_active(&state.ext, &meta, true)
        || (meta.get("active").is_none()
            && state.language.is_some()
            && state.language.as_deref() != Some(&state.default_language))
    {
        let prefix = if ext_is_r { "#" } else { comment };
        return uncomment(&lines, prefix, "");
    }

    let mut result = lines;
    script_uncomment_code_and_magics(
        &mut result,
        state,
        comment,
        comment_suffix,
        markdown_prefix,
    );
    result
}

// =========================================================================
// LightScriptCellReader
// =========================================================================

pub struct LightScriptCellReader {
    state: ReaderState,
    comment: String,
    comment_suffix: String,
    start_code_re: Regex,
    simple_start_code_re: Option<Regex>,
    end_code_re: Option<Regex>,
    cell_marker_start: Option<String>,
    cell_marker_end: Option<String>,
    ignore_end_marker: bool,
    explicit_end_marker_required: bool,
    markdown_prefix: Option<String>,
}

impl LightScriptCellReader {
    pub fn new(fmt: &FormatOptions, default_language: Option<&str>) -> Self {
        let mut state = ReaderState::new(fmt, default_language);
        if state.ext.is_empty() {
            state.ext = ".py".to_string();
        }
        state.comment_magics = fmt.comment_magics.or(Some(true));

        let script = SCRIPT_EXTENSIONS
            .get(state.ext.as_str())
            .cloned()
            .unwrap_or(crate::languages::ScriptLanguage {
                language: "python",
                comment: "#",
                comment_suffix: "",
            });

        if default_language.is_none() {
            state.default_language = script.language.to_string();
        }
        let comment = script.comment.to_string();
        let comment_suffix = script.comment_suffix.to_string();

        let mut cell_marker_start: Option<String> = None;
        let mut cell_marker_end: Option<String> = None;
        let start_code_re;
        let mut end_code_re: Option<Regex> = None;

        let format_name = fmt.format_name.as_deref().unwrap_or("light");
        if format_name == "light" {
            if let Some(ref markers) = fmt.cell_markers {
                if markers != "+,-" {
                    let parts: Vec<&str> = markers.splitn(2, ',').collect();
                    if parts.len() == 2 {
                        cell_marker_start = Some(parts[0].to_string());
                        cell_marker_end = Some(parts[1].to_string());
                        let pat = format!(
                            r"^{}\s*{}(.*)$",
                            regex::escape(&comment),
                            regex::escape(parts[0])
                        );
                        start_code_re = Regex::new(&pat).unwrap();
                        let end_pat = format!(
                            r"^{}\s*{}\s*$",
                            regex::escape(&comment),
                            regex::escape(parts[1])
                        );
                        end_code_re = Some(Regex::new(&end_pat).unwrap());
                    } else {
                        let pat = format!(r"^{}\s*\+(.*)$", regex::escape(&comment));
                        start_code_re = Regex::new(&pat).unwrap();
                    }
                } else {
                    let pat = format!(r"^{}\s*\+(.*)$", regex::escape(&comment));
                    start_code_re = Regex::new(&pat).unwrap();
                }
            } else {
                let pat = format!(r"^{}\s*\+(.*)$", regex::escape(&comment));
                start_code_re = Regex::new(&pat).unwrap();
            }
        } else {
            let pat = format!(r"^{}\s*\+(.*)$", regex::escape(&comment));
            start_code_re = Regex::new(&pat).unwrap();
        }

        LightScriptCellReader {
            state,
            comment,
            comment_suffix,
            start_code_re,
            simple_start_code_re: None, // set externally if needed
            end_code_re,
            cell_marker_start,
            cell_marker_end,
            ignore_end_marker: true,
            explicit_end_marker_required: false,
            markdown_prefix: None,
        }
    }

    fn metadata_and_language_from_option_line(&mut self, line: &str) {
        if self.start_code_re.is_match(line) {
            // Remove the comment suffix if present
            let mut trimmed_line = line.to_string();
            if !self.comment_suffix.is_empty() {
                let space_suffix = format!(" {}", self.comment_suffix);
                if trimmed_line.ends_with(&space_suffix) {
                    trimmed_line =
                        trimmed_line[..trimmed_line.len() - space_suffix.len()].to_string();
                } else if trimmed_line.ends_with(&self.comment_suffix) {
                    trimmed_line = trimmed_line
                        [..trimmed_line.len() - self.comment_suffix.len()]
                        .to_string();
                }
            }

            if let Some(caps) = self.start_code_re.captures(&trimmed_line) {
                let options = caps.get(1).map(|m| m.as_str()).unwrap_or("");
                let (lang, meta) = self.options_to_metadata(options);
                self.state.language = lang;
                self.state.metadata = Some(meta);
            }
            self.ignore_end_marker = false;
            if self.cell_marker_start.is_some() {
                self.explicit_end_marker_required = true;
            }
        } else if let Some(ref re) = self.simple_start_code_re {
            if re.is_match(line) {
                self.state.metadata = Some(BTreeMap::new());
                self.ignore_end_marker = false;
            }
        } else if self.cell_marker_end.is_some() {
            if let Some(ref re) = self.end_code_re {
                if re.is_match(line) {
                    self.state.metadata = None;
                    self.state.cell_type = Some(CellKind::Code);
                }
            }
        }
    }

    fn options_to_metadata(
        &mut self,
        options: &str,
    ) -> (Option<String>, BTreeMap<String, Value>) {
        self.state.cell_metadata_json =
            self.state.cell_metadata_json || is_json_metadata(options);
        let (title, mut metadata) = text_to_metadata(options, true);

        // Cell type
        for cell_type in &["markdown", "raw", "md"] {
            let code = format!("[{}]", cell_type);
            if title.contains(&code) {
                let cleaned_title = title.replace(&code, "").trim().to_string();
                let actual_type = if *cell_type == "md" {
                    metadata.insert(
                        "region_name".to_string(),
                        Value::String("md".to_string()),
                    );
                    "markdown"
                } else {
                    cell_type
                };
                metadata.insert(
                    "cell_type".to_string(),
                    Value::String(actual_type.to_string()),
                );
                if !cleaned_title.is_empty() {
                    metadata.insert(
                        "title".to_string(),
                        Value::String(cleaned_title),
                    );
                }
                return (None, metadata);
            }
        }

        // Spyder sub cells
        let mut cell_depth: usize = 0;
        let mut remaining_title = title.clone();
        while remaining_title.starts_with('%') {
            cell_depth += 1;
            remaining_title = remaining_title[1..].to_string();
        }
        if cell_depth > 0 {
            metadata.insert(
                "cell_depth".to_string(),
                Value::from(cell_depth as i64),
            );
            remaining_title = remaining_title.trim().to_string();
        }

        if !remaining_title.is_empty() {
            metadata.insert(
                "title".to_string(),
                Value::String(remaining_title),
            );
        }

        (None, metadata)
    }

    fn find_cell_end(&mut self, lines: &[String]) -> (usize, usize, bool) {
        if self.state.metadata.is_none()
            && !(self.cell_marker_end.is_some()
                && self
                    .end_code_re
                    .as_ref()
                    .map(|re| re.is_match(&lines[0]))
                    .unwrap_or(false))
            && paragraph_is_fully_commented(
                lines,
                &self.comment,
                &self.state.default_language,
            )
        {
            self.state.cell_type = Some(CellKind::Markdown);
            for (i, line) in lines.iter().enumerate() {
                if BLANK_LINE.is_match(line) {
                    return (i, i + 1, false);
                }
            }
            return (lines.len(), lines.len(), false);
        }

        if self.state.metadata.is_none() {
            self.end_code_re = None;
        } else if self.cell_marker_end.is_none() {
            let meta = self.state.metadata.clone().unwrap_or_default();
            let end_of_cell = meta
                .get("endofcell")
                .and_then(|v| v.as_str())
                .unwrap_or("-");
            let pat = format!(
                r"^{} {}\s*$",
                regex::escape(&self.comment),
                regex::escape(end_of_cell)
            );
            self.end_code_re = Some(Regex::new(&pat).unwrap());
        }

        self.find_region_end(lines)
    }

    fn find_region_end(&mut self, lines: &[String]) -> (usize, usize, bool) {
        let meta = self.state.metadata.clone().unwrap_or_default();
        if let Some(ct_val) = meta.get("cell_type") {
            if let Some(ct_str) = ct_val.as_str() {
                match ct_str {
                    "markdown" => self.state.cell_type = Some(CellKind::Markdown),
                    "raw" => self.state.cell_type = Some(CellKind::Raw),
                    _ => self.state.cell_type = Some(CellKind::Code),
                }
                if let Some(ref mut m) = self.state.metadata {
                    m.remove("cell_type");
                }
            } else {
                self.state.cell_type = Some(CellKind::Code);
            }
        } else {
            self.state.cell_type = Some(CellKind::Code);
        }

        let lang = self.state.effective_language().to_string();
        let mut parser = StringParser::new(&lang);
        for (i, line) in lines.iter().enumerate() {
            if self.state.metadata.is_some() && i == 0 {
                continue;
            }
            if parser.is_quoted() {
                parser.read_line(line);
                continue;
            }
            parser.read_line(line);

            // New code region
            let simple_match = self.simple_start_code_re.as_ref().map(|re| {
                re.is_match(line)
                    && (self.cell_marker_start.is_some()
                        || i == 0
                        || BLANK_LINE.is_match(&lines[i - 1]))
            });
            if self.start_code_re.is_match(line)
                || simple_match.unwrap_or(false)
            {
                if self.explicit_end_marker_required {
                    self.state.metadata = None;
                    self.state.language = None;
                }

                if i > 0 && BLANK_LINE.is_match(&lines[i - 1]) {
                    if i > 1 && BLANK_LINE.is_match(&lines[i - 2]) {
                        return (i - 2, i, false);
                    }
                    return (i - 1, i, false);
                }
                return (i, i, false);
            }

            if !self.ignore_end_marker {
                if let Some(ref re) = self.end_code_re {
                    if re.is_match(line) {
                        return (i, i + 1, true);
                    }
                }
            } else if BLANK_LINE.is_match(line) {
                if !next_code_is_indented(&lines[i..]) {
                    if i > 0 {
                        return (i, i + 1, false);
                    }
                    if lines.len() > 1 && !BLANK_LINE.is_match(&lines[1]) {
                        return (1, 1, false);
                    }
                    return (1, 2, false);
                }
            }
        }

        (lines.len(), lines.len(), false)
    }

    fn find_cell_content(&mut self, lines: &[String]) -> usize {
        let (cell_end_marker, mut next_cell_start, explicit_eoc) =
            self.find_cell_end(lines);
        self.state.explicit_eoc = explicit_eoc;

        let cell_start = if self.state.metadata.is_none() {
            self.state.metadata = Some(BTreeMap::new());
            0
        } else {
            1
        };

        let source: Vec<String> = lines
            [cell_start..cell_end_marker.min(lines.len())]
            .to_vec();
        self.state.org_content = source.clone();

        // PEP8 blank lines
        if self.state.ext == ".py" && explicit_eoc {
            let lines_to_end = if last_two_lines_blank(&source) {
                2
            } else {
                0
            };
            let trimmed_source = if lines_to_end == 2 {
                &source[..source.len() - 2]
            } else {
                &source[..]
            };
            let pep8 = pep8_lines_between_cells(
                trimmed_source,
                &lines[cell_end_marker..],
                &self.state.ext,
            );
            let expected = if pep8 == 1 { 0 } else { 2 };
            if lines_to_end != expected {
                let m = self.state.metadata.get_or_insert_with(BTreeMap::new);
                m.insert(
                    "lines_to_end_of_cell_marker".to_string(),
                    Value::from(lines_to_end as i64),
                );
            }
        }

        self.state.explicit_soc = cell_start > 0;
        let source_for_extract = lines
            [cell_start..cell_end_marker.min(lines.len())]
            .to_vec();
        self.state.content = script_extract_content(
            source_for_extract,
            &mut self.state,
            &self.comment.clone(),
            &self.comment_suffix.clone(),
            self.markdown_prefix.as_deref(),
        );

        // Is this an inactive cell?
        if self.state.cell_type == Some(CellKind::Code) {
            let meta = self.state.metadata.clone().unwrap_or_default();
            if !is_active(".ipynb", &meta, true) {
                if meta.get("active").and_then(|v| v.as_str()) == Some("") {
                    if let Some(ref mut m) = self.state.metadata {
                        m.remove("active");
                    }
                }
                self.state.cell_type = Some(CellKind::Raw);
            }
        }

        // Advance past blank lines
        if next_cell_start + 1 < lines.len()
            && BLANK_LINE.is_match(&lines[next_cell_start])
            && !BLANK_LINE.is_match(&lines[next_cell_start + 1])
        {
            next_cell_start += 1;
        } else if self.state.explicit_eoc
            && next_cell_start + 2 < lines.len()
            && BLANK_LINE.is_match(&lines[next_cell_start])
            && BLANK_LINE.is_match(&lines[next_cell_start + 1])
            && !BLANK_LINE.is_match(&lines[next_cell_start + 2])
        {
            next_cell_start += 2;
        }

        self.state.lines_to_next_cell = count_lines_to_next_cell(
            cell_end_marker,
            next_cell_start,
            lines.len(),
            self.state.explicit_eoc,
        );

        next_cell_start
    }
}

impl CellReader for LightScriptCellReader {
    fn read(&mut self, lines: &[String]) -> CellReadResult {
        self.metadata_and_language_from_option_line(&lines[0]);

        if let Some(ref mut m) = self.state.metadata {
            if let Some(lang) = m.remove("language") {
                if let Some(s) = lang.as_str() {
                    self.state.language = Some(s.to_string());
                }
            }
        }

        let pos_next_cell = self.find_cell_content(lines);

        if self.state.metadata.is_none() {
            self.state.metadata = Some(BTreeMap::new());
        }

        let empty_fallback = vec!["".to_string()];
        let expected_blank_lines = if self.state.ext == ".py" {
            let org = if self.state.org_content.is_empty() {
                &empty_fallback
            } else {
                &self.state.org_content
            };
            pep8_lines_between_cells(
                org,
                &lines[pos_next_cell..],
                &self.state.ext,
            )
        } else {
            1
        };

        if self.state.lines_to_next_cell != expected_blank_lines {
            let m = self.state.metadata.get_or_insert_with(BTreeMap::new);
            m.insert(
                "lines_to_next_cell".to_string(),
                Value::from(self.state.lines_to_next_cell as i64),
            );
        }

        if let Some(ref lang) = self.state.language {
            let m = self.state.metadata.get_or_insert_with(BTreeMap::new);
            m.insert("language".to_string(), Value::String(lang.clone()));
        }

        let cell = self.state.build_cell();
        CellReadResult {
            cell,
            next_position: pos_next_cell,
        }
    }
}

// =========================================================================
// DoublePercentScriptCellReader
// =========================================================================

pub struct DoublePercentScriptCellReader {
    state: ReaderState,
    comment: String,
    comment_suffix: String,
    start_code_re: Regex,
    alternative_start_code_re: Regex,
    markdown_prefix: Option<String>,
}

impl DoublePercentScriptCellReader {
    pub fn new(fmt: &FormatOptions, default_language: Option<&str>) -> Self {
        let mut state = ReaderState::new(fmt, default_language);
        if state.ext.is_empty() {
            state.ext = ".py".to_string();
        }
        state.comment_magics = fmt.comment_magics.or(Some(true));

        let script = SCRIPT_EXTENSIONS
            .get(state.ext.as_str())
            .cloned()
            .unwrap_or(crate::languages::ScriptLanguage {
                language: "python",
                comment: "#",
                comment_suffix: "",
            });

        if default_language.is_none() {
            state.default_language = script.language.to_string();
        }
        let comment = script.comment.to_string();
        let comment_suffix = script.comment_suffix.to_string();
        state.explicit_soc = true;

        let start_pat = format!(
            r"^\s*{}\s*%%(%*)\s(.*)$",
            regex::escape(&comment)
        );
        let alt_pat = format!(
            r"^\s*{}\s*(%%|<codecell>|In\[[0-9 ]*\]:?)\s*$",
            regex::escape(&comment)
        );

        DoublePercentScriptCellReader {
            state,
            comment,
            comment_suffix,
            start_code_re: Regex::new(&start_pat).unwrap(),
            alternative_start_code_re: Regex::new(&alt_pat).unwrap(),
            markdown_prefix: None,
        }
    }

    fn metadata_and_language_from_option_line(&mut self, line: &str) {
        if self.start_code_re.is_match(line) {
            let uncommented =
                uncomment(&[line.to_string()], &self.comment, &self.comment_suffix);
            let l = &uncommented[0];
            if let Some(pos) = l.find("%%") {
                let after_pct = &l[pos + 2..];
                let (lang, meta) = self.options_to_metadata(after_pct);
                self.state.language = lang;
                self.state.metadata = Some(meta);
            } else {
                self.state.metadata = Some(BTreeMap::new());
            }
        } else {
            self.state.metadata = Some(BTreeMap::new());
        }
    }

    fn options_to_metadata(
        &mut self,
        options: &str,
    ) -> (Option<String>, BTreeMap<String, Value>) {
        // Reuse the same parsing as LightScriptCellReader
        self.state.cell_metadata_json =
            self.state.cell_metadata_json || is_json_metadata(options);
        let (title, mut metadata) = text_to_metadata(options, true);

        // Cell type
        for cell_type in &["markdown", "raw", "md"] {
            let code = format!("[{}]", cell_type);
            if title.contains(&code) {
                let cleaned_title = title.replace(&code, "").trim().to_string();
                let actual_type = if *cell_type == "md" {
                    metadata.insert(
                        "region_name".to_string(),
                        Value::String("md".to_string()),
                    );
                    "markdown"
                } else {
                    cell_type
                };
                metadata.insert(
                    "cell_type".to_string(),
                    Value::String(actual_type.to_string()),
                );
                if !cleaned_title.is_empty() {
                    metadata.insert(
                        "title".to_string(),
                        Value::String(cleaned_title),
                    );
                }
                return (None, metadata);
            }
        }

        // Spyder sub cells
        let mut cell_depth: usize = 0;
        let mut remaining_title = title.clone();
        while remaining_title.starts_with('%') {
            cell_depth += 1;
            remaining_title = remaining_title[1..].to_string();
        }
        if cell_depth > 0 {
            metadata.insert(
                "cell_depth".to_string(),
                Value::from(cell_depth as i64),
            );
            remaining_title = remaining_title.trim().to_string();
        }

        if !remaining_title.is_empty() {
            metadata.insert(
                "title".to_string(),
                Value::String(remaining_title),
            );
        }

        (None, metadata)
    }

    fn find_cell_end(&mut self, lines: &[String]) -> (usize, usize, bool) {
        let meta = self.state.metadata.clone().unwrap_or_default();
        if let Some(ct_val) = meta.get("cell_type") {
            if let Some(ct_str) = ct_val.as_str() {
                match ct_str {
                    "markdown" => self.state.cell_type = Some(CellKind::Markdown),
                    "raw" => self.state.cell_type = Some(CellKind::Raw),
                    _ => self.state.cell_type = Some(CellKind::Code),
                }
                if let Some(ref mut m) = self.state.metadata {
                    m.remove("cell_type");
                }
            } else {
                self.state.cell_type = Some(CellKind::Code);
            }
        } else if !is_active(".ipynb", &meta, true) {
            if meta.get("active").and_then(|v| v.as_str()) == Some("") {
                if let Some(ref mut m) = self.state.metadata {
                    m.remove("active");
                }
            }
            self.state.cell_type = Some(CellKind::Raw);
            if is_active(&self.state.ext, &meta, true) {
                // We don't need to comment: cell is raw but active for this ext
            }
        } else {
            self.state.cell_type = Some(CellKind::Code);
        }

        let next_cell = lines.len();
        let lang = self.state.effective_language().to_string();
        let mut parser = StringParser::new(&lang);
        let mut found_next = next_cell;
        for (i, line) in lines.iter().enumerate() {
            if parser.is_quoted() {
                parser.read_line(line);
                continue;
            }
            parser.read_line(line);
            if i > 0
                && (self.start_code_re.is_match(line)
                    || self.alternative_start_code_re.is_match(line))
            {
                found_next = i;
                break;
            }
        }

        let sub = &lines[..found_next];
        if last_two_lines_blank(sub) {
            return (found_next - 2, found_next, false);
        }
        if found_next > 0 && BLANK_LINE.is_match(&lines[found_next - 1]) {
            return (found_next - 1, found_next, false);
        }
        (found_next, found_next, false)
    }

    fn find_cell_content(&mut self, lines: &[String]) -> usize {
        let (cell_end_marker, next_cell_start, explicit_eoc) =
            self.find_cell_end(lines);

        // Determine cell_start
        let cell_start =
            if self.start_code_re.is_match(&lines[0])
                || self.alternative_start_code_re.is_match(&lines[0])
            {
                1
            } else {
                0
            };

        let source: Vec<String> = lines
            [cell_start..cell_end_marker.min(lines.len())]
            .to_vec();
        self.state.org_content = source.clone();
        self.state.explicit_soc = true;
        self.state.content = script_extract_content(
            source,
            &mut self.state,
            &self.comment.clone(),
            &self.comment_suffix.clone(),
            self.markdown_prefix.as_deref(),
        );

        self.state.lines_to_next_cell = count_lines_to_next_cell(
            cell_end_marker,
            next_cell_start,
            lines.len(),
            explicit_eoc,
        );

        next_cell_start
    }
}

impl CellReader for DoublePercentScriptCellReader {
    fn read(&mut self, lines: &[String]) -> CellReadResult {
        self.metadata_and_language_from_option_line(&lines[0]);

        if let Some(ref mut m) = self.state.metadata {
            if let Some(lang) = m.remove("language") {
                if let Some(s) = lang.as_str() {
                    self.state.language = Some(s.to_string());
                }
            }
        }

        let pos_next_cell = self.find_cell_content(lines);

        if self.state.metadata.is_none() {
            self.state.metadata = Some(BTreeMap::new());
        }

        let empty_fallback = vec!["".to_string()];
        let expected_blank_lines = if self.state.ext == ".py" {
            let org = if self.state.org_content.is_empty() {
                &empty_fallback
            } else {
                &self.state.org_content
            };
            pep8_lines_between_cells(
                org,
                &lines[pos_next_cell..],
                &self.state.ext,
            )
        } else {
            1
        };

        if self.state.lines_to_next_cell != expected_blank_lines {
            let m = self.state.metadata.get_or_insert_with(BTreeMap::new);
            m.insert(
                "lines_to_next_cell".to_string(),
                Value::from(self.state.lines_to_next_cell as i64),
            );
        }

        if let Some(ref lang) = self.state.language {
            let m = self.state.metadata.get_or_insert_with(BTreeMap::new);
            m.insert("language".to_string(), Value::String(lang.clone()));
        }

        let cell = self.state.build_cell();
        CellReadResult {
            cell,
            next_position: pos_next_cell,
        }
    }
}

// =========================================================================
// HydrogenCellReader
// =========================================================================

/// Read notebook cells from Hydrogen scripts (#59).
/// Identical to DoublePercentScriptCellReader except `comment_magics` defaults to false.
pub struct HydrogenCellReader {
    inner: DoublePercentScriptCellReader,
}

impl HydrogenCellReader {
    pub fn new(fmt: &FormatOptions, default_language: Option<&str>) -> Self {
        let mut inner = DoublePercentScriptCellReader::new(fmt, default_language);
        // Hydrogen: magics are NOT commented
        inner.state.comment_magics = fmt.comment_magics.or(Some(false));
        HydrogenCellReader { inner }
    }
}

impl CellReader for HydrogenCellReader {
    fn read(&mut self, lines: &[String]) -> CellReadResult {
        self.inner.read(lines)
    }
}

// =========================================================================
// SphinxGalleryScriptCellReader
// =========================================================================

#[allow(dead_code)]
pub struct SphinxGalleryScriptCellReader {
    state: ReaderState,
    comment: String,
    comment_suffix: String,
    markdown_marker: Option<String>,
    twenty_hash: Regex,
    default_markdown_cell_marker: String,
    rst2md: bool,
}

impl SphinxGalleryScriptCellReader {
    pub fn new(fmt: &FormatOptions, default_language: Option<&str>) -> Self {
        let mut state = ReaderState::new(fmt, Some(default_language.unwrap_or("python")));
        state.ext = ".py".to_string();
        state.comment_magics = fmt.comment_magics.or(Some(true));

        SphinxGalleryScriptCellReader {
            state,
            comment: "#".to_string(),
            comment_suffix: String::new(),
            markdown_marker: None,
            twenty_hash: Regex::new(r"^#( |)#{19,}\s*$").unwrap(),
            default_markdown_cell_marker: "#".repeat(79),
            rst2md: fmt.rst2md,
        }
    }

    fn start_of_new_markdown_cell(&self, line: &str) -> Option<String> {
        for empty in &["\"\"", "''"] {
            if line == *empty {
                return Some(empty.to_string());
            }
        }
        for triple in &["\"\"\"", "'''"] {
            if line.starts_with(triple) {
                return Some(triple.to_string());
            }
        }
        if self.twenty_hash.is_match(line) {
            return Some(line.to_string());
        }
        None
    }

    fn metadata_and_language_from_option_line(&mut self, line: &str) {
        self.markdown_marker = self.start_of_new_markdown_cell(line);
        if self.markdown_marker.is_some() {
            self.state.cell_type = Some(CellKind::Markdown);
            if let Some(ref marker) = self.markdown_marker {
                if *marker != self.default_markdown_cell_marker {
                    let mut meta = BTreeMap::new();
                    meta.insert(
                        "cell_marker".to_string(),
                        Value::String(marker.clone()),
                    );
                    self.state.metadata = Some(meta);
                }
            }
        } else {
            self.state.cell_type = Some(CellKind::Code);
        }
    }

    fn find_cell_end(&mut self, lines: &[String]) -> (usize, usize, bool) {
        if self.state.cell_type == Some(CellKind::Markdown) {
            let marker = self
                .markdown_marker
                .as_deref()
                .unwrap_or("");

            // Empty cell "" or ''
            if marker.len() <= 2 {
                if lines.len() == 1
                    || (lines.len() > 1 && BLANK_LINE.is_match(&lines[1]))
                {
                    return (0, 2.min(lines.len()), true);
                }
                return (0, 1, true);
            }

            // Multi-line comment with triple quote
            if marker.len() == 3 {
                for (i, line) in lines.iter().enumerate() {
                    if (i > 0 || line.trim() != marker)
                        && line.trim_end().ends_with(marker)
                    {
                        let explicit_end = line.trim() == marker;
                        let end_of_cell = if explicit_end { i } else { i + 1 };
                        if lines.len() <= i + 1
                            || BLANK_LINE.is_match(&lines[i + 1])
                        {
                            return (
                                end_of_cell,
                                (i + 2).min(lines.len()),
                                explicit_end,
                            );
                        }
                        return (end_of_cell, i + 1, explicit_end);
                    }
                }
            } else {
                // 20 # or more
                for (i, line) in lines.iter().enumerate().skip(1) {
                    if !line.starts_with('#') {
                        if BLANK_LINE.is_match(line) {
                            return (i, i + 1, false);
                        }
                        return (i, i, false);
                    }
                }
            }
        } else if self.state.cell_type == Some(CellKind::Code) {
            let mut parser = StringParser::new("python");
            for (i, line) in lines.iter().enumerate() {
                if parser.is_quoted() {
                    parser.read_line(line);
                    continue;
                }
                if self.start_of_new_markdown_cell(line).is_some() {
                    if i > 0 && BLANK_LINE.is_match(&lines[i - 1]) {
                        return (i - 1, i, false);
                    }
                    return (i, i, false);
                }
                parser.read_line(line);
            }
        }

        (lines.len(), lines.len(), false)
    }

    fn find_cell_content(&mut self, lines: &[String]) -> usize {
        let (cell_end_marker, next_cell_start, explicit_eoc) =
            self.find_cell_end(lines);

        let mut cell_start = 0;
        let marker = self
            .markdown_marker
            .clone()
            .unwrap_or_default();

        // Make a mutable copy of the relevant lines for manipulation
        let mut working_lines: Vec<String> = lines.to_vec();

        if self.state.cell_type == Some(CellKind::Markdown) {
            if marker == "\"\"\"" || marker == "'''" {
                // Remove the triple quotes
                if working_lines[0].trim() == marker {
                    cell_start = 1;
                } else {
                    working_lines[0] = working_lines[0][3..].to_string();
                }
                if !explicit_eoc && cell_end_marker > 0 && cell_end_marker <= working_lines.len() {
                    let idx = cell_end_marker - 1;
                    let last = working_lines[idx].clone();
                    if let Some(pos) = last.rfind(&marker) {
                        working_lines[idx] = last[..pos].to_string();
                    }
                }
            }
            if self.twenty_hash.is_match(&marker) {
                cell_start = 1;
            }
        } else {
            self.state.metadata = Some(BTreeMap::new());
        }

        let source: Vec<String> = working_lines
            [cell_start..cell_end_marker.min(working_lines.len())]
            .to_vec();
        self.state.org_content = source.clone();

        let mut content = source;
        if self.state.cell_type == Some(CellKind::Code)
            && self.state.comment_magics.unwrap_or(true)
        {
            let lang = self.state.effective_language().to_string();
            uncomment_magic(&mut content, &lang, true, false);
        }

        if self.state.cell_type == Some(CellKind::Markdown) && !content.is_empty() {
            if marker.starts_with('#') {
                content = uncomment(&content, "#", "");
            }
            // rst2md conversion would go here if we supported sphinx_gallery
        }

        self.state.content = content;
        self.state.lines_to_next_cell = count_lines_to_next_cell(
            cell_end_marker,
            next_cell_start,
            lines.len(),
            explicit_eoc,
        );

        next_cell_start
    }
}

impl CellReader for SphinxGalleryScriptCellReader {
    fn read(&mut self, lines: &[String]) -> CellReadResult {
        self.metadata_and_language_from_option_line(&lines[0]);

        let pos_next_cell = self.find_cell_content(lines);

        if self.state.metadata.is_none() {
            self.state.metadata = Some(BTreeMap::new());
        }

        let empty_fallback = vec!["".to_string()];
        let expected_blank_lines = if self.state.ext == ".py" {
            let org = if self.state.org_content.is_empty() {
                &empty_fallback
            } else {
                &self.state.org_content
            };
            pep8_lines_between_cells(
                org,
                &lines[pos_next_cell..],
                &self.state.ext,
            )
        } else {
            1
        };

        if self.state.lines_to_next_cell != expected_blank_lines {
            let m = self.state.metadata.get_or_insert_with(BTreeMap::new);
            m.insert(
                "lines_to_next_cell".to_string(),
                Value::from(self.state.lines_to_next_cell as i64),
            );
        }

        if let Some(ref lang) = self.state.language {
            let m = self.state.metadata.get_or_insert_with(BTreeMap::new);
            m.insert("language".to_string(), Value::String(lang.clone()));
        }

        let cell = self.state.build_cell();
        CellReadResult {
            cell,
            next_position: pos_next_cell,
        }
    }
}

// =========================================================================
// RScriptCellReader
// =========================================================================

pub struct RScriptCellReader {
    state: ReaderState,
    comment: String,
    comment_suffix: String,
    markdown_prefix: String,
    start_code_re: Regex,
}

impl RScriptCellReader {
    pub fn new(fmt: &FormatOptions, default_language: Option<&str>) -> Self {
        let mut state = ReaderState::new(fmt, Some(default_language.unwrap_or("R")));
        state.default_language = default_language.unwrap_or("R").to_string();
        state.comment_magics = fmt.comment_magics.or(Some(true));

        RScriptCellReader {
            state,
            comment: "#'".to_string(),
            comment_suffix: String::new(),
            markdown_prefix: "#'".to_string(),
            start_code_re: Regex::new(r"^#\+(.*)\s*$").unwrap(),
        }
    }

    fn metadata_and_language_from_option_line(&mut self, line: &str) {
        if self.start_code_re.is_match(line) {
            if let Some(caps) = self.start_code_re.captures(line) {
                let options = caps.get(1).map(|m| m.as_str()).unwrap_or("");
                let opt_str = format!("r {}", options);
                let (lang, meta) =
                    rmd_options_to_metadata(&opt_str, self.state.use_runtools);
                self.state.language = Some(lang);
                self.state.metadata = Some(meta);
            }
        }
    }

    fn find_cell_end(&mut self, lines: &[String]) -> (usize, usize, bool) {
        // If no metadata and first line starts with #', this is a markdown cell
        if self.state.metadata.is_none() && lines[0].starts_with("#'") {
            self.state.cell_type = Some(CellKind::Markdown);
            for (i, line) in lines.iter().enumerate() {
                if !line.starts_with("#'") {
                    if BLANK_LINE.is_match(line) {
                        return (i, i + 1, false);
                    }
                    return (i, i, false);
                }
            }
            return (lines.len(), lines.len(), false);
        }

        let meta = self.state.metadata.clone().unwrap_or_default();
        if let Some(ct_val) = meta.get("cell_type") {
            if let Some(ct_str) = ct_val.as_str() {
                match ct_str {
                    "markdown" => self.state.cell_type = Some(CellKind::Markdown),
                    "raw" => self.state.cell_type = Some(CellKind::Raw),
                    _ => self.state.cell_type = Some(CellKind::Code),
                }
                if let Some(ref mut m) = self.state.metadata {
                    m.remove("cell_type");
                }
            } else {
                self.state.cell_type = Some(CellKind::Code);
            }
        } else {
            self.state.cell_type = Some(CellKind::Code);
        }

        let lang = self.state.effective_language().to_string();
        let mut parser = StringParser::new(&lang);
        for (i, line) in lines.iter().enumerate() {
            if self.state.metadata.is_some() && i == 0 {
                continue;
            }
            if parser.is_quoted() {
                parser.read_line(line);
                continue;
            }
            parser.read_line(line);

            if self.start_code_re.is_match(line) || line.starts_with("#'") {
                if i > 0 && BLANK_LINE.is_match(&lines[i - 1]) {
                    if i > 1 && BLANK_LINE.is_match(&lines[i - 2]) {
                        return (i - 2, i, false);
                    }
                    return (i - 1, i, false);
                }
                return (i, i, false);
            }

            if BLANK_LINE.is_match(line) && !next_code_is_indented(&lines[i..]) {
                if i > 0 {
                    return (i, i + 1, false);
                }
                if lines.len() > 1 && !BLANK_LINE.is_match(&lines[1]) {
                    return (1, 1, false);
                }
                return (1, 2, false);
            }
        }

        (lines.len(), lines.len(), false)
    }

    fn find_cell_content(&mut self, lines: &[String]) -> usize {
        let (cell_end_marker, mut next_cell_start, explicit_eoc) =
            self.find_cell_end(lines);
        self.state.explicit_eoc = explicit_eoc;

        let cell_start = if self.state.metadata.is_none() {
            self.state.metadata = Some(BTreeMap::new());
            0
        } else {
            1
        };

        let source: Vec<String> = lines
            [cell_start..cell_end_marker.min(lines.len())]
            .to_vec();
        self.state.org_content = source.clone();

        self.state.explicit_soc = cell_start > 0;
        self.state.content = script_extract_content(
            source,
            &mut self.state,
            &self.comment.clone(),
            &self.comment_suffix.clone(),
            Some(&self.markdown_prefix.clone()),
        );

        // Advance past blank lines
        if next_cell_start + 1 < lines.len()
            && BLANK_LINE.is_match(&lines[next_cell_start])
            && !BLANK_LINE.is_match(&lines[next_cell_start + 1])
        {
            next_cell_start += 1;
        } else if self.state.explicit_eoc
            && next_cell_start + 2 < lines.len()
            && BLANK_LINE.is_match(&lines[next_cell_start])
            && BLANK_LINE.is_match(&lines[next_cell_start + 1])
            && !BLANK_LINE.is_match(&lines[next_cell_start + 2])
        {
            next_cell_start += 2;
        }

        self.state.lines_to_next_cell = count_lines_to_next_cell(
            cell_end_marker,
            next_cell_start,
            lines.len(),
            self.state.explicit_eoc,
        );

        next_cell_start
    }
}

impl CellReader for RScriptCellReader {
    fn read(&mut self, lines: &[String]) -> CellReadResult {
        self.metadata_and_language_from_option_line(&lines[0]);

        if let Some(ref mut m) = self.state.metadata {
            if let Some(lang) = m.remove("language") {
                if let Some(s) = lang.as_str() {
                    self.state.language = Some(s.to_string());
                }
            }
        }

        let pos_next_cell = self.find_cell_content(lines);

        if self.state.metadata.is_none() {
            self.state.metadata = Some(BTreeMap::new());
        }

        let expected_blank_lines = 1;
        if self.state.lines_to_next_cell != expected_blank_lines {
            let m = self.state.metadata.get_or_insert_with(BTreeMap::new);
            m.insert(
                "lines_to_next_cell".to_string(),
                Value::from(self.state.lines_to_next_cell as i64),
            );
        }

        if let Some(ref lang) = self.state.language {
            let m = self.state.metadata.get_or_insert_with(BTreeMap::new);
            m.insert("language".to_string(), Value::String(lang.clone()));
        }

        let cell = self.state.build_cell();
        CellReadResult {
            cell,
            next_position: pos_next_cell,
        }
    }
}

// =========================================================================
// Factory function
// =========================================================================

/// Create the appropriate cell reader based on the format.
pub fn create_cell_reader(
    fmt: &FormatOptions,
    default_language: Option<&str>,
) -> Box<dyn CellReader> {
    let ext = fmt.ext();
    let format_name = fmt.format_name.as_deref().unwrap_or("");

    match format_name {
        "percent" => Box::new(DoublePercentScriptCellReader::new(fmt, default_language)),
        "hydrogen" => Box::new(HydrogenCellReader::new(fmt, default_language)),
        "sphinx" | "sphinx-gallery" => {
            Box::new(SphinxGalleryScriptCellReader::new(fmt, default_language))
        }
        "spin" | "spin-r" => Box::new(RScriptCellReader::new(fmt, default_language)),
        "light" => Box::new(LightScriptCellReader::new(fmt, default_language)),
        _ => {
            // Infer from extension
            match ext {
                ".md" | ".markdown" => {
                    Box::new(MarkdownCellReader::new(fmt, default_language))
                }
                ".Rmd" => Box::new(RMarkdownCellReader::new(fmt, default_language)),
                ".R" | ".r" => {
                    // Could be spin or light/percent depending on format_name
                    Box::new(RScriptCellReader::new(fmt, default_language))
                }
                _ => {
                    // Default to light script reader for all other script extensions
                    Box::new(LightScriptCellReader::new(fmt, default_language))
                }
            }
        }
    }
}

// =========================================================================
// Tests
// =========================================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn s(lines: &[&str]) -> Vec<String> {
        lines.iter().map(|l| l.to_string()).collect()
    }

    // -- helpers ------------------------------------------------------------

    #[test]
    fn test_blank_line_regex() {
        assert!(BLANK_LINE.is_match(""));
        assert!(BLANK_LINE.is_match("   "));
        assert!(BLANK_LINE.is_match("\t"));
        assert!(!BLANK_LINE.is_match("x"));
    }

    #[test]
    fn test_count_lines_to_next_cell_basic() {
        assert_eq!(count_lines_to_next_cell(5, 6, 10, false), 1);
        assert_eq!(count_lines_to_next_cell(5, 7, 10, false), 2);
        assert_eq!(count_lines_to_next_cell(5, 7, 10, true), 1);
        assert_eq!(count_lines_to_next_cell(10, 10, 10, false), 1);
    }

    #[test]
    fn test_last_two_lines_blank() {
        assert!(!last_two_lines_blank(&s(&["", ""])));
        assert!(last_two_lines_blank(&s(&["code", "", ""])));
        assert!(!last_two_lines_blank(&s(&["", "", ""])));
    }

    // -- MarkdownCellReader -----------------------------------------------

    #[test]
    fn test_markdown_reader_simple_code() {
        let fmt = FormatOptions {
            extension: Some(".md".to_string()),
            ..Default::default()
        };
        let mut reader = MarkdownCellReader::new(&fmt, None);
        let lines = s(&["```python", "x = 1", "```", ""]);
        let result = reader.read(&lines);
        assert_eq!(result.cell.source, "x = 1");
        assert_eq!(result.cell.cell_type, crate::notebook::CellType::Code);
        // Position 3 = past the closing ```, the blank line is consumed as lines_to_next_cell
        assert_eq!(result.next_position, 3);
    }

    #[test]
    fn test_markdown_reader_markdown_paragraph() {
        let fmt = FormatOptions {
            extension: Some(".md".to_string()),
            ..Default::default()
        };
        let mut reader = MarkdownCellReader::new(&fmt, None);
        let lines = s(&["# Title", "", "Some text", "", "```python", "x = 1", "```"]);
        let result = reader.read(&lines);
        assert_eq!(result.cell.cell_type, crate::notebook::CellType::Markdown);
        // Should stop before the code cell
        assert!(result.next_position <= 4);
    }

    // -- LightScriptCellReader -------------------------------------------

    #[test]
    fn test_light_reader_comment_block() {
        let fmt = FormatOptions {
            extension: Some(".py".to_string()),
            ..Default::default()
        };
        let mut reader = LightScriptCellReader::new(&fmt, None);
        let lines = s(&["# A comment", "# another", "", "x = 1"]);
        let result = reader.read(&lines);
        assert_eq!(result.cell.cell_type, crate::notebook::CellType::Markdown);
    }

    #[test]
    fn test_light_reader_code_cell() {
        let fmt = FormatOptions {
            extension: Some(".py".to_string()),
            ..Default::default()
        };
        let mut reader = LightScriptCellReader::new(&fmt, None);
        let lines = s(&["x = 1", "", "y = 2"]);
        let result = reader.read(&lines);
        assert_eq!(result.cell.cell_type, crate::notebook::CellType::Code);
        assert_eq!(result.cell.source, "x = 1");
    }

    // -- DoublePercentScriptCellReader ------------------------------------

    #[test]
    fn test_percent_reader_basic() {
        let fmt = FormatOptions {
            extension: Some(".py".to_string()),
            ..Default::default()
        };
        let mut reader = DoublePercentScriptCellReader::new(&fmt, None);
        let lines = s(&["# %%", "x = 1", "", "# %%", "y = 2"]);
        let result = reader.read(&lines);
        assert_eq!(result.cell.cell_type, crate::notebook::CellType::Code);
        assert_eq!(result.cell.source, "x = 1");
        assert_eq!(result.next_position, 3);
    }

    // -- RScriptCellReader ------------------------------------------------

    #[test]
    fn test_rscript_reader_markdown() {
        let fmt = FormatOptions {
            extension: Some(".R".to_string()),
            ..Default::default()
        };
        let mut reader = RScriptCellReader::new(&fmt, None);
        let lines = s(&["#' # Title", "#' some text", "", "x <- 1"]);
        let result = reader.read(&lines);
        assert_eq!(result.cell.cell_type, crate::notebook::CellType::Markdown);
    }

    // -- Factory ----------------------------------------------------------

    #[test]
    fn test_create_cell_reader_markdown() {
        let fmt = FormatOptions {
            extension: Some(".md".to_string()),
            ..Default::default()
        };
        let _reader = create_cell_reader(&fmt, None);
    }

    #[test]
    fn test_create_cell_reader_percent() {
        let fmt = FormatOptions {
            extension: Some(".py".to_string()),
            format_name: Some("percent".to_string()),
            ..Default::default()
        };
        let _reader = create_cell_reader(&fmt, None);
    }
}
