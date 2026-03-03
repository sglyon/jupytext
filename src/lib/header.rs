//! Parse headers of text notebooks (YAML front matter)

use once_cell::sync::Lazy;
use regex::Regex;
use serde_json::Value;
use std::collections::BTreeMap;

use crate::languages::{comment_lines, default_language_from_metadata_and_ext, SCRIPT_EXTENSIONS};
use crate::metadata_filter::{filter_metadata, DEFAULT_NOTEBOOK_METADATA};
use crate::notebook::Cell;
use crate::pep8::pep8_lines_between_cells;

static HEADER_RE: Lazy<Regex> = Lazy::new(|| Regex::new(r"^---\s*$").unwrap());
static BLANK_RE: Lazy<Regex> = Lazy::new(|| Regex::new(r"^\s*$").unwrap());
static JUPYTER_RE: Lazy<Regex> = Lazy::new(|| Regex::new(r"^jupyter\s*:\s*$").unwrap());
static LEFTSPACE_RE: Lazy<Regex> = Lazy::new(|| Regex::new(r"^\s").unwrap());

#[allow(dead_code)]
const UTF8_HEADER: &str = " -*- coding: utf-8 -*-";
pub static INSERT_AND_CHECK_VERSION_NUMBER: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(true);

pub fn insert_or_test_version_number() -> bool {
    INSERT_AND_CHECK_VERSION_NUMBER.load(std::sync::atomic::Ordering::Relaxed)
}

/// Uncomment a single line
fn uncomment_line(line: &str, prefix: &str, suffix: &str) -> String {
    let mut result = line.to_string();
    if !prefix.is_empty() {
        let ps = format!("{} ", prefix);
        if result.starts_with(&ps) {
            result = result[ps.len()..].to_string();
        } else if result.starts_with(prefix) {
            result = result[prefix.len()..].to_string();
        }
    }
    if !suffix.is_empty() {
        let ss = format!("{} ", suffix);
        if result.ends_with(&ss) {
            result = result[..result.len() - ss.len()].to_string();
        } else if result.ends_with(suffix) {
            result = result[..result.len() - suffix.len()].to_string();
        }
    }
    result
}

/// Result of parsing header
pub struct HeaderParseResult {
    pub metadata: serde_json::Map<String, Value>,
    pub has_jupyter_md: bool,
    pub header_cell: Option<Cell>,
    pub next_line: usize,
}

/// Parse the YAML header from text lines
pub fn header_to_metadata_and_cell(
    lines: &[String],
    header_prefix: &str,
    header_suffix: &str,
    ext: &str,
    root_level_metadata_as_raw_cell: bool,
) -> HeaderParseResult {
    let mut header_lines = Vec::new();
    let mut jupyter_lines = Vec::new();
    let mut in_jupyter = false;
    let mut in_html_div = false;
    let mut start = 0;
    let mut started = false;
    let mut ended = false;
    let mut metadata = serde_json::Map::new();

    let comment = if header_prefix == "#'" {
        "#"
    } else {
        header_prefix
    };

    let encoding_pattern = format!(
        r"^[ \t\f]*{}.*?coding[:=][ \t]*([-_.a-zA-Z0-9]+)",
        regex::escape(comment)
    );
    let encoding_re = Regex::new(&encoding_pattern).ok();

    let mut i: usize = 0;
    for (idx, line) in lines.iter().enumerate() {
        i = idx;

        if idx == 0 && line.starts_with("#!") {
            let jupytext = metadata
                .entry("jupytext".to_string())
                .or_insert_with(|| Value::Object(serde_json::Map::new()));
            if let Some(obj) = jupytext.as_object_mut() {
                obj.insert(
                    "executable".to_string(),
                    Value::String(line[2..].to_string()),
                );
            }
            start = idx + 1;
            continue;
        }

        if idx == 0 || (idx == 1 && encoding_re.as_ref().map_or(true, |re| !re.is_match(&lines[0])))
        {
            if let Some(ref re) = encoding_re {
                if re.is_match(line) {
                    let jupytext = metadata
                        .entry("jupytext".to_string())
                        .or_insert_with(|| Value::Object(serde_json::Map::new()));
                    if let Some(obj) = jupytext.as_object_mut() {
                        obj.insert(
                            "encoding".to_string(),
                            Value::String(line.to_string()),
                        );
                    }
                    start = idx + 1;
                    continue;
                }
            }
        }

        if !line.starts_with(header_prefix) && !header_prefix.is_empty() {
            break;
        }

        if header_prefix.is_empty() && line.trim().starts_with("<!--") {
            in_html_div = true;
            continue;
        }

        if in_html_div {
            if ended && line.contains("-->") {
                i = idx;
                break;
            }
            if !started && line.trim().is_empty() {
                continue;
            }
        }

        let uncommented = uncomment_line(line, header_prefix, header_suffix);

        if HEADER_RE.is_match(&uncommented) {
            if !started {
                started = true;
                continue;
            }
            ended = true;
            if in_html_div {
                continue;
            }
            break;
        }

        if !started && !uncommented.trim().is_empty() {
            break;
        }

        if JUPYTER_RE.is_match(&uncommented) {
            in_jupyter = true;
        } else if !uncommented.is_empty() && !LEFTSPACE_RE.is_match(&uncommented) {
            in_jupyter = false;
        }

        if in_jupyter {
            jupyter_lines.push(uncommented);
        } else {
            header_lines.push(uncommented);
        }
    }

    if ended {
        if !jupyter_lines.is_empty() {
            let yaml_str = jupyter_lines.join("\n");
            if let Ok(yaml_val) = serde_yaml::from_str::<Value>(&yaml_str) {
                if let Some(jupyter) = yaml_val.get("jupyter") {
                    if let Value::Object(map) = jupyter {
                        let extra = metadata.clone();
                        metadata = map.clone();
                        for (k, v) in extra {
                            metadata.insert(k, v);
                        }
                    }
                }
            }
        }

        let mut lines_to_next_cell: usize = 1;
        if i + 1 < lines.len() {
            let next_line = uncomment_line(&lines[i + 1], header_prefix, "");
            if !BLANK_RE.is_match(&next_line) {
                lines_to_next_cell = 0;
            } else {
                i += 1;
            }
        } else {
            lines_to_next_cell = 0;
        }

        let header_cell = if !header_lines.is_empty() {
            if root_level_metadata_as_raw_cell {
                let source = format!(
                    "---\n{}\n---",
                    header_lines.join("\n")
                );
                let mut cell = Cell::new_raw(&source);
                let expected = pep8_lines_between_cells(
                    &["---".to_string()],
                    &lines[i + 1..].iter().map(|s| s.to_string()).collect::<Vec<_>>(),
                    ext,
                );
                if lines_to_next_cell != expected {
                    cell.metadata.insert(
                        "lines_to_next_cell".to_string(),
                        Value::Number(serde_json::Number::from(lines_to_next_cell)),
                    );
                }
                Some(cell)
            } else {
                // Store as root_level_metadata
                let header_yaml = header_lines.join("\n");
                if let Ok(root_meta) = serde_yaml::from_str::<Value>(&header_yaml) {
                    let jupytext = metadata
                        .entry("jupytext".to_string())
                        .or_insert_with(|| Value::Object(serde_json::Map::new()));
                    if let Some(obj) = jupytext.as_object_mut() {
                        obj.insert("root_level_metadata".to_string(), root_meta);
                    }
                }
                None
            }
        } else {
            None
        };

        return HeaderParseResult {
            metadata,
            has_jupyter_md: !jupyter_lines.is_empty(),
            header_cell,
            next_line: i + 1,
        };
    }

    HeaderParseResult {
        metadata,
        has_jupyter_md: false,
        header_cell: None,
        next_line: start,
    }
}

/// Get encoding and executable lines for a notebook
pub fn encoding_and_executable(
    metadata: &mut serde_json::Map<String, Value>,
    ext: &str,
) -> Vec<String> {
    let mut lines = Vec::new();
    let comment = SCRIPT_EXTENSIONS.get(ext).map(|sl| sl.comment);

    if let Some(_comment) = comment {
        if let Some(jupytext) = metadata.get_mut("jupytext") {
            if let Some(obj) = jupytext.as_object_mut() {
                if let Some(Value::String(exec)) = obj.remove("executable") {
                    lines.push(format!("#!{}", exec));
                }
                if let Some(Value::String(enc)) = obj.remove("encoding") {
                    lines.push(enc);
                } else {
                    // Check if we need UTF-8 encoding header
                    let lang = default_language_from_metadata_and_ext(metadata, ext, false);
                    if lang.as_deref() != Some("python") {
                        // TODO: check cell content for non-ASCII
                    }
                }
            }
        }
    }

    lines
}

/// Generate the text header from notebook metadata
pub fn metadata_and_cell_to_header(
    metadata: &serde_json::Map<String, Value>,
    fmt: &BTreeMap<String, Value>,
    header_prefix: &str,
    header_suffix: &str,
) -> (Vec<String>, Option<usize>) {
    let mut header = Vec::new();

    // Filter metadata
    let notebook_metadata_filter = fmt
        .get("notebook_metadata_filter")
        .and_then(|v| v.as_str())
        .unwrap_or("");

    let btree_meta: BTreeMap<String, Value> = metadata.iter().map(|(k, v)| (k.clone(), v.clone())).collect();
    let filtered = filter_metadata(&btree_meta, notebook_metadata_filter, DEFAULT_NOTEBOOK_METADATA);

    if !filtered.is_empty() {
        let mut wrapper = BTreeMap::new();
        wrapper.insert("jupyter".to_string(), Value::Object(
            filtered.into_iter().collect()
        ));
        if let Ok(yaml) = serde_yaml::to_string(&wrapper) {
            for line in yaml.trim().lines() {
                header.push(line.to_string());
            }
        }
    }

    if !header.is_empty() {
        let mut full_header = vec!["---".to_string()];
        full_header.extend(header);
        full_header.push("---".to_string());

        let hide = fmt
            .get("hide_notebook_metadata")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        if hide {
            let mut wrapped = vec!["<!--".to_string(), String::new()];
            wrapped.extend(full_header);
            wrapped.push(String::new());
            wrapped.push("-->".to_string());
            full_header = wrapped;
        }

        let commented = comment_lines(
            &full_header.iter().map(|s| s.as_str().to_string()).collect::<Vec<_>>(),
            header_prefix,
            header_suffix,
        );
        return (commented, None);
    }

    (Vec::new(), Some(0))
}

/// Recursively update a nested map
pub fn recursive_update(
    target: &mut serde_json::Map<String, Value>,
    update: &serde_json::Map<String, Value>,
    overwrite: bool,
) {
    for (key, value) in update {
        if value.is_null() {
            target.remove(key);
        } else if let Value::Object(update_map) = value {
            let entry = target
                .entry(key.clone())
                .or_insert_with(|| Value::Object(serde_json::Map::new()));
            if let Value::Object(target_map) = entry {
                recursive_update(target_map, update_map, overwrite);
            }
        } else if overwrite {
            target.insert(key.clone(), value.clone());
        } else {
            target.entry(key.clone()).or_insert_with(|| value.clone());
        }
    }
}
