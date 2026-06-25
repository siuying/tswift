//! AST dump rendering for `quick-swift dump`.
//!
//! This is the **presentation** layer for the AST: it turns a [`Node`] subtree
//! into a human-readable text outline or a structured JSON tree. It lives apart
//! from [`crate::Node`] on purpose — `Node` is the safe FFI/lifetime boundary
//! (how you *reach* the AST), while this module is policy about how to *display*
//! it. Keeping them separate stops the FFI wrapper from accreting formatting
//! knobs every time the dump grows a field.
//!
//! Both renderers walk the tree through `Node`'s public accessors only, so they
//! contain no `unsafe` and need no knowledge of msf's raw layout.

use std::fmt::Write as _;

use crate::{ModifierSet, Node, NodeKind};

/// Render `node`'s subtree as an indented text outline, one node per line:
///
/// ```text
/// Kind "text" L<line> :<ResolvedType> [mod,mod,0xNN]
/// ```
///
/// `0xNN` is appended to the modifier group only when the node carries modifier
/// bits this build cannot name (see [`ModifierSet::unknown_bits`]); it makes a
/// dump lossless rather than silently dropping unrecognised bits.
pub(crate) fn render_text(node: Node<'_>) -> String {
    let mut w = TextWriter { out: String::new() };
    w.write(node, 0);
    w.out
}

/// Render `node`'s subtree as a single JSON object with `kind`, optional `raw`
/// (for an unnamed [`NodeKind::Other`]), `text`, `line`, `type`, `modifiers`,
/// `modifier_bits`, and nested `children`. Always valid JSON: all string fields
/// are escaped per RFC 8259, including control characters.
pub(crate) fn render_json(node: Node<'_>) -> String {
    let mut w = JsonWriter { out: String::new() };
    w.write(node);
    w.out
}

/// Indented-outline writer. Holds only the output buffer; depth is threaded
/// through the recursion so the type stays a trivial sink.
struct TextWriter {
    out: String,
}

impl TextWriter {
    fn write(&mut self, node: Node<'_>, depth: usize) {
        let indent = "  ".repeat(depth);
        let _ = write!(self.out, "{indent}{}", kind_label(node.kind()));
        if let Some(text) = node.text() {
            if !text.is_empty() {
                let _ = write!(self.out, " {text:?}");
            }
        }
        let line = node.line();
        if line > 0 {
            let _ = write!(self.out, " L{line}");
        }
        if let Some(ty) = node.type_name() {
            let _ = write!(self.out, " :{ty}");
        }
        self.write_modifiers(&node.modifier_set());
        let _ = writeln!(self.out);
        for child in node.children() {
            self.write(child, depth + 1);
        }
    }

    /// `[name,name,0xNN]` — names first, then a hex group of any bits this build
    /// could not name, so the outline never hides a set bit.
    fn write_modifiers(&mut self, mods: &ModifierSet) {
        if mods.is_empty() {
            return;
        }
        let mut parts: Vec<String> = mods.names().iter().map(|m| m.to_string()).collect();
        if mods.unknown_bits() != 0 {
            parts.push(format!("0x{:x}", mods.unknown_bits()));
        }
        let _ = write!(self.out, " [{}]", parts.join(","));
    }
}

/// JSON tree writer.
struct JsonWriter {
    out: String,
}

impl JsonWriter {
    fn write(&mut self, node: Node<'_>) {
        let _ = write!(self.out, "{{\"kind\":{}", json_string(node.kind().name()));
        if let NodeKind::Other(n) = node.kind() {
            let _ = write!(self.out, ",\"raw\":{n}");
        }
        if let Some(text) = node.text() {
            if !text.is_empty() {
                let _ = write!(self.out, ",\"text\":{}", json_string(&text));
            }
        }
        let line = node.line();
        if line > 0 {
            let _ = write!(self.out, ",\"line\":{line}");
        }
        if let Some(ty) = node.type_name() {
            let _ = write!(self.out, ",\"type\":{}", json_string(&ty));
        }
        let mods = node.modifier_set();
        if !mods.is_empty() {
            let parts: Vec<String> = mods.names().iter().map(|m| json_string(m)).collect();
            let _ = write!(self.out, ",\"modifiers\":[{}]", parts.join(","));
            // Emit the raw mask too so a consumer can recover bits we did not
            // name (and verify the decode). Always present when any bit is set.
            let _ = write!(self.out, ",\"modifier_bits\":{}", mods.raw());
        }
        let mut children = node.children().peekable();
        if children.peek().is_some() {
            self.out.push_str(",\"children\":[");
            for (i, child) in children.enumerate() {
                if i > 0 {
                    self.out.push(',');
                }
                self.write(child);
            }
            self.out.push(']');
        }
        self.out.push('}');
    }
}

/// `Other(N)` prints as `Other(N)`; every named kind prints its PascalCase
/// debug name. Shared by the text renderer's header.
fn kind_label(kind: NodeKind) -> String {
    match kind {
        NodeKind::Other(n) => format!("Other({n})"),
        k => format!("{k:?}"),
    }
}

/// Encode `s` as a JSON string literal (with surrounding quotes), escaping per
/// RFC 8259: the named short escapes, and **every** remaining control character
/// (`U+0000`..=`U+001F`) as `\uXXXX`. Without the control-character clause,
/// token/type text containing a raw `U+0000`..`U+001F` byte would produce
/// invalid JSON from `quick-swift dump --json`.
fn json_string(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\u{08}' => out.push_str("\\b"),
            '\u{0c}' => out.push_str("\\f"),
            c if (c as u32) < 0x20 => {
                let _ = write!(out, "\\u{:04x}", c as u32);
            }
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

#[cfg(test)]
mod tests {
    use super::json_string;

    #[test]
    fn escapes_named_short_escapes() {
        assert_eq!(json_string("a\"b\\c"), r#""a\"b\\c""#);
        assert_eq!(
            json_string("line\nbreak\ttab\rcr"),
            r#""line\nbreak\ttab\rcr""#
        );
        assert_eq!(json_string("\u{08}\u{0c}"), r#""\b\f""#);
    }

    #[test]
    fn escapes_other_control_characters_as_u() {
        // A bare U+0001 / U+001F must become \u0001 / \u001f, not a raw byte
        // that would make the surrounding JSON invalid.
        assert_eq!(json_string("\u{01}"), r#""\u0001""#);
        assert_eq!(json_string("\u{1f}"), r#""\u001f""#);
        assert_eq!(json_string("\u{00}"), r#""\u0000""#);
    }

    #[test]
    fn leaves_printable_unicode_intact() {
        assert_eq!(json_string("héllo→世界"), "\"héllo→世界\"");
    }
}
