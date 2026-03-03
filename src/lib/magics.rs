//! Jupyter magic command handling

use once_cell::sync::Lazy;
use regex::Regex;
use std::collections::HashMap;

use crate::languages::{usual_language_name, COMMENT_FOR_LANG, SCRIPT_EXTENSIONS};
use crate::string_parser::StringParser;

/// Regex for magic commands by language
static MAGIC_RE: Lazy<HashMap<String, Regex>> = Lazy::new(|| {
    let mut m = HashMap::new();
    for sl in SCRIPT_EXTENSIONS.values() {
        let comment = regex::escape(sl.comment);
        let pattern = format!(r"^\s*({0} |{0})*(%|%%|%%%)[a-zA-Z]", comment);
        if let Ok(re) = Regex::new(&pattern) {
            m.insert(sl.language.to_string(), re);
        }
    }
    // Rust magics start with ':'
    m.insert(
        "rust".to_string(),
        Regex::new(r"^(// |//)*:[a-zA-Z]").unwrap(),
    );
    // C# magics start with '#!'
    m.insert(
        "csharp".to_string(),
        Regex::new(r"^(// |//)*#![a-zA-Z]").unwrap(),
    );
    // Go magics
    m.insert(
        "go".to_string(),
        Regex::new(r"^(// |//)*(!|!\*|%|%%|%%%)[a-zA-Z]").unwrap(),
    );
    m
});

static MAGIC_FORCE_ESC_RE: Lazy<HashMap<String, Regex>> = Lazy::new(|| {
    let mut m = HashMap::new();
    for sl in SCRIPT_EXTENSIONS.values() {
        let comment = regex::escape(sl.comment);
        let pattern = format!(r"^\s*({0} |{0})*(%|%%|%%%)[a-zA-Z](.*){0}\s*escape", comment);
        if let Ok(re) = Regex::new(&pattern) {
            m.insert(sl.language.to_string(), re);
        }
    }
    m
});

static MAGIC_NOT_ESC_RE: Lazy<HashMap<String, Regex>> = Lazy::new(|| {
    let mut m = HashMap::new();
    for sl in SCRIPT_EXTENSIONS.values() {
        let comment = regex::escape(sl.comment);
        let pattern = format!(
            r"^\s*({0} |{0})*(%|%%|%%%)[a-zA-Z](.*){0}\s*noescape",
            comment
        );
        if let Ok(re) = Regex::new(&pattern) {
            m.insert(sl.language.to_string(), re);
        }
    }
    m
});

static LINE_CONTINUATION_RE: Lazy<Regex> = Lazy::new(|| Regex::new(r".*\\\s*$").unwrap());

static PYTHON_HELP_OR_BASH_CMD: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"^\s*(# |#)*\s*(\?|!)\s*[A-Za-z\.\~\$\\/\{\}]").unwrap());

static PYTHON_MAGIC_CMD: Lazy<Regex> = Lazy::new(|| {
    let cmds = [
        "cat", "cd", "cp", "mv", "rm", "rmdir", "mkdir", "copy", "ddir", "echo", "ls", "ldir",
        "mkdir", "ren", "rmdir",
    ];
    let cmds_str = cmds.join("|");
    Regex::new(&format!(r"^(# |#)*({})(|\s|$|\s[^=,])", cmds_str)).unwrap()
});

static IPYTHON_MAGIC_HELP: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"^\s*(# )*[^\s]*\?\s*$").unwrap());

static PYTHON_MAGIC_ASSIGN: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"^(# |#)*\s*([a-zA-Z_][a-zA-Z_$0-9]*)\s*=\s*(%|%%|%%%|!)[a-zA-Z](.*)").unwrap()
});

/// Is the line a (possibly escaped) Jupyter magic that should be commented?
pub fn is_magic(line: &str, language: &str, global_escape_flag: bool, explicitly_code: bool) -> bool {
    let language = usual_language_name(language);
    if matches!(
        language.as_str(),
        "octave" | "matlab" | "SAS" | "logtalk"
    ) {
        return false;
    }

    // Check if language has magic regexes
    if let Some(force_esc_re) = MAGIC_FORCE_ESC_RE.get(&language) {
        if force_esc_re.is_match(line) {
            return true;
        }
    }

    if !global_escape_flag {
        return false;
    }

    if let Some(not_esc_re) = MAGIC_NOT_ESC_RE.get(&language) {
        if not_esc_re.is_match(line) {
            return false;
        }
    }

    if let Some(magic_re) = MAGIC_RE.get(&language) {
        if magic_re.is_match(line) {
            return true;
        }
    } else {
        return false;
    }

    if language != "python" {
        return false;
    }

    if PYTHON_HELP_OR_BASH_CMD.is_match(line) {
        return true;
    }
    if PYTHON_MAGIC_ASSIGN.is_match(line) {
        return true;
    }
    if explicitly_code && IPYTHON_MAGIC_HELP.is_match(line) {
        return true;
    }
    PYTHON_MAGIC_CMD.is_match(line)
}

/// Does this code need an explicit cell marker?
pub fn need_explicit_marker(
    source: &[String],
    language: &str,
    global_escape_flag: bool,
) -> bool {
    if language != "python" || !global_escape_flag {
        return false;
    }

    let mut parser = StringParser::new(language);
    for line in source {
        if !parser.is_quoted() && is_magic(line, language, global_escape_flag, true) {
            if !is_magic(line, language, global_escape_flag, false) {
                return true;
            }
        }
        parser.read_line(line);
    }
    false
}

/// Comment magic commands with the language-specific comment prefix
pub fn comment_magic(
    source: &mut Vec<String>,
    language: &str,
    global_escape_flag: bool,
    explicitly_code: bool,
) {
    let comment = COMMENT_FOR_LANG
        .get(usual_language_name(language).as_str())
        .copied()
        .unwrap_or("#");

    let mut parser = StringParser::new(language);
    let mut next_is_magic = false;

    for pos in 0..source.len() {
        let line = source[pos].clone();
        if !parser.is_quoted() && (next_is_magic || is_magic(&line, language, global_escape_flag, explicitly_code))
        {
            let (indent, unindented) = if next_is_magic {
                (String::new(), line.clone())
            } else {
                let trimmed = line.trim_start();
                let indent_len = line.len() - trimmed.len();
                (line[..indent_len].to_string(), trimmed.to_string())
            };
            source[pos] = format!("{}{} {}", indent, comment, unindented);
            next_is_magic = language == "python" && LINE_CONTINUATION_RE.is_match(&line);
        }
        parser.read_line(&line);
    }
}

/// Uncomment a single line
fn unesc(line: &str, language: &str) -> String {
    let comment = COMMENT_FOR_LANG
        .get(usual_language_name(language).as_str())
        .copied()
        .unwrap_or("#");

    let trimmed = line.trim_start();
    let indent_len = line.len() - trimmed.len();
    let indent = &line[..indent_len];
    let comment_space = format!("{} ", comment);

    if trimmed.starts_with(&comment_space) {
        format!("{}{}", indent, &trimmed[comment_space.len()..])
    } else if trimmed.starts_with(comment) {
        format!("{}{}", indent, &trimmed[comment.len()..])
    } else {
        line.to_string()
    }
}

/// Uncomment magic commands
pub fn uncomment_magic(
    source: &mut Vec<String>,
    language: &str,
    global_escape_flag: bool,
    explicitly_code: bool,
) {
    let mut parser = StringParser::new(language);
    let mut next_is_magic = false;

    for pos in 0..source.len() {
        let line = source[pos].clone();
        if !parser.is_quoted()
            && (next_is_magic || is_magic(&line, language, global_escape_flag, explicitly_code))
        {
            source[pos] = unesc(&line, language);
            next_is_magic = language == "python" && LINE_CONTINUATION_RE.is_match(&line);
        }
        parser.read_line(&line);
    }
}

/// Escaped code start patterns by extension
pub fn is_escaped_code_start(line: &str, ext: &str) -> bool {
    match ext {
        ".Rmd" => {
            let re = Regex::new(r"^(# |#)*```\{.*\}").unwrap();
            re.is_match(line)
        }
        ".md" | ".markdown" => {
            let re = Regex::new(r"^(# |#)*```").unwrap();
            re.is_match(line)
        }
        _ => {
            if let Some(sl) = SCRIPT_EXTENSIONS.get(ext) {
                let comment = regex::escape(sl.comment);
                let pattern = format!(r"^({0} |{0})*({0}|{0} )\+", comment);
                if let Ok(re) = Regex::new(&pattern) {
                    return re.is_match(line);
                }
            }
            false
        }
    }
}

/// Escape code start markers
pub fn escape_code_start(source: &mut Vec<String>, ext: &str, language: &str) {
    let comment = SCRIPT_EXTENSIONS
        .get(ext)
        .map(|sl| sl.comment)
        .unwrap_or("#");

    let mut parser = StringParser::new(language);
    for pos in 0..source.len() {
        let line = source[pos].clone();
        if !parser.is_quoted() && is_escaped_code_start(&line, ext) {
            source[pos] = format!("{} {}", comment, line);
        }
        parser.read_line(&line);
    }
}

/// Unescape code start markers
pub fn unescape_code_start(source: &mut Vec<String>, ext: &str, language: &str) {
    let mut parser = StringParser::new(language);
    for pos in 0..source.len() {
        let line = source[pos].clone();
        if !parser.is_quoted() && is_escaped_code_start(&line, ext) {
            let unescaped = unesc(&line, language);
            if is_escaped_code_start(&unescaped, ext) {
                source[pos] = unescaped;
            }
        }
        parser.read_line(&line);
    }
}
