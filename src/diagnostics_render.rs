//! Shared diagnostic-location parsing and source-snippet rendering, used by
//! both the CLI's error output (`gx run`/`gx check`/`gx build`/...) and the
//! LSP's `publishDiagnostics`.
//!
//! GX's parser/lexer/interpreter errors are plain `String`s, not a
//! structured error type — threading `(line, col, message)` as real fields
//! through every one of the hundreds of existing `Result<_, String>` call
//! sites across the parser, lexer, and interpreter would be a large, risky
//! refactor for this pass. Instead, error *construction* already follows a
//! consistent textual convention (`"Line N: message"` or `"Line N, col C:
//! message"` — see `Parser::err_here` and the lexer's error sites), and this
//! module parses that convention back into structured data at the two
//! places that actually need it, leaving every internal call site
//! untouched. If a message doesn't follow the convention (or the reported
//! location is out of range), callers fall back to the raw message
//! unchanged — this must never hide information a stricter parser would
//! have shown.

/// A location parsed out of an error message's leading `"Line N"` or
/// `"Line N, col C"` prefix, plus the message text that followed it.
#[derive(Debug, Clone, PartialEq)]
pub struct ParsedLocation {
    /// 1-based source line.
    pub line: usize,
    /// 1-based source column, when the message embedded one.
    pub col: Option<usize>,
    /// The message text after the `"Line N[, col C]: "` prefix.
    pub message: String,
}

/// Parse a location out of `s` — either the `"Line N: msg"` / `"Line N,
/// col C: msg"` prefix convention parser/lexer errors use, or the
/// `"{msg} at line N[\n  in {call stack}]"` suffix convention `run_stmt`'s
/// wrapper attaches to runtime errors (see `Interpreter::run_stmt`).
/// Returns `None` if `s` matches neither — most callers should print `s`
/// verbatim in that case rather than treat it as an error.
pub fn parse_location(s: &str) -> Option<ParsedLocation> {
    parse_prefix_location(s).or_else(|| parse_suffix_location(s))
}

/// Parse a `"{msg} at line N"` suffix, tolerating trailing text after the
/// number (the call-stack context `run_stmt` appends as `"\n  in {...}"`)
/// by folding it back into `message` rather than discarding it — the
/// snippet then sits below the message *and* its call-stack context,
/// rather than swallowing one to show the other.
fn parse_suffix_location(s: &str) -> Option<ParsedLocation> {
    let marker = " at line ";
    let idx = s.find(marker)?;
    let before = &s[..idx];
    let after = &s[idx + marker.len()..];
    let digit_end = after
        .find(|c: char| !c.is_ascii_digit())
        .unwrap_or(after.len());
    if digit_end == 0 {
        return None;
    }
    let line: usize = after[..digit_end].parse().ok()?;
    let rest = &after[digit_end..];
    let message = if rest.trim().is_empty() {
        before.to_string()
    } else {
        format!("{}{}", before, rest)
    };
    Some(ParsedLocation {
        line,
        col: None,
        message,
    })
}

/// Parse a `"Line N: msg"` or `"Line N, col C: msg"` prefix from `s`,
/// tolerating a leading `"<path>: "` prefix (the shape `main.rs` builds via
/// `format!("{}: {}", path, e)`) before it.
fn parse_prefix_location(s: &str) -> Option<ParsedLocation> {
    // The "Line " marker can appear either at the very start, or after a
    // "<path>: " prefix. Rather than trying to characterize what a path can
    // look like, just search for the first occurrence of "Line " that is
    // immediately followed by a digit — good enough for the message shapes
    // this crate actually produces, and a false miss just means falling
    // back to the unparsed message, never a wrong location.
    let idx = find_line_marker(s)?;
    let rest = &s[idx + "Line ".len()..];

    let (line_str, after_line) = rest.split_once([':', ','])?;
    let line: usize = line_str.trim().parse().ok()?;
    let is_comma = rest.as_bytes()[line_str.len()] == b',';

    if is_comma {
        // "col C: msg"
        let after_col_kw = after_line.trim_start();
        let after_col_kw = after_col_kw.strip_prefix("col ")?;
        let (col_str, after_col) = after_col_kw.split_once(':')?;
        let col: usize = col_str.trim().parse().ok()?;
        Some(ParsedLocation {
            line,
            col: Some(col),
            message: after_col.trim_start().to_string(),
        })
    } else {
        Some(ParsedLocation {
            line,
            col: None,
            message: after_line.trim_start().to_string(),
        })
    }
}

fn find_line_marker(s: &str) -> Option<usize> {
    let bytes = s.as_bytes();
    let mut search_from = 0;
    while let Some(rel) = s[search_from..].find("Line ") {
        let idx = search_from + rel;
        let digit_pos = idx + "Line ".len();
        if bytes.get(digit_pos).is_some_and(u8::is_ascii_digit) {
            return Some(idx);
        }
        search_from = idx + "Line ".len();
    }
    None
}

/// Render `raw` (an error/message string) with a source snippet and — when
/// a column was embedded — a caret pointing at the offending position,
/// Rust-compiler style. Falls back to returning `raw` completely unchanged
/// if it doesn't parse as a location, or the reported line is out of
/// `source`'s range (e.g. an "unexpected end of file" error past the last
/// line) — never hides the original message.
pub fn render_diagnostic(raw: &str, path: &str, source: &str) -> String {
    let Some(loc) = parse_location(raw) else {
        return raw.to_string();
    };
    let lines: Vec<&str> = source.lines().collect();
    let Some(src_line) = loc.line.checked_sub(1).and_then(|i| lines.get(i)) else {
        return raw.to_string();
    };

    let gutter = format!("{}", loc.line).len().max(2);
    let mut out = String::new();
    // Deliberately doesn't include a leading "error: {message}" line —
    // every caller already has its own "Error: "/"Warning: " convention
    // (the CLI's `eprintln!("Error: {}", ...)`, the LSP's `severity`
    // field); prepending one here would either duplicate it or fight it.
    out.push_str(&format!("{}\n", loc.message));
    out.push_str(&format!(
        "{:gutter$}--> {}:{}{}\n",
        "",
        path,
        loc.line,
        loc.col.map(|c| format!(":{}", c)).unwrap_or_default(),
        gutter = gutter
    ));
    out.push_str(&format!("{:gutter$} |\n", "", gutter = gutter));
    out.push_str(&format!(
        "{:>gutter$} | {}\n",
        loc.line,
        src_line,
        gutter = gutter
    ));
    if let Some(col) = loc.col {
        let caret_pad = " ".repeat(col.saturating_sub(1));
        out.push_str(&format!(
            "{:gutter$} | {}^\n",
            "",
            caret_pad,
            gutter = gutter
        ));
    } else {
        out.push_str(&format!("{:gutter$} |\n", "", gutter = gutter));
    }
    // Trim the trailing newline — callers (eprintln!, LSP message bodies)
    // add their own line ending.
    out.pop();
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_line_only_messages() {
        let loc = parse_location("Line 12: expected RBrace, got Eof").unwrap();
        assert_eq!(loc.line, 12);
        assert_eq!(loc.col, None);
        assert_eq!(loc.message, "expected RBrace, got Eof");
    }

    #[test]
    fn parses_line_and_col_messages() {
        let loc = parse_location("Line 5, col 3: expected identifier, got Eq").unwrap();
        assert_eq!(loc.line, 5);
        assert_eq!(loc.col, Some(3));
        assert_eq!(loc.message, "expected identifier, got Eq");
    }

    #[test]
    fn parses_a_path_prefixed_message() {
        let loc = parse_location("script.gx: Line 5, col 3: expected identifier").unwrap();
        assert_eq!(loc.line, 5);
        assert_eq!(loc.col, Some(3));
        assert_eq!(loc.message, "expected identifier");
    }

    #[test]
    fn returns_none_for_a_message_with_no_line_marker() {
        assert!(parse_location("cannot read file: permission denied").is_none());
    }

    #[test]
    fn parses_the_runtime_error_suffix_convention() {
        // The shape `run_stmt`'s wrapper produces for an uncaught
        // Signal::Error: "{msg} at line {N}".
        let loc = parse_location("undefined function 'foo' at line 7").unwrap();
        assert_eq!(loc.line, 7);
        assert_eq!(loc.col, None);
        assert_eq!(loc.message, "undefined function 'foo'");
    }

    #[test]
    fn parses_the_runtime_error_suffix_convention_with_a_call_stack_tail() {
        let loc =
            parse_location("undefined function 'foo' at line 7\n  in agent \"demo\"").unwrap();
        assert_eq!(loc.line, 7);
        assert_eq!(loc.message, "undefined function 'foo'\n  in agent \"demo\"");
    }

    #[test]
    fn path_prefixed_convention_is_tried_before_the_runtime_suffix_convention() {
        // Both conventions could theoretically match the same string if a
        // runtime error's own text happened to contain "Line N:" — the
        // prefix form (parser/lexer errors) is checked first since it
        // carries more precise (often column-level) location data.
        let loc = parse_location("script.gx: Line 5, col 3: expected identifier").unwrap();
        assert_eq!(loc.col, Some(3));
    }

    #[test]
    fn does_not_misfire_on_an_unrelated_capital_l_word() {
        // "Login" contains "Log" but not "Line " — must not match. More
        // importantly, a message that merely *contains* the substring
        // "Line" without being followed by a space+digit must not parse.
        assert!(parse_location("Line item not found").is_none());
    }

    #[test]
    fn render_diagnostic_shows_the_offending_source_line() {
        let source = "x = 1\ny = 2\nif x >\n  z = 3\n";
        let out = render_diagnostic("Line 3, col 7: expected expression", "t.gx", source);
        assert!(out.contains("t.gx:3:7"));
        assert!(out.contains("if x >"));
        assert!(out.contains('^'));
    }

    #[test]
    fn render_diagnostic_falls_back_when_line_is_out_of_range() {
        let source = "x = 1\n";
        let raw = "Line 99: unexpected end of file";
        assert_eq!(render_diagnostic(raw, "t.gx", source), raw);
    }

    #[test]
    fn render_diagnostic_falls_back_when_message_has_no_location() {
        let raw = "cannot read file: permission denied";
        assert_eq!(render_diagnostic(raw, "t.gx", "x = 1\n"), raw);
    }

    #[test]
    fn render_diagnostic_without_a_column_omits_the_caret_but_still_shows_the_line() {
        let source = "x = 1\nif x\n";
        let out = render_diagnostic("Line 2: unclosed block", "t.gx", source);
        assert!(out.contains("t.gx:2"));
        assert!(out.contains("if x"));
        assert!(!out.contains('^'));
    }
}
