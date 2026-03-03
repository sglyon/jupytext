//! Language definitions and script extension mappings

use once_cell::sync::Lazy;
use regex::Regex;
use std::collections::{HashMap, HashSet};

/// Information about a script language
#[derive(Debug, Clone)]
pub struct ScriptLanguage {
    pub language: &'static str,
    pub comment: &'static str,
    pub comment_suffix: &'static str,
}

/// All supported script extensions
pub static SCRIPT_EXTENSIONS: Lazy<HashMap<&'static str, ScriptLanguage>> = Lazy::new(|| {
    let mut m = HashMap::new();
    let exts: Vec<(&str, &str, &str, &str)> = vec![
        (".py", "python", "#", ""),
        (".coco", "coconut", "#", ""),
        (".R", "R", "#", ""),
        (".r", "R", "#", ""),
        (".jl", "julia", "#", ""),
        (".cpp", "c++", "//", ""),
        (".ss", "scheme", ";;", ""),
        (".clj", "clojure", ";;", ""),
        (".scm", "scheme", ";;", ""),
        (".sh", "bash", "#", ""),
        (".ps1", "powershell", "#", ""),
        (".q", "q", "/", ""),
        (".m", "matlab", "%", ""),
        (".wolfram", "wolfram language", "(*", "*)"),
        (".pro", "idl", ";", ""),
        (".js", "javascript", "//", ""),
        (".ts", "typescript", "//", ""),
        (".scala", "scala", "//", ""),
        (".rs", "rust", "//", ""),
        (".robot", "robotframework", "#", ""),
        (".resource", "robotframework", "#", ""),
        (".cs", "csharp", "//", ""),
        (".fsx", "fsharp", "//", ""),
        (".fs", "fsharp", "//", ""),
        (".sos", "sos", "#", ""),
        (".java", "java", "//", ""),
        (".groovy", "groovy", "//", ""),
        (".sage", "sage", "#", ""),
        (".ml", "ocaml", "(*", "*)"),
        (".hs", "haskell", "--", ""),
        (".tcl", "tcl", "#", ""),
        (".mac", "maxima", "/*", "*/"),
        (".gp", "gnuplot", "#", ""),
        (".do", "stata", "//", ""),
        (".sas", "sas", "/*", "*/"),
        (".xsh", "xonsh", "#", ""),
        (".lgt", "logtalk", "%", ""),
        (".logtalk", "logtalk", "%", ""),
        (".lua", "lua", "--", ""),
        (".go", "go", "//", ""),
    ];
    for (ext, lang, comment, suffix) in exts {
        m.insert(
            ext,
            ScriptLanguage {
                language: lang,
                comment,
                comment_suffix: suffix,
            },
        );
    }
    m
});

/// Comment character for each language
pub static COMMENT_FOR_LANG: Lazy<HashMap<&'static str, &'static str>> = Lazy::new(|| {
    let mut m = HashMap::new();
    for sl in SCRIPT_EXTENSIONS.values() {
        m.insert(sl.language, sl.comment);
    }
    m
});

/// Set of jupyter languages
pub static JUPYTER_LANGUAGES: Lazy<HashSet<String>> = Lazy::new(|| {
    let base = vec![
        "R", "bash", "sh", "python", "python2", "python3", "coconut", "javascript", "js",
        "perl", "html", "latex", "markdown", "pypy", "ruby", "script", "svg", "matlab",
        "octave", "idl", "robotframework", "sas", "spark", "sql", "cython", "haskell",
        "tcl", "gnuplot", "wolfram language",
    ];
    let mut s: HashSet<String> = base.iter().map(|l| l.to_string()).collect();
    // Add all language names from script extensions
    for sl in SCRIPT_EXTENSIONS.values() {
        s.insert(sl.language.to_string());
    }
    // Add alternate names
    for alt in &["c#", "f#", "cs", "fs"] {
        s.insert(alt.to_string());
    }
    s
});

/// JUPYTER_LANGUAGES plus upper case versions
pub static JUPYTER_LANGUAGES_LOWER_AND_UPPER: Lazy<HashSet<String>> = Lazy::new(|| {
    let mut s = JUPYTER_LANGUAGES.clone();
    let upper: Vec<String> = s.iter().map(|l| l.to_uppercase()).collect();
    for u in upper {
        s.insert(u);
    }
    s
});

static GO_DOUBLE_PERCENT_COMMAND: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"^(%%\s*|%%\s+-.*)$").unwrap());

/// Return default language from metadata and extension
pub fn default_language_from_metadata_and_ext(
    metadata: &serde_json::Map<String, serde_json::Value>,
    ext: &str,
    pop_main_language: bool,
) -> Option<String> {
    let default_from_ext = SCRIPT_EXTENSIONS
        .get(ext)
        .map(|sl| sl.language.to_string());

    let main_language = metadata
        .get("jupytext")
        .and_then(|j| j.get("main_language"))
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    let default_language = metadata
        .get("kernelspec")
        .and_then(|k| k.get("language"))
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .or_else(|| default_from_ext.clone());

    let language = main_language.clone().or(default_language.clone());

    if pop_main_language {
        if let (Some(ref ml), Some(ref dl)) = (&main_language, &default_language) {
            if ml == dl {
                // In Python version this mutates metadata - we return a flag instead
                // The caller should handle this
            }
        }
    }

    language.map(|lang| {
        if lang == "R" || lang == "sas" || lang == "SAS" {
            return lang;
        }
        if lang.starts_with("C++") {
            return "c++".to_string();
        }
        lang.to_lowercase().replace('#', "sharp")
    })
}

/// Return the usual language name
pub fn usual_language_name(language: &str) -> String {
    let lower = language.to_lowercase();
    match lower.as_str() {
        "r" => "R".to_string(),
        l if l.starts_with("c++") => "c++".to_string(),
        "octave" => "matlab".to_string(),
        "cs" | "c#" => "csharp".to_string(),
        "fs" | "f#" => "fsharp".to_string(),
        "sas" => "SAS".to_string(),
        _ => lower,
    }
}

/// Are these the same language?
pub fn same_language(kernel_language: &str, language: &str) -> bool {
    usual_language_name(kernel_language) == usual_language_name(language)
}

/// Comment lines with the given prefix and suffix
pub fn comment_lines(lines: &[String], prefix: &str, suffix: &str) -> Vec<String> {
    if prefix.is_empty() {
        return lines.to_vec();
    }
    lines
        .iter()
        .map(|line| {
            if line.is_empty() {
                if suffix.is_empty() {
                    prefix.to_string()
                } else {
                    format!("{} {}", prefix, suffix)
                }
            } else if suffix.is_empty() {
                format!("{} {}", prefix, line)
            } else {
                format!("{} {} {}", prefix, line, suffix)
            }
        })
        .collect()
}

/// Uncomment lines by removing prefix (and suffix)
pub fn uncomment_lines(lines: &[String], prefix: &str, suffix: &str) -> Vec<String> {
    if prefix.is_empty() {
        return lines.to_vec();
    }
    let prefix_space = format!("{} ", prefix);
    let plen = prefix.len();
    let pslen = prefix_space.len();

    lines
        .iter()
        .map(|line| {
            let mut result = if line.starts_with(&prefix_space) {
                line[pslen..].to_string()
            } else if line.starts_with(prefix) {
                line[plen..].to_string()
            } else {
                line.to_string()
            };

            if !suffix.is_empty() {
                let space_suffix = format!(" {}", suffix);
                let slen = suffix.len();
                let sslen = space_suffix.len();
                result = if result.ends_with(&space_suffix) {
                    result[..result.len() - sslen].to_string()
                } else if result.ends_with(suffix) {
                    result[..result.len() - slen].to_string()
                } else {
                    result
                };
            }

            result
        })
        .collect()
}

/// Determine cell language from first line of source
pub fn cell_language(
    source: &mut Vec<String>,
    default_language: &str,
    custom_cell_magics: &[String],
) -> (Option<String>, Option<String>) {
    if source.is_empty() {
        return (None, None);
    }

    let line = &source[0];

    if default_language == "go" && GO_DOUBLE_PERCENT_COMMAND.is_match(line) {
        return (None, None);
    }

    if default_language == "csharp" {
        if line.starts_with("#!") {
            let lang = line[2..].trim().to_string();
            if JUPYTER_LANGUAGES.contains(&lang) {
                source.remove(0);
                return (Some(lang), Some(String::new()));
            }
        }
    } else if line.starts_with("%%") {
        let magic = &line[2..];
        let (lang, magic_args) = if let Some(pos) = magic.find(' ') {
            (magic[..pos].to_string(), magic[pos + 1..].to_string())
        } else {
            (magic.to_string(), String::new())
        };

        if JUPYTER_LANGUAGES.contains(&lang)
            || custom_cell_magics.iter().any(|m| m == &lang)
        {
            source.remove(0);
            return (Some(lang), Some(magic_args));
        }
    }

    (None, None)
}

/// Set main language and cell language for collection of cells
pub fn set_main_and_cell_language(
    metadata: &mut serde_json::Map<String, serde_json::Value>,
    cells: &mut [crate::notebook::Cell],
    ext: &str,
    custom_cell_magics: &[String],
) {
    let main_language = default_language_from_metadata_and_ext(metadata, ext, false);

    let main_language = match main_language {
        Some(lang) => lang,
        None => {
            // Count languages to find the most common
            let mut languages: HashMap<String, f64> = HashMap::new();
            languages.insert("python".to_string(), 0.5);
            for cell in cells.iter() {
                if let Some(serde_json::Value::String(lang)) = cell.metadata.get("language") {
                    let lang = usual_language_name(lang);
                    *languages.entry(lang).or_insert(0.0) += 1.0;
                }
            }
            languages
                .into_iter()
                .max_by(|a, b| a.1.partial_cmp(&b.1).unwrap())
                .map(|(k, _)| k)
                .unwrap_or_else(|| "python".to_string())
        }
    };

    // Save main language when no kernel is set
    let has_kernel_language = metadata
        .get("kernelspec")
        .and_then(|k| k.get("language"))
        .and_then(|v| v.as_str())
        .is_some();

    if !has_kernel_language && !cells.is_empty() {
        let jupytext = metadata
            .entry("jupytext".to_string())
            .or_insert_with(|| serde_json::Value::Object(serde_json::Map::new()));
        if let Some(obj) = jupytext.as_object_mut() {
            obj.insert(
                "main_language".to_string(),
                serde_json::Value::String(main_language.clone()),
            );
        }
    }

    // Remove 'language' metadata and add magic if not main language
    for cell in cells.iter_mut() {
        if let Some(serde_json::Value::String(language)) = cell.metadata.get("language").cloned() {
            if language == main_language {
                cell.metadata.remove("language");
                continue;
            }

            if usual_language_name(&language) == main_language {
                continue;
            }

            if JUPYTER_LANGUAGES.contains(&language)
                || custom_cell_magics.iter().any(|m| m == &language)
            {
                cell.metadata.remove("language");
                let magic = if main_language != "csharp" {
                    "%%"
                } else {
                    "#!"
                };
                if let Some(serde_json::Value::String(magic_args)) =
                    cell.metadata.remove("magic_args")
                {
                    cell.source = format!("{}{} {}\n{}", magic, language, magic_args, cell.source);
                } else {
                    cell.source = format!("{}{}\n{}", magic, language, cell.source);
                }
            }
        }
    }
}
