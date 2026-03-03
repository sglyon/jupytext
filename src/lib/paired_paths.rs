//! Compute paired notebook paths from a base path and format specifications.
//!
//! Given a notebook file and its Jupytext format metadata, this module can:
//! - Compute the base path (stripping prefix, suffix, and extension)
//! - Reconstruct the full path for any paired format
//! - List all paired paths for a notebook
//!
//! The prefix/suffix/extension model follows the Python jupytext implementation:
//! - **prefix**: directory path component and/or filename prefix, separated by `///`
//!   for prefix roots (e.g. `notebooks///py:percent`)
//! - **suffix**: string appended to the base name before the extension
//! - **extension**: the file extension (e.g. `.py`, `.ipynb`)

use std::collections::BTreeMap;
use std::path::Path;

use anyhow::Result;

use crate::config::find_jupytext_configuration_file;
use crate::formats::{short_form_one_format_str, NOTEBOOK_EXTENSIONS};

/// Error type for inconsistent paths.
#[derive(Debug, thiserror::Error)]
pub enum InconsistentPath {
    #[error("'{0}' is not a notebook. Supported extensions are: {1}")]
    NonNotebookExtension(String, String),

    #[error("Notebook path '{0}' was expected to have extension '{1}'")]
    InconsistentExtension(String, String),

    #[error("Notebook name '{0}' was expected to end with suffix '{1}'")]
    InconsistentSuffix(String, String),

    #[error("Notebook name '{0}' was expected to start with prefix '{1}'")]
    InconsistentPrefix(String, String),

    #[error("Notebook directory '{0}' does not match prefix '{1}'")]
    InconsistentPrefixDirectory(String, String),

    #[error("Notebook directory '{0}' does not match prefix root '{1}'")]
    InconsistentPrefixRoot(String, String),

    #[error("Paired paths do not include current notebook path '{0}'. Current format is '{1}', paired formats are '{2}'.")]
    CurrentPathNotInPairs(String, String, String),

    #[error("Duplicate paired paths for this notebook. Please fix jupytext.formats.")]
    DuplicatePairedPaths,

    #[error("Path '{0}' matches none of the export formats: {1}. Please make sure that jupytext.formats covers the current file.")]
    NoMatchingFormat(String, String),

    #[error("Notebook base name '{0}' is not compatible with format {1}. Make sure you use prefix roots in either none, or all of the paired formats.")]
    IncompatiblePrefixRoot(String, String),
}

/// Split a string at the last occurrence of `sep`, returning `("", s)` if sep is not found.
fn split_last(s: &str, sep: char) -> (&str, &str) {
    match s.rfind(sep) {
        Some(pos) => (&s[..pos], &s[pos + 1..]),
        None => ("", s),
    }
}

/// Join left and right with sep, omitting sep when left is empty.
fn join_parts(left: &str, right: &str, sep: char) -> String {
    if left.is_empty() {
        right.to_string()
    } else {
        format!("{}{}{}", left, sep, right)
    }
}

/// Return the path separator to use. Always '/' for cross-platform consistency,
/// except when the path already contains backslashes (Windows).
fn separator(path: &str) -> char {
    if std::path::MAIN_SEPARATOR == '\\' && path.contains('\\') {
        '\\'
    } else {
        '/'
    }
}

/// Decompose a prefix string into `(prefix_root, prefix_dir, prefix_file_name)`.
///
/// The prefix root is separated from the rest by `//`.
/// The prefix dir and prefix file name are separated by `/`.
fn decompose_prefix(prefix: &str) -> (String, String, String) {
    let (prefix_root, rest) = if let Some(pos) = prefix.rfind("//") {
        (prefix[..pos].to_string(), &prefix[pos + 2..])
    } else {
        (String::new(), prefix)
    };
    let (prefix_dir, prefix_file_name) = split_last(rest, '/');
    (
        prefix_root,
        prefix_dir.to_string(),
        prefix_file_name.to_string(),
    )
}

/// Reconstruct a prefix string from its three components.
#[allow(dead_code)]
fn compose_prefix(prefix_root: &str, prefix_dir: &str, prefix_file_name: &str) -> String {
    if !prefix_root.is_empty() {
        format!("{}//{}/{}", prefix_root, prefix_dir, prefix_file_name)
    } else if prefix_dir.is_empty() && prefix_file_name.is_empty() {
        String::new()
    } else {
        format!("{}/{}", prefix_dir, prefix_file_name)
    }
}

/// Compute the base path from a notebook path and its format specification.
///
/// The base path has the extension, suffix, and prefix stripped so that other
/// paired paths can be reconstructed with `full_path`.
///
/// If `formats` is provided, the matching format's prefix/suffix information
/// is used to extend `fmt`.
pub fn base_path(
    main_path: &str,
    fmt: &BTreeMap<String, String>,
    formats: &[BTreeMap<String, String>],
) -> Result<String> {
    let mut fmt = long_form_one_format_from_map(fmt);

    // Split into base name and extension
    let (base_name, ext) = split_extension(main_path);

    // If no extension in fmt, use the file's extension
    if !fmt.contains_key("extension") {
        if NOTEBOOK_EXTENSIONS.contains(&ext.as_str()) {
            fmt.insert("extension".to_string(), ext.clone());
        } else {
            return Err(InconsistentPath::NonNotebookExtension(
                main_path.to_string(),
                NOTEBOOK_EXTENSIONS.join("', '"),
            )
            .into());
        }
    }

    // Check extension matches
    let fmt_ext = fmt.get("extension").cloned().unwrap_or_default();
    if ext != fmt_ext {
        return Err(InconsistentPath::InconsistentExtension(
            main_path.to_string(),
            fmt_ext,
        )
        .into());
    }

    // Find a matching format in the list to inherit prefix/suffix
    let formats_to_check = if formats.is_empty() {
        vec![fmt.clone()]
    } else {
        formats.to_vec()
    };

    for f in &formats_to_check {
        let f_ext = f.get("extension").cloned().unwrap_or_default();
        if f_ext != fmt_ext {
            continue;
        }
        if let (Some(fn1), Some(fn2)) = (fmt.get("format_name"), f.get("format_name")) {
            if fn1 != fn2 {
                continue;
            }
        }
        // Extend fmt with prefix/suffix from the matching format
        for (key, value) in f {
            if !fmt.contains_key(key) {
                fmt.insert(key.clone(), value.clone());
            }
        }
        break;
    }

    let mut base = base_name.to_string();

    // Strip suffix
    let suffix = fmt.get("suffix").cloned().unwrap_or_default();
    if !suffix.is_empty() {
        if !base.ends_with(&suffix) {
            return Err(
                InconsistentPath::InconsistentSuffix(base.clone(), suffix).into()
            );
        }
        base = base[..base.len() - suffix.len()].to_string();
    }

    // Strip prefix
    let prefix = fmt.get("prefix").cloned().unwrap_or_default();
    if prefix.is_empty() {
        return Ok(base);
    }

    let (prefix_root, prefix_dir, prefix_file_name) = decompose_prefix(&prefix);
    let sep = separator(&base);
    let (mut notebook_dir, mut notebook_file_name) = split_at_sep(&base, sep);

    // Check for config file-based base directory
    let mut base_dir: Option<String> = None;
    if !notebook_dir.is_empty() {
        if let Some(config_file) = find_jupytext_configuration_file(Path::new(&notebook_dir)) {
            if let Some(config_dir) = config_file.parent().and_then(|p| p.to_str()) {
                if notebook_dir.starts_with(config_dir) {
                    base_dir = Some(config_dir.to_string());
                    notebook_dir = notebook_dir[config_dir.len()..].to_string();
                }
            }
        }
    }

    // Strip prefix_file_name from notebook_file_name
    if !prefix_file_name.is_empty() {
        if !notebook_file_name.starts_with(&prefix_file_name) {
            return Err(InconsistentPath::InconsistentPrefix(
                notebook_file_name,
                prefix_file_name,
            )
            .into());
        }
        notebook_file_name = notebook_file_name[prefix_file_name.len()..].to_string();
    }

    // Strip prefix_dir from notebook_dir
    if !prefix_dir.is_empty() {
        let mut parent_notebook_dir = notebook_dir.clone();
        let mut parent_prefix_dir = prefix_dir.clone();
        let mut actual_folders: Vec<String> = Vec::new();

        while !parent_prefix_dir.is_empty() {
            let ppd_clone = parent_prefix_dir.clone();
            let (rest, expected_folder) = split_last(&ppd_clone, '/');
            parent_prefix_dir = rest.to_string();

            if expected_folder == ".." {
                if actual_folders.is_empty() {
                    return Err(InconsistentPath::InconsistentPrefixDirectory(
                        notebook_dir.clone(),
                        prefix_dir.clone(),
                    )
                    .into());
                }
                let folder = actual_folders.pop().unwrap();
                parent_notebook_dir = join_parts(&parent_notebook_dir, &folder, sep);
            } else {
                let (rest_dir, actual_folder) = split_at_sep(&parent_notebook_dir, sep);
                actual_folders.push(actual_folder.clone());

                if actual_folder != expected_folder {
                    return Err(InconsistentPath::InconsistentPrefixDirectory(
                        notebook_dir.clone(),
                        prefix_dir.clone(),
                    )
                    .into());
                }
                parent_notebook_dir = rest_dir;
            }
        }
        notebook_dir = parent_notebook_dir;
    }

    // Strip prefix_root from notebook_dir
    if !prefix_root.is_empty() {
        let long_prefix_root = format!(
            "{}{}{}",
            sep,
            prefix_root.replace('/', &sep.to_string()),
            sep
        );
        let long_notebook_dir = format!("{}{}{}", sep, notebook_dir, sep);

        if !long_notebook_dir.contains(&long_prefix_root) {
            return Err(InconsistentPath::InconsistentPrefixRoot(
                notebook_dir.clone(),
                prefix_root,
            )
            .into());
        }

        let pos = long_notebook_dir
            .rfind(&long_prefix_root)
            .unwrap();
        let left = &long_notebook_dir[..pos];
        let right = &long_notebook_dir[pos + long_prefix_root.len()..];
        notebook_dir = format!("{}{}//{}",left, sep, right);

        // Trim leading/trailing separator
        let sep_str = sep.to_string();
        if !right.is_empty() {
            // nothing extra
        } else {
            // We need to remember the sep
        }
        if notebook_dir.starts_with(&sep_str) {
            notebook_dir = notebook_dir[1..].to_string();
        }
        if notebook_dir.ends_with(&sep_str) {
            notebook_dir = notebook_dir[..notebook_dir.len() - 1].to_string();
        }
    }

    // Prepend base_dir
    if let Some(bd) = base_dir {
        notebook_dir = format!("{}{}", bd, notebook_dir);
    }

    if notebook_dir.is_empty() {
        Ok(notebook_file_name)
    } else {
        Ok(format!("{}{}{}", notebook_dir, sep, notebook_file_name))
    }
}

/// Compute the full path from a base path and a format specification.
///
/// This is the inverse of `base_path`: it reconstructs the notebook file path
/// by applying the prefix, suffix, and extension from the format.
pub fn full_path(base: &str, fmt: &BTreeMap<String, String>) -> Result<String> {
    let ext = fmt
        .get("extension")
        .cloned()
        .unwrap_or_else(|| ".ipynb".to_string());
    let suffix = fmt.get("suffix").cloned().unwrap_or_default();
    let prefix = fmt.get("prefix").cloned().unwrap_or_default();

    let mut full = base.to_string();

    if !prefix.is_empty() {
        let (prefix_root, prefix_rest) = if let Some(pos) = prefix.rfind("//") {
            (prefix[..pos].to_string(), prefix[pos + 2..].to_string())
        } else {
            (String::new(), prefix.clone())
        };

        let (prefix_dir, prefix_file_name) = split_last(&prefix_rest, '/');

        let sep = separator(base);
        let sep_str = sep.to_string();
        let prefix_dir = prefix_dir.replace('/', &sep_str);

        // Check prefix_root consistency
        let base_has_double_slash = base.contains("//");
        if (!prefix_root.is_empty()) != base_has_double_slash {
            return Err(InconsistentPath::IncompatiblePrefixRoot(
                base.to_string(),
                short_form_one_format_str(fmt),
            )
            .into());
        }

        let (mut notebook_dir, mut notebook_file_name) = if !prefix_root.is_empty() {
            let (left, right) = base.rsplit_once("//").unwrap();
            let (right_dir, file_name) = split_at_sep(right, sep);
            let dir = format!(
                "{}{}{}{}",
                left,
                prefix_root,
                sep,
                right_dir
            );
            (dir, file_name)
        } else {
            let (d, f) = split_at_sep(base, sep);
            (d, f)
        };

        // Prepend prefix_file_name
        if !prefix_file_name.is_empty() {
            notebook_file_name = format!("{}{}", prefix_file_name, notebook_file_name);
        }

        // Append prefix_dir (handling ".." components)
        if !prefix_dir.is_empty() {
            let dotdot = format!("..{}", sep);
            let mut pd = prefix_dir.clone();
            while pd.starts_with(&dotdot) {
                pd = pd[dotdot.len()..].to_string();
                let (parent, _) = split_at_sep(&notebook_dir, sep);
                notebook_dir = parent;
            }

            if !notebook_dir.is_empty() && !notebook_dir.ends_with(sep) {
                notebook_dir.push(sep);
            }
            notebook_dir.push_str(&pd);
        }

        if !notebook_dir.is_empty() && !notebook_dir.ends_with(sep) {
            notebook_dir.push(sep);
        }

        full = format!("{}{}", notebook_dir, notebook_file_name);
    }

    if !suffix.is_empty() {
        full.push_str(&suffix);
    }

    full.push_str(&ext);
    Ok(full)
}

/// Return the list of all paired paths for a notebook.
///
/// Given the `main_path` (the notebook file), the current format `fmt`,
/// and the list of all paired `formats`, returns a list of
/// `(path, format_dict)` pairs.
pub fn paired_paths(
    main_path: &str,
    fmt: &BTreeMap<String, String>,
    formats: &[BTreeMap<String, String>],
) -> Result<Vec<(String, BTreeMap<String, String>)>> {
    if formats.is_empty() {
        let ext = split_extension(main_path).1;
        let mut single_fmt = BTreeMap::new();
        single_fmt.insert("extension".to_string(), ext);
        return Ok(vec![(main_path.to_string(), single_fmt)]);
    }

    let base = base_path(main_path, fmt, formats)?;

    let mut paths = Vec::new();
    for f in formats {
        paths.push((full_path(&base, f)?, f.clone()));
    }

    // Verify that main_path is in the paired paths
    let path_strings: Vec<&str> = paths.iter().map(|(p, _)| p.as_str()).collect();
    if !path_strings.contains(&main_path) {
        let fmt_short = short_form_one_format_str(fmt);
        let formats_short: Vec<String> = formats.iter().map(|f| short_form_one_format_str(f)).collect();
        return Err(InconsistentPath::CurrentPathNotInPairs(
            main_path.to_string(),
            fmt_short,
            formats_short.join(","),
        )
        .into());
    }

    // Check for duplicates
    let unique_paths: std::collections::HashSet<&str> =
        path_strings.iter().copied().collect();
    if unique_paths.len() < path_strings.len() {
        return Err(InconsistentPath::DuplicatePairedPaths.into());
    }

    Ok(paths)
}

/// Find the base path and matching format from a list of formats.
///
/// Tries each format until one produces a valid base path for the given
/// `main_path`. Returns the base path and the matching format.
pub fn find_base_path_and_format(
    main_path: &str,
    formats: &[BTreeMap<String, String>],
) -> Result<(String, BTreeMap<String, String>)> {
    for fmt in formats {
        match base_path(main_path, fmt, formats) {
            Ok(bp) => return Ok((bp, fmt.clone())),
            Err(_) => continue,
        }
    }

    let ext = split_extension(main_path).1;
    let _ext_no_dot = ext.trim_start_matches('.');
    let formats_debug: Vec<String> = formats.iter().map(|f| format!("{:?}", f)).collect();
    Err(InconsistentPath::NoMatchingFormat(
        main_path.to_string(),
        formats_debug.join(", "),
    )
    .into())
}

// ---- Helper functions ----

/// Split a path into (base_without_extension, extension_with_dot).
fn split_extension(path: &str) -> (String, String) {
    let p = Path::new(path);
    let ext = p
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| format!(".{}", e))
        .unwrap_or_default();
    let base = if ext.is_empty() {
        path.to_string()
    } else {
        path[..path.len() - ext.len()].to_string()
    };
    (base, ext)
}

/// Split at the last occurrence of sep, returning (directory, filename).
/// If sep is not found, returns ("", path).
fn split_at_sep(path: &str, sep: char) -> (String, String) {
    match path.rfind(sep) {
        Some(pos) => (path[..pos].to_string(), path[pos + 1..].to_string()),
        None => (String::new(), path.to_string()),
    }
}

/// Normalize a format map by ensuring it has at least an "extension" key.
fn long_form_one_format_from_map(fmt: &BTreeMap<String, String>) -> BTreeMap<String, String> {
    fmt.clone()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fmt(ext: &str) -> BTreeMap<String, String> {
        let mut m = BTreeMap::new();
        m.insert("extension".to_string(), ext.to_string());
        m
    }

    fn fmt_with_suffix(ext: &str, suffix: &str) -> BTreeMap<String, String> {
        let mut m = fmt(ext);
        m.insert("suffix".to_string(), suffix.to_string());
        m
    }

    fn fmt_with_prefix(ext: &str, prefix: &str) -> BTreeMap<String, String> {
        let mut m = fmt(ext);
        m.insert("prefix".to_string(), prefix.to_string());
        m
    }

    #[test]
    fn test_base_path_simple() {
        let f = fmt(".py");
        let result = base_path("notebook.py", &f, &[]).unwrap();
        assert_eq!(result, "notebook");
    }

    #[test]
    fn test_base_path_ipynb() {
        let f = fmt(".ipynb");
        let result = base_path("test/notebook.ipynb", &f, &[]).unwrap();
        assert_eq!(result, "test/notebook");
    }

    #[test]
    fn test_base_path_with_suffix() {
        let f = fmt_with_suffix(".py", ".nb");
        let result = base_path("notebook.nb.py", &f, &[]).unwrap();
        assert_eq!(result, "notebook");
    }

    #[test]
    fn test_base_path_inconsistent_suffix() {
        let f = fmt_with_suffix(".py", ".nb");
        let result = base_path("notebook.py", &f, &[]);
        assert!(result.is_err());
    }

    #[test]
    fn test_base_path_wrong_extension() {
        let f = fmt(".py");
        let result = base_path("notebook.ipynb", &f, &[]);
        assert!(result.is_err());
    }

    #[test]
    fn test_full_path_simple() {
        let f = fmt(".py");
        let result = full_path("notebook", &f).unwrap();
        assert_eq!(result, "notebook.py");
    }

    #[test]
    fn test_full_path_with_suffix() {
        let f = fmt_with_suffix(".py", ".nb");
        let result = full_path("notebook", &f).unwrap();
        assert_eq!(result, "notebook.nb.py");
    }

    #[test]
    fn test_full_path_with_dir() {
        let f = fmt(".ipynb");
        let result = full_path("path/to/notebook", &f).unwrap();
        assert_eq!(result, "path/to/notebook.ipynb");
    }

    #[test]
    fn test_full_path_with_prefix_dir() {
        let f = fmt_with_prefix(".py", "scripts/");
        let result = full_path("path/notebook", &f).unwrap();
        assert_eq!(result, "path/scripts/notebook.py");
    }

    #[test]
    fn test_paired_paths_no_formats() {
        let f = fmt(".py");
        let result = paired_paths("notebook.py", &f, &[]).unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].0, "notebook.py");
    }

    #[test]
    fn test_paired_paths_ipynb_and_py() {
        let ipynb = fmt(".ipynb");
        let py = fmt(".py");
        let formats = vec![ipynb.clone(), py.clone()];

        let result = paired_paths("notebook.ipynb", &ipynb, &formats).unwrap();
        assert_eq!(result.len(), 2);
        assert_eq!(result[0].0, "notebook.ipynb");
        assert_eq!(result[1].0, "notebook.py");
    }

    #[test]
    fn test_split_extension() {
        assert_eq!(
            split_extension("notebook.ipynb"),
            ("notebook".to_string(), ".ipynb".to_string())
        );
        assert_eq!(
            split_extension("path/to/file.py"),
            ("path/to/file".to_string(), ".py".to_string())
        );
        assert_eq!(
            split_extension("noext"),
            ("noext".to_string(), String::new())
        );
    }

    #[test]
    fn test_decompose_prefix() {
        assert_eq!(
            decompose_prefix("notebooks//scripts/test_"),
            (
                "notebooks".to_string(),
                "scripts".to_string(),
                "test_".to_string()
            )
        );
        assert_eq!(
            decompose_prefix("scripts/"),
            (String::new(), "scripts".to_string(), String::new())
        );
        assert_eq!(
            decompose_prefix("test_"),
            (String::new(), String::new(), "test_".to_string())
        );
        assert_eq!(
            decompose_prefix(""),
            (String::new(), String::new(), String::new())
        );
    }

    #[test]
    fn test_roundtrip_base_full() {
        let formats = vec![fmt(".ipynb"), fmt(".py")];
        let bp = base_path("notebook.ipynb", &fmt(".ipynb"), &formats).unwrap();
        assert_eq!(bp, "notebook");
        let fp = full_path(&bp, &fmt(".py")).unwrap();
        assert_eq!(fp, "notebook.py");
    }
}
