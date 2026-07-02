//! String-literal decoding and interpolation evaluation.
//!
//! This module owns the journey from a string-literal lexeme (raw source text,
//! delimiters and escapes intact) to the runtime [`SwiftValue::Str`] it
//! denotes, including `\( … )` interpolation and the `$N` shorthand scan the
//! call dispatcher needs to size implicit closure parameters.

use tswift_frontend::Node;

use super::{Eval, EvalError, Interpreter, Signal};
use crate::fragment_cache::FragmentError;
use crate::value::SwiftValue;

impl<'w> Interpreter<'w> {
    pub(super) fn eval_string_literal(&mut self, node: &Node<'static>) -> Eval {
        let raw = node.text().unwrap_or_default();
        // Raw strings do not interpolate; decode handles delimiters/escapes.
        if raw.starts_with('#') {
            return Ok(SwiftValue::Str(decode_string_literal(&raw)));
        }
        let (body, multiline) = if let Some(b) = raw
            .strip_prefix("\"\"\"")
            .and_then(|s| s.strip_suffix("\"\"\""))
        {
            (strip_multiline_indent(b), true)
        } else {
            let b = raw
                .strip_prefix('"')
                .and_then(|s| s.strip_suffix('"'))
                .unwrap_or(&raw)
                .to_string();
            (b, false)
        };
        let _ = multiline;

        let mut out = String::new();
        let mut chars = body.chars().peekable();
        while let Some(c) = chars.next() {
            if c == '\\' && chars.peek() == Some(&'(') {
                chars.next(); // consume '('
                let mut depth = 1;
                let mut fragment = String::new();
                for fc in chars.by_ref() {
                    match fc {
                        '(' => depth += 1,
                        ')' => {
                            depth -= 1;
                            if depth == 0 {
                                break;
                            }
                        }
                        _ => {}
                    }
                    fragment.push(fc);
                }
                let value = self.eval_interpolation(&fragment)?;
                out.push_str(&self.render_description(&value));
            } else if c == '\\' {
                // Re-use the escape decoder for the next escape sequence.
                let mut esc = String::from("\\");
                if let Some(&n) = chars.peek() {
                    esc.push(n);
                    chars.next();
                    if n == 'u' && chars.peek() == Some(&'{') {
                        for h in chars.by_ref() {
                            esc.push(h);
                            if h == '}' {
                                break;
                            }
                        }
                    }
                }
                out.push_str(&decode_escapes(&esc));
            } else {
                out.push(c);
            }
        }
        Ok(SwiftValue::Str(out))
    }

    /// Evaluate an interpolated expression fragment against the current scope,
    /// reusing this interpreter (and thus its type/function tables).
    ///
    /// The fragment's analysis is owned by the interpreter's [`FragmentCache`]
    /// (ADR-0007): a repeated fragment is analyzed once, and every cached
    /// analysis is reclaimed when the interpreter drops, so a long-running host
    /// runs in bounded memory.
    ///
    /// [`FragmentCache`]: crate::fragment_cache::FragmentCache
    pub(super) fn eval_interpolation(&mut self, fragment: &str) -> Result<SwiftValue, Signal> {
        let analysis = self
            .fragment_cache
            .get_or_analyze(fragment)
            .map_err(|e| match e {
                FragmentError::Parse(msg) => {
                    EvalError::Type(format!("interpolation parse error: {msg}"))
                }
                FragmentError::Invalid => {
                    EvalError::Type(format!("invalid interpolation `{fragment}`"))
                }
            })?;
        let root = analysis.root();
        // Evaluate the wrapped expression statement directly.
        self.eval(&root)
    }
}

/// Scan a string literal's *interpolation* segments (`\( … )`) for shorthand
/// argument references (`$0`, `$1`, …) and return the greatest index found.
/// Only text inside `\(…)` is considered, so a literal `"$1"` outside an
/// interpolation is ignored.
pub(super) fn max_shorthand_in_interpolations(raw: &str) -> Option<usize> {
    let bytes: Vec<char> = raw.chars().collect();
    let mut i = 0;
    let mut max: Option<usize> = None;
    while i < bytes.len() {
        if bytes[i] == '\\' && i + 1 < bytes.len() && bytes[i + 1] == '(' {
            // Capture the balanced interpolation body.
            let mut depth = 1;
            let mut j = i + 2;
            while j < bytes.len() && depth > 0 {
                match bytes[j] {
                    '(' => depth += 1,
                    ')' => depth -= 1,
                    _ => {}
                }
                if depth == 0 {
                    break;
                }
                j += 1;
            }
            // Scan the fragment `bytes[i+2..j]` for `$<digits>`.
            let mut k = i + 2;
            while k < j {
                if bytes[k] == '$' {
                    let mut d = String::new();
                    let mut m = k + 1;
                    while m < j && bytes[m].is_ascii_digit() {
                        d.push(bytes[m]);
                        m += 1;
                    }
                    if let Ok(n) = d.parse::<usize>() {
                        max = Some(max.map_or(n, |c| c.max(n)));
                    }
                }
                k += 1;
            }
            i = j + 1;
        } else {
            i += 1;
        }
    }
    max
}

/// Decode a Swift string literal's *source text* (including its delimiters) into
/// the runtime string it denotes: strips quotes and processes escapes.
pub(super) fn decode_string_literal(raw: &str) -> String {
    if raw.starts_with('#') {
        let hashes = raw.chars().take_while(|&c| c == '#').count();
        let inner = &raw[hashes..raw.len().saturating_sub(hashes)];
        let inner = inner
            .strip_prefix('"')
            .and_then(|s| s.strip_suffix('"'))
            .unwrap_or(inner);
        return inner.to_string();
    }
    if let Some(body) = raw
        .strip_prefix("\"\"\"")
        .and_then(|s| s.strip_suffix("\"\"\""))
    {
        return decode_escapes(&strip_multiline_indent(body));
    }
    let body = raw
        .strip_prefix('"')
        .and_then(|s| s.strip_suffix('"'))
        .unwrap_or(raw);
    decode_escapes(body)
}

/// Apply Swift's multiline-literal shaping: the whitespace preceding the
/// closing `"""` is stripped from every line, and the newlines adjacent to
/// the delimiters are not part of the value. Blank lines may omit the indent.
fn strip_multiline_indent(body: &str) -> String {
    let body = body.strip_prefix('\n').unwrap_or(body);
    // The closing line (after the final newline) holds the reference indent.
    let (content, indent) = match body.rfind('\n') {
        Some(i) => (&body[..i], &body[i + 1..]),
        None => (body, ""),
    };
    if !indent.is_empty() && !indent.chars().all(|c| c == ' ' || c == '\t') {
        // Single-line body (`"""x"""`): nothing to strip.
        return body.to_string();
    }
    content
        .split('\n')
        .map(|l| l.strip_prefix(indent).unwrap_or(l))
        .collect::<Vec<_>>()
        .join("\n")
}

fn decode_escapes(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('n') => out.push('\n'),
            Some('t') => out.push('\t'),
            Some('r') => out.push('\r'),
            Some('0') => out.push('\0'),
            Some('\\') => out.push('\\'),
            Some('"') => out.push('"'),
            Some('\'') => out.push('\''),
            Some('u') => {
                if chars.peek() == Some(&'{') {
                    chars.next();
                    let mut hex = String::new();
                    for h in chars.by_ref() {
                        if h == '}' {
                            break;
                        }
                        hex.push(h);
                    }
                    if let Ok(cp) = u32::from_str_radix(&hex, 16) {
                        if let Some(ch) = char::from_u32(cp) {
                            out.push(ch);
                        }
                    }
                } else {
                    out.push('u');
                }
            }
            Some(other) => {
                out.push('\\');
                out.push(other);
            }
            None => out.push('\\'),
        }
    }
    out
}
