//! Re-layouts JSON text without changing its tokens.
//!
//! Both transforms are driven by the token stream rather than by a parsed value,
//! so key order, number literals and string escapes are copied through
//! byte-for-byte. Minifying only ever removes whitespace between tokens, and
//! formatting only ever adds it.

use std::fmt;

/// The shape a selection should be rewritten into.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JsonSelectionMode {
    /// Remove every byte of whitespace between tokens.
    Minify,
    /// Break each nested value onto its own line.
    Pretty,
    /// Minify selections that already span several lines, format the rest.
    Toggle,
}

/// A selection that is not a single, complete, strict JSON value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InvalidJson {
    pub line: usize,
    pub column: usize,
    pub message: String,
}

impl fmt::Display for InvalidJson {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for InvalidJson {}

/// Rewrites `text`, which must already be trimmed of the whitespace that
/// surrounds the selection.
///
/// `indent_unit` is used for every level of nesting; it is the buffer's indent
/// for one level, either a tab or `tab_size` spaces.
pub fn transform(
    text: &str,
    mode: JsonSelectionMode,
    indent_unit: &str,
) -> Result<String, InvalidJson> {
    validate(text)?;

    let mode = match mode {
        JsonSelectionMode::Toggle if spans_multiple_lines(text) => JsonSelectionMode::Minify,
        JsonSelectionMode::Toggle => JsonSelectionMode::Pretty,
        mode => mode,
    };

    Ok(match mode {
        JsonSelectionMode::Minify => minify_tokens(text),
        _ => pretty_tokens(text, indent_unit),
    })
}

/// Whether the text has a line break between tokens.
fn spans_multiple_lines(text: &str) -> bool {
    // A line break can't appear inside a JSON string: it has to be escaped as
    // `\n`, which is two bytes and not an actual line break. So any line break
    // in a valid document separates tokens.
    text.contains('\n') || text.contains('\r')
}

fn validate(text: &str) -> Result<(), InvalidJson> {
    // Parsing into `RawValue` checks the syntax without building a value tree,
    // and its error carries the position of the problem.
    serde_json::from_str::<&serde_json::value::RawValue>(text)
        .map(drop)
        .map_err(|error| InvalidJson {
            line: error.line(),
            column: error.column(),
            message: error.to_string(),
        })
}

fn minify_tokens(text: &str) -> String {
    let mut tokens = Tokens::new(text);
    let mut minified = String::with_capacity(text.len());
    while let Some(token) = tokens.next_token() {
        minified.push_str(token);
    }
    minified
}

fn pretty_tokens(text: &str, indent_unit: &str) -> String {
    let mut tokens = Tokens::new(text);
    let mut pretty = String::with_capacity(text.len());
    let mut depth = 0usize;
    // Set right after `{` or `[`, so that the first child is the only token that
    // has to introduce a new line, and an empty container stays on one line.
    let mut after_open = false;

    while let Some(token) = tokens.next_token() {
        match token {
            "{" | "[" => {
                pretty.push_str(token);
                depth += 1;
                after_open = true;
            }
            "}" | "]" => {
                depth = depth.saturating_sub(1);
                if !after_open {
                    pretty.push('\n');
                    push_indent(&mut pretty, depth, indent_unit);
                }
                pretty.push_str(token);
                after_open = false;
            }
            "," => {
                pretty.push(',');
                pretty.push('\n');
                push_indent(&mut pretty, depth, indent_unit);
                after_open = false;
            }
            ":" => {
                pretty.push_str(": ");
                after_open = false;
            }
            value => {
                if after_open {
                    pretty.push('\n');
                    push_indent(&mut pretty, depth, indent_unit);
                    after_open = false;
                }
                pretty.push_str(value);
            }
        }
    }

    pretty
}

fn push_indent(output: &mut String, depth: usize, indent_unit: &str) {
    for _ in 0..depth {
        output.push_str(indent_unit);
    }
}

/// Splits JSON text into tokens, skipping the whitespace between them.
struct Tokens<'a> {
    text: &'a str,
    bytes: &'a [u8],
    position: usize,
}

impl<'a> Tokens<'a> {
    fn new(text: &'a str) -> Self {
        Self {
            text,
            bytes: text.as_bytes(),
            position: 0,
        }
    }

    fn next_token(&mut self) -> Option<&'a str> {
        while let Some(byte) = self.bytes.get(self.position) {
            if !matches!(byte, b' ' | b'\t' | b'\n' | b'\r') {
                break;
            }
            self.position += 1;
        }

        let start = self.position;
        match *self.bytes.get(start)? {
            b'"' => {
                self.position += 1;
                while let Some(byte) = self.bytes.get(self.position) {
                    self.position += 1;
                    match byte {
                        // Skip whichever byte the backslash escapes.
                        b'\\' => self.position += 1,
                        b'"' => break,
                        _ => {}
                    }
                }
            }
            b'{' | b'}' | b'[' | b']' | b':' | b',' => {
                self.position += 1;
            }
            b'-' | b'0'..=b'9' => {
                self.position += 1;
                while matches!(
                    self.bytes.get(self.position),
                    Some(b'0'..=b'9' | b'.' | b'e' | b'E' | b'+' | b'-')
                ) {
                    self.position += 1;
                }
            }
            _ => {
                // `true`, `false` and `null` are the only remaining tokens.
                self.position += 1;
                while matches!(self.bytes.get(self.position), Some(b'a'..=b'z')) {
                    self.position += 1;
                }
            }
        }

        self.text.get(start..self.position)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn minify(text: &str) -> String {
        transform(text, JsonSelectionMode::Minify, "  ").expect("valid JSON")
    }

    fn pretty(text: &str) -> String {
        transform(text, JsonSelectionMode::Pretty, "  ").expect("valid JSON")
    }

    fn toggle(text: &str) -> String {
        transform(text, JsonSelectionMode::Toggle, "  ").expect("valid JSON")
    }

    #[test]
    fn minify_removes_whitespace_between_tokens() {
        assert_eq!(minify("{ \"a\" : [ 1 , 2 ] }\n"), r#"{"a":[1,2]}"#);
        assert_eq!(minify("{\"a\":{\"b\":{}}}"), r#"{"a":{"b":{}}}"#);
        assert_eq!(minify("[\n  1,\n  2\n]"), "[1,2]");
    }

    #[test]
    fn minify_preserves_tokens_byte_for_byte() {
        // Key order, number literals and escapes would all be rewritten by a
        // round trip through `serde_json::Value`.
        assert_eq!(
            minify(r#"{ "z": 1.0, "a": -0, "m": 1e999, "b": 12345678901234567890 }"#),
            r#"{"z":1.0,"a":-0,"m":1e999,"b":12345678901234567890}"#
        );
        assert_eq!(
            minify("{\n  \"escaped\": \"a\\u00e9\\n\\t\\\\\\\"b\"\n}"),
            r#"{"escaped":"a\u00e9\n\t\\\"b"}"#
        );
    }

    #[test]
    fn pretty_indents_nested_values() {
        assert_eq!(
            pretty(r#"{"a":[1,{"b":2}],"c":true}"#),
            "{\n  \"a\": [\n    1,\n    {\n      \"b\": 2\n    }\n  ],\n  \"c\": true\n}"
        );
    }

    #[test]
    fn pretty_keeps_empty_containers_on_one_line() {
        assert_eq!(
            pretty(r#"{"a":{},"b":[]}"#),
            "{\n  \"a\": {},\n  \"b\": []\n}"
        );
        assert_eq!(pretty("[]"), "[]");
        assert_eq!(pretty("{}"), "{}");
    }

    #[test]
    fn pretty_uses_the_given_indent_unit() {
        assert_eq!(
            transform(r#"{"a":1}"#, JsonSelectionMode::Pretty, "\t").expect("valid JSON"),
            "{\n\t\"a\": 1\n}"
        );
        assert_eq!(
            transform(r#"{"a":1}"#, JsonSelectionMode::Pretty, "    ").expect("valid JSON"),
            "{\n    \"a\": 1\n}"
        );
    }

    #[test]
    fn pretty_handles_top_level_scalars() {
        assert_eq!(pretty("  42  "), "42");
        assert_eq!(pretty("\"a b\""), "\"a b\"");
        assert_eq!(pretty("null"), "null");
    }

    #[test]
    fn both_modes_are_idempotent() {
        let text = r#"{"a":[1,{"b":null}],"c":"d"}"#;
        let formatted = pretty(text);
        assert_eq!(pretty(&formatted), formatted);
        let minified = minify(&formatted);
        assert_eq!(minify(&minified), minified);
        assert_eq!(minified, minify(text));
    }

    #[test]
    fn toggle_minifies_multiline_selections_only() {
        assert_eq!(toggle("{\n  \"a\": 1\n}"), r#"{"a":1}"#);
        assert_eq!(toggle(r#"{"a":1}"#), "{\n  \"a\": 1\n}");
        // A line break inside a string is escaped, so this still counts as one line.
        assert_eq!(toggle(r#"{"a":"x\ny"}"#), "{\n  \"a\": \"x\\ny\"\n}");
        // Toggling twice returns to the original shape.
        assert_eq!(toggle(&toggle("{\n  \"a\": 1\n}")), "{\n  \"a\": 1\n}");
    }

    #[test]
    fn rejects_incomplete_values() {
        for text in [
            r#"{"a":1,}"#,
            r#"{"a":1"#,
            r#""a":1, "b":2"#,
            r#"{"a":1} {}"#,
            "{ /* comment */ \"a\": 1 }",
            r#"{"a": 'single'}"#,
            "",
            "   ",
        ] {
            let error =
                transform(text, JsonSelectionMode::Minify, "  ").expect_err("should be rejected");
            assert!(!error.message.is_empty(), "{text:?}");
        }
    }

    #[test]
    fn reports_the_position_of_the_problem() {
        let error = transform("{\n  \"a\": 1,\n}\n", JsonSelectionMode::Pretty, "  ")
            .expect_err("rejected");
        assert_eq!(error.line, 3);
        assert!(error.message.contains("line 3"), "{}", error.message);
    }
}
