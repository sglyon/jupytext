//! Command-line interface for jupytext (Rust implementation)
//!
//! This module implements the full CLI matching the Python jupytext CLI flags.

use std::collections::BTreeMap;
use std::fs;
use std::io::{self, Read as IoRead, Write as IoWrite};
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{bail, Context, Result};
use clap::Parser;
use serde_json::Value;

use crate::combine::combine_inputs_with_outputs;
use crate::compare::compare_notebooks;
#[allow(unused_imports)]
use crate::compare::NotebookDifference;
use crate::config::{load_jupytext_config, JupytextConfig};
#[allow(unused_imports)]
use crate::formats::{
    long_form_multiple_formats_as_strings, long_form_one_format_as_strings,
    NOTEBOOK_EXTENSIONS,
};
#[allow(unused_imports)]
use crate::formats::short_form_one_format_str;
use crate::jupytext::{reads_notebook, write_notebook, writes_notebook};
#[allow(unused_imports)]
use crate::jupytext::read_notebook;
use crate::notebook::{Notebook, reads_ipynb, writes_ipynb};
use crate::paired_paths::paired_paths;
#[allow(unused_imports)]
use crate::paired_paths::{base_path, full_path};

/// The jupytext version string.
const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Convert Jupyter notebooks to/from text formats.
///
/// Jupytext reads and writes Jupyter notebooks as plain text files
/// in many formats: Markdown, R Markdown, Python scripts, and more.
#[derive(Parser, Debug)]
#[command(name = "jupytext", version = VERSION, about, long_about = None)]
pub struct Cli {
    /// One or more notebook files (reads stdin if empty).
    #[arg(value_name = "NOTEBOOKS")]
    pub notebooks: Vec<PathBuf>,

    /// Input format (e.g. py:percent, md, ipynb).
    #[arg(long = "from", value_name = "FORMAT")]
    pub from_fmt: Option<String>,

    /// Output format (e.g. py:percent, md, ipynb).
    #[arg(long = "to", value_name = "FORMAT")]
    pub to_fmt: Option<String>,

    /// Output file (use '-' for stdout).
    #[arg(short = 'o', long = "output", value_name = "FILE")]
    pub output: Option<String>,

    /// Set paired formats in notebook metadata.
    #[arg(long = "set-formats", value_name = "FMT")]
    pub set_formats: Option<String>,

    /// Format options as key=value pairs (may be repeated).
    #[arg(long = "format-options", alias = "opt", value_name = "KEY=VALUE")]
    pub format_options: Vec<String>,

    /// Synchronize all paired representations.
    #[arg(short = 's', long = "sync")]
    pub sync: bool,

    /// List paired file paths and exit.
    #[arg(short = 'p', long = "paired-paths")]
    pub paired_paths: bool,

    /// Round-trip conversion test.
    #[arg(long = "test")]
    pub test: bool,

    /// Strict round-trip conversion test.
    #[arg(long = "test-strict")]
    pub test_strict: bool,

    /// Show differences between notebook and text file.
    #[arg(short = 'd', long = "diff")]
    pub diff: bool,

    /// Execute the notebook after conversion.
    #[arg(long = "execute")]
    pub execute: bool,

    /// Set the kernel (language) for the notebook.
    #[arg(short = 'k', long = "set-kernel", value_name = "KERNEL")]
    pub set_kernel: Option<String>,

    /// Pipe notebook to an external program.
    #[arg(long = "pipe", value_name = "CMD")]
    pub pipe: Option<String>,

    /// Run external program as a check (non-zero exit = error).
    #[arg(long = "check", value_name = "CMD")]
    pub check: Option<String>,

    /// Preserve outputs when updating an existing .ipynb file.
    #[arg(long = "update")]
    pub update: bool,

    /// Update notebook metadata with JSON string.
    #[arg(long = "update-metadata", value_name = "JSON")]
    pub update_metadata: Option<String>,

    /// Set the output file timestamp to match the source file.
    #[arg(long = "use-source-timestamp")]
    pub use_source_timestamp: bool,

    /// Format to use when piping (--pipe or --check).
    #[arg(long = "pipe-fmt", value_name = "FMT")]
    pub pipe_fmt: Option<String>,

    /// Format for diff output.
    #[arg(long = "diff-format", value_name = "FMT")]
    pub diff_format: Option<String>,

    /// Working directory for notebook execution.
    #[arg(long = "run-path", value_name = "PATH")]
    pub run_path: Option<PathBuf>,

    /// Enable pre-commit mode (affects timestamp logic).
    #[arg(long = "pre-commit-mode")]
    pub pre_commit_mode: bool,

    /// Check that the source file is newer than the output file.
    #[arg(long = "check-source-is-newer")]
    pub check_source_is_newer: bool,

    /// Suppress informational messages.
    #[arg(short = 'q', long = "quiet")]
    pub quiet: bool,

    /// Show a diff for each file that is modified.
    #[arg(long = "show-changes")]
    pub show_changes: bool,

    /// Continue processing on errors (warn only).
    #[arg(short = 'w', long = "warn-only")]
    pub warn_only: bool,

    /// Stop on first error.
    #[arg(short = 'x', long = "stop")]
    pub stop: bool,
}

/// Parsed format options from --opt / --format-options flags.
fn parse_format_options(opts: &[String]) -> BTreeMap<String, Value> {
    let mut map = BTreeMap::new();
    for opt in opts {
        if let Some(pos) = opt.find('=') {
            let key = opt[..pos].trim().to_string();
            let val_str = opt[pos + 1..].trim();
            let value = match val_str {
                "true" | "True" => Value::Bool(true),
                "false" | "False" => Value::Bool(false),
                _ => {
                    if let Ok(n) = val_str.parse::<i64>() {
                        Value::Number(serde_json::Number::from(n))
                    } else {
                        Value::String(val_str.to_string())
                    }
                }
            };
            map.insert(key, value);
        }
    }
    map
}

/// Read text from stdin.
fn read_stdin() -> Result<String> {
    let mut buf = String::new();
    io::stdin()
        .read_to_string(&mut buf)
        .context("Failed to read from stdin")?;
    Ok(buf)
}

/// Determine the input format from a file path.
fn input_format_from_path(path: &Path) -> Option<BTreeMap<String, String>> {
    let ext = path.extension()?.to_str()?;
    let full_ext = format!(".{}", ext);
    let mut fmt = BTreeMap::new();
    fmt.insert("extension".to_string(), full_ext);
    Some(fmt)
}

/// Convert a string format spec into a BTreeMap format dict.
fn format_spec_to_dict(spec: &str) -> BTreeMap<String, String> {
    long_form_one_format_as_strings(spec)
}

/// Run an external command with notebook text piped to its stdin.
fn pipe_notebook(cmd: &str, text: &str) -> Result<String> {
    let output = if cfg!(target_os = "windows") {
        Command::new("cmd")
            .args(["/C", cmd])
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
    } else {
        Command::new("sh")
            .args(["-c", cmd])
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
    };

    let mut child = output.context("Failed to spawn pipe command")?;
    if let Some(ref mut stdin) = child.stdin {
        stdin
            .write_all(text.as_bytes())
            .context("Failed to write to pipe stdin")?;
    }
    let result = child
        .wait_with_output()
        .context("Failed to wait for pipe command")?;
    if !result.status.success() {
        let stderr = String::from_utf8_lossy(&result.stderr);
        bail!(
            "Command '{}' exited with status {}: {}",
            cmd,
            result.status,
            stderr
        );
    }
    Ok(String::from_utf8_lossy(&result.stdout).to_string())
}

/// Log a message unless --quiet is set.
fn log_message(cli: &Cli, msg: &str) {
    if !cli.quiet {
        eprintln!("[jupytext] {}", msg);
    }
}

/// Execute a single notebook file according to the CLI flags.
fn process_notebook(cli: &Cli, nb_path: Option<&Path>) -> Result<()> {
    let format_options = parse_format_options(&cli.format_options);

    // ---- Determine input format ----
    let from_fmt = if let Some(ref fmt_str) = cli.from_fmt {
        format_spec_to_dict(fmt_str)
    } else if let Some(path) = nb_path {
        input_format_from_path(path).unwrap_or_default()
    } else {
        BTreeMap::new()
    };

    // ---- Read the notebook ----
    let nb = if let Some(path) = nb_path {
        log_message(cli, &format!("Reading {}", path.display()));
        let text =
            fs::read_to_string(path).with_context(|| format!("Cannot read {}", path.display()))?;
        let ext = from_fmt
            .get("extension")
            .cloned()
            .unwrap_or_else(|| ".ipynb".to_string());
        if ext == ".ipynb" {
            reads_ipynb(&text)
                .with_context(|| format!("Cannot parse {} as ipynb", path.display()))?
        } else {
            reads_notebook(&text, &from_fmt)?
        }
    } else {
        // Read from stdin
        let text = read_stdin()?;
        let ext = from_fmt
            .get("extension")
            .cloned()
            .unwrap_or_else(|| ".ipynb".to_string());
        if ext == ".ipynb" {
            reads_ipynb(&text).context("Cannot parse stdin as ipynb")?
        } else {
            reads_notebook(&text, &from_fmt)?
        }
    };

    // ---- Load configuration ----
    let config = if let Some(path) = nb_path {
        load_jupytext_config(path).ok().flatten()
    } else {
        None
    };

    // ---- Handle --paired-paths ----
    if cli.paired_paths {
        return handle_paired_paths(cli, nb_path, &nb, &config);
    }

    // ---- Handle --set-formats ----
    let mut nb = nb;
    if let Some(ref fmt_str) = cli.set_formats {
        set_formats_metadata(&mut nb, fmt_str);
        log_message(cli, &format!("Setting formats to '{}'", fmt_str));
    }

    // ---- Handle --update-metadata ----
    if let Some(ref json_str) = cli.update_metadata {
        update_notebook_metadata(&mut nb, json_str)?;
    }

    // ---- Handle --set-kernel ----
    if let Some(ref kernel) = cli.set_kernel {
        set_kernel_metadata(&mut nb, kernel);
        log_message(cli, &format!("Setting kernel to '{}'", kernel));
    }

    // ---- Handle --sync ----
    if cli.sync {
        return handle_sync(cli, nb_path, &mut nb, &config);
    }

    // ---- Handle --test / --test-strict ----
    if cli.test || cli.test_strict {
        return handle_test(cli, nb_path, &nb, &from_fmt);
    }

    // ---- Handle --diff ----
    if cli.diff {
        return handle_diff(cli, nb_path, &nb, &from_fmt);
    }

    // ---- Handle --pipe ----
    if let Some(ref pipe_cmd) = cli.pipe {
        return handle_pipe(cli, nb_path, &mut nb, pipe_cmd, &from_fmt, &format_options);
    }

    // ---- Handle --check ----
    if let Some(ref check_cmd) = cli.check {
        return handle_check(cli, nb_path, &nb, check_cmd, &from_fmt, &format_options);
    }

    // ---- Handle conversion (--to / --output) ----
    if cli.to_fmt.is_some() || cli.output.is_some() {
        return handle_conversion(cli, nb_path, &nb, &from_fmt, &format_options, &config);
    }

    // ---- If --set-formats or --set-kernel or --update-metadata was the only action, write back ----
    if cli.set_formats.is_some() || cli.set_kernel.is_some() || cli.update_metadata.is_some() {
        if let Some(path) = nb_path {
            let ext = path
                .extension()
                .and_then(|e| e.to_str())
                .unwrap_or("ipynb");
            if ext == "ipynb" {
                let text = writes_ipynb(&nb)?;
                fs::write(path, text)?;
            } else {
                write_notebook(&nb, path, &from_fmt)?;
            }
            log_message(cli, &format!("Wrote {}", path.display()));
        }
        return Ok(());
    }

    // ---- No action specified: show help-like message ----
    if nb_path.is_none() && cli.notebooks.is_empty() {
        bail!("No notebook specified. Use --help for usage information.");
    }

    Ok(())
}

/// Handle the --paired-paths flag: list all paired paths for the notebook.
fn handle_paired_paths(
    cli: &Cli,
    nb_path: Option<&Path>,
    nb: &Notebook,
    _config: &Option<JupytextConfig>,
) -> Result<()> {
    let path = nb_path.ok_or_else(|| anyhow::anyhow!("--paired-paths requires a file path"))?;
    let path_str = path.to_string_lossy().to_string();

    // Get formats from notebook metadata
    let formats_str = nb
        .metadata
        .get("jupytext")
        .and_then(|j| j.get("formats"))
        .and_then(|v| v.as_str())
        .unwrap_or("");

    if formats_str.is_empty() {
        println!("{}", path.display());
        return Ok(());
    }

    let formats = long_form_multiple_formats_as_strings(formats_str);
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| format!(".{}", e))
        .unwrap_or_default();
    let mut current_fmt = BTreeMap::new();
    current_fmt.insert("extension".to_string(), ext);

    match paired_paths(&path_str, &current_fmt, &formats) {
        Ok(pairs) => {
            for (p, _fmt) in &pairs {
                println!("{}", p);
            }
        }
        Err(e) => {
            if !cli.warn_only {
                return Err(e);
            }
            eprintln!("[jupytext] Warning: {}", e);
        }
    }

    Ok(())
}

/// Set formats in the notebook jupytext metadata.
fn set_formats_metadata(nb: &mut Notebook, formats_str: &str) {
    let jupytext = nb
        .metadata
        .entry("jupytext".to_string())
        .or_insert_with(|| Value::Object(serde_json::Map::new()));
    if let Value::Object(ref mut map) = jupytext {
        map.insert(
            "formats".to_string(),
            Value::String(formats_str.to_string()),
        );
    }
}

/// Update notebook metadata from a JSON string.
fn update_notebook_metadata(nb: &mut Notebook, json_str: &str) -> Result<()> {
    let update_val: Value =
        serde_json::from_str(json_str).context("--update-metadata requires valid JSON")?;
    if let Value::Object(update_map) = update_val {
        for (key, value) in update_map {
            if value.is_null() {
                nb.metadata.remove(&key);
            } else {
                nb.metadata.insert(key, value);
            }
        }
    } else {
        bail!("--update-metadata value must be a JSON object");
    }
    Ok(())
}

/// Set kernel metadata on the notebook.
fn set_kernel_metadata(nb: &mut Notebook, kernel: &str) {
    let kernelspec = nb
        .metadata
        .entry("kernelspec".to_string())
        .or_insert_with(|| Value::Object(serde_json::Map::new()));
    if let Value::Object(ref mut map) = kernelspec {
        // Simple kernel specification: set name and language
        map.insert("name".to_string(), Value::String(kernel.to_string()));
        map.insert("language".to_string(), Value::String(kernel.to_string()));
        map.insert(
            "display_name".to_string(),
            Value::String(kernel.to_string()),
        );
    }
}

/// Handle --sync: synchronize all paired representations.
fn handle_sync(
    cli: &Cli,
    nb_path: Option<&Path>,
    nb: &mut Notebook,
    config: &Option<JupytextConfig>,
) -> Result<()> {
    let path = nb_path.ok_or_else(|| anyhow::anyhow!("--sync requires a file path"))?;
    let path_str = path.to_string_lossy().to_string();

    // Get formats from notebook or config
    let formats_str = nb
        .metadata
        .get("jupytext")
        .and_then(|j| j.get("formats"))
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .or_else(|| config.as_ref().and_then(|c| c.formats.clone()))
        .unwrap_or_default();

    if formats_str.is_empty() {
        log_message(cli, "No paired formats found. Nothing to sync.");
        return Ok(());
    }

    let formats = long_form_multiple_formats_as_strings(&formats_str);
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| format!(".{}", e))
        .unwrap_or_default();
    let mut current_fmt = BTreeMap::new();
    current_fmt.insert("extension".to_string(), ext.clone());

    let pairs = paired_paths(&path_str, &current_fmt, &formats)?;

    // Find the most recent input file
    let mut newest_path: Option<(PathBuf, std::time::SystemTime)> = None;
    for (p, _fmt) in &pairs {
        let pb = PathBuf::from(p);
        if pb.exists() {
            if let Ok(meta) = fs::metadata(&pb) {
                if let Ok(modified) = meta.modified() {
                    if newest_path
                        .as_ref()
                        .map_or(true, |(_, t)| modified > *t)
                    {
                        newest_path = Some((pb, modified));
                    }
                }
            }
        }
    }

    let source_path = newest_path
        .map(|(p, _)| p)
        .unwrap_or_else(|| path.to_path_buf());

    // Read the source notebook
    let source_ext = source_path
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| format!(".{}", e))
        .unwrap_or_default();
    let source_text = fs::read_to_string(&source_path)?;
    let mut source_fmt = BTreeMap::new();
    source_fmt.insert("extension".to_string(), source_ext.clone());

    let source_nb = if source_ext == ".ipynb" {
        reads_ipynb(&source_text)?
    } else {
        reads_notebook(&source_text, &source_fmt)?
    };

    // Write to each paired format
    for (p, fmt) in &pairs {
        let target_path = PathBuf::from(p);
        let target_ext = fmt
            .get("extension")
            .cloned()
            .unwrap_or_else(|| ".ipynb".to_string());

        // If the target exists and is an ipynb, combine outputs
        let nb_to_write = if target_ext == ".ipynb" && target_path.exists() && source_ext != ".ipynb"
        {
            let existing_text = fs::read_to_string(&target_path)?;
            let existing_nb = reads_ipynb(&existing_text)?;
            combine_inputs_with_outputs(&source_nb, &existing_nb, Some(fmt))
        } else {
            source_nb.clone()
        };

        if target_ext == ".ipynb" {
            let text = writes_ipynb(&nb_to_write)?;
            fs::write(&target_path, text)?;
        } else {
            write_notebook(&nb_to_write, &target_path, fmt)?;
        }
        log_message(cli, &format!("Wrote {}", target_path.display()));
    }

    Ok(())
}

/// Handle --test / --test-strict: round-trip conversion test.
fn handle_test(
    cli: &Cli,
    nb_path: Option<&Path>,
    nb: &Notebook,
    from_fmt: &BTreeMap<String, String>,
) -> Result<()> {
    let path = nb_path.ok_or_else(|| anyhow::anyhow!("--test requires a file path"))?;
    log_message(cli, &format!("Testing round-trip for {}", path.display()));

    let to_fmt = if let Some(ref fmt_str) = cli.to_fmt {
        format_spec_to_dict(fmt_str)
    } else {
        from_fmt.clone()
    };

    // Write to text
    let text = writes_notebook(nb, &to_fmt)?;
    // Read back
    let round_trip = reads_notebook(&text, &to_fmt)?;

    // Combine outputs if updating
    let round_trip = if cli.update {
        combine_inputs_with_outputs(&round_trip, nb, Some(&to_fmt))
    } else {
        round_trip
    };

    let allow_expected_differences = !cli.test_strict;
    match compare_notebooks(
        &round_trip,
        nb,
        Some(&to_fmt),
        allow_expected_differences,
        cli.stop,
    ) {
        Ok(()) => {
            log_message(cli, &format!("Round-trip test passed for {}", path.display()));
            Ok(())
        }
        Err(e) => {
            bail!(
                "Round-trip test failed for {}: {}",
                path.display(),
                e
            );
        }
    }
}

/// Handle --diff: show differences between two representations.
fn handle_diff(
    cli: &Cli,
    nb_path: Option<&Path>,
    nb: &Notebook,
    from_fmt: &BTreeMap<String, String>,
) -> Result<()> {
    let _path = nb_path.ok_or_else(|| anyhow::anyhow!("--diff requires a file path"))?;

    let to_fmt = if let Some(ref fmt_str) = cli.to_fmt {
        format_spec_to_dict(fmt_str)
    } else if let Some(ref fmt_str) = cli.diff_format {
        format_spec_to_dict(fmt_str)
    } else {
        from_fmt.clone()
    };

    let text = writes_notebook(nb, &to_fmt)?;

    // If output file exists, diff against it
    if let Some(ref output_path) = cli.output {
        let output_pb = PathBuf::from(output_path);
        if output_pb.exists() {
            let existing = fs::read_to_string(&output_pb)?;
            let diff = diff_strings(&existing, &text, &output_pb.to_string_lossy(), "new");
            if diff.is_empty() {
                log_message(cli, "Files are identical.");
            } else {
                println!("{}", diff);
            }
            return Ok(());
        }
    }

    // Otherwise just output the text representation
    println!("{}", text);
    Ok(())
}

/// Handle --pipe: pipe notebook text through an external command.
fn handle_pipe(
    cli: &Cli,
    nb_path: Option<&Path>,
    nb: &mut Notebook,
    pipe_cmd: &str,
    from_fmt: &BTreeMap<String, String>,
    _format_options: &BTreeMap<String, Value>,
) -> Result<()> {
    let pipe_fmt = if let Some(ref fmt_str) = cli.pipe_fmt {
        format_spec_to_dict(fmt_str)
    } else {
        from_fmt.clone()
    };

    let text = writes_notebook(nb, &pipe_fmt)?;
    let cmd = pipe_cmd.replace("{}", &text);
    let piped = pipe_notebook(&cmd, &text)?;

    // Read the piped output back as a notebook
    let piped_nb = reads_notebook(&piped, &pipe_fmt)?;

    // Combine with original to preserve outputs
    let combined = combine_inputs_with_outputs(&piped_nb, nb, Some(&pipe_fmt));
    *nb = combined;

    // Write back
    if let Some(path) = nb_path {
        let ext = path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("ipynb");
        if ext == "ipynb" {
            let out = writes_ipynb(nb)?;
            fs::write(path, out)?;
        } else {
            write_notebook(nb, path, from_fmt)?;
        }
        log_message(cli, &format!("Wrote {}", path.display()));
    } else {
        let out = writes_ipynb(nb)?;
        print!("{}", out);
    }

    Ok(())
}

/// Handle --check: check notebook with an external command.
fn handle_check(
    cli: &Cli,
    nb_path: Option<&Path>,
    nb: &Notebook,
    check_cmd: &str,
    from_fmt: &BTreeMap<String, String>,
    _format_options: &BTreeMap<String, Value>,
) -> Result<()> {
    let check_fmt = if let Some(ref fmt_str) = cli.pipe_fmt {
        format_spec_to_dict(fmt_str)
    } else {
        from_fmt.clone()
    };

    let text = writes_notebook(nb, &check_fmt)?;
    let cmd = check_cmd.replace("{}", &text);
    pipe_notebook(&cmd, &text)?;

    let path_display = nb_path
        .map(|p| p.display().to_string())
        .unwrap_or_else(|| "<stdin>".to_string());
    log_message(cli, &format!("Check passed for {}", path_display));
    Ok(())
}

/// Handle --to / --output: convert notebook to a different format.
fn handle_conversion(
    cli: &Cli,
    nb_path: Option<&Path>,
    nb: &Notebook,
    _from_fmt: &BTreeMap<String, String>,
    format_options: &BTreeMap<String, Value>,
    _config: &Option<JupytextConfig>,
) -> Result<()> {
    let to_fmt = if let Some(ref fmt_str) = cli.to_fmt {
        let mut fmt = format_spec_to_dict(fmt_str);
        // Apply format options
        for (k, v) in format_options {
            if let Value::String(s) = v {
                fmt.insert(k.clone(), s.clone());
            }
        }
        fmt
    } else if let Some(ref output) = cli.output {
        // Infer format from output extension
        let pb = PathBuf::from(output);
        input_format_from_path(&pb).unwrap_or_default()
    } else {
        bail!("Either --to or --output must be specified for conversion");
    };

    let to_ext = to_fmt
        .get("extension")
        .cloned()
        .unwrap_or_else(|| ".ipynb".to_string());

    // Determine output path
    let output_path: Option<PathBuf> = if let Some(ref output) = cli.output {
        if output == "-" {
            None // stdout
        } else {
            Some(PathBuf::from(output))
        }
    } else if let Some(path) = nb_path {
        // Replace extension
        let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("notebook");
        let parent = path.parent().unwrap_or_else(|| Path::new("."));
        let suffix = to_fmt.get("suffix").cloned().unwrap_or_default();
        let new_name = format!("{}{}{}", stem, suffix, to_ext);
        Some(parent.join(new_name))
    } else {
        None // stdout
    };

    // If updating an existing ipynb, combine outputs
    let nb_to_write = if cli.update && to_ext == ".ipynb" {
        if let Some(ref out_path) = output_path {
            if out_path.exists() {
                let existing_text = fs::read_to_string(out_path)?;
                let existing_nb = reads_ipynb(&existing_text)?;
                combine_inputs_with_outputs(nb, &existing_nb, Some(&to_fmt))
            } else {
                nb.clone()
            }
        } else {
            nb.clone()
        }
    } else {
        nb.clone()
    };

    // Write output
    if let Some(ref out_path) = output_path {
        // Create parent directories if needed
        if let Some(parent) = out_path.parent() {
            if !parent.exists() {
                fs::create_dir_all(parent)?;
            }
        }

        if to_ext == ".ipynb" {
            let text = writes_ipynb(&nb_to_write)?;
            fs::write(out_path, text)?;
        } else {
            write_notebook(&nb_to_write, out_path, &to_fmt)?;
        }
        log_message(cli, &format!("Wrote {}", out_path.display()));
    } else {
        // Write to stdout
        if to_ext == ".ipynb" {
            let text = writes_ipynb(&nb_to_write)?;
            print!("{}", text);
        } else {
            let text = writes_notebook(&nb_to_write, &to_fmt)?;
            print!("{}", text);
        }
    }

    Ok(())
}

/// Simple unified diff of two strings.
fn diff_strings(a: &str, b: &str, a_name: &str, b_name: &str) -> String {
    let a_lines: Vec<&str> = a.lines().collect();
    let b_lines: Vec<&str> = b.lines().collect();

    if a_lines == b_lines {
        return String::new();
    }

    let mut output = Vec::new();
    output.push(format!("--- {}", a_name));
    output.push(format!("+++ {}", b_name));

    // Simple line-by-line comparison
    let max_len = a_lines.len().max(b_lines.len());
    let mut i = 0;
    while i < max_len {
        let a_line = a_lines.get(i).copied().unwrap_or("");
        let b_line = b_lines.get(i).copied().unwrap_or("");

        if a_line != b_line {
            if i < a_lines.len() {
                output.push(format!("-{}", a_line));
            }
            if i < b_lines.len() {
                output.push(format!("+{}", b_line));
            }
        } else {
            output.push(format!(" {}", a_line));
        }
        i += 1;
    }

    output.join("\n")
}

/// Entry point: parse CLI arguments and run the appropriate action.
pub fn run_cli() -> Result<()> {
    let cli = Cli::parse();

    if cli.notebooks.is_empty() {
        // Read from stdin
        process_notebook(&cli, None)?;
    } else {
        let mut errors = Vec::new();
        for nb_path in &cli.notebooks {
            match process_notebook(&cli, Some(nb_path)) {
                Ok(()) => {}
                Err(e) => {
                    if cli.stop {
                        return Err(e);
                    }
                    if cli.warn_only {
                        eprintln!("[jupytext] Warning: {}: {}", nb_path.display(), e);
                    } else {
                        errors.push((nb_path.clone(), e));
                    }
                }
            }
        }
        if !errors.is_empty() {
            let msgs: Vec<String> = errors
                .iter()
                .map(|(p, e)| format!("  {}: {}", p.display(), e))
                .collect();
            bail!("Errors processing notebooks:\n{}", msgs.join("\n"));
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_format_options_empty() {
        let opts: Vec<String> = vec![];
        let result = parse_format_options(&opts);
        assert!(result.is_empty());
    }

    #[test]
    fn test_parse_format_options_key_value() {
        let opts = vec![
            "comment_magics=true".to_string(),
            "cell_markers=region,endregion".to_string(),
            "notebook_metadata_filter=-all".to_string(),
        ];
        let result = parse_format_options(&opts);
        assert_eq!(result.get("comment_magics"), Some(&Value::Bool(true)));
        assert_eq!(
            result.get("cell_markers"),
            Some(&Value::String("region,endregion".to_string()))
        );
        assert_eq!(
            result.get("notebook_metadata_filter"),
            Some(&Value::String("-all".to_string()))
        );
    }

    #[test]
    fn test_diff_strings_identical() {
        let diff = diff_strings("a\nb\nc\n", "a\nb\nc\n", "file1", "file2");
        assert!(diff.is_empty());
    }

    #[test]
    fn test_diff_strings_different() {
        let diff = diff_strings("a\nb\nc", "a\nB\nc", "file1", "file2");
        assert!(!diff.is_empty());
        assert!(diff.contains("-b"));
        assert!(diff.contains("+B"));
    }
}
