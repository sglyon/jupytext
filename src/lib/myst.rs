//! MyST Markdown notebook reader and writer
//!
//! Converts between MyST-formatted Markdown and Jupyter notebooks.
//! MyST uses `{code-cell}` and `{raw-cell}` directives inside fenced code
//! blocks, YAML front matter for notebook metadata, and `+++` markers for
//! markdown cell boundaries/metadata.

use serde_json::Value;
use std::collections::BTreeMap;

use crate::cell_to_text::three_backticks_or_more;
use crate::notebook::{Cell, CellType, Notebook};

const CODE_DIRECTIVE: &str = "{code-cell}";
const RAW_DIRECTIVE: &str = "{raw-cell}";

// ---------------------------------------------------------------------------
// Reader: MyST text → Notebook
// ---------------------------------------------------------------------------

/// Parse MyST-formatted text into a Jupyter Notebook.
pub fn myst_to_notebook(text: &str) -> Result<Notebook, String> {
    let lines: Vec<&str> = text.lines().collect();
    let total_lines = lines.len();

    // 1. Parse optional YAML front matter
    let (metadata_nb, md_start) = parse_front_matter(&lines);

    let mut notebook = Notebook::new_with_metadata(metadata_nb);
    let mut md_start_line = md_start;
    let mut md_metadata: BTreeMap<String, Value> = BTreeMap::new();
    let mut default_lexer: Option<String> = None;

    // 2. Walk through lines to find directives and block breaks
    let mut pos = md_start;
    while pos < total_lines {
        let line = lines[pos];

        // Check for `+++` block break (markdown cell separator)
        if is_block_break(line) {
            // Flush preceding markdown
            flush_markdown(&lines, md_start_line, pos, &md_metadata, &mut notebook);
            md_metadata = parse_block_break_metadata(line);
            md_start_line = pos + 1;
            pos += 1;
            continue;
        }

        // Check for fenced code block opening: ```{code-cell} or ```{raw-cell}
        if let Some((fence, directive, lexer)) = parse_fence_open(line) {
            let cell_type = if directive == CODE_DIRECTIVE {
                CellType::Code
            } else if directive == RAW_DIRECTIVE {
                CellType::Raw
            } else {
                // Not a cell directive, skip
                pos += 1;
                continue;
            };

            // Track default lexer from first code cell
            if cell_type == CellType::Code {
                if let Some(ref lex) = lexer {
                    if default_lexer.is_none() {
                        default_lexer = Some(lex.clone());
                    }
                }
            }

            // Flush preceding markdown
            flush_markdown(&lines, md_start_line, pos, &md_metadata, &mut notebook);
            md_metadata = BTreeMap::new();

            // Find the closing fence
            let content_start = pos + 1;
            let mut end = content_start;
            while end < total_lines {
                if is_closing_fence(lines[end], &fence) {
                    break;
                }
                end += 1;
            }

            // Parse cell content (options + body)
            let content_lines: Vec<&str> = lines[content_start..end].to_vec();
            let (options, body_lines) = parse_directive_content(&content_lines);

            let source = body_lines.join("\n");
            let mut cell = Cell::new_with_type(cell_type, &source);
            cell.metadata = options;
            notebook.cells.push(cell);

            // Move past closing fence
            md_start_line = if end < total_lines { end + 1 } else { end };
            pos = md_start_line;
            continue;
        }

        pos += 1;
    }

    // Flush any trailing markdown
    flush_markdown(&lines, md_start_line, total_lines, &md_metadata, &mut notebook);

    // Store default lexer in metadata if no language_info present
    if !notebook.metadata.contains_key("language_info") {
        if let Some(ref lexer) = default_lexer {
            let jupytext = notebook
                .metadata
                .entry("jupytext".to_string())
                .or_insert_with(|| Value::Object(serde_json::Map::new()));
            if let Some(obj) = jupytext.as_object_mut() {
                obj.insert(
                    "default_lexer".to_string(),
                    Value::String(lexer.clone()),
                );
            }
        }
    }

    Ok(notebook)
}

/// Parse YAML front matter delimited by `---`. Returns (metadata, next_line_index).
fn parse_front_matter(lines: &[&str]) -> (BTreeMap<String, Value>, usize) {
    if lines.is_empty() || lines[0].trim() != "---" {
        return (BTreeMap::new(), 0);
    }

    // Find closing `---`
    let mut end = 1;
    while end < lines.len() {
        if lines[end].trim() == "---" {
            break;
        }
        end += 1;
    }

    if end >= lines.len() {
        // No closing ---, treat as no front matter
        return (BTreeMap::new(), 0);
    }

    let yaml_text: String = lines[1..end].join("\n");
    let metadata: BTreeMap<String, Value> = match serde_yaml::from_str(&yaml_text) {
        Ok(m) => m,
        Err(_) => BTreeMap::new(),
    };

    (metadata, end + 1)
}

/// Check if a line is a MyST block break (`+++` with optional metadata).
fn is_block_break(line: &str) -> bool {
    let trimmed = line.trim();
    trimmed == "+++" || trimmed.starts_with("+++ ")
}

/// Parse metadata from a block break line like `+++ {"key": "value"}`.
fn parse_block_break_metadata(line: &str) -> BTreeMap<String, Value> {
    let trimmed = line.trim();
    if trimmed == "+++" {
        return BTreeMap::new();
    }
    let json_str = trimmed.strip_prefix("+++").unwrap_or("").trim();
    if json_str.is_empty() {
        return BTreeMap::new();
    }
    match serde_json::from_str::<BTreeMap<String, Value>>(json_str) {
        Ok(m) => m,
        Err(_) => BTreeMap::new(),
    }
}

/// Try to parse a fence opening line. Returns (fence_str, directive, optional_lexer).
/// e.g. "```{code-cell} python3" → ("```", "{code-cell}", Some("python3"))
fn parse_fence_open(line: &str) -> Option<(String, &str, Option<String>)> {
    let trimmed = line.trim();
    if !trimmed.starts_with('`') {
        return None;
    }

    // Count backticks
    let backtick_count = trimmed.chars().take_while(|&c| c == '`').count();
    if backtick_count < 3 {
        return None;
    }

    let fence = "`".repeat(backtick_count);
    let after_fence = &trimmed[backtick_count..];

    // Check for directives
    for directive in &[CODE_DIRECTIVE, RAW_DIRECTIVE] {
        if after_fence.starts_with(directive) {
            let rest = after_fence[directive.len()..].trim();
            let lexer = if rest.is_empty() {
                None
            } else {
                Some(rest.to_string())
            };
            return Some((fence, directive, lexer));
        }
    }

    None
}

/// Check if a line is a closing fence matching the opening fence.
fn is_closing_fence(line: &str, fence: &str) -> bool {
    let trimmed = line.trim();
    trimmed == fence
}

/// Parse directive content into (options_metadata, body_lines).
/// Handles both `:key: value` compact form and `---` delimited YAML blocks.
fn parse_directive_content(lines: &[&str]) -> (BTreeMap<String, Value>, Vec<String>) {
    if lines.is_empty() {
        return (BTreeMap::new(), Vec::new());
    }

    let mut pos = 0;
    let mut options: BTreeMap<String, Value> = BTreeMap::new();

    // Check for `---` delimited YAML block
    if lines[pos].trim() == "---" {
        pos += 1;
        let mut yaml_lines = Vec::new();
        while pos < lines.len() {
            if lines[pos].trim().starts_with("---") {
                pos += 1;
                break;
            }
            yaml_lines.push(lines[pos]);
            pos += 1;
        }
        let yaml_text = yaml_lines.join("\n");
        if let Ok(parsed) = serde_yaml::from_str::<BTreeMap<String, Value>>(&yaml_text) {
            options = parsed;
        }
    }
    // Check for `:key: value` compact form
    else if lines[pos].trim().starts_with(':') {
        let mut yaml_lines = Vec::new();
        while pos < lines.len() && lines[pos].trim().starts_with(':') {
            // Strip leading `:` to make valid YAML
            let line = lines[pos].trim();
            yaml_lines.push(&line[1..]);
            pos += 1;
        }
        let yaml_text = yaml_lines.join("\n");
        if let Ok(parsed) = serde_yaml::from_str::<BTreeMap<String, Value>>(&yaml_text) {
            options = parsed;
        }
    }

    // Remove first blank line after options (separator between options and content)
    if pos < lines.len() && lines[pos].trim().is_empty() {
        pos += 1;
    }

    let body: Vec<String> = lines[pos..].iter().map(|l| l.to_string()).collect();
    (options, body)
}

/// Flush accumulated markdown lines into a markdown cell.
fn flush_markdown(
    lines: &[&str],
    start: usize,
    end: usize,
    metadata: &BTreeMap<String, Value>,
    notebook: &mut Notebook,
) {
    if start >= end {
        if !metadata.is_empty() {
            // Metadata-only block break with no following content before next cell
            let mut cell = Cell::new_markdown("");
            cell.metadata = metadata.clone();
            notebook.cells.push(cell);
        }
        return;
    }

    let source = strip_blank_lines(&lines[start..end].join("\n"));
    if source.is_empty() && metadata.is_empty() {
        return;
    }

    let mut cell = Cell::new_markdown(&source);
    cell.metadata = metadata.clone();
    notebook.cells.push(cell);
}

/// Remove leading and trailing blank lines from text.
fn strip_blank_lines(text: &str) -> String {
    let trimmed = text.trim_end();
    let mut s = trimmed;
    while s.starts_with('\n') {
        s = &s[1..];
    }
    s.to_string()
}

// ---------------------------------------------------------------------------
// Writer: Notebook → MyST text
// ---------------------------------------------------------------------------

/// Convert a Jupyter Notebook to MyST-formatted text.
pub fn notebook_to_myst(nb: &Notebook) -> String {
    let mut output = String::new();

    // Get the pygments lexer for code cells
    let pygments_lexer = nb
        .metadata
        .get("language_info")
        .and_then(|li| li.get("pygments_lexer"))
        .and_then(|v| v.as_str())
        .or_else(|| {
            nb.metadata
                .get("jupytext")
                .and_then(|j| j.get("default_lexer"))
                .and_then(|v| v.as_str())
        });

    // Write notebook metadata as YAML front matter
    let has_metadata = !nb.metadata.is_empty();
    if has_metadata {
        output.push_str(&dump_yaml_front_matter(&nb.metadata));
    }

    let mut last_cell_md = false;

    for cell in &nb.cells {
        match cell.cell_type {
            CellType::Markdown => {
                if !cell.metadata.is_empty() || last_cell_md {
                    if !cell.metadata.is_empty() {
                        let json = serde_json::to_string(&cell.metadata).unwrap_or_default();
                        output.push_str(&format!("\n+++ {}\n", json));
                    } else {
                        output.push_str("\n+++\n");
                    }
                }
                output.push('\n');
                output.push_str(&cell.source);
                if !cell.source.ends_with('\n') {
                    output.push('\n');
                }
                last_cell_md = true;
            }
            CellType::Code | CellType::Raw => {
                let source_lines: Vec<String> =
                    cell.source.lines().map(|l| l.to_string()).collect();
                let delimiter = three_backticks_or_more(&source_lines);

                output.push('\n');
                output.push_str(&delimiter);
                if cell.cell_type == CellType::Code {
                    output.push_str(CODE_DIRECTIVE);
                    if let Some(lexer) = pygments_lexer {
                        output.push(' ');
                        output.push_str(lexer);
                    }
                } else {
                    output.push_str(RAW_DIRECTIVE);
                }
                output.push('\n');

                if !cell.metadata.is_empty() {
                    output.push_str(&dump_yaml_blocks(&cell.metadata));
                } else if cell.source.starts_with("---") || cell.source.starts_with(':') {
                    // Add blank line to separate from content that looks like YAML
                    output.push('\n');
                }

                output.push_str(&cell.source);
                if !cell.source.ends_with('\n') {
                    output.push('\n');
                }
                output.push_str(&delimiter);
                output.push('\n');
                last_cell_md = false;
            }
        }
    }

    // If no notebook metadata, remove leading blank line
    if !has_metadata && output.starts_with('\n') {
        output = output[1..].to_string();
    }

    // Ensure single trailing newline
    let trimmed = output.trim_end().to_string();
    trimmed + "\n"
}

/// Dump metadata as YAML front matter block (`---` delimited).
fn dump_yaml_front_matter(metadata: &BTreeMap<String, Value>) -> String {
    let yaml_str = yaml_dump(metadata);
    format!("---\n{}---\n", yaml_str)
}

/// Dump metadata as YAML blocks for cell options.
///
/// For blocks with no nested dicts, uses compact colon-prefixed form:
///   :key: value
///   :tags: [tag1, tag2]
///
/// For blocks with nesting, uses `---` delimited form:
///   ---
///   key:
///     nested: value
///   ---
fn dump_yaml_blocks(data: &BTreeMap<String, Value>) -> String {
    let yaml_str = yaml_dump(data);
    let yaml_lines: Vec<&str> = yaml_str.lines().collect();

    // Check if compact form is possible (no nested dicts, all lines start with alpha)
    let can_compact = yaml_lines
        .iter()
        .all(|line| line.is_empty() || line.chars().next().map_or(false, |c| c.is_alphabetic()));

    if can_compact {
        let mut result = String::new();
        for line in &yaml_lines {
            if !line.is_empty() {
                result.push(':');
                result.push_str(line);
                result.push('\n');
            }
        }
        result.push('\n');
        result
    } else {
        format!("---\n{}---\n", yaml_str)
    }
}

/// Dump a BTreeMap as YAML with compact list style.
fn yaml_dump(data: &BTreeMap<String, Value>) -> String {
    // Convert BTreeMap<String, Value> to a serde_yaml::Value for dumping
    let value = serde_json::to_value(data).unwrap_or(Value::Null);
    match serde_yaml::to_string(&value) {
        Ok(s) => {
            // serde_yaml prepends "---\n", remove it
            let s = s.strip_prefix("---\n").unwrap_or(&s);
            s.to_string()
        }
        Err(_) => String::new(),
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_front_matter() {
        let text = "---\ntitle: Test\n---\nContent";
        let lines: Vec<&str> = text.lines().collect();
        let (meta, next) = parse_front_matter(&lines);
        assert_eq!(next, 3);
        assert_eq!(
            meta.get("title").and_then(|v| v.as_str()),
            Some("Test")
        );
    }

    #[test]
    fn test_parse_front_matter_no_front_matter() {
        let text = "# Just a heading\nContent";
        let lines: Vec<&str> = text.lines().collect();
        let (meta, next) = parse_front_matter(&lines);
        assert_eq!(next, 0);
        assert!(meta.is_empty());
    }

    #[test]
    fn test_block_break() {
        assert!(is_block_break("+++"));
        assert!(is_block_break("+++ {\"key\": \"value\"}"));
        assert!(!is_block_break("# heading"));
    }

    #[test]
    fn test_block_break_metadata() {
        let meta = parse_block_break_metadata("+++ {\"tags\": [\"hide\"]}");
        assert!(meta.contains_key("tags"));
    }

    #[test]
    fn test_parse_fence_open_code() {
        let result = parse_fence_open("```{code-cell} python3");
        assert!(result.is_some());
        let (fence, directive, lexer) = result.unwrap();
        assert_eq!(fence, "```");
        assert_eq!(directive, "{code-cell}");
        assert_eq!(lexer, Some("python3".to_string()));
    }

    #[test]
    fn test_parse_fence_open_raw() {
        let result = parse_fence_open("```{raw-cell}");
        assert!(result.is_some());
        let (fence, directive, lexer) = result.unwrap();
        assert_eq!(fence, "```");
        assert_eq!(directive, "{raw-cell}");
        assert_eq!(lexer, None);
    }

    #[test]
    fn test_parse_fence_open_not_directive() {
        let result = parse_fence_open("```python");
        assert!(result.is_none());
    }

    #[test]
    fn test_directive_content_yaml_block() {
        let lines = vec!["---", "tags: [hide]", "---", "", "print('hello')"];
        let (options, body) = parse_directive_content(&lines);
        assert!(options.contains_key("tags"));
        assert_eq!(body, vec!["print('hello')"]);
    }

    #[test]
    fn test_directive_content_compact() {
        let lines = vec![":tags: [hide]", "", "print('hello')"];
        let (options, body) = parse_directive_content(&lines);
        assert!(options.contains_key("tags"));
        assert_eq!(body, vec!["print('hello')"]);
    }

    #[test]
    fn test_directive_content_no_options() {
        let lines = vec!["print('hello')", "print('world')"];
        let (options, body) = parse_directive_content(&lines);
        assert!(options.is_empty());
        assert_eq!(body, vec!["print('hello')", "print('world')"]);
    }

    #[test]
    fn test_simple_myst_roundtrip() {
        let myst_text = r#"---
jupytext:
  text_representation:
    extension: .md
    format_name: myst
    format_version: '0.13'
kernelspec:
  display_name: Python 3
  language: python
  name: python3
---

# My Notebook

Some markdown content.

```{code-cell} ipython3
x = 1
print(x)
```

More markdown.

```{code-cell} ipython3
y = 2
```
"#;

        let nb = myst_to_notebook(myst_text).unwrap();
        assert_eq!(nb.cells.len(), 4);
        assert_eq!(nb.cells[0].cell_type, CellType::Markdown);
        assert!(nb.cells[0].source.contains("# My Notebook"));
        assert_eq!(nb.cells[1].cell_type, CellType::Code);
        assert!(nb.cells[1].source.contains("x = 1"));
        assert_eq!(nb.cells[2].cell_type, CellType::Markdown);
        assert!(nb.cells[2].source.contains("More markdown"));
        assert_eq!(nb.cells[3].cell_type, CellType::Code);
        assert!(nb.cells[3].source.contains("y = 2"));
    }

    #[test]
    fn test_myst_with_cell_metadata() {
        let myst_text = r#"```{code-cell} python3
:tags: [hide-output]

print("hello")
```
"#;

        let nb = myst_to_notebook(myst_text).unwrap();
        assert_eq!(nb.cells.len(), 1);
        assert_eq!(nb.cells[0].cell_type, CellType::Code);
        assert!(nb.cells[0].metadata.contains_key("tags"));
    }

    #[test]
    fn test_myst_with_block_break() {
        let myst_text = r#"First cell.

+++ {"tags": ["special"]}

Second cell with metadata.
"#;

        let nb = myst_to_notebook(myst_text).unwrap();
        assert_eq!(nb.cells.len(), 2);
        assert_eq!(nb.cells[0].source, "First cell.");
        assert!(nb.cells[1].metadata.contains_key("tags"));
        assert_eq!(nb.cells[1].source, "Second cell with metadata.");
    }

    #[test]
    fn test_notebook_to_myst_simple() {
        let mut nb = Notebook::new();
        nb.cells.push(Cell::new_markdown("# Hello"));
        nb.cells.push(Cell::new_code("x = 1"));

        let text = notebook_to_myst(&nb);
        assert!(text.contains("# Hello"));
        assert!(text.contains("{code-cell}"));
        assert!(text.contains("x = 1"));
    }

    #[test]
    fn test_notebook_to_myst_with_metadata() {
        let mut nb = Notebook::new();
        nb.metadata.insert(
            "kernelspec".to_string(),
            serde_json::json!({"display_name": "Python 3", "language": "python", "name": "python3"}),
        );
        nb.cells.push(Cell::new_code("x = 1"));

        let text = notebook_to_myst(&nb);
        assert!(text.starts_with("---\n"));
        assert!(text.contains("kernelspec"));
    }

    #[test]
    fn test_dump_yaml_blocks_compact() {
        let mut data = BTreeMap::new();
        data.insert("tags".to_string(), serde_json::json!("hide"));
        let result = dump_yaml_blocks(&data);
        assert!(result.starts_with(':'), "Expected compact form starting with ':', got: {}", result);
    }

    #[test]
    fn test_dump_yaml_blocks_nested() {
        let mut data = BTreeMap::new();
        data.insert("nested".to_string(), serde_json::json!({"key": "value"}));
        let result = dump_yaml_blocks(&data);
        assert!(result.starts_with("---"), "Expected block form starting with '---', got: {}", result);
    }

    #[test]
    fn test_raw_cell_roundtrip() {
        let myst_text = r#"```{raw-cell}
<div>raw html</div>
```
"#;

        let nb = myst_to_notebook(myst_text).unwrap();
        assert_eq!(nb.cells.len(), 1);
        assert_eq!(nb.cells[0].cell_type, CellType::Raw);
        assert!(nb.cells[0].source.contains("<div>raw html</div>"));
    }
}
