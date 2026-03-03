//! A simple parser that tracks whether code is inside a quoted string

use crate::languages::COMMENT_FOR_LANG;

/// Tracks whether the current position is inside a quoted string
pub struct StringParser {
    ignore: bool,
    python: bool,
    comment: Option<String>,
    single: Option<char>,
    triple: Option<char>,
    triple_start: i64,
}

impl StringParser {
    pub fn new(language: &str) -> Self {
        let comment = COMMENT_FOR_LANG.get(language).map(|s| s.to_string());
        StringParser {
            ignore: false,
            python: language != "R",
            comment,
            single: None,
            triple: None,
            triple_start: -1,
        }
    }

    pub fn new_opt(language: Option<&str>) -> Self {
        match language {
            Some(lang) => Self::new(lang),
            None => StringParser {
                ignore: true,
                python: true,
                comment: None,
                single: None,
                triple: None,
                triple_start: -1,
            },
        }
    }

    /// Is the next line inside a quoted string?
    pub fn is_quoted(&self) -> bool {
        if self.ignore {
            return false;
        }
        self.single.is_some() || self.triple.is_some()
    }

    /// Process a new line
    pub fn read_line(&mut self, line: &str) {
        if self.ignore {
            return;
        }

        // Don't search for quotes when line is commented and not in a quoted string
        if !self.is_quoted() {
            if let Some(ref comment) = self.comment {
                if line.trim_start().starts_with(comment.as_str()) {
                    return;
                }
            }
        }

        self.triple_start = -1;
        let chars: Vec<char> = line.chars().collect();

        for (i, &ch) in chars.iter().enumerate() {
            // Check for comment start
            if self.single.is_none() && self.triple.is_none() {
                if let Some(ref comment) = self.comment {
                    if comment.starts_with(ch) && line[i..].starts_with(comment.as_str()) {
                        break;
                    }
                }
            }

            if ch != '"' && ch != '\'' {
                continue;
            }

            // Check if escaped
            if i > 0 && chars[i - 1] == '\\' {
                continue;
            }

            // Check if this ends a single quote
            if self.single == Some(ch) {
                self.single = None;
                continue;
            }
            if self.single.is_some() {
                continue;
            }

            if !self.python {
                continue;
            }

            // Check for triple quote
            if i >= 2
                && chars[i - 1] == ch
                && chars[i - 2] == ch
                && (i as i64) >= self.triple_start + 3
            {
                if self.triple == Some(ch) {
                    self.triple = None;
                    self.triple_start = i as i64;
                    continue;
                }

                if self.triple.is_some() {
                    continue;
                }

                // Triple quote starting
                self.triple = Some(ch);
                self.triple_start = i as i64;
                continue;
            }

            // Inside a multiline quote
            if self.triple.is_some() {
                continue;
            }

            self.single = Some(ch);
        }

        // Line ended - single quotes don't carry across lines in Python
        if self.python {
            self.single = None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_simple_string() {
        let mut parser = StringParser::new("python");
        parser.read_line("x = 'hello'");
        assert!(!parser.is_quoted());
    }

    #[test]
    fn test_triple_quote_multiline() {
        let mut parser = StringParser::new("python");
        parser.read_line("x = \"\"\"hello");
        assert!(parser.is_quoted());
        parser.read_line("world\"\"\"");
        assert!(!parser.is_quoted());
    }

    #[test]
    fn test_comment_line() {
        let mut parser = StringParser::new("python");
        parser.read_line("# x = 'hello");
        assert!(!parser.is_quoted());
    }

    #[test]
    fn test_r_parser() {
        let mut parser = StringParser::new("R");
        parser.read_line("x <- 'hello'");
        // R single quotes don't span lines but the parser tracks them within lines
        assert!(!parser.is_quoted());
    }
}
