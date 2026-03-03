//! Combine inputs from one notebook with outputs from another.
//!
//! When synchronizing paired notebooks, the text representation contains the
//! authoritative cell sources while the `.ipynb` representation contains
//! execution outputs. This module merges them by matching cells between the
//! two notebooks and producing a combined result.

use std::collections::{BTreeMap, BTreeSet};

use serde_json::Value;

#[allow(unused_imports)]
use crate::formats::long_form_one_format;
use crate::metadata_filter::{filter_metadata, DEFAULT_NOTEBOOK_METADATA};
use crate::notebook::{Cell, CellType, Notebook};

/// Characters removed when comparing cell content for black-invariant matching.
const BLACK_CHARS: &[char] = &[' ', '\t', '\n', ',', '\'', '"', '(', ')', '\\'];

/// Remove whitespace and formatting characters that black might change.
fn black_invariant(text: &str) -> String {
    let mut result = String::with_capacity(text.len());
    for ch in text.chars() {
        if !BLACK_CHARS.contains(&ch) {
            result.push(ch);
        }
    }
    result
}

/// Check whether two cell sources have the same content, modulo black formatting.
///
/// If `endswith` is `true`, the check succeeds when the black-invariant of `ref_text`
/// ends with the black-invariant of `test_text`.
fn same_content(ref_text: &str, test_text: &str, endswith: bool) -> bool {
    let r = black_invariant(ref_text);
    let t = black_invariant(test_text);

    if endswith && !t.is_empty() {
        r.ends_with(&t)
    } else {
        r == t
    }
}

/// Build a mapping from input cells to output cells.
///
/// Returns a vector of length `cells_inputs.len()` where each entry is either
/// `Some(j)` (the index of the matched output cell) or `None`.
///
/// The matching algorithm uses four progressively looser rules:
/// 1. Exact match (same cell type and black-invariant content), in order per cell type
/// 2. Re-match unused outputs by content
/// 3. Suffix match (output source ends with input source)
/// 4. Index-based match for non-empty cells of the same type
fn map_outputs_to_inputs(cells_inputs: &[Cell], cells_outputs: &[Cell]) -> Vec<Option<usize>> {
    let n_in = cells_inputs.len();
    let n_out = cells_outputs.len();
    let mut outputs_map: Vec<Option<usize>> = vec![None; n_in];

    // ---- Rule 1: exact match, in order, per cell type ----
    let mut first_unmatched: BTreeMap<String, usize> = BTreeMap::new();
    for (i, cell_input) in cells_inputs.iter().enumerate() {
        let cell_type_key = cell_input.cell_type.to_string();
        let start_j = *first_unmatched.get(&cell_type_key).unwrap_or(&0);
        for j in start_j..n_out {
            let cell_output = &cells_outputs[j];
            if cell_input.cell_type == cell_output.cell_type
                && same_content(&cell_input.source, &cell_output.source, false)
            {
                outputs_map[i] = Some(j);
                first_unmatched.insert(cell_type_key.clone(), j + 1);
                break;
            }
        }
    }

    // Collect unused output indices
    let used: BTreeSet<usize> = outputs_map.iter().filter_map(|&v| v).collect();
    let mut unused: BTreeSet<usize> = (0..n_out).filter(|j| !used.contains(j)).collect();

    // ---- Rules 2 & 3: match unused outputs (exact, then suffix) ----
    for endswith in [false, true] {
        if unused.is_empty() {
            break;
        }
        for i in 0..n_in {
            if outputs_map[i].is_some() {
                continue;
            }
            let cell_input = &cells_inputs[i];
            let mut matched_j = None;
            for &j in &unused {
                let cell_output = &cells_outputs[j];
                if cell_input.cell_type == cell_output.cell_type
                    && same_content(&cell_output.source, &cell_input.source, endswith)
                {
                    matched_j = Some(j);
                    break;
                }
            }
            if let Some(j) = matched_j {
                outputs_map[i] = Some(j);
                unused.remove(&j);
            }
        }
    }

    // ---- Rule 4: index-based match for non-empty cells ----
    if !unused.is_empty() {
        let mut prev_j: Option<usize> = None;
        for i in 0..n_in {
            if outputs_map[i].is_some() {
                prev_j = outputs_map[i];
                continue;
            }
            let next_j = prev_j.map_or(0, |pj| pj + 1);
            if !unused.contains(&next_j) {
                continue;
            }
            let cell_input = &cells_inputs[i];
            let cell_output = &cells_outputs[next_j];
            if cell_input.cell_type == cell_output.cell_type
                && !cell_input.source.trim().is_empty()
            {
                outputs_map[i] = Some(next_j);
                unused.remove(&next_j);
                prev_j = Some(next_j);
            }
        }
    }

    outputs_map
}

/// Restore filtered metadata by merging source metadata into output metadata.
///
/// Keys present in `source_meta` take precedence, but keys only present in
/// `output_meta` are preserved (they were filtered out of the text representation).
fn restore_filtered_metadata(
    source_meta: &BTreeMap<String, Value>,
    output_meta: &BTreeMap<String, Value>,
    cell_metadata_filter: Option<&str>,
    default_filter: &str,
) -> BTreeMap<String, Value> {
    if cell_metadata_filter == Some("-all") {
        // Source says "all metadata was filtered" -- take everything from output
        return output_meta.clone();
    }

    let filter_str = cell_metadata_filter.unwrap_or("");
    let filtered_source = filter_metadata(source_meta, filter_str, default_filter);

    // Start with output metadata, overlay with source
    let mut result = output_meta.clone();
    for (key, value) in &filtered_source {
        result.insert(key.clone(), value.clone());
    }
    // Also add any source keys not present in filtered but in source
    for (key, value) in source_meta {
        result.entry(key.clone()).or_insert_with(|| value.clone());
    }
    result
}

/// Combine the cell sources from `nb_source` with the outputs from `nb_outputs`.
///
/// The returned notebook has:
/// - `nbformat` / `nbformat_minor` from `nb_outputs`
/// - Notebook metadata merged from both (source metadata overlaid on output metadata)
/// - Cell sources from `nb_source`, outputs and execution counts from `nb_outputs`
/// - Cell metadata merged per cell
///
/// The optional `fmt` map can contain `extension` and `format_name` to control
/// format-specific behavior (e.g. suppressing cell metadata for the `nomarker`
/// or `sphinx` formats).
pub fn combine_inputs_with_outputs(
    nb_source: &Notebook,
    nb_outputs: &Notebook,
    fmt: Option<&BTreeMap<String, String>>,
) -> Notebook {
    let fmt = fmt.cloned().unwrap_or_default();

    // Determine extension and format_name
    let text_repr = nb_source
        .metadata
        .get("jupytext")
        .and_then(|j| j.get("text_representation"))
        .cloned()
        .unwrap_or(Value::Object(serde_json::Map::new()));

    let ext = fmt
        .get("extension")
        .cloned()
        .or_else(|| text_repr.get("extension").and_then(|v| v.as_str()).map(|s| s.to_string()))
        .unwrap_or_default();

    let format_name = fmt
        .get("format_name")
        .cloned()
        .or_else(|| {
            text_repr
                .get("format_name")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string())
        })
        .unwrap_or_default();

    // ---- Merge notebook metadata ----
    let notebook_metadata_filter = nb_source
        .metadata
        .get("jupytext")
        .and_then(|j| j.get("notebook_metadata_filter"))
        .and_then(|v| v.as_str());

    let nb_metadata = if notebook_metadata_filter == Some("-all") {
        nb_outputs.metadata.clone()
    } else {
        restore_filtered_metadata(
            &nb_source.metadata,
            &nb_outputs.metadata,
            notebook_metadata_filter,
            DEFAULT_NOTEBOOK_METADATA,
        )
    };

    // Clean up text_representation in metadata when formats are set or extension is markdown
    let mut nb_metadata = nb_metadata;
    let has_formats = nb_metadata
        .get("jupytext")
        .and_then(|j| j.get("formats"))
        .is_some();
    let is_md_ext = matches!(ext.as_str(), ".md" | ".markdown" | ".Rmd");

    if has_formats || is_md_ext {
        if let Some(Value::Object(ref mut jt)) = nb_metadata.get_mut("jupytext") {
            jt.remove("text_representation");
        }
    }

    // Remove empty jupytext section
    let should_remove_jupytext = nb_metadata
        .get("jupytext")
        .and_then(|v| v.as_object())
        .map_or(false, |obj| obj.is_empty());
    if should_remove_jupytext {
        nb_metadata.remove("jupytext");
    }

    // ---- Determine cell_metadata_filter ----
    let source_is_md_v1 = is_md_ext
        && text_repr
            .get("format_version")
            .and_then(|v| v.as_str())
            == Some("1.0");

    let cell_metadata_filter = if matches!(
        format_name.as_str(),
        "nomarker" | "sphinx" | "marimo"
    ) || source_is_md_v1
    {
        Some("-all".to_string())
    } else {
        nb_metadata
            .get("jupytext")
            .and_then(|j| j.get("cell_metadata_filter"))
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
    };

    // ---- Map and merge cells ----
    let outputs_map = map_outputs_to_inputs(&nb_source.cells, &nb_outputs.cells);

    let mut cells = Vec::with_capacity(nb_source.cells.len());
    for (i, source_cell) in nb_source.cells.iter().enumerate() {
        let j = outputs_map[i];
        if j.is_none() {
            cells.push(source_cell.clone());
            continue;
        }
        let j = j.unwrap();
        let output_cell = &nb_outputs.cells[j];

        // Start from the output cell (to get outputs, execution_count, id)
        let mut cell = output_cell.clone();

        // Cell text comes from the source notebook
        cell.source = source_cell.source.clone();

        // Merge cell metadata
        let ignore_filter = crate::cell_metadata::IGNORE_CELL_METADATA;
        let effective_filter = if format_name == "spin" && source_cell.cell_type != CellType::Code {
            Some("-all".to_string())
        } else {
            cell_metadata_filter.clone()
        };

        cell.metadata = restore_filtered_metadata(
            &source_cell.metadata,
            &output_cell.metadata,
            effective_filter.as_deref(),
            ignore_filter,
        );

        cells.push(cell);
    }

    Notebook {
        nbformat: nb_outputs.nbformat,
        nbformat_minor: nb_outputs.nbformat_minor,
        metadata: nb_metadata,
        cells,
    }
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
    fn test_black_invariant() {
        assert_eq!(black_invariant("a = 1"), "a=1");
        assert_eq!(black_invariant("print( 'hello' )"), "printhello");
    }

    #[test]
    fn test_same_content_exact() {
        assert!(same_content("a = 1", "a = 1", false));
        assert!(!same_content("a = 1", "b = 2", false));
    }

    #[test]
    fn test_same_content_black_invariant() {
        // Black might change quotes and spacing
        assert!(same_content("a = 1", "a=1", false));
        assert!(same_content("print('hello')", "print(\"hello\")", false));
    }

    #[test]
    fn test_same_content_endswith() {
        assert!(same_content("line1\nline2", "line2", true));
        assert!(!same_content("line1\nline2", "line3", true));
    }

    #[test]
    fn test_map_outputs_exact_match() {
        let inputs = vec![
            Cell::new_code("a = 1"),
            Cell::new_markdown("# Title"),
            Cell::new_code("b = 2"),
        ];
        let outputs = vec![
            Cell::new_code("a = 1"),
            Cell::new_markdown("# Title"),
            Cell::new_code("b = 2"),
        ];

        let mapping = map_outputs_to_inputs(&inputs, &outputs);
        assert_eq!(mapping, vec![Some(0), Some(1), Some(2)]);
    }

    #[test]
    fn test_map_outputs_reordered() {
        let inputs = vec![Cell::new_code("a = 1"), Cell::new_code("b = 2")];
        let outputs = vec![Cell::new_code("b = 2"), Cell::new_code("a = 1")];

        let mapping = map_outputs_to_inputs(&inputs, &outputs);
        // Rule 1: input[0]="a=1" scans from j=0: output[0]="b=2" no, output[1]="a=1" yes => map[0]=1
        // input[1]="b=2" scans from j=2 (first_unmatched["code"]=2): no match in rule 1
        // Rule 2: unused={0}, input[1]="b=2" matches output[0]="b=2" => map[1]=0
        assert_eq!(mapping, vec![Some(1), Some(0)]);
    }

    #[test]
    fn test_map_outputs_extra_input() {
        let inputs = vec![
            Cell::new_code("a = 1"),
            Cell::new_code("new cell"),
            Cell::new_code("b = 2"),
        ];
        let outputs = vec![Cell::new_code("a = 1"), Cell::new_code("b = 2")];

        let mapping = map_outputs_to_inputs(&inputs, &outputs);
        assert_eq!(mapping[0], Some(0));
        assert_eq!(mapping[1], None);
        assert_eq!(mapping[2], Some(1));
    }

    #[test]
    fn test_combine_basic() {
        let source = make_nb(vec![
            Cell::new_code("a = 1"),
            Cell::new_markdown("# Hello"),
        ]);

        let mut output_cell = Cell::new_code("a = 1");
        output_cell.execution_count = Some(Value::Number(serde_json::Number::from(1)));
        output_cell.outputs = Some(vec![Value::String("1".to_string())]);

        let outputs = make_nb(vec![output_cell, Cell::new_markdown("# Hello")]);

        let combined = combine_inputs_with_outputs(&source, &outputs, None);

        assert_eq!(combined.cells.len(), 2);
        assert_eq!(combined.cells[0].source, "a = 1");
        assert_eq!(
            combined.cells[0].execution_count,
            Some(Value::Number(serde_json::Number::from(1)))
        );
        assert!(combined.cells[0].outputs.is_some());
        assert_eq!(combined.nbformat, 4);
    }

    #[test]
    fn test_combine_new_cell_no_output() {
        let source = make_nb(vec![
            Cell::new_code("a = 1"),
            Cell::new_code("new cell"),
        ]);
        let mut output_cell = Cell::new_code("a = 1");
        output_cell.execution_count = Some(Value::Number(serde_json::Number::from(1)));
        let outputs = make_nb(vec![output_cell]);

        let combined = combine_inputs_with_outputs(&source, &outputs, None);
        assert_eq!(combined.cells.len(), 2);
        assert_eq!(combined.cells[0].source, "a = 1");
        assert!(combined.cells[0].execution_count.is_some());
        // New cell has no matched output
        assert_eq!(combined.cells[1].source, "new cell");
    }
}
