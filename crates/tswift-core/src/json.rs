//! A small, self-contained JSON layer for `Codable` round-trips.
//!
//! The plan calls for `serde_json`; to keep the crate dependency-free and
//! offline-buildable, this module implements just the slice of JSON tswift
//! needs: serialize the runtime values produced by `Codable` types, and parse a
//! JSON document into a generic [`Json`] tree that the interpreter maps back
//! onto a struct's fields.

use crate::format_double_json;
use std::fmt::Write as _;

/// A parsed JSON value.
#[derive(Debug, Clone, PartialEq)]
pub enum Json {
    Null,
    Bool(bool),
    Int(i64),
    Double(f64),
    Str(String),
    Array(Vec<Json>),
    Object(Vec<(String, Json)>),
}

impl Json {
    /// Look up a key in a JSON object.
    pub fn get(&self, key: &str) -> Option<&Json> {
        match self {
            Json::Object(entries) => entries.iter().find(|(k, _)| k == key).map(|(_, v)| v),
            _ => None,
        }
    }
}

/// Output-formatting options for `JSONEncoder`, mirroring Swift's
/// `JSONEncoder.OutputFormatting` OptionSet.
#[derive(Clone, Copy, Default, Debug)]
pub struct OutputFormatting {
    /// Indent with 2 spaces and use `" : "` between keys and values.
    pub pretty_printed: bool,
    /// Sort object keys lexicographically at every nesting level.
    pub sorted_keys: bool,
}

/// Serialize a JSON tree to a compact string, matching `JSONEncoder`'s default
/// (no whitespace, keys in insertion order).
pub fn to_string(value: &Json) -> String {
    to_string_fmt(value, &OutputFormatting::default())
}

/// Serialize a JSON tree respecting the given output-formatting options.
pub fn to_string_fmt(value: &Json, fmt: &OutputFormatting) -> String {
    let mut out = String::new();
    write_value_fmt(&mut out, value, fmt, 0);
    out
}

/// Write a JSON value with optional pretty-printing and/or key sorting.
/// `depth` tracks the current nesting depth for indentation.
fn write_value_fmt(out: &mut String, value: &Json, fmt: &OutputFormatting, depth: usize) {
    match value {
        Json::Null => out.push_str("null"),
        Json::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
        Json::Int(i) => {
            let _ = write!(out, "{i}");
        }
        Json::Double(d) => {
            out.push_str(&format_double_json(*d));
        }
        Json::Str(s) => write_string(out, s),
        Json::Array(items) => {
            out.push('[');
            if fmt.pretty_printed && !items.is_empty() {
                for (i, item) in items.iter().enumerate() {
                    out.push('\n');
                    push_indent(out, depth + 1);
                    write_value_fmt(out, item, fmt, depth + 1);
                    if i + 1 < items.len() {
                        out.push(',');
                    }
                }
                out.push('\n');
                push_indent(out, depth);
            } else {
                for (i, item) in items.iter().enumerate() {
                    if i > 0 {
                        out.push(',');
                    }
                    write_value_fmt(out, item, fmt, depth + 1);
                }
            }
            out.push(']');
        }
        Json::Object(entries) => {
            out.push('{');
            // Collect references so we can sort without cloning.
            let ordered: Vec<(&str, &Json)> = if fmt.sorted_keys {
                let mut v: Vec<(&str, &Json)> =
                    entries.iter().map(|(k, v)| (k.as_str(), v)).collect();
                v.sort_by_key(|(k, _)| *k);
                v
            } else {
                entries.iter().map(|(k, v)| (k.as_str(), v)).collect()
            };
            if fmt.pretty_printed && !ordered.is_empty() {
                for (i, (k, v)) in ordered.iter().enumerate() {
                    out.push('\n');
                    push_indent(out, depth + 1);
                    write_string(out, k);
                    out.push_str(" : ");
                    write_value_fmt(out, v, fmt, depth + 1);
                    if i + 1 < ordered.len() {
                        out.push(',');
                    }
                }
                out.push('\n');
                push_indent(out, depth);
            } else {
                for (i, (k, v)) in ordered.iter().enumerate() {
                    if i > 0 {
                        out.push(',');
                    }
                    write_string(out, k);
                    out.push(':');
                    write_value_fmt(out, v, fmt, depth);
                }
            }
            out.push('}');
        }
    }
}

/// Push `depth * 2` spaces of indentation.
fn push_indent(out: &mut String, depth: usize) {
    for _ in 0..depth {
        out.push_str("  ");
    }
}

fn write_string(out: &mut String, s: &str) {
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            '\r' => out.push_str("\\r"),
            // Escape remaining C0 control characters (U+0000–U+001F) per the
            // JSON spec (RFC 8259 § 7). \b and \f get their shorthand; the rest
            // use the \uXXXX six-character form.
            '\x08' => out.push_str("\\b"),
            '\x0C' => out.push_str("\\f"),
            c if (c as u32) < 0x20 => {
                let _ = write!(out, "\\u{:04X}", c as u32);
            }
            _ => out.push(c),
        }
    }
    out.push('"');
}

/// Parse a JSON document. Returns `Err` with a message on malformed input.
pub fn parse(input: &str) -> Result<Json, String> {
    let mut p = Parser {
        chars: input.chars().collect(),
        pos: 0,
    };
    p.skip_ws();
    let v = p.parse_value()?;
    p.skip_ws();
    if p.pos != p.chars.len() {
        return Err("trailing characters after JSON value".into());
    }
    Ok(v)
}

struct Parser {
    chars: Vec<char>,
    pos: usize,
}

impl Parser {
    fn peek(&self) -> Option<char> {
        self.chars.get(self.pos).copied()
    }

    fn bump(&mut self) -> Option<char> {
        let c = self.peek();
        if c.is_some() {
            self.pos += 1;
        }
        c
    }

    fn skip_ws(&mut self) {
        while matches!(self.peek(), Some(' ' | '\t' | '\n' | '\r')) {
            self.pos += 1;
        }
    }

    fn parse_value(&mut self) -> Result<Json, String> {
        self.skip_ws();
        match self.peek() {
            Some('{') => self.parse_object(),
            Some('[') => self.parse_array(),
            Some('"') => Ok(Json::Str(self.parse_string()?)),
            Some('t') | Some('f') => self.parse_bool(),
            Some('n') => self.parse_null(),
            Some(c) if c == '-' || c.is_ascii_digit() => self.parse_number(),
            other => Err(format!("unexpected token: {other:?}")),
        }
    }

    fn parse_object(&mut self) -> Result<Json, String> {
        self.bump(); // {
        let mut entries = Vec::new();
        self.skip_ws();
        if self.peek() == Some('}') {
            self.bump();
            return Ok(Json::Object(entries));
        }
        loop {
            self.skip_ws();
            let key = self.parse_string()?;
            self.skip_ws();
            if self.bump() != Some(':') {
                return Err("expected ':' in object".into());
            }
            let value = self.parse_value()?;
            entries.push((key, value));
            self.skip_ws();
            match self.bump() {
                Some(',') => continue,
                Some('}') => break,
                other => return Err(format!("expected ',' or '}}', got {other:?}")),
            }
        }
        Ok(Json::Object(entries))
    }

    fn parse_array(&mut self) -> Result<Json, String> {
        self.bump(); // [
        let mut items = Vec::new();
        self.skip_ws();
        if self.peek() == Some(']') {
            self.bump();
            return Ok(Json::Array(items));
        }
        loop {
            let value = self.parse_value()?;
            items.push(value);
            self.skip_ws();
            match self.bump() {
                Some(',') => continue,
                Some(']') => break,
                other => return Err(format!("expected ',' or ']', got {other:?}")),
            }
        }
        Ok(Json::Array(items))
    }

    fn parse_string(&mut self) -> Result<String, String> {
        if self.bump() != Some('"') {
            return Err("expected string".into());
        }
        let mut s = String::new();
        while let Some(c) = self.bump() {
            match c {
                '"' => return Ok(s),
                '\\' => match self.bump() {
                    Some('"') => s.push('"'),
                    Some('\\') => s.push('\\'),
                    Some('/') => s.push('/'),
                    Some('n') => s.push('\n'),
                    Some('t') => s.push('\t'),
                    Some('r') => s.push('\r'),
                    Some('u') => {
                        let mut hex = String::new();
                        for _ in 0..4 {
                            if let Some(h) = self.bump() {
                                hex.push(h);
                            }
                        }
                        if let Ok(cp) = u32::from_str_radix(&hex, 16) {
                            if let Some(ch) = char::from_u32(cp) {
                                s.push(ch);
                            }
                        }
                    }
                    other => return Err(format!("invalid escape: {other:?}")),
                },
                _ => s.push(c),
            }
        }
        Err("unterminated string".into())
    }

    fn parse_bool(&mut self) -> Result<Json, String> {
        if self.starts_with("true") {
            self.pos += 4;
            Ok(Json::Bool(true))
        } else if self.starts_with("false") {
            self.pos += 5;
            Ok(Json::Bool(false))
        } else {
            Err("invalid literal".into())
        }
    }

    fn parse_null(&mut self) -> Result<Json, String> {
        if self.starts_with("null") {
            self.pos += 4;
            Ok(Json::Null)
        } else {
            Err("invalid literal".into())
        }
    }

    fn parse_number(&mut self) -> Result<Json, String> {
        let start = self.pos;
        let mut is_float = false;
        if self.peek() == Some('-') {
            self.pos += 1;
        }
        while let Some(c) = self.peek() {
            match c {
                '0'..='9' => self.pos += 1,
                '.' | 'e' | 'E' | '+' | '-' => {
                    is_float = true;
                    self.pos += 1;
                }
                _ => break,
            }
        }
        let text: String = self.chars[start..self.pos].iter().collect();
        if is_float {
            text.parse::<f64>()
                .map(Json::Double)
                .map_err(|e| e.to_string())
        } else {
            text.parse::<i64>()
                .map(Json::Int)
                .map_err(|e| e.to_string())
        }
    }

    fn starts_with(&self, s: &str) -> bool {
        self.chars[self.pos..]
            .iter()
            .take(s.len())
            .collect::<String>()
            == s
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_object() {
        let json = parse(r#"{"name":"Sam","age":30,"tags":["a","b"],"ok":true}"#).unwrap();
        assert_eq!(json.get("name"), Some(&Json::Str("Sam".into())));
        assert_eq!(json.get("age"), Some(&Json::Int(30)));
        let s = to_string(&json);
        assert_eq!(s, r#"{"name":"Sam","age":30,"tags":["a","b"],"ok":true}"#);
    }

    #[test]
    fn control_characters_are_escaped() {
        // \n, \t, \r get short escapes; other C0 controls get \uXXXX.
        let s = to_string(&Json::Str("a\nb\tc\rd\x00e\x01f\x08g\x0Ch".into()));
        assert_eq!(s, r#""a\nb\tc\rd\u0000e\u0001f\bg\fh""#);
    }

    #[test]
    fn quote_and_backslash_are_escaped() {
        let s = to_string(&Json::Str("say \"hi\" and \\\\".into()));
        assert_eq!(s, "\"say \\\"hi\\\" and \\\\\\\\\"");
    }
}
