//! XML builtins — `xml_parse`/`xml_stringify`.
//!
//! A deliberately narrow, hand-written parser rather than a full XML 1.0
//! implementation or an added dependency:
//!
//! - **No DTD/entity processing at all.** A `<!DOCTYPE ...>` declaration
//!   (including any internal `[ ... ]` subset, which is where a malicious
//!   document would define custom or external entities) is skipped as
//!   opaque text and never interpreted. Only the five predefined XML
//!   entities (`&amp;` `&lt;` `&gt;` `&quot;` `&apos;`) and numeric
//!   character references (`&#NNN;` / `&#xHH;`) are recognized — anything
//!   else (`&xxe;`, a custom entity a document tried to define) is a parse
//!   *error*, not a silent pass-through or expansion. This isn't a
//!   trimmed-down feature — it's the standard, recommended defense
//!   against XXE (XML External Entity) injection and "billion laughs"
//!   entity-expansion denial-of-service: the only way to be safe against
//!   an attack built entirely out of entity definitions is to never
//!   resolve entity definitions in the first place.
//! - **Mixed content is not preserved.** An element with both text *and*
//!   child elements interleaved (`<p>Hello <b>world</b>!</p>`) collapses
//!   into separate `text` and `children` fields, losing their relative
//!   order — sufficient for the common case this exists for (config
//!   files, simple data/API documents), not a document/markup format.
//! - **Recursion depth is capped** (see `MAX_DEPTH`) so a deeply nested
//!   malicious document is a clean parse error, not a stack overflow.
//!
//! An element parses to `{ tag, attrs, children, text }` — `attrs` an
//! object, `children` an array of same-shaped elements, `text` the
//! concatenated direct (non-child) text content, whitespace-trimmed.

use super::Signal;
use crate::value::Value;
use std::collections::HashMap;

const MAX_DEPTH: usize = 512;

// ── Parsing ──────────────────────────────────────────────────────────────────

struct Parser {
    chars: Vec<char>,
    pos: usize,
}

impl Parser {
    fn new(src: &str) -> Self {
        Parser {
            chars: src.chars().collect(),
            pos: 0,
        }
    }

    fn peek(&self) -> Option<char> {
        self.chars.get(self.pos).copied()
    }

    fn peek_at(&self, offset: usize) -> Option<char> {
        self.chars.get(self.pos + offset).copied()
    }

    fn starts_with(&self, s: &str) -> bool {
        s.chars()
            .enumerate()
            .all(|(i, c)| self.peek_at(i) == Some(c))
    }

    fn advance(&mut self) -> Option<char> {
        let c = self.peek();
        if c.is_some() {
            self.pos += 1;
        }
        c
    }

    fn skip_ws(&mut self) {
        while matches!(self.peek(), Some(c) if c.is_whitespace()) {
            self.pos += 1;
        }
    }

    fn err(&self, msg: &str) -> String {
        format!("xml_parse: {} (at character {})", msg, self.pos)
    }

    /// Skip whitespace, comments (`<!-- ... -->`), the XML declaration
    /// (`<?xml ... ?>`), and any processing instruction (`<?...?>`) or
    /// DOCTYPE declaration (`<!DOCTYPE ...>`, including a bracketed
    /// internal subset) — all as opaque text, never interpreted. This is
    /// what keeps entity definitions from ever being registered at all.
    fn skip_misc(&mut self) -> Result<(), String> {
        loop {
            self.skip_ws();
            if self.starts_with("<!--") {
                self.pos += 4;
                self.skip_until("-->")?;
            } else if self.starts_with("<?") {
                self.pos += 2;
                self.skip_until("?>")?;
            } else if self.starts_with("<!DOCTYPE") || self.starts_with("<!doctype") {
                self.pos += 9;
                self.skip_doctype()?;
            } else {
                break;
            }
        }
        Ok(())
    }

    fn skip_until(&mut self, terminator: &str) -> Result<(), String> {
        while !self.starts_with(terminator) {
            if self.advance().is_none() {
                return Err(self.err(&format!("unterminated (expected '{}')", terminator)));
            }
        }
        for _ in terminator.chars() {
            self.advance();
        }
        Ok(())
    }

    /// Skip a DOCTYPE declaration, tracking bracket depth so a `[ ... ]`
    /// internal subset (where entity definitions would live) is skipped
    /// as a single opaque unit rather than needing its own parser.
    fn skip_doctype(&mut self) -> Result<(), String> {
        let mut bracket_depth = 0i32;
        loop {
            match self.advance() {
                None => return Err(self.err("unterminated <!DOCTYPE ...>")),
                Some('[') => bracket_depth += 1,
                Some(']') => bracket_depth -= 1,
                Some('>') if bracket_depth <= 0 => return Ok(()),
                _ => {}
            }
        }
    }

    fn parse_name(&mut self) -> Result<String, String> {
        let start = self.pos;
        while matches!(self.peek(), Some(c) if c.is_alphanumeric() || "_-.:".contains(c)) {
            self.pos += 1;
        }
        if self.pos == start {
            return Err(self.err("expected an element or attribute name"));
        }
        Ok(self.chars[start..self.pos].iter().collect())
    }

    /// Decode entities/character references in `raw` text (attribute
    /// values and text content) — see the module doc comment for exactly
    /// which ones, and why nothing else is supported.
    fn decode_entities(&self, raw: &str) -> Result<String, String> {
        let mut out = String::with_capacity(raw.len());
        let mut chars = raw.chars().peekable();
        while let Some(c) = chars.next() {
            if c != '&' {
                out.push(c);
                continue;
            }
            let mut entity = String::new();
            let mut closed = false;
            for ec in chars.by_ref() {
                if ec == ';' {
                    closed = true;
                    break;
                }
                entity.push(ec);
                if entity.len() > 12 {
                    break; // no legitimate entity/char-ref is this long
                }
            }
            if !closed {
                return Err(format!(
                    "xml_parse: unterminated entity reference '&{}'",
                    entity
                ));
            }
            match entity.as_str() {
                "amp" => out.push('&'),
                "lt" => out.push('<'),
                "gt" => out.push('>'),
                "quot" => out.push('"'),
                "apos" => out.push('\''),
                _ if entity.starts_with("#x") || entity.starts_with("#X") => {
                    let code = u32::from_str_radix(&entity[2..], 16).map_err(|_| {
                        format!(
                            "xml_parse: invalid numeric character reference '&{};'",
                            entity
                        )
                    })?;
                    out.push(char::from_u32(code).ok_or_else(|| {
                        format!(
                            "xml_parse: invalid numeric character reference '&{};'",
                            entity
                        )
                    })?);
                }
                _ if entity.starts_with('#') => {
                    let code = entity[1..].parse::<u32>().map_err(|_| {
                        format!(
                            "xml_parse: invalid numeric character reference '&{};'",
                            entity
                        )
                    })?;
                    out.push(char::from_u32(code).ok_or_else(|| {
                        format!(
                            "xml_parse: invalid numeric character reference '&{};'",
                            entity
                        )
                    })?);
                }
                other => {
                    // Deliberately not supported — see module doc comment.
                    // A custom/external entity is rejected, not silently
                    // dropped or passed through.
                    return Err(format!(
                        "xml_parse: unsupported entity '&{};' — only &amp; &lt; &gt; &quot; &apos; and numeric character references (&#NNN; / &#xHH;) are supported",
                        other
                    ));
                }
            }
        }
        Ok(out)
    }

    fn parse_attrs(&mut self) -> Result<HashMap<String, Value>, String> {
        let mut attrs = HashMap::new();
        loop {
            self.skip_ws();
            match self.peek() {
                Some(c) if c == '/' || c == '>' => break,
                None => return Err(self.err("unterminated start tag")),
                _ => {}
            }
            let name = self.parse_name()?;
            self.skip_ws();
            if self.peek() != Some('=') {
                return Err(self.err(&format!("expected '=' after attribute '{}'", name)));
            }
            self.advance();
            self.skip_ws();
            let quote = match self.advance() {
                Some(q @ ('"' | '\'')) => q,
                _ => {
                    return Err(
                        self.err(&format!("expected a quoted value for attribute '{}'", name))
                    )
                }
            };
            let start = self.pos;
            while self.peek() != Some(quote) {
                if self.advance().is_none() {
                    return Err(self.err("unterminated attribute value"));
                }
            }
            let raw: String = self.chars[start..self.pos].iter().collect();
            self.advance(); // closing quote
            attrs.insert(name, Value::Str(self.decode_entities(&raw)?));
        }
        Ok(attrs)
    }

    fn parse_element(&mut self, depth: usize) -> Result<Value, String> {
        if depth > MAX_DEPTH {
            return Err(self.err("exceeded maximum nesting depth"));
        }
        if self.advance() != Some('<') {
            return Err(self.err("expected '<'"));
        }
        let tag = self.parse_name()?;
        self.skip_ws();
        let attrs = self.parse_attrs()?;
        self.skip_ws();

        if self.starts_with("/>") {
            self.pos += 2;
            return Ok(xml_element_value(tag, attrs, Vec::new(), String::new()));
        }
        if self.advance() != Some('>') {
            return Err(self.err("expected '>' or '/>'"));
        }

        let mut children = Vec::new();
        let mut text = String::new();
        loop {
            if self.starts_with("</") {
                self.pos += 2;
                let close_tag = self.parse_name()?;
                if close_tag != tag {
                    return Err(self.err(&format!(
                        "mismatched closing tag: expected </{}>, found </{}>",
                        tag, close_tag
                    )));
                }
                self.skip_ws();
                if self.advance() != Some('>') {
                    return Err(self.err("expected '>' to close the end tag"));
                }
                break;
            } else if self.starts_with("<!--") {
                self.pos += 4;
                self.skip_until("-->")?;
            } else if self.starts_with("<![CDATA[") {
                self.pos += 9;
                let start = self.pos;
                self.skip_until("]]>")?;
                let end = self.pos - 3;
                text.push_str(&self.chars[start..end].iter().collect::<String>());
            } else if self.peek() == Some('<') {
                children.push(self.parse_element(depth + 1)?);
            } else if self.peek().is_none() {
                return Err(self.err(&format!("unterminated element <{}>", tag)));
            } else {
                let start = self.pos;
                while !matches!(self.peek(), Some('<') | None) {
                    self.pos += 1;
                }
                let raw: String = self.chars[start..self.pos].iter().collect();
                text.push_str(&self.decode_entities(&raw)?);
            }
        }

        Ok(xml_element_value(
            tag,
            attrs,
            children,
            text.trim().to_string(),
        ))
    }
}

fn xml_element_value(
    tag: String,
    attrs: HashMap<String, Value>,
    children: Vec<Value>,
    text: String,
) -> Value {
    let mut obj = HashMap::new();
    obj.insert("tag".to_string(), Value::Str(tag));
    obj.insert("attrs".to_string(), Value::Object(attrs));
    obj.insert("children".to_string(), Value::Array(children));
    obj.insert("text".to_string(), Value::Str(text));
    Value::Object(obj)
}

/// xml_parse(text) → { tag, attrs, children, text }
pub fn xml_parse_impl(args: &[Value]) -> Result<Value, Signal> {
    let text = args
        .first()
        .and_then(|v| v.as_str().map(String::from))
        .ok_or_else(|| Signal::Error("xml_parse(text)".into()))?;

    let mut parser = Parser::new(&text);
    parser.skip_misc().map_err(Signal::Error)?;
    if parser.peek().is_none() {
        return Err(Signal::Error("xml_parse: no root element found".into()));
    }
    let root = parser.parse_element(0).map_err(Signal::Error)?;
    parser.skip_misc().map_err(Signal::Error)?;
    // Trailing non-whitespace content after the root element's closing
    // tag means this wasn't a single well-formed document.
    if parser.peek().is_some() {
        return Err(Signal::Error(
            "xml_parse: unexpected content after the root element".into(),
        ));
    }
    Ok(root)
}

// ── Stringifying ─────────────────────────────────────────────────────────────

fn escape_xml_text(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

fn escape_xml_attr(s: &str) -> String {
    escape_xml_text(s).replace('"', "&quot;")
}

fn stringify_element(v: &Value, out: &mut String, depth: usize) -> Result<(), String> {
    if depth > MAX_DEPTH {
        return Err("xml_stringify: exceeded maximum nesting depth".into());
    }
    let Value::Object(obj) = v else {
        return Err(
            "xml_stringify: expected an element object ({ tag, attrs?, children?, text? })".into(),
        );
    };
    let tag = obj
        .get("tag")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "xml_stringify: element is missing a 'tag' (string) field".to_string())?;

    out.push('<');
    out.push_str(tag);
    if let Some(Value::Object(attrs)) = obj.get("attrs") {
        let mut keys: Vec<&String> = attrs.keys().collect();
        keys.sort(); // deterministic output, not dependent on HashMap iteration order
        for k in keys {
            out.push(' ');
            out.push_str(k);
            out.push_str("=\"");
            out.push_str(&escape_xml_attr(&attrs[k].to_string()));
            out.push('"');
        }
    }

    let children = match obj.get("children") {
        Some(Value::Array(c)) => c.as_slice(),
        _ => &[],
    };
    let text = obj.get("text").map(|v| v.to_string()).unwrap_or_default();

    if children.is_empty() && text.is_empty() {
        out.push_str("/>");
        return Ok(());
    }
    out.push('>');
    out.push_str(&escape_xml_text(&text));
    for child in children {
        stringify_element(child, out, depth + 1)?;
    }
    out.push_str("</");
    out.push_str(tag);
    out.push('>');
    Ok(())
}

/// xml_stringify({ tag, attrs?, children?, text? }) → text
pub fn xml_stringify_impl(args: &[Value]) -> Result<Value, Signal> {
    let root = args.first().cloned().unwrap_or(Value::Null);
    let mut out = String::new();
    stringify_element(&root, &mut out, 0).map_err(Signal::Error)?;
    Ok(Value::Str(out))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(s: &str) -> Result<Value, Signal> {
        xml_parse_impl(&[Value::Str(s.to_string())])
    }

    fn field<'a>(v: &'a Value, key: &str) -> &'a Value {
        match v {
            Value::Object(m) => m.get(key).unwrap_or(&Value::Null),
            _ => panic!("expected an object"),
        }
    }

    #[test]
    fn parses_a_simple_element() {
        let v = parse("<root>hello</root>").unwrap();
        assert_eq!(field(&v, "tag").as_str(), Some("root"));
        assert_eq!(field(&v, "text").as_str(), Some("hello"));
    }

    #[test]
    fn parses_attributes() {
        let v = parse(r#"<user id="42" active="true"/>"#).unwrap();
        let attrs = field(&v, "attrs");
        assert_eq!(field(attrs, "id").as_str(), Some("42"));
        assert_eq!(field(attrs, "active").as_str(), Some("true"));
    }

    #[test]
    fn parses_nested_children() {
        let v = parse("<root><a>1</a><b>2</b></root>").unwrap();
        let Value::Array(children) = field(&v, "children") else {
            panic!("expected array");
        };
        assert_eq!(children.len(), 2);
        assert_eq!(field(&children[0], "tag").as_str(), Some("a"));
        assert_eq!(field(&children[0], "text").as_str(), Some("1"));
        assert_eq!(field(&children[1], "tag").as_str(), Some("b"));
    }

    #[test]
    fn decodes_the_five_predefined_entities_and_numeric_refs() {
        let v =
            parse("<r>a &amp; b &lt;tag&gt; &quot;q&quot; &apos;a&apos; &#65; &#x42;</r>").unwrap();
        assert_eq!(
            field(&v, "text").as_str(),
            Some("a & b <tag> \"q\" 'a' A B")
        );
    }

    #[test]
    fn rejects_an_unknown_entity_rather_than_silently_dropping_or_passing_it_through() {
        let err = parse("<r>&xxe;</r>").unwrap_err();
        let Signal::Error(msg) = err else {
            panic!("expected Signal::Error");
        };
        assert!(msg.contains("unsupported entity"));
    }

    #[test]
    fn skips_doctype_without_processing_any_entity_definitions_inside_it() {
        // The classic XXE/billion-laughs shape: a DOCTYPE with an internal
        // subset defining custom entities. Must parse successfully by
        // skipping the whole declaration as opaque text — and the defined
        // entity must NOT be usable (proven by the next test using the
        // same shape and asserting it's rejected).
        let v = parse(
            r#"<?xml version="1.0"?><!DOCTYPE root [ <!ENTITY x "ignored"> ]><root>ok</root>"#,
        )
        .unwrap();
        assert_eq!(field(&v, "text").as_str(), Some("ok"));
    }

    #[test]
    fn a_custom_entity_defined_in_the_doctype_is_never_usable() {
        let err = parse(
            r#"<!DOCTYPE root [ <!ENTITY xxe SYSTEM "file:///etc/passwd"> ]><root>&xxe;</root>"#,
        )
        .unwrap_err();
        let Signal::Error(msg) = err else {
            panic!("expected Signal::Error");
        };
        assert!(msg.contains("unsupported entity"));
    }

    #[test]
    fn skips_comments_and_processing_instructions() {
        let v =
            parse("<?xml version=\"1.0\"?><!-- a comment --><root><!-- inner -->x</root>").unwrap();
        assert_eq!(field(&v, "text").as_str(), Some("x"));
    }

    #[test]
    fn cdata_is_passed_through_literally_without_entity_decoding() {
        let v = parse("<r><![CDATA[<not a tag> & not an entity]]></r>").unwrap();
        assert_eq!(
            field(&v, "text").as_str(),
            Some("<not a tag> & not an entity")
        );
    }

    #[test]
    fn rejects_a_mismatched_closing_tag() {
        assert!(parse("<a><b></c></a>").is_err());
    }

    #[test]
    fn rejects_deeply_nested_input_instead_of_overflowing_the_stack() {
        let mut s = String::new();
        for _ in 0..2000 {
            s.push_str("<a>");
        }
        s.push('x');
        for _ in 0..2000 {
            s.push_str("</a>");
        }
        let err = parse(&s).unwrap_err();
        let Signal::Error(msg) = err else {
            panic!("expected Signal::Error");
        };
        assert!(msg.contains("nesting depth"));
    }

    #[test]
    fn round_trips_through_stringify_and_back() {
        let original = r#"<book id="1"><title>GX</title><author>Ada</author></book>"#;
        let parsed = parse(original).unwrap();
        let Value::Str(stringified) = xml_stringify_impl(std::slice::from_ref(&parsed)).unwrap()
        else {
            panic!("expected a string");
        };
        let reparsed = parse(&stringified).unwrap();
        assert_eq!(field(&reparsed, "tag").as_str(), Some("book"));
        let Value::Array(children) = field(&reparsed, "children") else {
            panic!()
        };
        assert_eq!(children.len(), 2);
    }

    #[test]
    fn stringify_escapes_special_characters() {
        let mut obj = HashMap::new();
        obj.insert("tag".to_string(), Value::Str("r".to_string()));
        obj.insert("text".to_string(), Value::Str("a < b & c > d".to_string()));
        let Value::Str(s) = xml_stringify_impl(&[Value::Object(obj)]).unwrap() else {
            panic!()
        };
        assert!(s.contains("a &lt; b &amp; c &gt; d"));
        assert!(!s.contains("a < b"));
    }

    #[test]
    fn stringify_requires_a_tag_field() {
        let mut obj = HashMap::new();
        obj.insert("text".to_string(), Value::Str("x".to_string()));
        assert!(xml_stringify_impl(&[Value::Object(obj)]).is_err());
    }
}
