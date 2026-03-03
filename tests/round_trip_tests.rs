//! Round-trip integration tests for jupytext-rs
//!
//! These tests verify that notebooks can be read, written, and round-tripped
//! through the public API and the cell reader/exporter components.
//!
//! The tests are modelled after the Python jupytext test suite, in particular:
//! - tests/functional/simple_notebooks/test_read_simple_python.py
//! - tests/functional/simple_notebooks/test_read_simple_markdown.py
//! - tests/functional/simple_notebooks/test_read_simple_percent.py
//! - tests/functional/simple_notebooks/test_ipynb_to_py.py
//! - tests/functional/round_trip/test_mirror.py

use std::collections::BTreeMap;

use serde_json::Value;

use jupytext::cell_reader::{
    create_cell_reader, CellReader, DoublePercentScriptCellReader, FormatOptions,
    LightScriptCellReader, MarkdownCellReader,
};
use jupytext::cell_to_text::{
    CellExporter, DoublePercentCellExporter, LightScriptCellExporter, MarkdownCellExporter,
};
use jupytext::jupytext::{reads, writes};
use jupytext::notebook::{Cell, CellType, Notebook};

// ---------------------------------------------------------------------------
// Helper functions
// ---------------------------------------------------------------------------

/// Build a Vec<String> from a slice of &str.
fn lines(input: &[&str]) -> Vec<String> {
    input.iter().map(|s| s.to_string()).collect()
}

/// Build a BTreeMap<String, Value> with common format options.
fn py_percent_fmt() -> BTreeMap<String, Value> {
    let mut m = BTreeMap::new();
    m.insert("extension".to_string(), Value::String(".py".to_string()));
    m.insert(
        "format_name".to_string(),
        Value::String("percent".to_string()),
    );
    m
}

fn py_light_fmt() -> BTreeMap<String, Value> {
    let mut m = BTreeMap::new();
    m.insert("extension".to_string(), Value::String(".py".to_string()));
    m.insert(
        "format_name".to_string(),
        Value::String("light".to_string()),
    );
    m
}

fn md_fmt() -> BTreeMap<String, Value> {
    let mut m = BTreeMap::new();
    m.insert("extension".to_string(), Value::String(".md".to_string()));
    m.insert(
        "format_name".to_string(),
        Value::String("markdown".to_string()),
    );
    m
}

fn format_options_percent() -> FormatOptions {
    FormatOptions {
        extension: Some(".py".to_string()),
        format_name: Some("percent".to_string()),
        ..Default::default()
    }
}

fn format_options_light() -> FormatOptions {
    FormatOptions {
        extension: Some(".py".to_string()),
        format_name: Some("light".to_string()),
        ..Default::default()
    }
}

fn format_options_markdown() -> FormatOptions {
    FormatOptions {
        extension: Some(".md".to_string()),
        format_name: Some("markdown".to_string()),
        ..Default::default()
    }
}

// =========================================================================
// 1. Reading Python percent-format scripts via cell reader
// =========================================================================

#[test]
fn test_read_percent_simple_code_cell() {
    // Equivalent to test_read_simple_file in test_read_simple_percent.py
    let input = lines(&["# %%", "1 + 2 + 3 + 4", "5", "6", ""]);

    let fmt = format_options_percent();
    let mut reader = DoublePercentScriptCellReader::new(&fmt, Some("python"));
    let result = reader.read(&input);

    assert_eq!(result.cell.cell_type, CellType::Code);
    assert_eq!(result.cell.source, "1 + 2 + 3 + 4\n5\n6");
}

#[test]
fn test_read_percent_markdown_cell() {
    let input = lines(&["# %% [markdown]", "# This is a markdown cell", ""]);

    let fmt = format_options_percent();
    let mut reader = DoublePercentScriptCellReader::new(&fmt, Some("python"));
    let result = reader.read(&input);

    assert_eq!(result.cell.cell_type, CellType::Markdown);
    assert_eq!(result.cell.source, "This is a markdown cell");
}

#[test]
fn test_read_percent_raw_cell() {
    let input = lines(&["# %% [raw]", "# This is a raw cell", ""]);

    let fmt = format_options_percent();
    let mut reader = DoublePercentScriptCellReader::new(&fmt, Some("python"));
    let result = reader.read(&input);

    assert_eq!(result.cell.cell_type, CellType::Raw);
    assert_eq!(result.cell.source, "This is a raw cell");
}

#[test]
fn test_read_percent_cell_with_title() {
    // From test_read_simple_file: # %% And now a code cell
    let input = lines(&["# %% And now a code cell", "1 + 2", ""]);

    let fmt = format_options_percent();
    let mut reader = DoublePercentScriptCellReader::new(&fmt, Some("python"));
    let result = reader.read(&input);

    assert_eq!(result.cell.cell_type, CellType::Code);
    assert_eq!(result.cell.source, "1 + 2");
    assert_eq!(
        result.cell.metadata.get("title"),
        Some(&Value::String("And now a code cell".to_string()))
    );
}

#[test]
fn test_read_percent_cell_with_metadata() {
    // From test_read_cell_with_metadata in test_read_simple_percent.py
    let input = lines(&[
        r#"# %% a code cell with parameters {"tags": ["parameters"]}"#,
        "a = 3",
        "",
    ]);

    let fmt = format_options_percent();
    let mut reader = DoublePercentScriptCellReader::new(&fmt, Some("python"));
    let result = reader.read(&input);

    assert_eq!(result.cell.cell_type, CellType::Code);
    assert_eq!(result.cell.source, "a = 3");
    assert_eq!(
        result.cell.metadata.get("title"),
        Some(&Value::String("a code cell with parameters".to_string()))
    );
    // The tags metadata should be parsed from the JSON
    let tags = result.cell.metadata.get("tags");
    assert!(tags.is_some(), "Expected tags metadata to be present");
}

#[test]
fn test_read_percent_multiple_cells() {
    // Read multiple cells sequentially, simulating the reader loop
    let input = lines(&[
        "# %% [markdown]",
        "# Docstring",
        "",
        "# %%",
        "from math import pi",
        "",
        "# %% [markdown]",
        "# Another markdown cell",
        "",
    ]);

    let fmt = format_options_percent();

    // Read first cell
    let mut reader1 = DoublePercentScriptCellReader::new(&fmt, Some("python"));
    let result1 = reader1.read(&input);
    assert_eq!(result1.cell.cell_type, CellType::Markdown);
    assert_eq!(result1.cell.source, "Docstring");

    // Read second cell from remaining lines
    let remaining = &input[result1.next_position..];
    let mut reader2 = DoublePercentScriptCellReader::new(&fmt, Some("python"));
    let result2 = reader2.read(remaining);
    assert_eq!(result2.cell.cell_type, CellType::Code);
    assert_eq!(result2.cell.source, "from math import pi");

    // Read third cell
    let remaining2 = &remaining[result2.next_position..];
    let mut reader3 = DoublePercentScriptCellReader::new(&fmt, Some("python"));
    let result3 = reader3.read(remaining2);
    assert_eq!(result3.cell.cell_type, CellType::Markdown);
    assert_eq!(result3.cell.source, "Another markdown cell");
}

// =========================================================================
// 2. Reading Markdown notebooks via cell reader
// =========================================================================

#[test]
fn test_read_markdown_code_cell() {
    let input = lines(&["```python", "import numpy as np", "x = np.arange(0, 10)", "```", ""]);

    let fmt = format_options_markdown();
    let mut reader = MarkdownCellReader::new(&fmt, Some("python"));
    let result = reader.read(&input);

    assert_eq!(result.cell.cell_type, CellType::Code);
    assert_eq!(result.cell.source, "import numpy as np\nx = np.arange(0, 10)");
}

#[test]
fn test_read_markdown_text_cell() {
    let input = lines(&[
        "This is a paragraph",
        "that spans multiple lines.",
        "",
        "```python",
        "x = 1",
        "```",
    ]);

    let fmt = format_options_markdown();
    let mut reader = MarkdownCellReader::new(&fmt, Some("python"));
    let result = reader.read(&input);

    assert_eq!(result.cell.cell_type, CellType::Markdown);
    assert_eq!(
        result.cell.source,
        "This is a paragraph\nthat spans multiple lines."
    );
}

#[test]
fn test_read_markdown_raw_cell() {
    let input = lines(&[
        "<!-- #raw -->",
        "this is a raw cell",
        "<!-- #endraw -->",
        "",
    ]);

    let fmt = format_options_markdown();
    let mut reader = MarkdownCellReader::new(&fmt, Some("python"));
    let result = reader.read(&input);

    assert_eq!(result.cell.cell_type, CellType::Raw);
    assert_eq!(result.cell.source, "this is a raw cell");
}

#[test]
fn test_read_markdown_multiple_cells() {
    // Based on test_read_mostly_py_markdown_file
    let input = lines(&[
        "```python",
        "import numpy as np",
        "```",
        "",
        "This is a Markdown cell",
        "",
        "```python",
        "x = 1",
        "```",
        "",
    ]);

    let fmt = format_options_markdown();

    // Read first cell (code)
    let mut reader1 = MarkdownCellReader::new(&fmt, Some("python"));
    let result1 = reader1.read(&input);
    assert_eq!(result1.cell.cell_type, CellType::Code);
    assert_eq!(result1.cell.source, "import numpy as np");

    // Read second cell (markdown text)
    let remaining = &input[result1.next_position..];
    let mut reader2 = MarkdownCellReader::new(&fmt, Some("python"));
    let result2 = reader2.read(remaining);
    assert_eq!(result2.cell.cell_type, CellType::Markdown);
    assert_eq!(result2.cell.source, "This is a Markdown cell");

    // Read third cell (code)
    let remaining2 = &remaining[result2.next_position..];
    let mut reader3 = MarkdownCellReader::new(&fmt, Some("python"));
    let result3 = reader3.read(remaining2);
    assert_eq!(result3.cell.cell_type, CellType::Code);
    assert_eq!(result3.cell.source, "x = 1");
}

#[test]
fn test_read_markdown_code_cell_with_metadata() {
    // From test_code_cell_with_metadata in test_read_simple_markdown.py
    let input = lines(&[
        r#"```python tags=["parameters"]"#,
        "a = 1",
        "b = 2",
        "```",
        "",
    ]);

    let fmt = format_options_markdown();
    let mut reader = MarkdownCellReader::new(&fmt, Some("python"));
    let result = reader.read(&input);

    assert_eq!(result.cell.cell_type, CellType::Code);
    assert_eq!(result.cell.source, "a = 1\nb = 2");
    let tags = result.cell.metadata.get("tags");
    assert!(tags.is_some(), "Expected tags metadata to be present");
}

#[test]
fn test_read_markdown_raw_cell_with_metadata() {
    // From test_raw_cell_with_metadata in test_read_simple_markdown.py
    let input = lines(&[
        r#"<!-- #raw key="value" -->"#,
        "raw content",
        "<!-- #endraw -->",
        "",
    ]);

    let fmt = format_options_markdown();
    let mut reader = MarkdownCellReader::new(&fmt, Some("python"));
    let result = reader.read(&input);

    assert_eq!(result.cell.cell_type, CellType::Raw);
    assert_eq!(result.cell.source, "raw content");
    assert_eq!(
        result.cell.metadata.get("key"),
        Some(&Value::String("value".to_string()))
    );
}

// =========================================================================
// 3. Writing to Python percent format and reading back (round-trip)
// =========================================================================

#[test]
fn test_write_percent_code_cell_and_read_back() {
    let cell = Cell::new_code("x = 1\ny = 2");
    let fmt = py_percent_fmt();

    // Write
    let mut exporter = DoublePercentCellExporter::new(&cell, "python", &fmt);
    let text = exporter.cell_to_text();
    assert_eq!(text[0], "# %%");
    assert_eq!(text[1], "x = 1");
    assert_eq!(text[2], "y = 2");

    // Read back
    let read_fmt = format_options_percent();
    let mut reader = DoublePercentScriptCellReader::new(&read_fmt, Some("python"));
    let result = reader.read(&text);

    assert_eq!(result.cell.cell_type, CellType::Code);
    assert_eq!(result.cell.source, "x = 1\ny = 2");
}

#[test]
fn test_write_percent_markdown_cell_and_read_back() {
    let cell = Cell::new_markdown("A short paragraph.");
    let fmt = py_percent_fmt();

    // Write
    let mut exporter = DoublePercentCellExporter::new(&cell, "python", &fmt);
    let text = exporter.cell_to_text();
    assert_eq!(text[0], "# %% [markdown]");
    assert_eq!(text[1], "# A short paragraph.");

    // Read back
    let read_fmt = format_options_percent();
    let mut reader = DoublePercentScriptCellReader::new(&read_fmt, Some("python"));
    let mut text_with_trailing = text.clone();
    text_with_trailing.push(String::new()); // blank line to end cell
    let result = reader.read(&text_with_trailing);

    assert_eq!(result.cell.cell_type, CellType::Markdown);
    assert_eq!(result.cell.source, "A short paragraph.");
}

#[test]
fn test_write_percent_raw_cell_and_read_back() {
    let cell = Cell::new_raw("Raw cell content");
    let fmt = py_percent_fmt();

    // Write
    let mut exporter = DoublePercentCellExporter::new(&cell, "python", &fmt);
    let text = exporter.cell_to_text();
    assert_eq!(text[0], "# %% [raw]");

    // Read back
    let read_fmt = format_options_percent();
    let mut reader = DoublePercentScriptCellReader::new(&read_fmt, Some("python"));
    let mut text_with_trailing = text.clone();
    text_with_trailing.push(String::new());
    let result = reader.read(&text_with_trailing);

    assert_eq!(result.cell.cell_type, CellType::Raw);
    assert_eq!(result.cell.source, "Raw cell content");
}

#[test]
fn test_round_trip_percent_multiline_code() {
    let source = "def f(x):\n    return x + 1\n\n\ndef g(x):\n    return x - 1";
    let cell = Cell::new_code(source);
    let fmt = py_percent_fmt();

    // Write
    let mut exporter = DoublePercentCellExporter::new(&cell, "python", &fmt);
    let text = exporter.cell_to_text();

    // Read back
    let read_fmt = format_options_percent();
    let mut reader = DoublePercentScriptCellReader::new(&read_fmt, Some("python"));
    let mut text_with_end = text.clone();
    text_with_end.push(String::new());
    let result = reader.read(&text_with_end);

    assert_eq!(result.cell.cell_type, CellType::Code);
    assert_eq!(result.cell.source, source);
}

// =========================================================================
// 4. Writing to Markdown and reading back (round-trip)
// =========================================================================

#[test]
fn test_write_markdown_code_cell_and_read_back() {
    let cell = Cell::new_code("import numpy as np\nx = np.arange(0, 10)");
    let fmt = md_fmt();

    // Write
    let mut exporter = MarkdownCellExporter::new(&cell, "python", &fmt);
    let text = exporter.cell_to_text();
    assert_eq!(text[0], "```python");
    assert_eq!(text[1], "import numpy as np");
    assert_eq!(text[2], "x = np.arange(0, 10)");
    assert_eq!(text[3], "```");

    // Read back
    let read_fmt = format_options_markdown();
    let mut reader = MarkdownCellReader::new(&read_fmt, Some("python"));
    let mut text_with_end = text.clone();
    text_with_end.push(String::new());
    let result = reader.read(&text_with_end);

    assert_eq!(result.cell.cell_type, CellType::Code);
    assert_eq!(
        result.cell.source,
        "import numpy as np\nx = np.arange(0, 10)"
    );
}

#[test]
fn test_write_markdown_text_cell_and_read_back() {
    let cell = Cell::new_markdown("This is a paragraph.\n\nWith two lines.");
    let fmt = md_fmt();

    // Write
    let mut exporter = MarkdownCellExporter::new(&cell, "python", &fmt);
    let text = exporter.cell_to_text();

    // The markdown cell should be written as plain text
    // (possibly with region markers if it contains blank lines)
    assert!(!text.is_empty());

    // Read back
    let read_fmt = format_options_markdown();
    let mut reader = MarkdownCellReader::new(&read_fmt, Some("python"));
    let mut text_with_end = text.clone();
    text_with_end.push(String::new());
    let result = reader.read(&text_with_end);

    assert_eq!(result.cell.cell_type, CellType::Markdown);
    // The source should survive the round-trip.
    // The markdown reader may include a trailing newline from the region markers,
    // so we trim trailing whitespace for comparison.
    let read_source = result.cell.source.trim_end();
    assert_eq!(read_source, "This is a paragraph.\n\nWith two lines.");
}

#[test]
fn test_write_markdown_raw_cell_and_read_back() {
    let cell = Cell::new_raw("Raw content here");
    let fmt = md_fmt();

    // Write
    let mut exporter = MarkdownCellExporter::new(&cell, "python", &fmt);
    let text = exporter.cell_to_text();

    // Raw cells should use HTML comment markers
    assert!(
        text[0].starts_with("<!-- #raw"),
        "Expected raw cell marker, got: {}",
        text[0]
    );

    // Read back
    let read_fmt = format_options_markdown();
    let mut reader = MarkdownCellReader::new(&read_fmt, Some("python"));
    let mut text_with_end = text.clone();
    text_with_end.push(String::new());
    let result = reader.read(&text_with_end);

    assert_eq!(result.cell.cell_type, CellType::Raw);
    assert_eq!(result.cell.source, "Raw content here");
}

// =========================================================================
// 5. Reading and writing .ipynb JSON files via public API
// =========================================================================

const SIMPLE_IPYNB: &str = r##"{
  "nbformat": 4,
  "nbformat_minor": 5,
  "metadata": {
    "kernelspec": {
      "display_name": "Python 3",
      "language": "python",
      "name": "python3"
    }
  },
  "cells": [
    {
      "cell_type": "markdown",
      "source": "# Jupyter notebook\n\nThis is a simple notebook.",
      "metadata": {}
    },
    {
      "cell_type": "code",
      "source": "a = 1\nb = 2\na + b",
      "metadata": {},
      "execution_count": null,
      "outputs": []
    },
    {
      "cell_type": "markdown",
      "source": "Now we return a tuple",
      "metadata": {}
    },
    {
      "cell_type": "code",
      "source": "a, b",
      "metadata": {},
      "execution_count": 2,
      "outputs": [
        {
          "data": {"text/plain": "(1, 2)"},
          "execution_count": 2,
          "metadata": {},
          "output_type": "execute_result"
        }
      ]
    }
  ]
}"##;

#[test]
fn test_read_ipynb() {
    let nb = reads(SIMPLE_IPYNB, Some("ipynb")).unwrap();
    assert_eq!(nb.nbformat, 4);
    assert_eq!(nb.cells.len(), 4);

    assert_eq!(nb.cells[0].cell_type, CellType::Markdown);
    assert_eq!(
        nb.cells[0].source,
        "# Jupyter notebook\n\nThis is a simple notebook."
    );

    assert_eq!(nb.cells[1].cell_type, CellType::Code);
    assert_eq!(nb.cells[1].source, "a = 1\nb = 2\na + b");

    assert_eq!(nb.cells[2].cell_type, CellType::Markdown);
    assert_eq!(nb.cells[2].source, "Now we return a tuple");

    assert_eq!(nb.cells[3].cell_type, CellType::Code);
    assert_eq!(nb.cells[3].source, "a, b");
}

#[test]
fn test_ipynb_round_trip() {
    // Read ipynb -> write ipynb -> read again and compare
    let nb1 = reads(SIMPLE_IPYNB, Some("ipynb")).unwrap();
    let written = writes(&nb1, "ipynb").unwrap();
    let nb2 = reads(&written, Some("ipynb")).unwrap();

    assert_eq!(nb2.nbformat, nb1.nbformat);
    assert_eq!(nb2.cells.len(), nb1.cells.len());

    for (c1, c2) in nb1.cells.iter().zip(nb2.cells.iter()) {
        assert_eq!(c1.cell_type, c2.cell_type);
        assert_eq!(c1.source, c2.source);
    }
}

#[test]
fn test_ipynb_kernelspec_preserved() {
    let nb = reads(SIMPLE_IPYNB, Some("ipynb")).unwrap();

    // The kernelspec should be in the metadata
    let kernelspec = nb.metadata.get("kernelspec");
    assert!(kernelspec.is_some(), "Expected kernelspec in metadata");

    let ks = kernelspec.unwrap();
    assert_eq!(
        ks.get("display_name").and_then(|v| v.as_str()),
        Some("Python 3")
    );
    assert_eq!(
        ks.get("language").and_then(|v| v.as_str()),
        Some("python")
    );
    assert_eq!(
        ks.get("name").and_then(|v| v.as_str()),
        Some("python3")
    );
}

#[test]
fn test_ipynb_outputs_and_execution_count() {
    let nb = reads(SIMPLE_IPYNB, Some("ipynb")).unwrap();

    // The first code cell has execution_count null and empty outputs.
    // Note: serde deserializes JSON null as None for Option<Value>,
    // so execution_count: null becomes None.
    let cell1 = &nb.cells[1];
    assert!(
        cell1.execution_count.is_none()
            || cell1.execution_count == Some(Value::Null),
        "Expected execution_count to be None or Some(Null), got {:?}",
        cell1.execution_count
    );
    assert!(cell1.outputs.is_some());

    // The second code cell has execution_count 2 and non-empty outputs
    let cell2 = &nb.cells[3];
    assert_eq!(cell2.execution_count, Some(Value::Number(2.into())));
    assert!(!cell2.outputs.as_ref().unwrap().is_empty());
}

#[test]
fn test_ipynb_auto_detect() {
    // reads with None format should auto-detect ipynb from JSON content
    let nb = reads(SIMPLE_IPYNB, None).unwrap();
    assert_eq!(nb.nbformat, 4);
    assert_eq!(nb.cells.len(), 4);
}

const MINIMAL_IPYNB: &str = r##"{
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
}"##;

#[test]
fn test_ipynb_minimal_round_trip() {
    let nb = reads(MINIMAL_IPYNB, Some("ipynb")).unwrap();
    let text = writes(&nb, "ipynb").unwrap();
    let nb2 = reads(&text, Some("ipynb")).unwrap();

    assert_eq!(nb2.cells.len(), 1);
    assert_eq!(nb2.cells[0].source, "print('hello')");
    assert_eq!(nb2.cells[0].cell_type, CellType::Code);
}

#[test]
fn test_ipynb_with_all_cell_types() {
    let ipynb = r##"{
      "nbformat": 4,
      "nbformat_minor": 5,
      "metadata": {},
      "cells": [
        {
          "cell_type": "raw",
          "source": "---\ntitle: Test\n---",
          "metadata": {}
        },
        {
          "cell_type": "markdown",
          "source": "# Header\n\nSome text.",
          "metadata": {}
        },
        {
          "cell_type": "code",
          "source": "x = 42",
          "metadata": {},
          "execution_count": null,
          "outputs": []
        }
      ]
    }"##;

    let nb = reads(ipynb, Some("ipynb")).unwrap();
    assert_eq!(nb.cells.len(), 3);
    assert_eq!(nb.cells[0].cell_type, CellType::Raw);
    assert_eq!(nb.cells[0].source, "---\ntitle: Test\n---");
    assert_eq!(nb.cells[1].cell_type, CellType::Markdown);
    assert_eq!(nb.cells[1].source, "# Header\n\nSome text.");
    assert_eq!(nb.cells[2].cell_type, CellType::Code);
    assert_eq!(nb.cells[2].source, "x = 42");

    // Round-trip
    let text = writes(&nb, "ipynb").unwrap();
    let nb2 = reads(&text, Some("ipynb")).unwrap();
    assert_eq!(nb2.cells.len(), 3);
    for (c1, c2) in nb.cells.iter().zip(nb2.cells.iter()) {
        assert_eq!(c1.cell_type, c2.cell_type);
        assert_eq!(c1.source, c2.source);
    }
}

#[test]
fn test_ipynb_empty_notebook() {
    let ipynb = r#"{
      "nbformat": 4,
      "nbformat_minor": 5,
      "metadata": {},
      "cells": []
    }"#;

    let nb = reads(ipynb, Some("ipynb")).unwrap();
    assert_eq!(nb.cells.len(), 0);

    let text = writes(&nb, "ipynb").unwrap();
    let nb2 = reads(&text, Some("ipynb")).unwrap();
    assert_eq!(nb2.cells.len(), 0);
}

// =========================================================================
// 6. Cell metadata round-trips
// =========================================================================

#[test]
fn test_percent_cell_metadata_survives_round_trip() {
    // Code cell with tags metadata
    let mut cell = Cell::new_code("a = 3");
    cell.metadata.insert(
        "tags".to_string(),
        Value::Array(vec![Value::String("parameters".to_string())]),
    );
    let fmt = py_percent_fmt();

    // Write
    let mut exporter = DoublePercentCellExporter::new(&cell, "python", &fmt);
    let text = exporter.cell_to_text();

    // The first line should contain [parameters] or similar metadata
    assert!(
        text[0].contains("tags") || text[0].contains("parameters"),
        "Expected metadata in header line, got: {}",
        text[0]
    );

    // Read back
    let read_fmt = format_options_percent();
    let mut reader = DoublePercentScriptCellReader::new(&read_fmt, Some("python"));
    let mut text_with_end = text.clone();
    text_with_end.push(String::new());
    let result = reader.read(&text_with_end);

    assert_eq!(result.cell.cell_type, CellType::Code);
    assert_eq!(result.cell.source, "a = 3");

    // Verify that the tags metadata survived the round-trip
    let tags = result.cell.metadata.get("tags");
    assert!(
        tags.is_some(),
        "Expected tags to survive round-trip. Metadata: {:?}",
        result.cell.metadata
    );
}

#[test]
fn test_percent_cell_title_metadata_round_trip() {
    let mut cell = Cell::new_code("x = 1");
    cell.metadata.insert(
        "title".to_string(),
        Value::String("My cell title".to_string()),
    );
    let fmt = py_percent_fmt();

    // Write
    let mut exporter = DoublePercentCellExporter::new(&cell, "python", &fmt);
    let text = exporter.cell_to_text();

    // The title should appear in the cell marker line
    assert!(
        text[0].contains("My cell title"),
        "Expected title in header, got: {}",
        text[0]
    );

    // Read back
    let read_fmt = format_options_percent();
    let mut reader = DoublePercentScriptCellReader::new(&read_fmt, Some("python"));
    let mut text_with_end = text.clone();
    text_with_end.push(String::new());
    let result = reader.read(&text_with_end);

    assert_eq!(result.cell.source, "x = 1");
    assert_eq!(
        result.cell.metadata.get("title"),
        Some(&Value::String("My cell title".to_string()))
    );
}

#[test]
fn test_markdown_code_cell_metadata_round_trip() {
    let mut cell = Cell::new_code("a = 1\nb = 2");
    cell.metadata.insert(
        "tags".to_string(),
        Value::Array(vec![Value::String("parameters".to_string())]),
    );
    let fmt = md_fmt();

    // Write
    let mut exporter = MarkdownCellExporter::new(&cell, "python", &fmt);
    let text = exporter.cell_to_text();

    // The first line should include the tags metadata
    assert!(
        text[0].contains("parameters"),
        "Expected metadata in fence line, got: {}",
        text[0]
    );

    // Read back
    let read_fmt = format_options_markdown();
    let mut reader = MarkdownCellReader::new(&read_fmt, Some("python"));
    let mut text_with_end = text.clone();
    text_with_end.push(String::new());
    let result = reader.read(&text_with_end);

    assert_eq!(result.cell.cell_type, CellType::Code);
    assert_eq!(result.cell.source, "a = 1\nb = 2");
    let tags = result.cell.metadata.get("tags");
    assert!(tags.is_some(), "Expected tags to survive round-trip");
}

#[test]
fn test_markdown_raw_cell_metadata_round_trip() {
    let mut cell = Cell::new_raw("raw content");
    cell.metadata.insert(
        "key".to_string(),
        Value::String("value".to_string()),
    );
    let fmt = md_fmt();

    // Write
    let mut exporter = MarkdownCellExporter::new(&cell, "python", &fmt);
    let text = exporter.cell_to_text();

    // Read back
    let read_fmt = format_options_markdown();
    let mut reader = MarkdownCellReader::new(&read_fmt, Some("python"));
    let mut text_with_end = text.clone();
    text_with_end.push(String::new());
    let result = reader.read(&text_with_end);

    assert_eq!(result.cell.cell_type, CellType::Raw);
    assert_eq!(result.cell.source, "raw content");
    assert_eq!(
        result.cell.metadata.get("key"),
        Some(&Value::String("value".to_string()))
    );
}

#[test]
fn test_ipynb_cell_metadata_round_trip() {
    let ipynb = r#"{
      "nbformat": 4,
      "nbformat_minor": 5,
      "metadata": {},
      "cells": [
        {
          "cell_type": "code",
          "source": "x = 1",
          "metadata": {
            "tags": ["parameters"],
            "scrolled": true
          },
          "execution_count": null,
          "outputs": []
        }
      ]
    }"#;

    let nb = reads(ipynb, Some("ipynb")).unwrap();
    assert_eq!(nb.cells[0].metadata.get("tags").unwrap(), &Value::Array(vec![Value::String("parameters".to_string())]));
    assert_eq!(nb.cells[0].metadata.get("scrolled").unwrap(), &Value::Bool(true));

    // Round-trip
    let text = writes(&nb, "ipynb").unwrap();
    let nb2 = reads(&text, Some("ipynb")).unwrap();
    assert_eq!(
        nb2.cells[0].metadata.get("tags"),
        nb.cells[0].metadata.get("tags")
    );
    assert_eq!(
        nb2.cells[0].metadata.get("scrolled"),
        nb.cells[0].metadata.get("scrolled")
    );
}

// =========================================================================
// Additional round-trip tests using cell readers and exporters
// =========================================================================

#[test]
fn test_light_script_code_cell_round_trip() {
    let cell = Cell::new_code("x = 1");
    let fmt = py_light_fmt();

    // Write
    let mut exporter = LightScriptCellExporter::new(&cell, "python", &fmt);
    let text = exporter.cell_to_text();
    assert_eq!(text, vec!["x = 1"]);

    // Read back
    let read_fmt = format_options_light();
    let mut reader = LightScriptCellReader::new(&read_fmt, Some("python"));
    let mut text_with_end = text.clone();
    text_with_end.push(String::new());
    let result = reader.read(&text_with_end);

    assert_eq!(result.cell.cell_type, CellType::Code);
    assert_eq!(result.cell.source, "x = 1");
}

#[test]
fn test_create_cell_reader_percent() {
    let fmt = format_options_percent();
    let mut reader = create_cell_reader(&fmt, Some("python"));

    let input = lines(&["# %%", "x = 1", ""]);
    let result = reader.read(&input);

    assert_eq!(result.cell.cell_type, CellType::Code);
    assert_eq!(result.cell.source, "x = 1");
}

#[test]
fn test_create_cell_reader_markdown() {
    let fmt = format_options_markdown();
    let mut reader = create_cell_reader(&fmt, Some("python"));

    let input = lines(&["```python", "x = 1", "```", ""]);
    let result = reader.read(&input);

    assert_eq!(result.cell.cell_type, CellType::Code);
    assert_eq!(result.cell.source, "x = 1");
}

#[test]
fn test_create_cell_reader_light() {
    let fmt = format_options_light();
    let mut reader = create_cell_reader(&fmt, Some("python"));

    let input = lines(&["x = 1", ""]);
    let result = reader.read(&input);

    assert_eq!(result.cell.cell_type, CellType::Code);
    assert_eq!(result.cell.source, "x = 1");
}

// =========================================================================
// Notebook data model tests
// =========================================================================

#[test]
fn test_notebook_new() {
    let nb = Notebook::new();
    assert_eq!(nb.nbformat, 4);
    assert_eq!(nb.nbformat_minor, 5);
    assert_eq!(nb.cells.len(), 0);
    assert!(nb.metadata.is_empty());
}

#[test]
fn test_cell_constructors() {
    let code = Cell::new_code("x = 1");
    assert_eq!(code.cell_type, CellType::Code);
    assert_eq!(code.source, "x = 1");
    assert_eq!(code.execution_count, Some(Value::Null));
    assert_eq!(code.outputs, Some(vec![]));

    let md = Cell::new_markdown("# Title");
    assert_eq!(md.cell_type, CellType::Markdown);
    assert_eq!(md.source, "# Title");
    assert!(md.execution_count.is_none());
    assert!(md.outputs.is_none());

    let raw = Cell::new_raw("raw text");
    assert_eq!(raw.cell_type, CellType::Raw);
    assert_eq!(raw.source, "raw text");
    assert!(raw.execution_count.is_none());
    assert!(raw.outputs.is_none());
}

#[test]
fn test_cell_type_from_str() {
    assert_eq!(CellType::from_str("code"), Some(CellType::Code));
    assert_eq!(CellType::from_str("markdown"), Some(CellType::Markdown));
    assert_eq!(CellType::from_str("md"), Some(CellType::Markdown));
    assert_eq!(CellType::from_str("raw"), Some(CellType::Raw));
    assert_eq!(CellType::from_str("unknown"), None);
}

#[test]
fn test_cell_type_display() {
    assert_eq!(format!("{}", CellType::Code), "code");
    assert_eq!(format!("{}", CellType::Markdown), "markdown");
    assert_eq!(format!("{}", CellType::Raw), "raw");
}

#[test]
fn test_notebook_with_metadata() {
    let mut meta = BTreeMap::new();
    meta.insert(
        "kernelspec".to_string(),
        Value::Object({
            let mut m = serde_json::Map::new();
            m.insert("name".to_string(), Value::String("python3".to_string()));
            m
        }),
    );
    let nb = Notebook::new_with_metadata(meta.clone());
    assert_eq!(nb.metadata.get("kernelspec"), meta.get("kernelspec"));
}

// =========================================================================
// Error handling tests
// =========================================================================

#[test]
fn test_writes_rejects_nbformat_3() {
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
fn test_reads_invalid_json() {
    let result = reads("not json at all {{{", Some("ipynb"));
    assert!(result.is_err());
}

// =========================================================================
// Multi-cell percent format round-trip via cell readers/exporters
// =========================================================================

#[test]
fn test_full_percent_notebook_round_trip_via_components() {
    // Build a notebook with multiple cell types
    let cells = vec![
        Cell::new_markdown("This is a markdown cell"),
        Cell::new_code("import numpy as np\nx = np.arange(0, 10)"),
        Cell::new_markdown("Another markdown cell"),
        Cell::new_code("print(x)"),
    ];

    let fmt = py_percent_fmt();

    // Export all cells
    let mut all_lines: Vec<String> = Vec::new();
    for (i, cell) in cells.iter().enumerate() {
        let mut exporter = DoublePercentCellExporter::new(cell, "python", &fmt);
        let text = exporter.cell_to_text();
        if i > 0 {
            all_lines.push(String::new()); // blank line between cells
        }
        all_lines.extend(text);
    }
    all_lines.push(String::new()); // trailing blank line

    // Now read all cells back
    let read_fmt = format_options_percent();
    let mut pos = 0;
    let mut read_cells: Vec<Cell> = Vec::new();

    while pos < all_lines.len() {
        // Skip blank lines between cells
        while pos < all_lines.len() && all_lines[pos].trim().is_empty() {
            pos += 1;
        }
        if pos >= all_lines.len() {
            break;
        }

        let remaining = &all_lines[pos..];
        let mut reader = DoublePercentScriptCellReader::new(&read_fmt, Some("python"));
        let result = reader.read(remaining);
        read_cells.push(result.cell);
        pos += result.next_position;
    }

    // Verify we got the same cells back
    assert_eq!(
        read_cells.len(),
        cells.len(),
        "Expected {} cells, got {}",
        cells.len(),
        read_cells.len()
    );

    for (original, read_back) in cells.iter().zip(read_cells.iter()) {
        assert_eq!(original.cell_type, read_back.cell_type);
        assert_eq!(
            original.source, read_back.source,
            "Source mismatch for {:?} cell",
            original.cell_type
        );
    }
}

#[test]
fn test_full_markdown_notebook_round_trip_via_components() {
    // Build a notebook with code and markdown cells
    let cells = vec![
        Cell::new_code("import numpy as np"),
        Cell::new_markdown("A paragraph of text."),
        Cell::new_code("x = np.arange(0, 10)"),
    ];

    let fmt = md_fmt();

    // Export all cells
    let mut all_lines: Vec<String> = Vec::new();
    for (i, cell) in cells.iter().enumerate() {
        let mut exporter = MarkdownCellExporter::new(cell, "python", &fmt);
        let text = exporter.cell_to_text();
        if i > 0 {
            all_lines.push(String::new()); // blank line between cells
        }
        all_lines.extend(text);
    }
    all_lines.push(String::new()); // trailing blank line

    // Read all cells back
    let read_fmt = format_options_markdown();
    let mut pos = 0;
    let mut read_cells: Vec<Cell> = Vec::new();

    while pos < all_lines.len() {
        let remaining = &all_lines[pos..];
        if remaining.iter().all(|l| l.trim().is_empty()) {
            break;
        }

        let mut reader = MarkdownCellReader::new(&read_fmt, Some("python"));
        let result = reader.read(remaining);
        if result.next_position == 0 {
            // No progress: break to avoid infinite loop
            break;
        }
        read_cells.push(result.cell);
        pos += result.next_position;
    }

    // Verify we got the same cells back
    assert_eq!(
        read_cells.len(),
        cells.len(),
        "Expected {} cells, got {}. Lines:\n{}",
        cells.len(),
        read_cells.len(),
        all_lines.join("\n")
    );

    for (original, read_back) in cells.iter().zip(read_cells.iter()) {
        assert_eq!(original.cell_type, read_back.cell_type);
        assert_eq!(
            original.source, read_back.source,
            "Source mismatch for {:?} cell",
            original.cell_type
        );
    }
}

// =========================================================================
// ipynb to text format conversion tests
// =========================================================================

#[test]
fn test_ipynb_to_percent_via_components() {
    // Read an ipynb, then export each cell in percent format
    let nb = reads(SIMPLE_IPYNB, Some("ipynb")).unwrap();
    let fmt = py_percent_fmt();

    let mut all_exported: Vec<Vec<String>> = Vec::new();
    for cell in &nb.cells {
        let mut exporter = DoublePercentCellExporter::new(cell, "python", &fmt);
        let text = exporter.cell_to_text();
        all_exported.push(text);
    }

    // First cell is markdown
    assert_eq!(all_exported[0][0], "# %% [markdown]");

    // Second cell is code
    assert_eq!(all_exported[1][0], "# %%");
    assert_eq!(all_exported[1][1], "a = 1");

    // Third cell is markdown
    assert_eq!(all_exported[2][0], "# %% [markdown]");

    // Fourth cell is code
    assert_eq!(all_exported[3][0], "# %%");
    assert_eq!(all_exported[3][1], "a, b");
}

#[test]
fn test_ipynb_to_markdown_via_components() {
    // Read an ipynb, then export each cell in markdown format
    let nb = reads(SIMPLE_IPYNB, Some("ipynb")).unwrap();
    let fmt = md_fmt();

    let mut all_exported: Vec<Vec<String>> = Vec::new();
    for cell in &nb.cells {
        let mut exporter = MarkdownCellExporter::new(cell, "python", &fmt);
        let text = exporter.cell_to_text();
        all_exported.push(text);
    }

    // First cell is markdown - should be plain text
    assert_eq!(all_exported[0][0], "# Jupyter notebook");

    // Second cell is code - should have python fence
    assert_eq!(all_exported[1][0], "```python");
    assert_eq!(*all_exported[1].last().unwrap(), "```");

    // Third cell is markdown
    assert_eq!(all_exported[2][0], "Now we return a tuple");

    // Fourth cell is code
    assert_eq!(all_exported[3][0], "```python");
    assert_eq!(all_exported[3][1], "a, b");
}

// =========================================================================
// Edge case tests
// =========================================================================

#[test]
fn test_percent_empty_code_cell() {
    let cell = Cell::new_code("");
    let fmt = py_percent_fmt();

    let mut exporter = DoublePercentCellExporter::new(&cell, "python", &fmt);
    let text = exporter.cell_to_text();

    // Even an empty code cell should have the %% marker
    assert_eq!(text[0], "# %%");
}

#[test]
fn test_markdown_empty_code_cell() {
    let cell = Cell::new_code("");
    let fmt = md_fmt();

    let mut exporter = MarkdownCellExporter::new(&cell, "python", &fmt);
    let text = exporter.cell_to_text();

    // An empty code cell should still produce a code fence
    assert!(text[0].starts_with("```"));
}

#[test]
fn test_percent_multiline_string_in_code() {
    // Ensure multiline strings in code cells don't confuse the reader
    let source = "text = \"\"\"This is\na multiline\nstring\"\"\"";
    let cell = Cell::new_code(source);
    let fmt = py_percent_fmt();

    let mut exporter = DoublePercentCellExporter::new(&cell, "python", &fmt);
    let text = exporter.cell_to_text();

    // Read back
    let read_fmt = format_options_percent();
    let mut reader = DoublePercentScriptCellReader::new(&read_fmt, Some("python"));
    let mut text_with_end = text.clone();
    text_with_end.push(String::new());
    let result = reader.read(&text_with_end);

    assert_eq!(result.cell.cell_type, CellType::Code);
    assert_eq!(result.cell.source, source);
}

#[test]
fn test_ipynb_multiline_source() {
    let ipynb = r#"{
      "nbformat": 4,
      "nbformat_minor": 5,
      "metadata": {},
      "cells": [
        {
          "cell_type": "code",
          "source": "def f(x):\n    return x + 1\n\n\ndef g(x):\n    return x - 1",
          "metadata": {},
          "execution_count": null,
          "outputs": []
        }
      ]
    }"#;

    let nb = reads(ipynb, Some("ipynb")).unwrap();
    assert_eq!(
        nb.cells[0].source,
        "def f(x):\n    return x + 1\n\n\ndef g(x):\n    return x - 1"
    );

    // Round-trip
    let text = writes(&nb, "ipynb").unwrap();
    let nb2 = reads(&text, Some("ipynb")).unwrap();
    assert_eq!(nb2.cells[0].source, nb.cells[0].source);
}

#[test]
fn test_ipynb_unicode_content() {
    let ipynb = r##"{
      "nbformat": 4,
      "nbformat_minor": 5,
      "metadata": {},
      "cells": [
        {
          "cell_type": "code",
          "source": "# Unicode: \u00e9\u00e8\u00ea\u00eb \u00fc\u00f6\u00e4 \u2603 \u2764",
          "metadata": {},
          "execution_count": null,
          "outputs": []
        }
      ]
    }"##;

    let nb = reads(ipynb, Some("ipynb")).unwrap();
    assert!(nb.cells[0].source.contains('\u{00e9}'));
    assert!(nb.cells[0].source.contains('\u{2603}'));

    let text = writes(&nb, "ipynb").unwrap();
    let nb2 = reads(&text, Some("ipynb")).unwrap();
    assert_eq!(nb2.cells[0].source, nb.cells[0].source);
}
