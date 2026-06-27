//! UIIR serialization — the view-value tree → canonical JSON (the Layer-B wire
//! format, plan §3.1).
//!
//! A view value is walked depth-first; each node is assigned a stable
//! structural-path `id` (`"0"`, `"0.0"`, `"0.1.2"`, …) used by both diffing and
//! event routing. Modifier values use the plan's tagged-union encoding: semantic
//! tokens become `{"$":"color","name":"white"}` &c.; plain values stay numeric /
//! string. Output is deterministic (fields emitted in a fixed order) so it can
//! be asserted byte-for-byte as a golden.

use tswift_core::SwiftValue;

use crate::{
    child_id, token_of, view_type_name, ACTION_FIELD, CHILDREN_FIELD, KEY_FIELD, MODIFIERS_FIELD,
    MODIFIER_TYPE,
};

/// Serialize a view-value tree rooted at `view` into canonical UIIR JSON, with
/// the root node assigned id `"0"`.
pub fn to_json(view: &SwiftValue) -> String {
    node_json(view, "0")
}

/// Serialize `view` (and its subtree) as a UIIR node rooted at structural path
/// `id` — used by patch ops that carry a full subtree (`mount`/`insert`/
/// `replace`).
pub fn node_json(view: &SwiftValue, id: &str) -> String {
    let mut out = String::new();
    write_node(view, id, &mut out);
    out
}

/// Serialize an ordered modifier list as a JSON array (the `setModifiers`
/// payload).
pub fn modifiers_json(mods: &[SwiftValue]) -> String {
    let mut out = String::new();
    out.push('[');
    for (i, m) in mods.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        write_modifier(m, &mut out);
    }
    out.push(']');
    out
}

/// Serialize the visible (non-internal) constructor args of `view` as a JSON
/// object (the `setArgs` payload).
pub fn args_json(view: &SwiftValue) -> String {
    let mut out = String::new();
    out.push('{');
    if let SwiftValue::Struct(obj) = view {
        let mut first = true;
        for (key, value) in &obj.fields {
            if key.starts_with('_') {
                continue;
            }
            if !first {
                out.push(',');
            }
            first = false;
            write_string(key, &mut out);
            out.push(':');
            write_value(value, &mut out);
        }
    }
    out.push('}');
    out
}

/// Write one UIIR node (and its subtree) at structural path `id`.
fn write_node(view: &SwiftValue, id: &str, out: &mut String) {
    let SwiftValue::Struct(obj) = view else {
        // A non-view value should never reach here; emit a null node so the
        // output stays valid JSON rather than panicking.
        out.push_str("null");
        return;
    };
    out.push('{');
    out.push_str("\"id\":");
    write_string(id, out);
    out.push_str(",\"kind\":");
    write_string(&obj.type_name, out);

    // args — constructor fields, excluding internal (`_`-prefixed) ones.
    out.push_str(",\"args\":{");
    let mut first = true;
    for (key, value) in &obj.fields {
        if key.starts_with('_') || key == ACTION_FIELD || key == KEY_FIELD {
            continue;
        }
        if !first {
            out.push(',');
        }
        first = false;
        write_string(key, out);
        out.push(':');
        write_value(value, out);
    }
    out.push('}');

    // modifiers — ordered list of `{name, value}`.
    out.push_str(",\"modifiers\":[");
    if let Some(SwiftValue::Array(mods)) = obj.get(MODIFIERS_FIELD) {
        for (i, m) in mods.iter().enumerate() {
            if i > 0 {
                out.push(',');
            }
            write_modifier(m, out);
        }
    }
    out.push(']');

    // children — recursive subtree with appended structural indices.
    out.push_str(",\"children\":[");
    if let Some(SwiftValue::Array(children)) = obj.get(CHILDREN_FIELD) {
        for (i, child) in children.iter().enumerate() {
            if i > 0 {
                out.push(',');
            }
            let cid = child_id(id, i, child);
            write_node(child, &cid, out);
        }
    }
    out.push(']');

    out.push('}');
}

/// Write a `_Modifier` record as `{"name":…,"value":…}`. A modifier with no
/// arguments encodes `value: null`; a single positional arg encodes that value
/// directly; multiple/labeled args encode an object keyed by label.
fn write_modifier(modifier: &SwiftValue, out: &mut String) {
    let SwiftValue::Struct(obj) = modifier else {
        out.push_str("null");
        return;
    };
    debug_assert_eq!(obj.type_name, MODIFIER_TYPE);
    let name = match obj.get("name") {
        Some(SwiftValue::Str(s)) => s.as_str(),
        _ => "",
    };
    out.push_str("{\"name\":");
    write_string(name, out);
    out.push_str(",\"value\":");

    let args: Vec<&(String, SwiftValue)> = obj.fields.iter().filter(|(k, _)| k != "name").collect();
    match args.as_slice() {
        [] => out.push_str("null"),
        [(key, value)] if key == "value" => write_value(value, out),
        _ => {
            out.push('{');
            for (i, (key, value)) in args.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                write_string(key, out);
                out.push(':');
                write_value(value, out);
            }
            out.push('}');
        }
    }
    out.push('}');
}

/// Write a Swift value with the plan's tagged-union encoding: prelude tokens
/// become `{"$":tag,"name":…}`; scalars stay numeric / string / bool / null.
fn write_value(value: &SwiftValue, out: &mut String) {
    if let Some((namespace, token)) = token_of(value) {
        let tag = match namespace {
            "Color" => "color",
            "Font" => "textStyle",
            "FontWeight" => "weight",
            _ => "token",
        };
        out.push_str("{\"$\":");
        write_string(tag, out);
        out.push_str(",\"name\":");
        write_string(token, out);
        out.push('}');
        return;
    }
    match value {
        SwiftValue::Int(i) => out.push_str(&i.raw.to_string()),
        SwiftValue::Double(d) => out.push_str(&tswift_core::format_double(*d)),
        SwiftValue::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
        SwiftValue::Str(s) => write_string(s, out),
        SwiftValue::Nil => out.push_str("null"),
        // A nested view value (e.g. `.background(SomeView())`) serializes as a
        // node; anything else falls back to its display string.
        other if view_type_name(other).is_some() => write_node(other, "0", out),
        other => write_string(&other.to_string(), out),
    }
}

/// Write a JSON string literal with the minimal required escaping.
fn write_string(s: &str, out: &mut String) {
    out.push('"');
    for ch in s.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{install, render_root, PRELUDE};
    use tswift_core::Interpreter;

    fn render_json(body: &str) -> String {
        let src = format!("{PRELUDE}\nstruct V: View {{ var body: some View {{ {body} }} }}\n");
        let analysis = tswift_frontend::Analysis::analyze(&src, "t.swift").expect("analyze");
        let analysis: &'static tswift_frontend::Analysis = Box::leak(Box::new(analysis));
        let mut sink = std::io::sink();
        let mut interp = Interpreter::new(&mut sink);
        install(&mut interp);
        interp.run(analysis).expect("run");
        let view = render_root(&mut interp, "V").expect("render");
        to_json(&view)
    }

    #[test]
    fn text_with_token_modifier_serializes_canonically() {
        let json = render_json(r#"Text("hi").font(.largeTitle).foregroundColor(.white)"#);
        assert_eq!(
            json,
            r#"{"id":"0","kind":"Text","args":{"verbatim":"hi"},"modifiers":[{"name":"font","value":{"$":"textStyle","name":"largeTitle"}},{"name":"foregroundColor","value":{"$":"color","name":"white"}}],"children":[]}"#
        );
    }

    #[test]
    fn vstack_assigns_structural_ids_to_children() {
        let json = render_json("VStack { Text(\"a\"); Text(\"b\") }");
        assert_eq!(
            json,
            r#"{"id":"0","kind":"VStack","args":{},"modifiers":[],"children":[{"id":"0.0","kind":"Text","args":{"verbatim":"a"},"modifiers":[],"children":[]},{"id":"0.1","kind":"Text","args":{"verbatim":"b"},"modifiers":[],"children":[]}]}"#
        );
    }

    #[test]
    fn frame_and_padding_encode_object_and_null_values() {
        let json = render_json("Text(\"x\").padding().frame(width: 56, height: 56)");
        assert_eq!(
            json,
            r#"{"id":"0","kind":"Text","args":{"verbatim":"x"},"modifiers":[{"name":"padding","value":null},{"name":"frame","value":{"width":56,"height":56}}],"children":[]}"#
        );
    }
}
