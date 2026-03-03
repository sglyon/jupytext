//! PEP8 blank line calculation between cells

use crate::string_parser::StringParser;

/// Is the first non-empty, non-commented line a function or class definition?
fn next_instruction_is_function_or_class(lines: &[String]) -> bool {
    let mut parser = StringParser::new("python");
    for (i, line) in lines.iter().enumerate() {
        if parser.is_quoted() {
            parser.read_line(line);
            continue;
        }
        parser.read_line(line);
        if line.trim().is_empty() {
            if i > 0 && lines[i - 1].trim().is_empty() {
                return false;
            }
            continue;
        }
        if line.starts_with("def ")
            || line.starts_with("async ")
            || line.starts_with("class ")
        {
            return true;
        }
        if line.starts_with('#')
            || line.starts_with('@')
            || line.starts_with(' ')
            || line.starts_with(')')
        {
            continue;
        }
        return false;
    }
    false
}

/// Does the cell end with a function or class (indented code)?
fn cell_ends_with_function_or_class(lines: &[String]) -> bool {
    let mut non_quoted_lines = Vec::new();
    let mut parser = StringParser::new("python");
    for line in lines {
        if !parser.is_quoted() {
            non_quoted_lines.push(line.as_str());
        }
        parser.read_line(line);
    }

    let reversed: Vec<&str> = non_quoted_lines.iter().rev().copied().collect();
    for (i, line) in reversed.iter().enumerate() {
        if line.trim().is_empty() {
            if i > 0 && reversed[i - 1].trim().is_empty() {
                return false;
            }
            continue;
        }
        if line.starts_with('#')
            || line.starts_with(' ')
            || line.starts_with(')')
        {
            continue;
        }
        if line.starts_with("def ")
            || line.starts_with("async ")
            || line.starts_with("class ")
        {
            return true;
        }
        return false;
    }
    false
}

/// Is the last line of the cell a line with code?
fn cell_ends_with_code(lines: &[String]) -> bool {
    if lines.is_empty() {
        return false;
    }
    let last = lines.last().unwrap();
    if last.trim().is_empty() {
        return false;
    }
    if last.starts_with('#') {
        return false;
    }
    true
}

/// Does the cell have any code?
fn cell_has_code(lines: &[String]) -> bool {
    for (i, line) in lines.iter().enumerate() {
        let stripped = line.trim();
        if stripped.starts_with('#') {
            continue;
        }
        if stripped.is_empty() {
            if i > 0 && lines[i - 1].trim().is_empty() {
                return false;
            }
            continue;
        }
        return true;
    }
    false
}

/// How many blank lines between two cells for PEP8 compliance?
pub fn pep8_lines_between_cells(prev_lines: &[String], next_lines: &[String], ext: &str) -> usize {
    if next_lines.is_empty() {
        return 1;
    }
    if prev_lines.is_empty() {
        return 0;
    }
    if ext != ".py" {
        return 1;
    }
    if cell_ends_with_function_or_class(prev_lines) {
        return if cell_has_code(next_lines) { 2 } else { 1 };
    }
    if cell_ends_with_code(prev_lines) && next_instruction_is_function_or_class(next_lines) {
        return 2;
    }
    1
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_pep8_basic() {
        let prev = vec!["x = 1".to_string()];
        let next = vec!["y = 2".to_string()];
        assert_eq!(pep8_lines_between_cells(&prev, &next, ".py"), 1);
    }

    #[test]
    fn test_pep8_function_after_code() {
        let prev = vec!["x = 1".to_string()];
        let next = vec!["def foo():".to_string(), "    pass".to_string()];
        assert_eq!(pep8_lines_between_cells(&prev, &next, ".py"), 2);
    }

    #[test]
    fn test_pep8_non_python() {
        let prev = vec!["x = 1".to_string()];
        let next = vec!["y = 2".to_string()];
        assert_eq!(pep8_lines_between_cells(&prev, &next, ".jl"), 1);
    }
}
