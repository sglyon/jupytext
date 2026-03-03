///! Comprehensive unit tests for jupytext-rs
///!
///! These tests are translations of the Python unit tests from the original
///! jupytext project, adapted to use the Rust API.

use std::collections::BTreeMap;

use serde_json::{json, Value};

use jupytext::cell_metadata::{
    is_active, is_json_metadata, metadata_to_text, parse_rmd_options, rmd_options_to_metadata,
    text_to_metadata,
};
use jupytext::compare::{compare, compare_notebooks};
use jupytext::formats::{
    divine_format, get_format_implementation, guess_format, long_form_multiple_formats,
    long_form_one_format, short_form_multiple_formats, short_form_one_format,
    validate_one_format,
};
use jupytext::header::{header_to_metadata_and_cell, metadata_and_cell_to_header, recursive_update};
use jupytext::magics::{comment_magic, is_magic, uncomment_magic};
use jupytext::metadata_filter::{
    filter_metadata, metadata_filter_as_dict, metadata_filter_as_string, FilterSpec,
    DEFAULT_NOTEBOOK_METADATA,
};
use jupytext::notebook::{Cell, CellType, Notebook};
use jupytext::pep8::pep8_lines_between_cells;
use jupytext::string_parser::StringParser;

// =========================================================================
// 1. StringParser tests
// =========================================================================

mod string_parser_tests {
    use super::*;

    #[test]
    fn test_long_string() {
        // Python: '''multiline string''' should mark lines 1-6 as quoted
        let text = "'''This is a multiline\n\
                     comment with \"quotes\", 'single quotes'\n\
                     # and comments\n\
                     and line breaks\n\
                     \n\
                     \n\
                     and it ends here'''\n\
                     \n\
                     \n\
                     1 + 1";
        let lines: Vec<&str> = text.lines().collect();
        let mut quoted = Vec::new();
        let mut sp = StringParser::new("python");
        for (i, line) in lines.iter().enumerate() {
            if sp.is_quoted() {
                quoted.push(i);
            }
            sp.read_line(line);
        }
        assert_eq!(quoted, vec![1, 2, 3, 4, 5, 6]);
    }

    #[test]
    fn test_single_chars() {
        // Single-line strings should not mark any line as quoted
        let text = "'This is a single line comment'''\n\
                     'and another one'\n\
                     # and comments\n\
                     \"and line breaks\"\n\
                     \n\
                     \n\
                     \"and it ends here'''\"\n\
                     \n\
                     \n\
                     1 + 1";
        let lines: Vec<&str> = text.lines().collect();
        let mut sp = StringParser::new("python");
        for line in &lines {
            assert!(!sp.is_quoted());
            sp.read_line(line);
        }
    }

    #[test]
    fn test_long_string_with_four_quotes() {
        let text = "''''This is a multiline\n\
                     comment that starts with four quotes\n\
                     '''\n\
                     \n\
                     1 + 1";
        let lines: Vec<&str> = text.lines().collect();
        let mut quoted = Vec::new();
        let mut sp = StringParser::new("python");
        for (i, line) in lines.iter().enumerate() {
            if sp.is_quoted() {
                quoted.push(i);
            }
            sp.read_line(line);
        }
        assert_eq!(quoted, vec![1, 2]);
    }

    #[test]
    fn test_long_string_ends_with_four_quotes() {
        let text = "'''This is a multiline\n\
                     comment that ends with four quotes\n\
                     ''''\n\
                     \n\
                     1 + 1";
        let lines: Vec<&str> = text.lines().collect();
        let mut quoted = Vec::new();
        let mut sp = StringParser::new("python");
        for (i, line) in lines.iter().enumerate() {
            if sp.is_quoted() {
                quoted.push(i);
            }
            sp.read_line(line);
        }
        assert_eq!(quoted, vec![1, 2]);
    }

    #[test]
    fn test_comment_line_not_quoted() {
        let mut sp = StringParser::new("python");
        sp.read_line("# x = 'hello");
        assert!(!sp.is_quoted());
    }

    #[test]
    fn test_simple_string_not_quoted() {
        let mut sp = StringParser::new("python");
        sp.read_line("x = 'hello'");
        assert!(!sp.is_quoted());
    }

    #[test]
    fn test_triple_quote_multiline() {
        let mut sp = StringParser::new("python");
        sp.read_line("x = \"\"\"hello");
        assert!(sp.is_quoted());
        sp.read_line("world\"\"\"");
        assert!(!sp.is_quoted());
    }

    #[test]
    fn test_r_parser() {
        let mut sp = StringParser::new("R");
        sp.read_line("x <- 'hello'");
        assert!(!sp.is_quoted());
    }

    #[test]
    fn test_none_language() {
        let sp = StringParser::new_opt(None);
        // With None language, is_quoted always returns false
        assert!(!sp.is_quoted());
    }
}

// =========================================================================
// 2. Magics tests
// =========================================================================

mod magics_tests {
    use super::*;

    #[test]
    fn test_escape_percent_magic() {
        let cases = vec![
            "%matplotlib inline",
            "%%HTML",
            "%autoreload",
            "%store",
        ];
        for line in cases {
            let mut source = vec![line.to_string()];
            comment_magic(&mut source, "python", true, false);
            assert_eq!(source, vec![format!("# {}", line)], "Failed on: {}", line);

            uncomment_magic(&mut source, "python", true, false);
            assert_eq!(source, vec![line.to_string()], "Roundtrip failed for: {}", line);
        }
    }

    #[test]
    fn test_escape_already_commented_magic() {
        let cases = vec![
            "#%matplotlib inline",
            "##%matplotlib inline",
        ];
        for line in cases {
            let mut source = vec![line.to_string()];
            comment_magic(&mut source, "python", true, false);
            assert_eq!(source, vec![format!("# {}", line)]);

            uncomment_magic(&mut source, "python", true, false);
            assert_eq!(source, vec![line.to_string()]);
        }
    }

    #[test]
    fn test_escape_non_magic() {
        // @pytest.fixture should NOT be commented
        let line = "@pytest.fixture";
        let mut source = vec![line.to_string()];
        comment_magic(&mut source, "python", true, false);
        assert_eq!(source, vec![line.to_string()]);
    }

    #[test]
    fn test_force_noescape() {
        // %matplotlib inline #noescape should NOT be escaped
        let line = "%matplotlib inline #noescape";
        let mut source = vec![line.to_string()];
        comment_magic(&mut source, "python", true, false);
        assert_eq!(source, vec![line.to_string()]);
    }

    #[test]
    fn test_force_noescape_with_global_flag() {
        let line = "%matplotlib inline #noescape";
        let mut source = vec![line.to_string()];
        comment_magic(&mut source, "python", true, false);
        assert_eq!(source, vec![line.to_string()]);
    }

    #[test]
    fn test_force_escape_with_global_flag_false() {
        // With global_escape_flag=false, but #escape tag forces it
        let line = "%matplotlib inline #escape";
        let mut source = vec![line.to_string()];
        comment_magic(&mut source, "python", false, false);
        assert_eq!(source, vec![format!("# {}", line)]);
    }

    #[test]
    fn test_is_magic_percent() {
        assert!(is_magic("%matplotlib inline", "python", true, false));
        assert!(is_magic("%%HTML", "python", true, false));
        assert!(is_magic("%autoreload", "python", true, false));
    }

    #[test]
    fn test_comment_bash_commands_in_python() {
        let magic_cmds = vec![
            "ls", "!ls", "ls -al", "!whoami", "# ls", "# mv a b",
            "! mkdir tmp", "!./script", "! ./script",
            "cat", "cat ", "cat hello.txt",
        ];
        for cmd in magic_cmds {
            let mut source = vec![cmd.to_string()];
            comment_magic(&mut source, "python", true, false);
            assert_eq!(source, vec![format!("# {}", cmd)], "Comment failed for: {}", cmd);

            let mut source2 = vec![format!("# {}", cmd)];
            uncomment_magic(&mut source2, "python", true, false);
            assert_eq!(source2, vec![cmd.to_string()], "Uncomment failed for: {}", cmd);
        }
    }

    #[test]
    fn test_do_not_comment_python_assignment() {
        // Regular Python statements that do not look like magic commands at
        // all (no leading %, !, ?, and no command-like word at the start).
        let not_magic = vec![
            "x = 42",
            "result = foo()",
            "my_cat = 5",
        ];
        for cmd in not_magic {
            let mut source = vec![cmd.to_string()];
            comment_magic(&mut source, "python", true, false);
            assert_eq!(source, vec![cmd.to_string()], "Should not comment: {}", cmd);

            uncomment_magic(&mut source, "python", true, false);
            assert_eq!(source, vec![cmd.to_string()], "Should not change: {}", cmd);
        }
    }

    #[test]
    fn test_non_python_lines_not_commented() {
        // Lines that don't start with known magic patterns should not be commented
        let not_magic = vec![
            "x = 42",
            "def foo():",
            "import os",
            "class MyClass:",
            "for i in range(10):",
        ];
        for cmd in not_magic {
            let mut source = vec![cmd.to_string()];
            comment_magic(&mut source, "python", true, false);
            assert_eq!(source, vec![cmd.to_string()], "Should not comment: {}", cmd);
        }
    }

    #[test]
    fn test_do_not_comment_bash_commands_in_r() {
        let magic_cmds = vec!["ls", "!ls", "ls -al", "!whoami", "# ls", "# mv a b"];
        for cmd in magic_cmds {
            let mut source = vec![cmd.to_string()];
            comment_magic(&mut source, "R", true, false);
            assert_eq!(source, vec![cmd.to_string()], "R should not comment: {}", cmd);

            uncomment_magic(&mut source, "R", true, false);
            assert_eq!(source, vec![cmd.to_string()], "R should not change: {}", cmd);
        }
    }

    #[test]
    fn test_markdown_image_is_not_magic() {
        assert!(is_magic("# !cmd", "python", true, false));
        assert!(!is_magic("# ![Image name](image.png", "python", true, false));
    }

    #[test]
    fn test_question_is_not_magic() {
        assert!(is_magic("float?", "python", true, true));
        assert!(is_magic("# float?", "python", true, true));
        assert!(!is_magic("# question: float?", "python", true, true));
    }

    #[test]
    fn test_indented_magic() {
        assert!(is_magic("    !rm file", "python", true, false));
        assert!(is_magic("    # !rm file", "python", true, false));
        assert!(is_magic("    %cd", "python", true, false));

        let mut source = vec!["    !rm file".to_string()];
        comment_magic(&mut source, "python", true, false);
        assert_eq!(source, vec!["    # !rm file"]);
        uncomment_magic(&mut source, "python", true, false);
        assert_eq!(source, vec!["    !rm file"]);

        let mut source = vec!["    %cd".to_string()];
        comment_magic(&mut source, "python", true, false);
        assert_eq!(source, vec!["    # %cd"]);
        uncomment_magic(&mut source, "python", true, false);
        assert_eq!(source, vec!["    %cd"]);
    }

    #[test]
    fn test_magic_assign() {
        assert!(is_magic("result = %sql SELECT * FROM quickdemo WHERE value > 25", "python", true, false));
        assert!(is_magic("name = %time 2+2", "python", true, false));
        assert!(is_magic("flake8_version = !flake8 --version", "python", true, false));
    }
}

// =========================================================================
// 3. PEP8 tests
// =========================================================================

mod pep8_tests {
    use super::*;

    fn to_strings(lines: &[&str]) -> Vec<String> {
        lines.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn test_pep8_basic() {
        let prev = to_strings(&["x = 1"]);
        let next = to_strings(&["y = 2"]);
        assert_eq!(pep8_lines_between_cells(&prev, &next, ".py"), 1);
    }

    #[test]
    fn test_pep8_function_after_code() {
        let prev = to_strings(&[
            "a = a_long_instruction(",
            "    over_two_lines=True)",
        ]);
        let next = to_strings(&[
            "def f(x):",
            "    return x",
        ]);
        assert_eq!(pep8_lines_between_cells(&prev, &next, ".py"), 2);
    }

    #[test]
    fn test_pep8_function_before_code() {
        let prev = to_strings(&[
            "def f(x):",
            "    return x",
        ]);
        let next = to_strings(&[
            "# A markdown cell",
            "",
            "# An instruction",
            "a = 5",
            "",
        ]);
        assert_eq!(pep8_lines_between_cells(&prev, &next, ".py"), 2);
    }

    #[test]
    fn test_pep8_function_before_markdown_only() {
        let prev = to_strings(&[
            "def f(x):",
            "    return x",
        ]);
        let next = to_strings(&[
            "# A markdown cell",
            "",
            "# Only markdown here",
            "# And here",
            "",
        ]);
        assert_eq!(pep8_lines_between_cells(&prev, &next, ".py"), 1);
    }

    #[test]
    fn test_pep8_code_before_decorated_function() {
        let prev = to_strings(&[
            "from jupytext.cell_to_text import RMarkdownCellExporter",
        ]);
        let next = to_strings(&[
            "@pytest.mark.parametrize(",
            "    \"lines\",",
            "    [",
            "        \"# text\",",
            "    ],",
            ")",
            "def test_paragraph_is_fully_commented(lines):",
            "    pass",
        ]);
        assert_eq!(pep8_lines_between_cells(&prev, &next, ".py"), 2);
    }

    #[test]
    fn test_pep8_non_python() {
        let prev = to_strings(&["x = 1"]);
        let next = to_strings(&["y = 2"]);
        assert_eq!(pep8_lines_between_cells(&prev, &next, ".jl"), 1);
    }

    #[test]
    fn test_pep8_empty_next() {
        let prev = to_strings(&["x = 1"]);
        let next: Vec<String> = Vec::new();
        assert_eq!(pep8_lines_between_cells(&prev, &next, ".py"), 1);
    }

    #[test]
    fn test_pep8_empty_prev() {
        let prev: Vec<String> = Vec::new();
        let next = to_strings(&["x = 1"]);
        assert_eq!(pep8_lines_between_cells(&prev, &next, ".py"), 0);
    }

    #[test]
    fn test_pep8_class_ending() {
        let prev = to_strings(&[
            "class A:",
            "    __init__():",
            "    '''A docstring",
            "with two lines or more'''",
            "        self.a = 0",
            "",
        ]);
        let next = to_strings(&[
            "# Some comment",
            "a = 5",
        ]);
        // Class ending followed by code -> 2 blank lines
        assert_eq!(pep8_lines_between_cells(&prev, &next, ".py"), 2);
    }
}

// =========================================================================
// 4. Cell metadata tests
// =========================================================================

mod cell_metadata_tests {
    use super::*;

    #[test]
    fn test_text_to_metadata_simple_language() {
        let (lang, meta) = text_to_metadata("python", false);
        assert_eq!(lang, "python");
        assert!(meta.is_empty());
    }

    #[test]
    fn test_text_to_metadata_language_with_options() {
        let (lang, meta) = text_to_metadata("python tags=[\"test\"]", false);
        assert_eq!(lang, "python");
        assert!(meta.contains_key("tags"));
    }

    #[test]
    fn test_text_to_metadata_json() {
        let (title, meta) = text_to_metadata("cell title {\"key\": \"value\"}", true);
        assert_eq!(title, "cell title");
        assert_eq!(meta.get("key"), Some(&Value::String("value".to_string())));
    }

    #[test]
    fn test_text_to_metadata_empty() {
        let (title, meta) = text_to_metadata("", false);
        assert_eq!(title, "");
        assert!(meta.is_empty());
    }

    #[test]
    fn test_metadata_to_text_simple() {
        let mut meta = BTreeMap::new();
        meta.insert("key".to_string(), Value::String("value".to_string()));
        let text = metadata_to_text(Some("python"), &meta, false);
        assert!(text.contains("python"));
        assert!(text.contains("key="));
    }

    #[test]
    fn test_metadata_to_text_empty() {
        let meta = BTreeMap::new();
        let text = metadata_to_text(Some("python"), &meta, false);
        assert_eq!(text, "python");
    }

    #[test]
    fn test_metadata_to_text_no_title() {
        let meta = BTreeMap::new();
        let text = metadata_to_text(None, &meta, false);
        assert_eq!(text, "");
    }

    #[test]
    fn test_metadata_to_text_json() {
        let mut meta = BTreeMap::new();
        meta.insert("key".to_string(), json!(42));
        let text = metadata_to_text(None, &meta, true);
        assert!(text.contains("\"key\":42") || text.contains("\"key\": 42"));
    }

    #[test]
    fn test_parse_rmd_options_simple() {
        let opts = parse_rmd_options("echo=TRUE, eval=FALSE");
        assert_eq!(opts.len(), 2);
        assert_eq!(opts[0], ("echo".to_string(), "TRUE".to_string()));
        assert_eq!(opts[1], ("eval".to_string(), "FALSE".to_string()));
    }

    #[test]
    fn test_parse_rmd_options_with_name() {
        let opts = parse_rmd_options("my_chunk, echo=TRUE");
        assert_eq!(opts.len(), 2);
        assert_eq!(opts[0].0, "");
        assert_eq!(opts[0].1, "my_chunk");
        assert_eq!(opts[1].0, "echo");
        assert_eq!(opts[1].1, "TRUE");
    }

    #[test]
    fn test_parse_rmd_options_empty() {
        let opts = parse_rmd_options("");
        assert!(opts.is_empty());
    }

    #[test]
    fn test_rmd_options_to_metadata_simple() {
        let (lang, meta) = rmd_options_to_metadata("r, echo=TRUE", false);
        assert_eq!(lang, "R");
        assert_eq!(meta.get("echo"), Some(&Value::Bool(true)));
    }

    #[test]
    fn test_rmd_options_to_metadata_python() {
        let (lang, meta) = rmd_options_to_metadata("python", false);
        assert_eq!(lang, "python");
        assert!(meta.is_empty());
    }

    #[test]
    fn test_is_json_metadata() {
        assert!(is_json_metadata("{\"key\": \"value\"}"));
        assert!(!is_json_metadata("key=value"));
        assert!(!is_json_metadata(""));
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

    #[test]
    fn test_is_active_with_tags() {
        let mut meta = BTreeMap::new();
        meta.insert(
            "tags".to_string(),
            json!(["active-py"]),
        );
        assert!(is_active(".py", &meta, true));
        assert!(!is_active(".Rmd", &meta, true));
    }

    #[test]
    fn test_is_active_frozen() {
        let mut meta = BTreeMap::new();
        meta.insert(
            "run_control".to_string(),
            json!({"frozen": true}),
        );
        assert!(is_active(".ipynb", &meta, true));
        assert!(!is_active(".py", &meta, true));
    }

    #[test]
    fn test_metadata_to_text_roundtrip() {
        let (lang, meta) = text_to_metadata("python tags=[\"test\"]", false);
        let text = metadata_to_text(Some(&lang), &meta, false);
        assert!(text.contains("python"));
        assert!(text.contains("tags"));
    }
}

// =========================================================================
// 5. Metadata filter tests
// =========================================================================

mod metadata_filter_tests {
    use super::*;

    #[test]
    fn test_parse_empty_filter() {
        let f = metadata_filter_as_dict("");
        assert!(matches!(f.additional, FilterSpec::Keys(ref k) if k.is_empty()));
        assert!(matches!(f.excluded, FilterSpec::Keys(ref k) if k.is_empty()));
    }

    #[test]
    fn test_parse_all_filter() {
        let f = metadata_filter_as_dict("all");
        assert!(f.additional.is_all());
    }

    #[test]
    fn test_parse_minus_all_filter() {
        let f = metadata_filter_as_dict("-all");
        assert!(f.excluded.is_all());
    }

    #[test]
    fn test_parse_mixed_filter() {
        let f = metadata_filter_as_dict("one,two,-three,-four");
        match &f.additional {
            FilterSpec::Keys(keys) => {
                assert!(keys.contains(&"one".to_string()));
                assert!(keys.contains(&"two".to_string()));
            }
            _ => panic!("Expected Keys variant"),
        }
        match &f.excluded {
            FilterSpec::Keys(keys) => {
                assert!(keys.contains(&"three".to_string()));
                assert!(keys.contains(&"four".to_string()));
            }
            _ => panic!("Expected Keys variant"),
        }
    }

    #[test]
    fn test_filter_as_string_roundtrip() {
        let f = metadata_filter_as_dict("kernelspec,-all");
        let s = metadata_filter_as_string(&f);
        assert!(s.contains("kernelspec"));
        assert!(s.contains("-all"));
    }

    #[test]
    fn test_filter_metadata_default_notebook() {
        let mut metadata = BTreeMap::new();
        metadata.insert("kernelspec".to_string(), json!({"name": "python3"}));
        metadata.insert("jupytext".to_string(), json!({"formats": "ipynb,py"}));
        metadata.insert("custom_key".to_string(), json!("value"));

        let filtered = filter_metadata(&metadata, "", DEFAULT_NOTEBOOK_METADATA);
        // With default notebook metadata filter and no user filter,
        // all keys are preserved (the default filter does not exclude unknown keys)
        assert!(filtered.contains_key("kernelspec"));
        assert!(filtered.contains_key("jupytext"));
        assert!(filtered.contains_key("custom_key"));
    }

    #[test]
    fn test_filter_metadata_default_notebook_with_minus_all() {
        let mut metadata = BTreeMap::new();
        metadata.insert("kernelspec".to_string(), json!({"name": "python3"}));
        metadata.insert("jupytext".to_string(), json!({"formats": "ipynb,py"}));
        metadata.insert("custom_key".to_string(), json!("value"));

        // With -all user filter, only default notebook metadata keys are kept
        let filtered = filter_metadata(&metadata, "-all", DEFAULT_NOTEBOOK_METADATA);
        assert!(filtered.is_empty());
    }

    #[test]
    fn test_filter_metadata_include_all() {
        let mut metadata = BTreeMap::new();
        metadata.insert("kernelspec".to_string(), json!({"name": "python3"}));
        metadata.insert("custom_key".to_string(), json!("value"));

        let filtered = filter_metadata(&metadata, "all", DEFAULT_NOTEBOOK_METADATA);
        assert!(filtered.contains_key("kernelspec"));
        assert!(filtered.contains_key("custom_key"));
    }

    #[test]
    fn test_filter_metadata_exclude_all() {
        let mut metadata = BTreeMap::new();
        metadata.insert("kernelspec".to_string(), json!({"name": "python3"}));
        metadata.insert("custom_key".to_string(), json!("value"));

        let filtered = filter_metadata(&metadata, "-all", DEFAULT_NOTEBOOK_METADATA);
        assert!(filtered.is_empty());
    }

    #[test]
    fn test_filter_metadata_additional_keys() {
        let mut metadata = BTreeMap::new();
        metadata.insert("kernelspec".to_string(), json!({"name": "python3"}));
        metadata.insert("jupytext".to_string(), json!({}));
        metadata.insert("custom_key".to_string(), json!("value"));

        let filtered = filter_metadata(&metadata, "custom_key", DEFAULT_NOTEBOOK_METADATA);
        assert!(filtered.contains_key("kernelspec"));
        assert!(filtered.contains_key("jupytext"));
        assert!(filtered.contains_key("custom_key"));
    }

    #[test]
    fn test_filter_spec_contains() {
        let all = FilterSpec::All;
        assert!(all.contains("anything"));

        let keys = FilterSpec::Keys(vec!["one".to_string(), "two".to_string()]);
        assert!(keys.contains("one"));
        assert!(!keys.contains("three"));
    }
}

// =========================================================================
// 6. Header tests
// =========================================================================

mod header_tests {
    use super::*;

    #[test]
    fn test_header_to_metadata_and_cell_blank_line() {
        let text = "---\n\
                     title: Sample header\n\
                     ---\n\
                     \n\
                     Header is followed by a blank line";
        let lines: Vec<String> = text.lines().map(|s| s.to_string()).collect();
        let result = header_to_metadata_and_cell(&lines, "", "", ".md", true);

        assert!(result.metadata.is_empty());
        assert!(result.header_cell.is_some());
        let cell = result.header_cell.unwrap();
        assert_eq!(cell.cell_type, CellType::Raw);
        assert!(cell.source.contains("title: Sample header"));
        assert!(lines[result.next_line].starts_with("Header is"));
    }

    #[test]
    fn test_header_to_metadata_and_cell_no_blank_line() {
        let text = "---\n\
                     title: Sample header\n\
                     ---\n\
                     Header is not followed by a blank line";
        let lines: Vec<String> = text.lines().map(|s| s.to_string()).collect();
        let result = header_to_metadata_and_cell(&lines, "", "", ".md", true);

        assert!(result.metadata.is_empty());
        assert!(result.header_cell.is_some());
        let cell = result.header_cell.unwrap();
        assert_eq!(cell.cell_type, CellType::Raw);
        assert!(cell.source.contains("title: Sample header"));
        // The cell should have lines_to_next_cell=0 because there is no blank line
        assert!(cell.metadata.contains_key("lines_to_next_cell"));
        assert!(lines[result.next_line].starts_with("Header is"));
    }

    #[test]
    fn test_header_to_metadata_and_cell_with_jupyter_metadata() {
        let text = "---\ntitle: Sample header\njupyter:\n  mainlanguage: python\n---";
        let lines: Vec<String> = text.lines().map(|s| s.to_string()).collect();
        let result = header_to_metadata_and_cell(&lines, "", "", ".md", true);

        assert!(result.metadata.contains_key("mainlanguage"));
        let mainlang = result.metadata.get("mainlanguage").unwrap();
        assert_eq!(mainlang, &Value::String("python".to_string()));
    }

    #[test]
    fn test_header_in_html_comment() {
        let text = "<!--\n\n---\njupyter:\n  title: Sample header\n---\n\n-->";
        let lines: Vec<String> = text.lines().map(|s| s.to_string()).collect();
        let result = header_to_metadata_and_cell(&lines, "", "", ".md", true);

        assert!(result.metadata.contains_key("title"));
        assert_eq!(
            result.metadata.get("title"),
            Some(&Value::String("Sample header".to_string()))
        );
        assert!(result.header_cell.is_none());
    }

    #[test]
    fn test_recursive_update_basic() {
        let mut target = serde_json::Map::new();
        target.insert("0".to_string(), json!({"1": 2}));

        let mut update = serde_json::Map::new();
        update.insert("0".to_string(), json!({"1": 3}));
        update.insert("4".to_string(), json!(5));

        recursive_update(&mut target, &update, true);

        let result: serde_json::Map<String, Value> = target;
        assert_eq!(result.get("0").unwrap().get("1").unwrap(), &json!(3));
        assert_eq!(result.get("4").unwrap(), &json!(5));
    }

    #[test]
    fn test_recursive_update_no_overwrite() {
        let mut target = serde_json::Map::new();
        target.insert("0".to_string(), json!({"1": 2}));

        let mut update = serde_json::Map::new();
        update.insert("0".to_string(), json!({"1": 3}));
        update.insert("4".to_string(), json!(5));

        recursive_update(&mut target, &update, false);

        let result = target;
        // "1" should keep original value 2 because overwrite=false
        assert_eq!(result.get("0").unwrap().get("1").unwrap(), &json!(2));
        assert_eq!(result.get("4").unwrap(), &json!(5));
    }

    #[test]
    fn test_recursive_update_none_removes() {
        let mut target = serde_json::Map::new();
        target.insert("0".to_string(), json!(1));

        let mut update = serde_json::Map::new();
        update.insert("0".to_string(), Value::Null);

        recursive_update(&mut target, &update, true);
        assert!(!target.contains_key("0"));
    }

    #[test]
    fn test_recursive_update_none_removes_no_overwrite() {
        let mut target = serde_json::Map::new();
        target.insert("0".to_string(), json!(1));

        let mut update = serde_json::Map::new();
        update.insert("0".to_string(), Value::Null);

        recursive_update(&mut target, &update, false);
        assert!(!target.contains_key("0"));
    }

    #[test]
    fn test_metadata_and_cell_to_header_empty_metadata() {
        let metadata = serde_json::Map::new();
        let mut fmt = BTreeMap::new();
        fmt.insert("extension".to_string(), Value::String(".md".to_string()));

        let (header, lines_to_next) = metadata_and_cell_to_header(&metadata, &fmt, "", "");
        assert!(header.is_empty());
        assert_eq!(lines_to_next, Some(0));
    }

    #[test]
    fn test_no_header_detected_on_non_header_lines() {
        let text = "This is not a header\n\
                     And this isn't either";
        let lines: Vec<String> = text.lines().map(|s| s.to_string()).collect();
        let result = header_to_metadata_and_cell(&lines, "", "", ".md", true);

        assert!(result.metadata.is_empty());
        assert!(result.header_cell.is_none());
        assert_eq!(result.next_line, 0);
    }
}

// =========================================================================
// 7. Formats tests
// =========================================================================

mod formats_tests {
    use super::*;

    #[test]
    fn test_guess_format_simple_percent() {
        let nb = "# %%\nprint(\"hello world!\")\n";
        let (fmt, _) = guess_format(nb, ".py");
        assert_eq!(fmt, "percent");
    }

    #[test]
    fn test_guess_format_simple_percent_with_magic() {
        let nb = "# %%\n# %time\nprint(\"hello world!\")\n";
        let (fmt, _) = guess_format(nb, ".py");
        assert_eq!(fmt, "percent");
    }

    #[test]
    fn test_guess_format_hydrogen_with_magic() {
        let nb = "# %%\n%time\nprint(\"hello world!\")\n";
        let (fmt, _) = guess_format(nb, ".py");
        assert_eq!(fmt, "hydrogen");
    }

    #[test]
    fn test_guess_format_hydrogen_cat() {
        let text = "# %%\ncat hello.txt\n";
        let (fmt, _) = guess_format(text, ".py");
        assert_eq!(fmt, "hydrogen");
    }

    #[test]
    fn test_guess_format_light() {
        let text = "def f(x):\n    return x + 1\n";
        let (fmt, _) = guess_format(text, ".py");
        assert_eq!(fmt, "light");
    }

    #[test]
    fn test_script_with_magics_not_percent() {
        let script = "# %%time\n1 + 2";
        let (fmt, _) = guess_format(script, ".py");
        assert_eq!(fmt, "light");
    }

    #[test]
    fn test_script_with_spyder_cell_is_percent() {
        let script = "#%%\n1 + 2";
        let (fmt, _) = guess_format(script, ".py");
        assert_eq!(fmt, "percent");
    }

    #[test]
    fn test_script_with_percent_cell_and_magic_is_hydrogen() {
        let script = "#%%\n%matplotlib inline\n";
        let (fmt, _) = guess_format(script, ".py");
        assert_eq!(fmt, "hydrogen");
    }

    #[test]
    fn test_spyder_cell_with_name_is_percent() {
        let script = "#%% cell name\n1 + 2";
        let (fmt, _) = guess_format(script, ".py");
        assert_eq!(fmt, "percent");
    }

    #[test]
    fn test_divine_format_ipynb() {
        assert_eq!(divine_format("{\"cells\":[]}"), "ipynb");
    }

    #[test]
    fn test_divine_format_python_light() {
        let text = "def f(x):\n    x + 1";
        assert_eq!(divine_format(text), "py:light");
    }

    #[test]
    fn test_divine_format_python_percent() {
        let text = "# %%\ndef f(x):\n    x + 1\n\n# %%\ndef g(x):\n    x + 2\n";
        assert_eq!(divine_format(text), "py:percent");
    }

    #[test]
    fn test_get_format_implementation_default() {
        let result = get_format_implementation(".py", None);
        assert!(result.is_ok());
        assert_eq!(result.unwrap().format_name, "light");
    }

    #[test]
    fn test_get_format_implementation_percent() {
        let result = get_format_implementation(".py", Some("percent"));
        assert!(result.is_ok());
        assert_eq!(result.unwrap().format_name, "percent");
    }

    #[test]
    fn test_get_format_implementation_wrong_format() {
        let result = get_format_implementation(".py", Some("wrong_format"));
        assert!(result.is_err());
    }

    #[test]
    fn test_long_form_one_format_ipynb() {
        let fmt = long_form_one_format("ipynb", None, None, false).unwrap();
        assert_eq!(fmt.get("extension").unwrap(), &Value::String(".ipynb".to_string()));
    }

    #[test]
    fn test_long_form_one_format_py_percent() {
        let fmt = long_form_one_format("py:percent", None, None, false).unwrap();
        assert_eq!(
            fmt.get("extension").unwrap(),
            &Value::String(".py".to_string())
        );
        assert_eq!(
            fmt.get("format_name").unwrap(),
            &Value::String("percent".to_string())
        );
    }

    #[test]
    fn test_long_form_one_format_with_suffix() {
        let fmt = long_form_one_format(".pct.py:percent", None, None, false).unwrap();
        assert_eq!(
            fmt.get("extension").unwrap(),
            &Value::String(".py".to_string())
        );
        assert_eq!(
            fmt.get("suffix").unwrap(),
            &Value::String(".pct".to_string())
        );
        assert_eq!(
            fmt.get("format_name").unwrap(),
            &Value::String("percent".to_string())
        );
    }

    #[test]
    fn test_long_form_one_format_empty() {
        let fmt = long_form_one_format("", None, None, false).unwrap();
        assert!(fmt.is_empty());
    }

    #[test]
    fn test_long_form_multiple_formats_ipynb() {
        let result = long_form_multiple_formats("ipynb", None, false);
        assert_eq!(result.len(), 1);
        assert_eq!(
            result[0].get("extension").unwrap(),
            &Value::String(".ipynb".to_string())
        );
    }

    #[test]
    fn test_long_form_multiple_formats_ipynb_md() {
        let result = long_form_multiple_formats("ipynb,md", None, false);
        assert_eq!(result.len(), 2);
        assert_eq!(
            result[0].get("extension").unwrap(),
            &Value::String(".ipynb".to_string())
        );
        assert_eq!(
            result[1].get("extension").unwrap(),
            &Value::String(".md".to_string())
        );
    }

    #[test]
    fn test_long_form_multiple_formats_ipynb_py_light() {
        let result = long_form_multiple_formats("ipynb,py:light", None, false);
        assert_eq!(result.len(), 2);
        assert_eq!(
            result[1].get("extension").unwrap(),
            &Value::String(".py".to_string())
        );
        assert_eq!(
            result[1].get("format_name").unwrap(),
            &Value::String("light".to_string())
        );
    }

    #[test]
    fn test_long_form_multiple_formats_with_suffix() {
        let result = long_form_multiple_formats(".pct.py:percent", None, false);
        assert_eq!(result.len(), 1);
        assert_eq!(
            result[0].get("extension").unwrap(),
            &Value::String(".py".to_string())
        );
        assert_eq!(
            result[0].get("suffix").unwrap(),
            &Value::String(".pct".to_string())
        );
        assert_eq!(
            result[0].get("format_name").unwrap(),
            &Value::String("percent".to_string())
        );
    }

    #[test]
    fn test_short_form_one_format_ipynb() {
        let mut fmt = BTreeMap::new();
        fmt.insert("extension".to_string(), Value::String(".ipynb".to_string()));
        assert_eq!(short_form_one_format(&fmt), "ipynb");
    }

    #[test]
    fn test_short_form_one_format_py_light() {
        let mut fmt = BTreeMap::new();
        fmt.insert("extension".to_string(), Value::String(".py".to_string()));
        fmt.insert(
            "format_name".to_string(),
            Value::String("light".to_string()),
        );
        assert_eq!(short_form_one_format(&fmt), "py:light");
    }

    #[test]
    fn test_short_form_one_format_with_suffix() {
        let mut fmt = BTreeMap::new();
        fmt.insert("extension".to_string(), Value::String(".py".to_string()));
        fmt.insert("suffix".to_string(), Value::String(".pct".to_string()));
        fmt.insert(
            "format_name".to_string(),
            Value::String("percent".to_string()),
        );
        assert_eq!(short_form_one_format(&fmt), ".pct.py:percent");
    }

    #[test]
    fn test_short_form_multiple_formats_ipynb() {
        let fmts = vec![{
            let mut m = BTreeMap::new();
            m.insert("extension".to_string(), Value::String(".ipynb".to_string()));
            m
        }];
        assert_eq!(short_form_multiple_formats(&fmts), "ipynb");
    }

    #[test]
    fn test_short_form_multiple_formats_ipynb_md() {
        let fmts = vec![
            {
                let mut m = BTreeMap::new();
                m.insert("extension".to_string(), Value::String(".ipynb".to_string()));
                m
            },
            {
                let mut m = BTreeMap::new();
                m.insert("extension".to_string(), Value::String(".md".to_string()));
                m
            },
        ];
        assert_eq!(short_form_multiple_formats(&fmts), "ipynb,md");
    }

    #[test]
    fn test_short_form_multiple_formats_ipynb_py_light() {
        let fmts = vec![
            {
                let mut m = BTreeMap::new();
                m.insert("extension".to_string(), Value::String(".ipynb".to_string()));
                m
            },
            {
                let mut m = BTreeMap::new();
                m.insert("extension".to_string(), Value::String(".py".to_string()));
                m.insert(
                    "format_name".to_string(),
                    Value::String("light".to_string()),
                );
                m
            },
        ];
        assert_eq!(short_form_multiple_formats(&fmts), "ipynb,py:light");
    }

    #[test]
    fn test_decompress_compress_roundtrip_ipynb() {
        let long = long_form_multiple_formats("ipynb", None, false);
        let short = short_form_multiple_formats(&long);
        assert_eq!(short, "ipynb");
    }

    #[test]
    fn test_decompress_compress_roundtrip_ipynb_md() {
        let long = long_form_multiple_formats("ipynb,md", None, false);
        let short = short_form_multiple_formats(&long);
        assert_eq!(short, "ipynb,md");
    }

    #[test]
    fn test_decompress_compress_roundtrip_ipynb_py_light() {
        let long = long_form_multiple_formats("ipynb,py:light", None, false);
        let short = short_form_multiple_formats(&long);
        assert_eq!(short, "ipynb,py:light");
    }

    #[test]
    fn test_decompress_compress_roundtrip_suffix() {
        let long = long_form_multiple_formats(".pct.py:percent", None, false);
        let short = short_form_multiple_formats(&long);
        assert_eq!(short, ".pct.py:percent");
    }

    #[test]
    fn test_validate_one_format_invalid_format_name() {
        let mut fmt = BTreeMap::new();
        fmt.insert("extension".to_string(), Value::String(".py".to_string()));
        fmt.insert(
            "format_name".to_string(),
            Value::String("invalid".to_string()),
        );
        let result = validate_one_format(&fmt);
        assert!(result.is_err());
    }

    #[test]
    fn test_validate_one_format_no_extension() {
        let fmt = BTreeMap::new();
        let result = validate_one_format(&fmt);
        assert!(result.is_err());
    }

    #[test]
    fn test_long_form_one_format_with_prefix() {
        let fmt = long_form_one_format("python//py:percent", None, None, false).unwrap();
        assert_eq!(
            fmt.get("prefix").unwrap(),
            &Value::String("python/".to_string())
        );
        assert_eq!(
            fmt.get("extension").unwrap(),
            &Value::String(".py".to_string())
        );
        assert_eq!(
            fmt.get("format_name").unwrap(),
            &Value::String("percent".to_string())
        );
    }

    #[test]
    fn test_long_form_one_format_with_prefix_root() {
        let fmt = long_form_one_format("notebooks///ipynb", None, None, false).unwrap();
        assert_eq!(
            fmt.get("prefix").unwrap(),
            &Value::String("notebooks//".to_string())
        );
        assert_eq!(
            fmt.get("extension").unwrap(),
            &Value::String(".ipynb".to_string())
        );
    }

    #[test]
    fn test_long_form_multiple_formats_prefix_root() {
        let formats = long_form_multiple_formats("notebooks///ipynb,scripts///py:percent", None, false);
        assert_eq!(formats.len(), 2);
        assert_eq!(
            formats[0].get("prefix").unwrap(),
            &Value::String("notebooks//".to_string())
        );
        assert_eq!(
            formats[1].get("prefix").unwrap(),
            &Value::String("scripts//".to_string())
        );
    }
}

// =========================================================================
// 8. Compare tests
// =========================================================================

mod compare_tests {
    use super::*;

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
    }

    #[test]
    fn test_compare_raises_on_diff() {
        let result = compare("hello", "world", "a", "b", false);
        assert!(result.is_err());
    }

    #[test]
    fn test_compare_notebooks_identical() {
        let nb1 = make_nb(vec![Cell::new_code("1 + 1"), Cell::new_markdown("# Title")]);
        let nb2 = make_nb(vec![Cell::new_code("1 + 1"), Cell::new_markdown("# Title")]);
        let result = compare_notebooks(&nb1, &nb2, None, true, true);
        assert!(result.is_ok());
    }

    #[test]
    fn test_compare_notebooks_different_cell_type() {
        let nb1 = make_nb(vec![Cell::new_code("text")]);
        let nb2 = make_nb(vec![Cell::new_markdown("text")]);
        let result = compare_notebooks(&nb1, &nb2, None, true, true);
        assert!(result.is_err());
    }

    #[test]
    fn test_compare_notebooks_different_cell_content() {
        let nb1 = make_nb(vec![Cell::new_code("1 + 1")]);
        let nb2 = make_nb(vec![Cell::new_code("2 + 2")]);
        let result = compare_notebooks(&nb1, &nb2, None, true, true);
        assert!(result.is_err());
    }

    #[test]
    fn test_compare_notebooks_different_cell_count() {
        let nb1 = make_nb(vec![Cell::new_code("1")]);
        let nb2 = make_nb(vec![Cell::new_code("1"), Cell::new_code("2")]);

        let result = compare_notebooks(&nb1, &nb2, None, true, true);
        assert!(result.is_err());

        let result = compare_notebooks(&nb2, &nb1, None, true, true);
        assert!(result.is_err());
    }

    #[test]
    fn test_compare_notebooks_collect_all_differences() {
        let nb1 = make_nb(vec![
            Cell::new_code("a"),
            Cell::new_code("changed"),
        ]);
        let nb2 = make_nb(vec![
            Cell::new_code("a"),
            Cell::new_code("original"),
        ]);
        // raise_on_first_difference = false
        let result = compare_notebooks(&nb1, &nb2, None, true, false);
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.message.contains("Cells"), "Error: {}", err.message);
    }

    #[test]
    fn test_compare_notebooks_blank_line_removed() {
        // Trailing blank line removal should be tolerated with allow_expected_differences=true
        let nb1 = make_nb(vec![Cell::new_code("1+1\n    ")]);
        let nb2 = make_nb(vec![Cell::new_code("1+1")]);
        let result = compare_notebooks(&nb2, &nb1, None, true, true);
        assert!(result.is_ok());
    }

    #[test]
    fn test_compare_notebooks_strict_blank_line() {
        let nb1 = make_nb(vec![Cell::new_code("1+1\n")]);
        let nb2 = make_nb(vec![Cell::new_code("1+1")]);
        // allow_expected_differences=false should fail
        let result = compare_notebooks(&nb2, &nb1, None, false, true);
        assert!(result.is_err());
    }

    #[test]
    fn test_compare_notebooks_multiple_cells_differ() {
        let nb1 = make_nb(vec![Cell::new_code(""), Cell::new_code("2")]);
        let nb2 = make_nb(vec![Cell::new_code("1+1"), Cell::new_code("2\n2")]);
        let result = compare_notebooks(&nb2, &nb1, None, true, false);
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.message.contains("Cells"), "Error: {}", err.message);
    }

    #[test]
    fn test_compare_notebooks_metadata_differ() {
        let mut meta = BTreeMap::new();
        meta.insert(
            "kernelspec".to_string(),
            json!({"language": "python", "name": "python", "display_name": "Python"}),
        );
        let nb1 = Notebook {
            nbformat: 4,
            nbformat_minor: 5,
            metadata: BTreeMap::new(),
            cells: vec![Cell::new_code("1"), Cell::new_code("2")],
        };
        let nb2 = Notebook {
            nbformat: 4,
            nbformat_minor: 5,
            metadata: meta,
            cells: vec![Cell::new_code("1"), Cell::new_code("2")],
        };
        let result = compare_notebooks(&nb2, &nb1, None, true, false);
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(
            err.message.contains("Notebook metadata differ"),
            "Error: {}",
            err.message
        );
    }

    #[test]
    fn test_compare_notebooks_cell_metadata_differ() {
        let mut meta1 = BTreeMap::new();
        meta1.insert(
            "additional".to_string(),
            Value::String("metadata1".to_string()),
        );
        let mut cell1 = Cell::new_code("2");
        cell1.metadata = meta1;

        let mut meta2 = BTreeMap::new();
        meta2.insert(
            "additional".to_string(),
            Value::String("metadata2".to_string()),
        );
        let mut cell2 = Cell::new_code("2");
        cell2.metadata = meta2;

        let nb1 = make_nb(vec![Cell::new_code("1"), cell1]);
        let nb2 = make_nb(vec![Cell::new_code("1"), cell2]);
        let result = compare_notebooks(&nb2, &nb1, None, true, false);
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(
            err.message.contains("Cell metadata") && err.message.contains("additional"),
            "Error: {}",
            err.message
        );
    }
}

// =========================================================================
// 9. Notebook tests
// =========================================================================

mod notebook_tests {
    use super::*;
    use jupytext::notebook::{reads_ipynb, writes_ipynb};

    #[test]
    fn test_cell_new_code() {
        let cell = Cell::new_code("print('hello')");
        assert_eq!(cell.cell_type, CellType::Code);
        assert_eq!(cell.source, "print('hello')");
        assert!(cell.metadata.is_empty());
        assert!(cell.execution_count.is_some());
        assert!(cell.outputs.is_some());
    }

    #[test]
    fn test_cell_new_markdown() {
        let cell = Cell::new_markdown("# Title");
        assert_eq!(cell.cell_type, CellType::Markdown);
        assert_eq!(cell.source, "# Title");
        assert!(cell.metadata.is_empty());
        assert!(cell.execution_count.is_none());
        assert!(cell.outputs.is_none());
    }

    #[test]
    fn test_cell_new_raw() {
        let cell = Cell::new_raw("raw content");
        assert_eq!(cell.cell_type, CellType::Raw);
        assert_eq!(cell.source, "raw content");
        assert!(cell.metadata.is_empty());
    }

    #[test]
    fn test_cell_new_with_type() {
        let code = Cell::new_with_type(CellType::Code, "x = 1");
        assert_eq!(code.cell_type, CellType::Code);

        let md = Cell::new_with_type(CellType::Markdown, "# Hi");
        assert_eq!(md.cell_type, CellType::Markdown);

        let raw = Cell::new_with_type(CellType::Raw, "raw");
        assert_eq!(raw.cell_type, CellType::Raw);
    }

    #[test]
    fn test_notebook_new() {
        let nb = Notebook::new();
        assert_eq!(nb.nbformat, 4);
        assert_eq!(nb.nbformat_minor, 5);
        assert!(nb.cells.is_empty());
        assert!(nb.metadata.is_empty());
    }

    #[test]
    fn test_notebook_default() {
        let nb = Notebook::default();
        assert_eq!(nb.nbformat, 4);
        assert_eq!(nb.nbformat_minor, 5);
    }

    #[test]
    fn test_notebook_new_with_metadata() {
        let mut meta = BTreeMap::new();
        meta.insert(
            "kernelspec".to_string(),
            json!({"name": "python3", "language": "python"}),
        );
        let nb = Notebook::new_with_metadata(meta.clone());
        assert_eq!(nb.metadata, meta);
        assert!(nb.cells.is_empty());
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
    fn test_notebook_serialization_roundtrip() {
        let mut nb = Notebook::new();
        nb.cells.push(Cell::new_code("x = 1"));
        nb.cells.push(Cell::new_markdown("# Hello"));

        let json_str = writes_ipynb(&nb).unwrap();
        let nb2 = reads_ipynb(&json_str).unwrap();

        assert_eq!(nb2.nbformat, nb.nbformat);
        assert_eq!(nb2.nbformat_minor, nb.nbformat_minor);
        assert_eq!(nb2.cells.len(), 2);
        assert_eq!(nb2.cells[0].cell_type, CellType::Code);
        assert_eq!(nb2.cells[0].source, "x = 1");
        assert_eq!(nb2.cells[1].cell_type, CellType::Markdown);
        assert_eq!(nb2.cells[1].source, "# Hello");
    }

    #[test]
    fn test_notebook_with_metadata_serialization() {
        let mut nb = Notebook::new();
        nb.metadata.insert(
            "kernelspec".to_string(),
            json!({
                "display_name": "Python 3",
                "language": "python",
                "name": "python3"
            }),
        );
        nb.cells.push(Cell::new_code("1 + 1"));

        let json_str = writes_ipynb(&nb).unwrap();
        let nb2 = reads_ipynb(&json_str).unwrap();

        assert!(nb2.metadata.contains_key("kernelspec"));
        let ks = nb2.metadata.get("kernelspec").unwrap();
        assert_eq!(ks.get("name").unwrap(), "python3");
    }

    #[test]
    fn test_cell_with_metadata() {
        let mut cell = Cell::new_code("x = 1");
        cell.metadata
            .insert("tags".to_string(), json!(["important"]));

        let nb = Notebook {
            nbformat: 4,
            nbformat_minor: 5,
            metadata: BTreeMap::new(),
            cells: vec![cell],
        };

        let json_str = writes_ipynb(&nb).unwrap();
        let nb2 = reads_ipynb(&json_str).unwrap();

        assert!(nb2.cells[0].metadata.contains_key("tags"));
    }

    #[test]
    fn test_reads_ipynb_minimal() {
        let json = r#"{
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
        }"#;
        let nb = reads_ipynb(json).unwrap();
        assert_eq!(nb.cells.len(), 1);
        assert_eq!(nb.cells[0].source, "print('hello')");
    }

    #[test]
    fn test_reads_ipynb_invalid() {
        let result = reads_ipynb("not json");
        assert!(result.is_err());
    }

    #[test]
    fn test_get_metadata_nested() {
        use jupytext::notebook::get_metadata_nested;

        let mut meta = BTreeMap::new();
        meta.insert(
            "jupytext".to_string(),
            json!({"formats": "ipynb,py", "text_representation": {"extension": ".py"}}),
        );

        let formats = get_metadata_nested(&meta, "jupytext.formats");
        assert_eq!(formats, Some(&json!("ipynb,py")));

        let ext = get_metadata_nested(&meta, "jupytext.text_representation.extension");
        assert_eq!(ext, Some(&json!(".py")));

        let missing = get_metadata_nested(&meta, "nonexistent.key");
        assert_eq!(missing, None);
    }

    #[test]
    fn test_set_metadata_nested() {
        use jupytext::notebook::set_metadata_nested;

        let mut meta = BTreeMap::new();
        set_metadata_nested(&mut meta, "jupytext.formats", json!("ipynb,py"));
        assert_eq!(
            meta.get("jupytext").unwrap().get("formats").unwrap(),
            &json!("ipynb,py")
        );
    }

    #[test]
    fn test_metadata_string() {
        use jupytext::notebook::metadata_string;

        let mut meta = BTreeMap::new();
        meta.insert("key".to_string(), json!("value"));
        assert_eq!(metadata_string(&meta, "key"), Some("value".to_string()));
        assert_eq!(metadata_string(&meta, "nonexistent"), None);
    }
}

// =========================================================================
// 10. Paired paths tests
// =========================================================================

mod paired_paths_tests {
    use super::*;
    use jupytext::formats::long_form_one_format_as_strings;
    use jupytext::paired_paths::{base_path, full_path, paired_paths};

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

    fn fmt_with_name(ext: &str, name: &str) -> BTreeMap<String, String> {
        let mut m = fmt(ext);
        m.insert("format_name".to_string(), name.to_string());
        m
    }

    #[test]
    fn test_simple_pair() {
        let ipynb = fmt(".ipynb");
        let py = fmt(".py");
        let formats = vec![ipynb.clone(), py.clone()];

        let paths = paired_paths("notebook.ipynb", &ipynb, &formats).unwrap();
        assert_eq!(paths.len(), 2);
        assert_eq!(paths[0].0, "notebook.ipynb");
        assert_eq!(paths[1].0, "notebook.py");

        let paths = paired_paths("notebook.py", &py, &formats).unwrap();
        assert_eq!(paths.len(), 2);
        assert_eq!(paths[0].0, "notebook.ipynb");
        assert_eq!(paths[1].0, "notebook.py");
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
    fn test_base_path_with_prefix() {
        let f = long_form_one_format_as_strings("dir/prefix_/ipynb");
        let result = base_path("dir/prefix_NAME.ipynb", &f, &[]).unwrap();
        assert_eq!(result, "NAME");
    }

    #[test]
    fn test_base_path_inconsistent_prefix() {
        let f = long_form_one_format_as_strings("dir/prefix_/ipynb");
        let result = base_path("dir/incorrect_prefix_NAME.ipynb", &f, &[]);
        assert!(result.is_err());
    }

    #[test]
    fn test_base_path_dotdot() {
        let f = long_form_one_format_as_strings("../scripts//py");
        let result = base_path("scripts/test.py", &f, &[]).unwrap();
        assert_eq!(result, "scripts/test");
    }

    #[test]
    fn test_full_path_dotdot() {
        let f = long_form_one_format_as_strings("../scripts//py");
        let result = full_path("scripts/test", &f).unwrap();
        assert_eq!(result, "scripts/test.py");
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
    fn test_base_path_in_tree_from_root() {
        let f = long_form_one_format_as_strings("scripts///py");
        assert_eq!(
            base_path("scripts/subfolder/test.py", &f, &[]).unwrap(),
            "//subfolder/test"
        );
    }

    #[test]
    fn test_full_path_in_tree_from_root() {
        let f = long_form_one_format_as_strings("notebooks///ipynb");
        assert_eq!(
            full_path("//subfolder/test", &f).unwrap(),
            "notebooks/subfolder/test.ipynb"
        );
    }

    #[test]
    fn test_full_path_in_tree_from_root_no_subfolder() {
        let f = long_form_one_format_as_strings("notebooks///ipynb");
        assert_eq!(
            full_path("//test", &f).unwrap(),
            "notebooks/test.ipynb"
        );
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
    fn test_many_and_suffix() {
        let ipynb = fmt(".ipynb");
        let pct = {
            let mut m = fmt(".py");
            m.insert("suffix".to_string(), ".pct".to_string());
            m
        };
        let lgt = {
            let mut m = fmt(".py");
            m.insert("suffix".to_string(), "_lgt".to_string());
            m
        };
        let formats = vec![ipynb.clone(), pct.clone(), lgt.clone()];

        let paths = paired_paths("notebook.ipynb", &ipynb, &formats).unwrap();
        assert_eq!(paths[0].0, "notebook.ipynb");
        assert_eq!(paths[1].0, "notebook.pct.py");
        assert_eq!(paths[2].0, "notebook_lgt.py");

        let paths = paired_paths("notebook.pct.py", &pct, &formats).unwrap();
        assert_eq!(paths[0].0, "notebook.ipynb");
        assert_eq!(paths[1].0, "notebook.pct.py");
        assert_eq!(paths[2].0, "notebook_lgt.py");
    }

    #[test]
    fn test_many_and_suffix_wrong_suffix() {
        let ipynb = fmt(".ipynb");
        let pct = {
            let mut m = fmt(".py");
            m.insert("suffix".to_string(), ".pct".to_string());
            m
        };
        let lgt = {
            let mut m = fmt(".py");
            m.insert("suffix".to_string(), "_lgt".to_string());
            m
        };
        let py = fmt(".py");
        let formats = vec![ipynb.clone(), pct.clone(), lgt.clone()];

        let result = paired_paths("wrong_suffix.py", &py, &formats);
        assert!(result.is_err());
    }

    #[test]
    fn test_duplicated_paths() {
        let ipynb = fmt(".ipynb");
        let py_pct = fmt_with_name(".py", "percent");
        let py_light = fmt_with_name(".py", "light");
        let formats = vec![ipynb.clone(), py_pct, py_light];

        let result = paired_paths("notebook.ipynb", &ipynb, &formats);
        assert!(result.is_err());
    }

    #[test]
    fn test_prefix_on_root() {
        let ipynb = fmt(".ipynb");
        let py_pct = {
            let mut m = fmt_with_name(".py", "percent");
            m.insert("prefix".to_string(), "python/".to_string());
            m
        };
        let formats = vec![ipynb.clone(), py_pct.clone()];

        let paths = paired_paths("Untitled.ipynb", &ipynb, &formats).unwrap();
        assert_eq!(paths[0].0, "Untitled.ipynb");
        assert_eq!(paths[1].0, "python/Untitled.py");
    }

    #[test]
    fn test_roundtrip_base_full() {
        let formats = vec![fmt(".ipynb"), fmt(".py")];
        let bp = base_path("notebook.ipynb", &fmt(".ipynb"), &formats).unwrap();
        assert_eq!(bp, "notebook");
        let fp = full_path(&bp, &fmt(".py")).unwrap();
        assert_eq!(fp, "notebook.py");
    }

    #[test]
    fn test_base_path_wrong_extension() {
        let f = fmt(".py");
        let result = base_path("notebook.ipynb", &f, &[]);
        assert!(result.is_err());
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
    fn test_paired_path_with_prefix_scripts() {
        let ipynb = fmt(".ipynb");
        let py_pct = {
            let mut m = fmt_with_name(".py", "percent");
            m.insert("prefix".to_string(), "scripts/".to_string());
            m
        };
        let formats = vec![ipynb.clone(), py_pct.clone()];

        let paths = paired_paths("scripts/test.py", &py_pct, &formats).unwrap();
        assert_eq!(paths.len(), 2);
        assert_eq!(paths[0].0, "test.ipynb");
        assert_eq!(paths[1].0, "scripts/test.py");
    }
}

// =========================================================================
// 11. Integration-style tests (cross-module)
// =========================================================================

mod integration_tests {
    use super::*;

    #[test]
    fn test_notebook_with_cells_compare_equal() {
        let nb1 = Notebook {
            nbformat: 4,
            nbformat_minor: 5,
            metadata: BTreeMap::new(),
            cells: vec![
                Cell::new_markdown("First cell"),
                Cell::new_code("1 + 1"),
                Cell::new_markdown("Second cell"),
            ],
        };
        let nb2 = Notebook {
            nbformat: 4,
            nbformat_minor: 5,
            metadata: BTreeMap::new(),
            cells: vec![
                Cell::new_markdown("First cell"),
                Cell::new_code("1 + 1"),
                Cell::new_markdown("Second cell"),
            ],
        };
        assert!(compare_notebooks(&nb1, &nb2, None, true, true).is_ok());
    }

    #[test]
    fn test_notebook_with_cells_compare_different() {
        let nb1 = Notebook {
            nbformat: 4,
            nbformat_minor: 5,
            metadata: BTreeMap::new(),
            cells: vec![
                Cell::new_markdown("First cell"),
                Cell::new_code("1 + 1"),
                Cell::new_markdown("Second cell"),
            ],
        };
        let nb2 = Notebook {
            nbformat: 4,
            nbformat_minor: 5,
            metadata: BTreeMap::new(),
            cells: vec![
                Cell::new_markdown("First cell"),
                Cell::new_code("1 + 1"),
                Cell::new_markdown("Modified cell"),
            ],
        };
        assert!(compare_notebooks(&nb1, &nb2, None, true, true).is_err());
    }

    #[test]
    fn test_format_roundtrip_long_short() {
        // Test that long_form -> short_form is stable for common formats
        let format_strings = vec![
            "ipynb",
            "py:light",
            "py:percent",
            "md",
            ".pct.py:percent",
        ];
        for fs in format_strings {
            let long = long_form_one_format(fs, None, None, false).unwrap();
            let short = short_form_one_format(&long);
            assert_eq!(short, fs, "Roundtrip failed for {}", fs);
        }
    }

    #[test]
    fn test_magic_comment_uncomment_roundtrip() {
        let lines_to_test = vec![
            "%matplotlib inline",
            "%%HTML",
            "%autoreload",
            "!ls",
            "ls -al",
        ];
        for line in lines_to_test {
            let mut source = vec![line.to_string()];
            comment_magic(&mut source, "python", true, false);
            assert_ne!(source[0], line, "Should have been commented: {}", line);
            uncomment_magic(&mut source, "python", true, false);
            assert_eq!(source[0], line, "Roundtrip failed for: {}", line);
        }
    }

    #[test]
    fn test_string_parser_and_pep8_interaction() {
        // A function definition after code needs 2 blank lines
        let prev = vec!["x = 1".to_string()];
        let next = vec!["def f(x):".to_string(), "    return x".to_string()];
        assert_eq!(pep8_lines_between_cells(&prev, &next, ".py"), 2);
    }

    #[test]
    fn test_divine_format_markdown() {
        let text = "This is a markdown file\n\
                     with one code block\n\
                     \n\
                     ```\n\
                     1 + 1\n\
                     ```\n";
        assert_eq!(divine_format(text), "md");
    }

    #[test]
    fn test_filter_and_compare_interaction() {
        // A notebook with jupytext metadata should filter properly
        let mut meta = BTreeMap::new();
        meta.insert(
            "jupytext".to_string(),
            json!({"notebook_metadata_filter": "-all"}),
        );
        let nb = Notebook {
            nbformat: 4,
            nbformat_minor: 5,
            metadata: meta,
            cells: vec![Cell::new_code("1 + 1")],
        };
        let nb2 = Notebook {
            nbformat: 4,
            nbformat_minor: 5,
            metadata: BTreeMap::new(),
            cells: vec![Cell::new_code("1 + 1")],
        };
        // These should compare as equal since the jupytext metadata is filtered
        assert!(compare_notebooks(&nb, &nb2, None, true, true).is_ok());
    }
}
