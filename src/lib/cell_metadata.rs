//! Cell metadata parsing and conversion

use once_cell::sync::Lazy;
use regex::Regex;
use serde_json::{json, Value};
use std::collections::BTreeMap;

/// Jupytext-specific cell metadata keys
pub const JUPYTEXT_CELL_METADATA: &[&str] = &[
    "skipline",
    "noskipline",
    "cell_marker",
    "lines_to_next_cell",
    "lines_to_end_of_cell_marker",
];

/// Default cell metadata to ignore in text representation
pub const IGNORE_CELL_METADATA: &str = "-autoscroll,-collapsed,-scrolled,-trusted,-execution,-ExecuteTime,-skipline,-noskipline,-cell_marker,-lines_to_next_cell,-lines_to_end_of_cell_marker";

static IS_IDENTIFIER: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"^[a-zA-Z_\.]+[a-zA-Z0-9_\.]*$").unwrap());

static IS_VALID_METADATA_KEY: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"^[a-zA-Z0-9_\.@/-]+$").unwrap());

/// Is the cell active for the given file extension?
pub fn is_active(ext: &str, metadata: &BTreeMap<String, Value>, default: bool) -> bool {
    // Check run_control.frozen
    if let Some(rc) = metadata.get("run_control") {
        if let Some(frozen) = rc.get("frozen") {
            if frozen.as_bool() == Some(true) {
                return ext == ".ipynb";
            }
        }
    }

    // Check tags
    if let Some(Value::Array(tags)) = metadata.get("tags") {
        for tag in tags {
            if let Some(tag_str) = tag.as_str() {
                if tag_str.starts_with("active-") {
                    let ext_no_dot = ext.replace('.', "");
                    return tag_str.split('-').any(|part| part == ext_no_dot);
                }
            }
        }
    }

    // Check active field
    match metadata.get("active") {
        Some(Value::String(active)) => {
            let ext_no_dot = ext.replace('.', "");
            active.split(['.', ','].as_ref()).any(|part| part == ext_no_dot)
        }
        None => default,
        _ => default,
    }
}

/// Is this a JSON metadata string?
pub fn is_json_metadata(text: &str) -> bool {
    let first_curly = text.find('{');
    let first_equal = text.find('=');

    match (first_curly, first_equal) {
        (Some(c), Some(e)) => c < e,
        (Some(_), None) => true,
        _ => false,
    }
}

/// Check if string is a valid identifier
pub fn is_identifier(text: &str) -> bool {
    IS_IDENTIFIER.is_match(text)
}

/// Check if string is a valid metadata key
pub fn is_valid_metadata_key(text: &str) -> bool {
    IS_VALID_METADATA_KEY.is_match(text)
}

/// Try to parse JSON, falling back to relaxed parsing
pub fn relax_json_loads(text: &str, catch: bool) -> Result<Value, String> {
    let text = text.trim();

    // Try JSON first
    if let Ok(v) = serde_json::from_str::<Value>(text) {
        return Ok(v);
    }

    // Try Python-like literal parsing
    if let Ok(v) = parse_python_literal(text) {
        return Ok(v);
    }

    if catch {
        Ok(json!({"incorrectly_encoded_metadata": text}))
    } else {
        Err(format!("Cannot parse: {}", text))
    }
}

/// Parse Python-like literal (True/False, lists, strings)
fn parse_python_literal(text: &str) -> Result<Value, String> {
    let text = text.trim();
    match text {
        "True" | "true" => Ok(Value::Bool(true)),
        "False" | "false" => Ok(Value::Bool(false)),
        "None" | "null" => Ok(Value::Null),
        _ => {
            // Try as number
            if let Ok(n) = text.parse::<i64>() {
                return Ok(json!(n));
            }
            if let Ok(n) = text.parse::<f64>() {
                return Ok(json!(n));
            }

            // Try as quoted string
            if (text.starts_with('"') && text.ends_with('"'))
                || (text.starts_with('\'') && text.ends_with('\''))
            {
                return Ok(Value::String(text[1..text.len() - 1].to_string()));
            }

            // Try as list
            if text.starts_with('[') && text.ends_with(']') {
                let inner = text[1..text.len() - 1].trim();
                if inner.is_empty() {
                    return Ok(json!([]));
                }
                let items: Result<Vec<Value>, String> = split_respecting_quotes(inner)
                    .iter()
                    .map(|item| relax_json_loads(item.trim(), true))
                    .collect();
                return Ok(Value::Array(items?));
            }

            Err(format!("Cannot parse literal: {}", text))
        }
    }
}

/// Split string by commas, respecting quotes
fn split_respecting_quotes(text: &str) -> Vec<String> {
    let mut result = Vec::new();
    let mut current = String::new();
    let mut in_single = false;
    let mut in_double = false;
    let mut depth = 0;

    for ch in text.chars() {
        match ch {
            '\'' if !in_double => {
                in_single = !in_single;
                current.push(ch);
            }
            '"' if !in_single => {
                in_double = !in_double;
                current.push(ch);
            }
            '(' | '[' | '{' if !in_single && !in_double => {
                depth += 1;
                current.push(ch);
            }
            ')' | ']' | '}' if !in_single && !in_double => {
                depth -= 1;
                current.push(ch);
            }
            ',' if !in_single && !in_double && depth == 0 => {
                result.push(current.trim().to_string());
                current = String::new();
            }
            _ => current.push(ch),
        }
    }
    if !current.trim().is_empty() {
        result.push(current.trim().to_string());
    }
    result
}

/// Parse key=value metadata string
pub fn parse_key_equal_value(text: &str) -> BTreeMap<String, Value> {
    let text = text.trim();
    if text.is_empty() {
        return BTreeMap::new();
    }

    // Try as JSON object
    if text.starts_with('{') {
        if let Ok(Value::Object(map)) = serde_json::from_str::<Value>(text) {
            let mut result = BTreeMap::new();
            for (k, v) in map {
                result.insert(k, v);
            }
            return result;
        }
    }

    let last_space = text.rfind(' ');

    // Just an identifier?
    if let Some(pos) = last_space {
        let after = &text[pos + 1..];
        if !after.starts_with("--") && is_identifier(after) && !after.contains('=') {
            let mut result = BTreeMap::new();
            result.insert(after.to_string(), Value::Null);
            if pos > 0 {
                result.extend(parse_key_equal_value(&text[..pos]));
            }
            return result;
        }
    } else if is_identifier(text) && !text.contains('=') {
        let mut result = BTreeMap::new();
        result.insert(text.to_string(), Value::Null);
        return result;
    }

    // Iterate on '=' signs from right to left
    let mut equal_pos = text.len();
    loop {
        match text[..equal_pos].rfind('=') {
            None => {
                let mut result = BTreeMap::new();
                result.insert(
                    "incorrectly_encoded_metadata".to_string(),
                    Value::String(text.to_string()),
                );
                return result;
            }
            Some(pos) => {
                equal_pos = pos;
                let before = &text[..equal_pos];
                let prev_ws = before.trim_end().rfind(' ').map(|p| p + 1).unwrap_or(0);
                let key = text[prev_ws..equal_pos].trim();

                if !is_valid_metadata_key(key) {
                    continue;
                }

                let value_str = &text[equal_pos + 1..];
                match relax_json_loads(value_str, false) {
                    Ok(value) => {
                        let mut metadata = if prev_ws > 0 {
                            parse_key_equal_value(&text[..prev_ws])
                        } else {
                            BTreeMap::new()
                        };
                        metadata.insert(key.to_string(), value);
                        return metadata;
                    }
                    Err(_) => continue,
                }
            }
        }
    }
}

/// Parse language/title and metadata from option line
pub fn text_to_metadata(text: &str, allow_title: bool) -> (String, BTreeMap<String, Value>) {
    let text = text.trim();
    if text.is_empty() {
        return (String::new(), BTreeMap::new());
    }

    let first_curly = text.find('{');
    let first_equal = text.find('=');

    // Check for JSON metadata
    if let Some(cb) = first_curly {
        if first_equal.is_none() || cb < first_equal.unwrap() {
            let title = text[..cb].trim().to_string();
            let json_str = &text[cb..];
            match relax_json_loads(json_str, true) {
                Ok(Value::Object(map)) => {
                    let mut metadata = BTreeMap::new();
                    for (k, v) in map {
                        metadata.insert(k, v);
                    }
                    return (title, metadata);
                }
                Ok(v) => {
                    let mut metadata = BTreeMap::new();
                    metadata.insert("incorrectly_encoded_metadata".to_string(), v);
                    return (title, metadata);
                }
                Err(_) => {}
            }
        }
    }

    // Key=value metadata
    if !allow_title {
        if is_jupyter_language(text) {
            return (text.to_string(), BTreeMap::new());
        }
        if !text.contains(' ') {
            return (String::new(), parse_key_equal_value(text));
        }
        if let Some(pos) = text.find(' ') {
            let language = &text[..pos];
            if is_jupyter_language(language) {
                return (
                    language.to_string(),
                    parse_key_equal_value(&text[pos + 1..]),
                );
            }
        }
        return (String::new(), parse_key_equal_value(text));
    }

    // With title
    if let Some(eq_pos) = first_equal {
        let words: Vec<&str> = text[..eq_pos].split_whitespace().collect();
        let mut title_words = words.clone();
        // Last word is the key before the equal sign
        if !title_words.is_empty() {
            title_words.pop();
        }
        // Remove words that are attributes (start with '.')
        while !title_words.is_empty() {
            let last = title_words.last().unwrap();
            if last.is_empty() || last.starts_with('.') {
                title_words.pop();
            } else {
                break;
            }
        }
        let title = title_words.join(" ");
        return (title.clone(), parse_key_equal_value(&text[title.len()..]));
    }

    // All words are the title
    let words: Vec<&str> = text.split_whitespace().collect();
    let mut title_words = words;
    while !title_words.is_empty() {
        let last = title_words.last().unwrap();
        if last.is_empty() || last.starts_with('.') {
            title_words.pop();
        } else {
            break;
        }
    }
    let title = title_words.join(" ");
    let remaining = &text[title.len()..];
    (title, parse_key_equal_value(remaining))
}

/// Is this a jupyter language?
pub fn is_jupyter_language(text: &str) -> bool {
    crate::languages::JUPYTER_LANGUAGES.contains(&text.to_lowercase())
        || crate::languages::JUPYTER_LANGUAGES.contains(text)
}

/// Convert metadata to text representation
pub fn metadata_to_text(
    language_or_title: Option<&str>,
    metadata: &BTreeMap<String, Value>,
    plain_json: bool,
) -> String {
    let metadata: BTreeMap<String, Value> = metadata
        .iter()
        .filter(|(k, _)| !JUPYTEXT_CELL_METADATA.contains(&k.as_str()))
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect();

    let mut parts: Vec<String> = Vec::new();
    if let Some(title) = language_or_title {
        if !title.is_empty() {
            parts.push(title.to_string());
        }
    }

    if plain_json {
        if !metadata.is_empty() {
            parts.push(serde_json::to_string(&metadata).unwrap_or_default());
        }
    } else {
        for (key, value) in &metadata {
            if key == "incorrectly_encoded_metadata" {
                if let Value::String(s) = value {
                    parts.push(s.clone());
                }
            } else if value.is_null() {
                parts.push(key.clone());
            } else {
                parts.push(format!("{}={}", key, serde_json::to_string(value).unwrap_or_default()));
            }
        }
    }

    parts.join(" ")
}

/// Convert metadata to double percent format options
pub fn metadata_to_double_percent_options(
    metadata: &mut BTreeMap<String, Value>,
    plain_json: bool,
) -> String {
    let mut text_parts: Vec<String> = Vec::new();

    if let Some(Value::String(title)) = metadata.remove("title") {
        text_parts.push(title);
    }
    if let Some(Value::Number(depth)) = metadata.remove("cell_depth") {
        if let Some(d) = depth.as_u64() {
            text_parts.insert(0, "%".repeat(d as usize));
        }
    }
    if let Some(Value::String(ct)) = metadata.remove("cell_type") {
        let region_name = metadata
            .remove("region_name")
            .and_then(|v| v.as_str().map(|s| s.to_string()))
            .unwrap_or(ct);
        text_parts.push(format!("[{}]", region_name));
    }

    let joined = text_parts.join(" ");
    metadata_to_text(
        if joined.is_empty() {
            None
        } else {
            Some(&joined)
        },
        metadata,
        plain_json,
    )
}

/// R Markdown option parsing context
struct ParsingContext {
    parenthesis_count: i32,
    curly_bracket_count: i32,
    square_bracket_count: i32,
    in_single_quote: bool,
    in_double_quote: bool,
}

impl ParsingContext {
    fn new() -> Self {
        ParsingContext {
            parenthesis_count: 0,
            curly_bracket_count: 0,
            square_bracket_count: 0,
            in_single_quote: false,
            in_double_quote: false,
        }
    }

    fn in_global_expression(&self) -> bool {
        self.parenthesis_count == 0
            && self.curly_bracket_count == 0
            && self.square_bracket_count == 0
            && !self.in_single_quote
            && !self.in_double_quote
    }

    fn count_special_chars(&mut self, ch: char, _prev: char) {
        match ch {
            '(' => self.parenthesis_count += 1,
            ')' => self.parenthesis_count -= 1,
            '{' => self.curly_bracket_count += 1,
            '}' => self.curly_bracket_count -= 1,
            '[' => self.square_bracket_count += 1,
            ']' => self.square_bracket_count -= 1,
            '\'' if !self.in_double_quote => self.in_single_quote = !self.in_single_quote,
            '"' if !self.in_single_quote => self.in_double_quote = !self.in_double_quote,
            _ => {}
        }
    }
}

/// Parse R Markdown options
pub fn parse_rmd_options(line: &str) -> Vec<(String, String)> {
    let mut ctx = ParsingContext::new();
    let mut result = Vec::new();
    let mut prev_char = ' ';
    let mut name = String::new();
    let mut value = String::new();

    let padded = format!(",{},", line);
    for ch in padded.chars() {
        if ctx.in_global_expression() {
            if ch == ',' {
                if !name.is_empty() || !value.is_empty() {
                    result.push((name.trim().to_string(), value.trim().to_string()));
                    name = String::new();
                    value = String::new();
                }
            } else if ch == '=' {
                if name.is_empty() {
                    name = value.clone();
                    value = String::new();
                } else {
                    value.push(ch);
                }
            } else {
                ctx.count_special_chars(ch, prev_char);
                value.push(ch);
            }
        } else {
            ctx.count_special_chars(ch, prev_char);
            value.push(ch);
        }
        prev_char = ch;
    }

    result
}

/// Convert R Markdown options to metadata
pub fn rmd_options_to_metadata(
    options: &str,
    use_runtools: bool,
) -> (String, BTreeMap<String, Value>) {
    let parts: Vec<&str> = options.splitn(2, |c: char| c.is_whitespace() || c == ',').collect();

    // Handle "wolfram language" as a special case
    let (language, chunk_options) = if options.starts_with("wolfram language") {
        ("wolfram language".to_string(), &options[16..])
    } else if parts.len() == 1 {
        (parts[0].to_string(), "")
    } else {
        (parts[0].trim_end_matches(',').to_string(), parts[1].trim_start_matches([' ', ',']))
    };

    let language = if language == "r" { "R".to_string() } else { language };

    let mut metadata = BTreeMap::new();
    let opts = if chunk_options.is_empty() {
        vec![]
    } else {
        parse_rmd_options(chunk_options)
    };

    for (i, (name, value)) in opts.iter().enumerate() {
        if i == 0 && name.is_empty() {
            metadata.insert("name".to_string(), Value::String(value.clone()));
            continue;
        }

        // R logical values
        match value.as_str() {
            "TRUE" | "T" => {
                metadata.insert(name.clone(), Value::Bool(true));
                continue;
            }
            "FALSE" | "F" => {
                metadata.insert(name.clone(), Value::Bool(false));
                continue;
            }
            _ => {}
        }

        // Try to evaluate value
        let parsed = try_eval_rmd_value(value);
        metadata.insert(name.clone(), parsed);
    }

    // Handle eval=FALSE (inactive cell)
    if let Some(Value::Bool(false)) = metadata.get("eval") {
        if !is_active(".Rmd", &metadata, true) {
            metadata.remove("eval");
        }
    }

    let _ = use_runtools; // TODO: implement runtools mapping

    let lang = metadata
        .remove("language")
        .and_then(|v| v.as_str().map(|s| s.to_string()))
        .unwrap_or(language);

    (lang, metadata)
}

/// Try to evaluate an R Markdown value to a Python/JSON type
fn try_eval_rmd_value(value: &str) -> Value {
    // Quoted strings
    if (value.starts_with('"') && value.ends_with('"'))
        || (value.starts_with('\'') && value.ends_with('\''))
    {
        return Value::String(value[1..value.len() - 1].to_string());
    }

    // R vector: c(...)
    if value.starts_with("c(") && value.ends_with(')') {
        let inner = &value[2..value.len() - 1];
        let list_str = format!("[{}]", inner);
        if let Ok(v) = serde_json::from_str::<Value>(&list_str) {
            return v;
        }
    }

    // R list: list(...)
    if value.starts_with("list(") && value.ends_with(')') {
        let inner = &value[5..value.len() - 1];
        let list_str = format!("[{}]", inner);
        if let Ok(v) = serde_json::from_str::<Value>(&list_str) {
            return v;
        }
    }

    // Try as number
    if let Ok(n) = value.parse::<i64>() {
        return json!(n);
    }
    if let Ok(n) = value.parse::<f64>() {
        return json!(n);
    }

    // Try as JSON
    if let Ok(v) = serde_json::from_str::<Value>(value) {
        return v;
    }

    // R code placeholder
    Value::String(format!("#R_CODE#{}", value))
}

/// Convert metadata to R Markdown options
pub fn metadata_to_rmd_options(
    language: Option<&str>,
    metadata: &BTreeMap<String, Value>,
    _use_runtools: bool,
) -> String {
    let mut options = language.unwrap_or("R").to_lowercase();

    let mut metadata = metadata.clone();

    if let Some(Value::String(name)) = metadata.remove("name") {
        options += &format!(" {},", name);
    }

    for (opt_name, opt_value) in &metadata {
        let opt_name = opt_name.trim();
        match opt_value {
            Value::Bool(true) => options += &format!(" {}=TRUE,", opt_name),
            Value::Bool(false) => options += &format!(" {}=FALSE,", opt_name),
            Value::String(s) if opt_name == "active" => {
                options += &format!(" {}=\"{}\",", opt_name, s);
            }
            Value::String(s) if s.starts_with("#R_CODE#") => {
                options += &format!(" {}={},", opt_name, &s[8..]);
            }
            Value::String(s) if !s.contains('"') => {
                options += &format!(" {}=\"{}\",", opt_name, s);
            }
            Value::String(s) => {
                options += &format!(" {}='{}',", opt_name, s);
            }
            Value::Array(arr) => {
                let items: Vec<String> = arr
                    .iter()
                    .map(|v| match v {
                        Value::String(s) => format!("\"{}\"", s),
                        other => other.to_string(),
                    })
                    .collect();
                options += &format!(" {}=c({}),", opt_name, items.join(", "));
            }
            other => {
                options += &format!(" {}={},", opt_name, other);
            }
        }
    }

    if language.is_none() {
        // Remove leading "r " for no-language case
        if options.starts_with("r ") {
            options = options[2..].to_string();
        }
    }

    options.trim_end_matches(',').trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_text_to_metadata_simple() {
        let (lang, meta) = text_to_metadata("python", false);
        assert_eq!(lang, "python");
        assert!(meta.is_empty());
    }

    #[test]
    fn test_text_to_metadata_with_options() {
        let (lang, meta) = text_to_metadata("python tags=[\"test\"]", false);
        assert_eq!(lang, "python");
        assert!(meta.contains_key("tags"));
    }

    #[test]
    fn test_is_active_default() {
        let meta = BTreeMap::new();
        assert!(is_active(".py", &meta, true));
        assert!(is_active(".ipynb", &meta, true));
    }

    #[test]
    fn test_is_active_specific() {
        let mut meta = BTreeMap::new();
        meta.insert("active".to_string(), Value::String("py".to_string()));
        assert!(is_active(".py", &meta, true));
        assert!(!is_active(".Rmd", &meta, true));
    }
}
