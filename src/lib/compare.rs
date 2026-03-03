//! Compare two Jupyter notebooks and report differences.
//!
//! This module provides:
//! - `compare` -- compare two strings / values and produce a unified diff
//! - `compare_notebooks` -- compare two `Notebook` structs cell-by-cell
//! - `NotebookDifference` -- the error type raised on mismatch
//! - `test_round_trip_conversion` -- verify that writing + reading a notebook
//!   round-trips without loss

use std::collections::BTreeMap;
use std::fmt;

#[allow(unused_imports)]
use regex::Regex;
use serde_json::Value;

use crate::cell_metadata::IGNORE_CELL_METADATA;
#[allow(unused_imports)]
use crate::formats::long_form_one_format;
use crate::metadata_filter::{filter_metadata, DEFAULT_NOTEBOOK_METADATA};
use crate::notebook::{Cell, CellType, Notebook};

// ---------------------------------------------------------------------------
// NotebookDifference
// ---------------------------------------------------------------------------

/// Error type raised when two notebooks differ.
#[derive(Debug, Clone)]
pub struct NotebookDifference {
    pub message: String,
}

impl fmt::Display for NotebookDifference {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.message)
    }
}

impl std::error::Error for NotebookDifference {}

impl NotebookDifference {
    pub fn new(msg: impl Into<String>) -> Self {
        NotebookDifference {
            message: msg.into(),
        }
    }
}

// ---------------------------------------------------------------------------
// String / value comparison
// ---------------------------------------------------------------------------

/// Split a string (or format a JSON value) into lines suitable for diffing.
fn multilines(s: &str) -> Vec<String> {
    let lines: Vec<String> = s.lines().map(|l| l.to_string()).collect();
    if s.ends_with('\n') && !lines.is_empty() {
        let mut v = lines;
        v.push(String::new());
        v
    } else {
        lines
    }
}

/// Produce a unified-diff string between two multi-line strings.
///
/// Returns an empty string when `actual == expected`.
/// If `return_diff` is false and the strings differ, returns an `Err`
/// containing the diff. If `return_diff` is true, always returns the diff
/// string (empty when equal).
pub fn compare(
    actual: &str,
    expected: &str,
    actual_name: &str,
    expected_name: &str,
    return_diff: bool,
) -> Result<String, NotebookDifference> {
    if actual == expected {
        return Ok(String::new());
    }

    let expected_lines = multilines(expected);
    let actual_lines = multilines(actual);

    let diff = unified_diff(&expected_lines, &actual_lines, expected_name, actual_name);

    if return_diff {
        Ok(diff)
    } else {
        Err(NotebookDifference::new(format!("\n{}", diff)))
    }
}

/// Minimal unified diff implementation.
///
/// Produces output similar to `difflib.unified_diff` in Python.
fn unified_diff(
    expected: &[String],
    actual: &[String],
    expected_name: &str,
    actual_name: &str,
) -> String {
    let mut output = Vec::new();
    if !expected_name.is_empty() || !actual_name.is_empty() {
        output.push(format!("--- {}", expected_name));
        output.push(format!("+++ {}", actual_name));
    }

    let max = expected.len().max(actual.len());
    let mut i = 0;
    while i < max {
        let e = expected.get(i).map(|s| s.as_str()).unwrap_or("");
        let a = actual.get(i).map(|s| s.as_str()).unwrap_or("");
        if e != a {
            if i < expected.len() {
                output.push(format!("-{}", e));
            }
            if i < actual.len() {
                output.push(format!("+{}", a));
            }
        } else {
            output.push(format!(" {}", e));
        }
        i += 1;
    }

    output.join("\n")
}

// ---------------------------------------------------------------------------
// Cell filtering helpers
// ---------------------------------------------------------------------------

/// A simplified view of a cell for comparison purposes.
#[derive(Debug, Clone, PartialEq)]
struct FilteredCell {
    cell_type: CellType,
    source: String,
    metadata: BTreeMap<String, Value>,
    execution_count: Option<Value>,
    outputs: Option<Vec<Value>>,
}

/// Build a `FilteredCell` from a notebook cell.
fn filtered_cell(
    cell: &Cell,
    preserve_outputs: bool,
    cell_metadata_filter: Option<&str>,
) -> FilteredCell {
    let filter_str = cell_metadata_filter.unwrap_or("");
    let metadata = filter_metadata(&cell.metadata, filter_str, IGNORE_CELL_METADATA);

    FilteredCell {
        cell_type: cell.cell_type.clone(),
        source: cell.source.clone(),
        metadata,
        execution_count: if preserve_outputs {
            cell.execution_count.clone()
        } else {
            None
        },
        outputs: if preserve_outputs {
            cell.outputs.clone()
        } else {
            None
        },
    }
}

/// Filter notebook metadata for comparison, removing Jupytext internal keys.
fn filtered_notebook_metadata(
    nb: &Notebook,
    ignore_kernelspec: bool,
) -> BTreeMap<String, Value> {
    let notebook_metadata_filter = nb
        .metadata
        .get("jupytext")
        .and_then(|j| j.get("notebook_metadata_filter"))
        .and_then(|v| v.as_str())
        .unwrap_or("");

    let mut metadata = filter_metadata(&nb.metadata, notebook_metadata_filter, DEFAULT_NOTEBOOK_METADATA);

    if ignore_kernelspec {
        metadata.remove("kernelspec");
    }

    metadata.remove("jupytext");
    metadata
}

// ---------------------------------------------------------------------------
// Content comparison helpers
// ---------------------------------------------------------------------------

/// Regex matching a blank (whitespace-only) line.
fn is_blank_line(line: &str) -> bool {
    line.trim().is_empty()
}

/// Are two cell sources the same, allowing for an optional trailing blank line?
fn same_content(ref_source: &str, test_source: &str, allow_removed_final_blank: bool) -> bool {
    if ref_source == test_source {
        return true;
    }
    if !allow_removed_final_blank {
        return false;
    }

    // Split into lines, preserving trailing empty lines.
    // `str::lines()` strips a trailing newline, so we use `split('\n')` instead
    // and handle the trailing empty element explicitly.
    let ref_lines: Vec<&str> = ref_source.split('\n').collect();
    let test_lines: Vec<&str> = test_source.split('\n').collect();

    if ref_lines.is_empty() {
        return false;
    }

    // Check if ref has exactly one more line than test and that extra line is blank
    if ref_lines.len() == test_lines.len() + 1
        && is_blank_line(ref_lines[ref_lines.len() - 1])
        && ref_lines[..ref_lines.len() - 1] == test_lines[..]
    {
        return true;
    }

    // Also check the reverse: test has one more trailing blank line than ref
    if test_lines.len() == ref_lines.len() + 1
        && is_blank_line(test_lines[test_lines.len() - 1])
        && test_lines[..test_lines.len() - 1] == ref_lines[..]
    {
        return true;
    }

    false
}

// ---------------------------------------------------------------------------
// Notebook comparison
// ---------------------------------------------------------------------------

/// Compare two notebooks cell-by-cell and metadata-by-metadata.
///
/// - `allow_expected_differences`: if true, tolerates minor discrepancies
///   (e.g. filtered metadata, trailing blank lines, format-specific metadata loss)
/// - `raise_on_first_difference`: if true, returns on the first mismatch;
///   otherwise collects all differences and reports them together
///
/// Returns `Ok(())` when the notebooks are equivalent, or
/// `Err(NotebookDifference)` describing what differs.
pub fn compare_notebooks(
    notebook_actual: &Notebook,
    notebook_expected: &Notebook,
    fmt: Option<&BTreeMap<String, String>>,
    allow_expected_differences: bool,
    raise_on_first_difference: bool,
) -> Result<(), NotebookDifference> {
    let fmt = fmt.cloned().unwrap_or_default();
    let format_name = fmt.get("format_name").cloned().unwrap_or_default();

    // Skip the first cell for sphinx format ("%matplotlib inline")
    let actual_cells = if format_name == "sphinx"
        && !notebook_actual.cells.is_empty()
        && notebook_actual.cells[0].source == "%matplotlib inline"
    {
        &notebook_actual.cells[1..]
    } else {
        &notebook_actual.cells[..]
    };

    let compare_outputs = false; // By default we don't compare outputs
    let compare_ids = compare_outputs;

    let cell_metadata_filter = if format_name == "marimo" {
        Some("-all".to_string())
    } else {
        notebook_actual
            .metadata
            .get("jupytext")
            .and_then(|j| j.get("cell_metadata_filter"))
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
    };

    let allow_missing_code_meta =
        allow_expected_differences && matches!(format_name.as_str(), "sphinx" | "marimo");
    let allow_missing_md_meta = allow_expected_differences
        && matches!(format_name.as_str(), "sphinx" | "spin" | "marimo");

    let (modified_cells, modified_cell_metadata) = compare_cells(
        actual_cells,
        &notebook_expected.cells,
        raise_on_first_difference,
        compare_outputs,
        compare_ids,
        cell_metadata_filter.as_deref(),
        allow_missing_code_meta,
        allow_missing_md_meta,
        allow_expected_differences, // allow_filtered_cell_metadata
        allow_expected_differences, // allow_removed_final_blank_line
    )?;

    // Compare notebook metadata
    let mut modified_metadata = false;
    if format_name != "marimo" {
        let ignore_kernelspec =
            fmt.get("extension").map(|e| e.as_str()) == Some(".qmd") && allow_expected_differences;
        let meta_actual = filtered_notebook_metadata(notebook_actual, ignore_kernelspec);
        let meta_expected = filtered_notebook_metadata(notebook_expected, ignore_kernelspec);
        if meta_actual != meta_expected {
            if raise_on_first_difference {
                let diff_str = compare(
                    &format!("{:?}", meta_actual),
                    &format!("{:?}", meta_expected),
                    "actual",
                    "expected",
                    true,
                )
                .unwrap_or_default();
                return Err(NotebookDifference::new(format!(
                    "Notebook metadata differ: {}",
                    diff_str
                )));
            }
            modified_metadata = true;
        }
    }

    // Build error message from collected differences
    let mut error_parts = Vec::new();
    if !modified_cells.is_empty() {
        let cell_nums: Vec<String> = modified_cells.iter().map(|i| i.to_string()).collect();
        error_parts.push(format!(
            "Cells {} differ ({}/{})",
            cell_nums.join(","),
            modified_cells.len(),
            notebook_expected.cells.len()
        ));
    }
    if !modified_cell_metadata.is_empty() {
        let keys: Vec<String> = modified_cell_metadata.iter().cloned().collect();
        error_parts.push(format!("Cell metadata '{}' differ", keys.join("', '")));
    }
    if modified_metadata {
        error_parts.push("Notebook metadata differ".to_string());
    }

    if !error_parts.is_empty() {
        return Err(NotebookDifference::new(error_parts.join(" | ")));
    }

    Ok(())
}

/// Compare two collections of cells.
///
/// Returns `(modified_cell_indices, modified_metadata_keys)`.
/// If `raise_on_first_difference` is true, returns an `Err` on the first
/// mismatch instead of collecting indices.
#[allow(clippy::too_many_arguments)]
fn compare_cells(
    actual_cells: &[Cell],
    expected_cells: &[Cell],
    raise_on_first_difference: bool,
    compare_outputs: bool,
    compare_ids: bool,
    cell_metadata_filter: Option<&str>,
    allow_missing_code_cell_metadata: bool,
    allow_missing_markdown_cell_metadata: bool,
    allow_filtered_cell_metadata: bool,
    allow_removed_final_blank_line: bool,
) -> Result<(Vec<usize>, Vec<String>), NotebookDifference> {
    let mut modified_cells = Vec::new();
    let mut modified_cell_metadata: Vec<String> = Vec::new();
    let mut actual_iter = actual_cells.iter();

    for (i, ref_cell) in expected_cells.iter().enumerate() {
        let cell_num = i + 1; // 1-indexed for display

        let test_cell = match actual_iter.next() {
            Some(c) => c,
            None => {
                if raise_on_first_difference {
                    return Err(NotebookDifference::new(format!(
                        "No cell corresponding to {} cell #{}:\n{}",
                        ref_cell.cell_type, cell_num, ref_cell.source
                    )));
                }
                for j in cell_num..=expected_cells.len() {
                    modified_cells.push(j);
                }
                break;
            }
        };

        // 1. Compare cell type
        if ref_cell.cell_type != test_cell.cell_type {
            if raise_on_first_difference {
                return Err(NotebookDifference::new(format!(
                    "When comparing cell #{}: expecting a {} cell, but got a {} cell.\nExpected content:\n{}\nActual content:\n{}",
                    cell_num, ref_cell.cell_type, test_cell.cell_type,
                    ref_cell.source, test_cell.source
                )));
            }
            modified_cells.push(cell_num);
        }

        // 2. Compare cell IDs
        if compare_ids && test_cell.id != ref_cell.id {
            if raise_on_first_difference {
                return Err(NotebookDifference::new(format!(
                    "Cell ids differ on {} cell #{}: '{:?}' != '{:?}'",
                    test_cell.cell_type, cell_num, test_cell.id, ref_cell.id
                )));
            }
            modified_cells.push(cell_num);
        }

        // 3. Compare cell metadata
        let skip_metadata = (ref_cell.cell_type == CellType::Code && allow_missing_code_cell_metadata)
            || (ref_cell.cell_type != CellType::Code && allow_missing_markdown_cell_metadata);

        if !skip_metadata {
            let (ref_meta, test_meta) = if allow_filtered_cell_metadata {
                let ignore: std::collections::HashSet<&str> =
                    IGNORE_CELL_METADATA.split(',').map(|s| s.trim_start_matches('-')).collect();
                let filter_fn =
                    |m: &BTreeMap<String, Value>| -> BTreeMap<String, Value> {
                        m.iter()
                            .filter(|(k, _)| !ignore.contains(k.as_str()))
                            .map(|(k, v)| (k.clone(), v.clone()))
                            .collect()
                    };
                (filter_fn(&ref_cell.metadata), filter_fn(&test_cell.metadata))
            } else {
                (ref_cell.metadata.clone(), test_cell.metadata.clone())
            };

            if ref_meta != test_meta {
                if raise_on_first_difference {
                    let diff_str = compare(
                        &format!("{:?}", test_meta),
                        &format!("{:?}", ref_meta),
                        "actual",
                        "expected",
                        true,
                    )
                    .unwrap_or_default();
                    return Err(NotebookDifference::new(format!(
                        "Metadata differ on {} cell #{}: {}\nCell content:\n{}",
                        test_cell.cell_type, cell_num, diff_str, ref_cell.source
                    )));
                }
                // Collect differing metadata keys
                for key in test_meta.keys() {
                    if !ref_meta.contains_key(key) && !modified_cell_metadata.contains(key) {
                        modified_cell_metadata.push(key.clone());
                    }
                }
                for key in ref_meta.keys() {
                    if !test_meta.contains_key(key) && !modified_cell_metadata.contains(key) {
                        modified_cell_metadata.push(key.clone());
                    }
                }
                for key in ref_meta.keys() {
                    if test_meta.contains_key(key)
                        && ref_meta[key] != test_meta[key]
                        && !modified_cell_metadata.contains(key)
                    {
                        modified_cell_metadata.push(key.clone());
                    }
                }
            }
        }

        // 4. Compare cell content (non-blank lines)
        let ref_lines: Vec<&str> = ref_cell
            .source
            .lines()
            .filter(|l| !l.trim().is_empty())
            .collect();
        let test_lines: Vec<&str> = test_cell
            .source
            .lines()
            .filter(|l| !l.trim().is_empty())
            .collect();

        if ref_lines != test_lines {
            if raise_on_first_difference {
                let diff_str = compare(
                    &test_lines.join("\n"),
                    &ref_lines.join("\n"),
                    "actual",
                    "expected",
                    true,
                )
                .unwrap_or_default();
                return Err(NotebookDifference::new(format!(
                    "Cell content differ on {} cell #{}: {}",
                    test_cell.cell_type, cell_num, diff_str
                )));
            }
            modified_cells.push(cell_num);
        }

        // 4b. Compare full cell content (with blank lines)
        if !same_content(&ref_cell.source, &test_cell.source, allow_removed_final_blank_line) {
            if ref_cell.source != test_cell.source {
                if raise_on_first_difference {
                    let diff_str = compare(
                        &test_cell.source,
                        &ref_cell.source,
                        "",
                        "",
                        true,
                    )
                    .unwrap_or_default();
                    return Err(NotebookDifference::new(format!(
                        "Cell content differ on {} cell #{}: {}",
                        test_cell.cell_type, cell_num, diff_str
                    )));
                }
                if !modified_cells.contains(&cell_num) {
                    modified_cells.push(cell_num);
                }
            }
        }

        // 5. Compare outputs
        if compare_outputs && ref_cell.cell_type == CellType::Code {
            let ref_filtered = filtered_cell(ref_cell, true, cell_metadata_filter);
            let test_filtered = filtered_cell(test_cell, true, cell_metadata_filter);

            if ref_filtered != test_filtered {
                if raise_on_first_difference {
                    return Err(NotebookDifference::new(format!(
                        "Cell outputs differ on {} cell #{}",
                        test_cell.cell_type, cell_num
                    )));
                }
                if !modified_cells.contains(&cell_num) {
                    modified_cells.push(cell_num);
                }
            }
        }
    }

    // Check for extra actual cells
    let mut remaining = 0;
    for test_cell in actual_iter {
        if raise_on_first_difference {
            return Err(NotebookDifference::new(format!(
                "Additional {} cell: {}",
                test_cell.cell_type, test_cell.source
            )));
        }
        remaining += 1;
    }

    if remaining > 0 {
        let start = expected_cells.len() + 1;
        for j in start..start + remaining {
            modified_cells.push(j);
        }
    }

    Ok((modified_cells, modified_cell_metadata))
}

/// Test that writing a notebook to text and reading it back produces an equivalent notebook.
///
/// This is the Rust equivalent of `test_round_trip_conversion` in the Python codebase.
pub fn test_round_trip_conversion(
    notebook: &Notebook,
    fmt: &BTreeMap<String, String>,
    update: bool,
    allow_expected_differences: bool,
    stop_on_first_error: bool,
) -> Result<(), NotebookDifference> {
    use crate::jupytext::{reads_notebook, writes_notebook};
    use crate::combine::combine_inputs_with_outputs;

    let text = writes_notebook(notebook, fmt).map_err(|e| {
        NotebookDifference::new(format!("Failed to write notebook: {}", e))
    })?;

    let round_trip = reads_notebook(&text, fmt).map_err(|e| {
        NotebookDifference::new(format!("Failed to read back notebook: {}", e))
    })?;

    let round_trip = if update {
        combine_inputs_with_outputs(&round_trip, notebook, Some(fmt))
    } else {
        round_trip
    };

    compare_notebooks(
        &round_trip,
        notebook,
        Some(fmt),
        allow_expected_differences,
        stop_on_first_error,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::notebook::{Cell, Notebook};

    fn make_nb(cells: Vec<Cell>) -> Notebook {
        Notebook {
            nbformat: 4,
            nbformat_minor: 5,
            metadata: BTreeMap::new(),
            cells,
        }
    }

    #[test]
    fn test_compare_identical_strings() {
        let result = compare("hello\nworld", "hello\nworld", "a", "b", true).unwrap();
        assert!(result.is_empty());
    }

    #[test]
    fn test_compare_different_strings() {
        let result = compare("hello\nworld", "hello\nearth", "a", "b", true).unwrap();
        assert!(!result.is_empty());
        assert!(result.contains("-earth"));
        assert!(result.contains("+world"));
    }

    #[test]
    fn test_compare_raises_on_diff() {
        let result = compare("hello", "world", "a", "b", false);
        assert!(result.is_err());
    }

    #[test]
    fn test_same_content_identical() {
        assert!(same_content("a\nb", "a\nb", false));
    }

    #[test]
    fn test_same_content_trailing_blank() {
        assert!(same_content("a\nb\n", "a\nb", true));
        assert!(!same_content("a\nb\n", "a\nb", false));
    }

    #[test]
    fn test_compare_notebooks_identical() {
        let nb1 = make_nb(vec![Cell::new_code("a = 1"), Cell::new_markdown("# Hi")]);
        let nb2 = make_nb(vec![Cell::new_code("a = 1"), Cell::new_markdown("# Hi")]);
        let result = compare_notebooks(&nb1, &nb2, None, true, true);
        assert!(result.is_ok());
    }

    #[test]
    fn test_compare_notebooks_different_source() {
        let nb1 = make_nb(vec![Cell::new_code("a = 1")]);
        let nb2 = make_nb(vec![Cell::new_code("b = 2")]);
        let result = compare_notebooks(&nb1, &nb2, None, true, true);
        assert!(result.is_err());
        assert!(result.unwrap_err().message.contains("Cell content differ"));
    }

    #[test]
    fn test_compare_notebooks_different_cell_count() {
        let nb1 = make_nb(vec![Cell::new_code("a = 1"), Cell::new_code("b = 2")]);
        let nb2 = make_nb(vec![Cell::new_code("a = 1")]);
        let result = compare_notebooks(&nb1, &nb2, None, true, true);
        assert!(result.is_err());
        assert!(result.unwrap_err().message.contains("Additional"));
    }

    #[test]
    fn test_compare_notebooks_different_cell_type() {
        let nb1 = make_nb(vec![Cell::new_code("text")]);
        let nb2 = make_nb(vec![Cell::new_markdown("text")]);
        let result = compare_notebooks(&nb1, &nb2, None, true, true);
        assert!(result.is_err());
    }

    #[test]
    fn test_compare_notebooks_collect_all() {
        let nb1 = make_nb(vec![
            Cell::new_code("a = 1"),
            Cell::new_code("changed"),
        ]);
        let nb2 = make_nb(vec![
            Cell::new_code("a = 1"),
            Cell::new_code("original"),
        ]);
        // raise_on_first_difference = false: collects all diffs
        let result = compare_notebooks(&nb1, &nb2, None, true, false);
        assert!(result.is_err());
        assert!(result.unwrap_err().message.contains("Cells"));
    }

    #[test]
    fn test_multilines_trailing_newline() {
        let lines = multilines("a\nb\n");
        assert_eq!(lines, vec!["a", "b", ""]);
    }

    #[test]
    fn test_multilines_no_trailing() {
        let lines = multilines("a\nb");
        assert_eq!(lines, vec!["a", "b"]);
    }
}
