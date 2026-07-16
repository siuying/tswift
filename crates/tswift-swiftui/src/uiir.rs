//! UIIR serialization — the view-value tree → canonical JSON (the Layer-B wire
//! format, plan §3.1).
//!
//! A view value is walked depth-first; each node is assigned a stable
//! structural-path `id` (`"0"`, `"0.0"`, `"0.1.2"`, …) used by both diffing and
//! event routing. Modifier values use the plan's tagged-union encoding: semantic
//! tokens become `{"$":"color","name":"white"}` &c.; plain values stay numeric /
//! string. Output is deterministic (fields emitted in a fixed order) so it can
//! be asserted byte-for-byte as a golden.

use tswift_core::{StructObj, SwiftValue};

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
    if let SwiftValue::Struct(color) = value {
        if color.type_name == "Color" {
            write_color(color, out);
            return;
        }
    }
    if let Some((namespace, token)) = token_of(value) {
        let tag = match namespace {
            "Color" => "color",
            "Font" => "textStyle",
            "FontWeight" => "weight",
            "TextAlignment" => "textAlign",
            "TextCase" => "textCase",
            "Axis" => "axis",
            "_ControlStyle" => "style",
            "Alignment" => "align",
            "HorizontalAlignment" => "hAlign",
            "VerticalAlignment" => "vAlign",
            "Edge" => "edge",
            "ContentMode" => "contentMode",
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
        // Non-finite layout lengths (e.g. a qualified `.frame(maxWidth:
        // Double.infinity)`) are deferred (issue #189). Emit a JSON-valid
        // sentinel token instead of the bare `inf`/`nan` (which is invalid
        // JSON); hosts ignore unknown tokens, so this degrades gracefully until
        // the typed-token work lands.
        SwiftValue::Double(d) if !d.is_finite() => out.push_str(if d.is_nan() {
            r#"{"$":"nan"}"#
        } else if *d > 0.0 {
            r#"{"$":"infinity"}"#
        } else {
            r#"{"$":"-infinity"}"#
        }),
        SwiftValue::Double(d) => out.push_str(&tswift_core::format_double(*d)),
        SwiftValue::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
        SwiftValue::Str(s) => write_string(s, out),
        SwiftValue::Nil => out.push_str("null"),
        // An array-valued arg (e.g. `LazyVGrid(columns:)`'s `[GridItem]`)
        // serializes as a JSON array of its elements (issue #205).
        SwiftValue::Array(items) => {
            out.push('[');
            for (i, item) in items.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                write_value(item, out);
            }
            out.push(']');
        }
        // A `GridItem` track sizer serializes as `{kind,value,spacing?}`.
        SwiftValue::Struct(obj) if obj.type_name == "GridItem" => write_grid_item(obj, out),
        // An `Animation` curve serializes as a tagged `{"$":"animation",…}`.
        SwiftValue::Struct(obj) if obj.type_name == "Animation" => write_animation(obj, out),
        // An `AnyTransition` serializes as a tagged `{"$":"transition",…}`.
        SwiftValue::Struct(obj) if obj.type_name == "AnyTransition" => write_transition(obj, out),
        // `PlottableValue` preserves declared value type (JSON string vs number)
        // so hosts do not coerce String("3") into a numeric plottable.
        SwiftValue::Struct(obj) if obj.type_name == "PlottableValue" => {
            write_plottable_value(obj, out)
        }
        // A nested view value (e.g. `.background(SomeView())`) serializes as a
        // node; anything else falls back to its display string.
        other if view_type_name(other).is_some() => write_node(other, "0", out),
        other => write_string(&other.to_string(), out),
    }
}

/// Serialize Charts `PlottableValue` as `{"$":"plottable","label":…,"value":…}`.
///
/// Wire guarantee: the nested `value` is **always** a JSON string or number —
/// `Str` → JSON string, `Int`/`Double` → JSON number. Any other `SwiftValue`
/// (bool, nil, array, object, token, …) is coerced to a JSON string via its
/// display form. Hosts can therefore treat plottable `value` as string|number
/// only (numeric-looking strings like `"3"` stay categorical strings).
fn write_plottable_value(obj: &StructObj, out: &mut String) {
    let field = |name: &str| obj.fields.iter().find(|(k, _)| k == name).map(|(_, v)| v);
    out.push_str("{\"$\":\"plottable\",\"label\":");
    match field("label") {
        Some(SwiftValue::Str(s)) => write_string(s, out),
        Some(other) => write_string(&other.to_string(), out),
        None => write_string("", out),
    }
    out.push_str(",\"value\":");
    match field("value") {
        Some(SwiftValue::Str(s)) => write_string(s, out),
        Some(SwiftValue::Int(i)) => out.push_str(&i.raw.to_string()),
        // Non-finite doubles are rare as plottables; still emit a JSON number
        // token when finite, else coerce to a display string (never null/object).
        Some(SwiftValue::Double(d)) if d.is_finite() => {
            out.push_str(&tswift_core::format_double(*d));
        }
        Some(other) => write_string(&other.to_string(), out),
        None => write_string("", out),
    }
    out.push('}');
}

/// Write a SwiftUI `Color` without lowering it to host pixels. Named colors
/// stay semantic tokens; explicit RGB colors retain their normalized RGBA
/// components so every host receives the same declared value.
fn write_color(color: &StructObj, out: &mut String) {
    if let Some(SwiftValue::Str(name)) = color.get("token") {
        out.push_str(r#"{"$":"color","name":"#);
        write_string(name, out);
        out.push('}');
        return;
    }

    let component = |name: &str| match color.get(name) {
        Some(SwiftValue::Double(value)) => *value,
        Some(SwiftValue::Int(value)) => value.raw as f64,
        _ => 0.0,
    };
    out.push_str(r#"{"$":"color","rgba":["#);
    for (index, name) in ["red", "green", "blue", "opacity"].iter().enumerate() {
        if index > 0 {
            out.push(',');
        }
        out.push_str(&tswift_core::format_double(component(name)));
    }
    out.push_str("]}");
}

/// Serialize a `GridItem` as `{"kind":…,"value":…,"spacing":…?}`. `spacing` is
/// omitted when the GridItem carried no explicit spacing (`nil`).
fn write_grid_item(obj: &StructObj, out: &mut String) {
    let field = |name: &str| obj.fields.iter().find(|(k, _)| k == name).map(|(_, v)| v);
    out.push_str("{\"kind\":");
    match field("kind") {
        Some(SwiftValue::Str(s)) => write_string(s, out),
        _ => out.push_str("\"flexible\""),
    }
    out.push_str(",\"value\":");
    match field("value") {
        Some(SwiftValue::Double(d)) => out.push_str(&tswift_core::format_double(*d)),
        Some(SwiftValue::Int(i)) => out.push_str(&i.raw.to_string()),
        _ => out.push('0'),
    }
    // `max` is emitted only for the flexible/adaptive sizers and only when the
    // bound is finite (the `.infinity` default means "unbounded" and is left
    // off, so the host picks its own fill behavior). `fixed` needs no max.
    let kind = match field("kind") {
        Some(SwiftValue::Str(s)) => s.as_str(),
        _ => "flexible",
    };
    if kind != "fixed" {
        if let Some(SwiftValue::Double(m)) = field("maximum") {
            if m.is_finite() {
                out.push_str(",\"max\":");
                out.push_str(&tswift_core::format_double(*m));
            }
        }
    }
    if let Some(spacing) = field("spacing") {
        if !matches!(spacing, SwiftValue::Nil) {
            out.push_str(",\"spacing\":");
            write_value(spacing, out);
        }
    }
    out.push('}');
}

/// Serialize an `Animation` value as `{"$":"animation","kind":…,…}`. Only the
/// set fields are emitted, in a fixed order: `kind`, curve params (`duration`
/// then the spring family), then the chained `delay`/`speed`/`repeat` modifiers.
/// `repeat` is `"forever"` (string) for `.repeatForever` or the integer count
/// for `.repeatCount`, and is followed by `autoreverses` when a repeat is set.
fn write_animation(obj: &StructObj, out: &mut String) {
    let field = |name: &str| obj.fields.iter().find(|(k, _)| k == name).map(|(_, v)| v);
    let num = |name: &str| match field(name) {
        Some(SwiftValue::Double(d)) => Some(*d),
        Some(SwiftValue::Int(i)) => Some(i.raw as f64),
        _ => None,
    };
    let kind = match field("kind") {
        Some(SwiftValue::Str(s)) => s.as_str(),
        _ => "default",
    };
    out.push_str("{\"$\":\"animation\",\"kind\":");
    write_string(kind, out);
    for (src, json) in [
        ("duration", "duration"),
        ("response", "response"),
        ("dampingFraction", "dampingFraction"),
        ("blendDuration", "blendDuration"),
        ("bounce", "bounce"),
        ("extraBounce", "extraBounce"),
        ("delayValue", "delay"),
        ("speedValue", "speed"),
    ] {
        if let Some(v) = num(src) {
            out.push(',');
            write_string(json, out);
            out.push(':');
            out.push_str(&tswift_core::format_double(v));
        }
    }
    if let Some(SwiftValue::Str(rk)) = field("repeatKind") {
        out.push_str(",\"repeat\":");
        if rk == "forever" {
            out.push_str("\"forever\"");
        } else if let Some(SwiftValue::Int(c)) = field("repeatCountValue") {
            out.push_str(&c.raw.to_string());
        } else {
            out.push('0');
        }
        if let Some(SwiftValue::Bool(b)) = field("autoreversesValue") {
            out.push_str(",\"autoreverses\":");
            out.push_str(if *b { "true" } else { "false" });
        }
    }
    out.push('}');
}

/// Serialize an `AnyTransition` as `{"$":"transition","type":…,…}`. Fields are
/// emitted in a fixed order per type: `type`, then the type-specific payload
/// (`edge` for move/push, `scale`/`anchor?` for a parameterized scale, `x`/`y`
/// for offset), and the recursive combinators (`transitions` for combined,
/// `insertion`/`removal` for asymmetric). See notes.md for the full schema.
fn write_transition(obj: &StructObj, out: &mut String) {
    let field = |name: &str| obj.fields.iter().find(|(k, _)| k == name).map(|(_, v)| v);
    let ty = match field("transitionType") {
        Some(SwiftValue::Str(s)) => s.as_str(),
        _ => "identity",
    };
    out.push_str("{\"$\":\"transition\",\"type\":");
    write_string(ty, out);
    match ty {
        "scale" => {
            if let Some(SwiftValue::Double(d)) = field("scaleValue") {
                out.push_str(",\"scale\":");
                out.push_str(&tswift_core::format_double(*d));
            }
            if let Some(SwiftValue::Str(a)) = field("anchor") {
                out.push_str(",\"anchor\":");
                write_string(a, out);
            }
        }
        "move" | "push" => {
            if let Some(SwiftValue::Str(e)) = field("edge") {
                out.push_str(",\"edge\":");
                write_string(e, out);
            }
        }
        "offset" => {
            out.push_str(",\"x\":");
            out.push_str(&tswift_core::format_double(
                num_field(field("offsetX")).unwrap_or(0.0),
            ));
            out.push_str(",\"y\":");
            out.push_str(&tswift_core::format_double(
                num_field(field("offsetY")).unwrap_or(0.0),
            ));
        }
        "combined" => {
            out.push_str(",\"transitions\":[");
            if let Some(SwiftValue::Array(items)) = field("transitions") {
                for (i, item) in items.iter().enumerate() {
                    if i > 0 {
                        out.push(',');
                    }
                    write_value(item, out);
                }
            }
            out.push(']');
        }
        "asymmetric" => {
            out.push_str(",\"insertion\":");
            match field("insertion") {
                Some(v) => write_value(v, out),
                None => out.push_str("null"),
            }
            out.push_str(",\"removal\":");
            match field("removal") {
                Some(v) => write_value(v, out),
                None => out.push_str("null"),
            }
        }
        _ => {}
    }
    out.push('}');
}

/// Read a Swift numeric field as `f64` (int widened, double as-is).
fn num_field(value: Option<&SwiftValue>) -> Option<f64> {
    match value {
        Some(SwiftValue::Double(d)) => Some(*d),
        Some(SwiftValue::Int(i)) => Some(i.raw as f64),
        _ => None,
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

    /// Prepend the `PRELUDE`, analyze + run `src`, and hand the ready
    /// interpreter to `f`. The single scaffolding all the golden helpers share.
    fn with_interp<R>(src: &str, f: impl FnOnce(&mut Interpreter) -> R) -> R {
        let src = format!("import SwiftUI\n{PRELUDE}\n{src}\n");
        let analysis = tswift_frontend::Analysis::analyze(&src, "t.swift").expect("analyze");
        let analysis: &'static tswift_frontend::Analysis = Box::leak(Box::new(analysis));
        let mut sink = std::io::sink();
        let mut interp = Interpreter::new(&mut sink);
        install(&mut interp);
        interp.run(analysis).expect("run");
        f(&mut interp)
    }

    /// Serialize a bare value expression through `write_value`: build a `Probe`
    /// struct whose computed `member: ty` returns `expr`, then evaluate it. The
    /// path a modifier arg of that type takes (`Animation`, `AnyTransition`, …).
    fn probe_json(member: &str, ty: &str, expr: &str) -> String {
        let decl = format!("struct Probe {{ var {member}: {ty} {{ {expr} }} }}");
        with_interp(&decl, |interp| {
            let probe = interp.make_struct("Probe", &[]).expect("probe");
            let value = interp.get_member(&probe, member).expect("member");
            let mut out = String::new();
            write_value(&value, &mut out);
            out
        })
    }

    fn anim_json(expr: &str) -> String {
        probe_json("anim", "Animation", expr)
    }

    fn transition_json(expr: &str) -> String {
        probe_json("t", "AnyTransition", expr)
    }

    #[test]
    fn transition_opacity_serializes() {
        assert_eq!(
            transition_json("AnyTransition.opacity"),
            r#"{"$":"transition","type":"opacity"}"#
        );
    }

    #[test]
    fn transition_move_edge_serializes() {
        assert_eq!(
            transition_json("AnyTransition.move(edge: .leading)"),
            r#"{"$":"transition","type":"move","edge":"leading"}"#
        );
    }

    #[test]
    fn transition_combined_serializes() {
        assert_eq!(
            transition_json("AnyTransition.opacity.combined(with: .scale)"),
            r#"{"$":"transition","type":"combined","transitions":[{"$":"transition","type":"opacity"},{"$":"transition","type":"scale"}]}"#
        );
    }

    #[test]
    fn transition_asymmetric_serializes() {
        assert_eq!(
            transition_json("AnyTransition.asymmetric(insertion: .scale, removal: .opacity)"),
            r#"{"$":"transition","type":"asymmetric","insertion":{"$":"transition","type":"scale"},"removal":{"$":"transition","type":"opacity"}}"#
        );
    }

    #[test]
    fn transition_modifier_forms_serialize() {
        let opacity = render_json(r#"Text("x").transition(.opacity)"#);
        assert_eq!(
            opacity,
            r#"{"id":"0","kind":"Text","args":{"verbatim":"x"},"modifiers":[{"name":"transition","value":{"$":"transition","type":"opacity"}}],"children":[]}"#
        );
        let mv = render_json(r#"Text("x").transition(.move(edge: .leading))"#);
        assert_eq!(
            mv,
            r#"{"id":"0","kind":"Text","args":{"verbatim":"x"},"modifiers":[{"name":"transition","value":{"$":"transition","type":"move","edge":"leading"}}],"children":[]}"#
        );
        let combined = render_json(r#"Text("x").transition(.opacity.combined(with: .scale))"#);
        assert_eq!(
            combined,
            r#"{"id":"0","kind":"Text","args":{"verbatim":"x"},"modifiers":[{"name":"transition","value":{"$":"transition","type":"combined","transitions":[{"$":"transition","type":"opacity"},{"$":"transition","type":"scale"}]}}],"children":[]}"#
        );
        let asym = render_json(
            r#"Text("x").transition(.asymmetric(insertion: .scale, removal: .opacity))"#,
        );
        assert_eq!(
            asym,
            r#"{"id":"0","kind":"Text","args":{"verbatim":"x"},"modifiers":[{"name":"transition","value":{"$":"transition","type":"asymmetric","insertion":{"$":"transition","type":"scale"},"removal":{"$":"transition","type":"opacity"}}}],"children":[]}"#
        );
    }

    #[test]
    fn animation_ease_in_out_duration_serializes() {
        assert_eq!(
            anim_json("Animation.easeInOut(duration: 0.3)"),
            r#"{"$":"animation","kind":"easeInOut","duration":0.3}"#
        );
    }

    #[test]
    fn animation_linear_repeat_forever_serializes() {
        assert_eq!(
            anim_json("Animation.linear.repeatForever(autoreverses: false)"),
            r#"{"$":"animation","kind":"linear","repeat":"forever","autoreverses":false}"#
        );
    }

    #[test]
    fn animation_spring_defaults_serialize() {
        assert_eq!(
            anim_json("Animation.spring()"),
            r#"{"$":"animation","kind":"spring","response":0.5,"dampingFraction":0.825,"blendDuration":0.0}"#
        );
    }

    #[test]
    fn animation_spring_duration_bounce_serializes() {
        assert_eq!(
            anim_json("Animation.spring(duration: 0.4, bounce: 0.3)"),
            r#"{"$":"animation","kind":"spring","duration":0.4,"bounce":0.3}"#
        );
    }

    #[test]
    fn animation_delay_speed_chain_serializes() {
        assert_eq!(
            anim_json("Animation.easeInOut.delay(0.2).speed(2)"),
            r#"{"$":"animation","kind":"easeInOut","delay":0.2,"speed":2.0}"#
        );
    }

    /// Render a full `View` struct source (with its own `@State`) and serialize.
    fn render_source_json(src: &str, root: &str) -> String {
        with_interp(src, |interp| {
            to_json(&render_root(interp, root).expect("render"))
        })
    }

    /// Render a `body` expression wrapped in a minimal stateless `View`.
    fn render_json(body: &str) -> String {
        render_source_json(
            &format!("struct V: View {{ var body: some View {{ {body} }} }}"),
            "V",
        )
    }

    #[test]
    fn animation_modifier_modern_and_deprecated_forms_serialize() {
        // Modern `.animation(_:value:)` — the curve plus the observed operand.
        let json = render_source_json(
            r#"struct V: View {
    @State private var flag = false
    var body: some View { Text("x").animation(.easeInOut(duration: 0.3), value: flag) }
}"#,
            "V",
        );
        assert_eq!(
            json,
            r#"{"id":"0","kind":"Text","args":{"verbatim":"x"},"modifiers":[{"name":"animation","value":{"animation":{"$":"animation","kind":"easeInOut","duration":0.3},"value":false}}],"children":[]}"#
        );
        // A spring curve with a numeric observed value.
        let json2 = render_source_json(
            r#"struct V: View {
    @State private var n = 0
    var body: some View { Text("x").animation(.spring(), value: n) }
}"#,
            "V",
        );
        assert_eq!(
            json2,
            r#"{"id":"0","kind":"Text","args":{"verbatim":"x"},"modifiers":[{"name":"animation","value":{"animation":{"$":"animation","kind":"spring","response":0.5,"dampingFraction":0.825,"blendDuration":0.0},"value":0}}],"children":[]}"#
        );
        // Deprecated single-arg `.animation(_:)` — curve only, no observed value.
        let json3 = render_json(r#"Text("x").animation(.linear)"#);
        assert_eq!(
            json3,
            r#"{"id":"0","kind":"Text","args":{"verbatim":"x"},"modifiers":[{"name":"animation","value":{"animation":{"$":"animation","kind":"linear"}}}],"children":[]}"#
        );
    }

    #[test]
    fn animation_modifier_nil_disables_without_crashing() {
        // `.animation(nil, value:)` must serialize the curve as JSON `null`.
        let json = render_source_json(
            r#"struct V: View {
    @State private var flag = false
    var body: some View { Text("x").animation(nil, value: flag) }
}"#,
            "V",
        );
        assert_eq!(
            json,
            r#"{"id":"0","kind":"Text","args":{"verbatim":"x"},"modifiers":[{"name":"animation","value":{"animation":null,"value":false}}],"children":[]}"#
        );
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
    fn explicit_rgb_color_serializes_as_host_neutral_rgba() {
        let json = render_json(
            r#"Text("hi").foregroundColor(Color(red: 0.25, green: 0.5, blue: 0.75, opacity: 0.4))"#,
        );
        assert_eq!(
            json,
            r#"{"id":"0","kind":"Text","args":{"verbatim":"hi"},"modifiers":[{"name":"foregroundColor","value":{"$":"color","rgba":[0.25,0.5,0.75,0.4]}}],"children":[]}"#
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
    fn c1_text_styling_modifiers_serialize() {
        let json = render_json(
            r#"Text("hi").bold().italic().opacity(0.5).foregroundStyle(.red).tint(.blue).lineLimit(2).multilineTextAlignment(.center).textCase(.uppercase)"#,
        );
        assert_eq!(
            json,
            r#"{"id":"0","kind":"Text","args":{"verbatim":"hi"},"modifiers":[{"name":"bold","value":null},{"name":"italic","value":null},{"name":"opacity","value":0.5},{"name":"foregroundStyle","value":{"$":"color","name":"red"}},{"name":"tint","value":{"$":"color","name":"blue"}},{"name":"lineLimit","value":2},{"name":"multilineTextAlignment","value":{"$":"textAlign","name":"center"}},{"name":"textCase","value":{"$":"textCase","name":"uppercase"}}],"children":[]}"#
        );
    }

    #[test]
    fn c2_layout_args_and_modifiers_serialize() {
        let json = render_json(
            r#"VStack(spacing: 12) { Spacer(minLength: 4); Text("x").frame(maxWidth: 300, minHeight: 44).offset(x: 2, y: 3) }"#,
        );
        assert_eq!(
            json,
            r#"{"id":"0","kind":"VStack","args":{"spacing":12},"modifiers":[],"children":[{"id":"0.0","kind":"Spacer","args":{"minLength":4},"modifiers":[],"children":[]},{"id":"0.1","kind":"Text","args":{"verbatim":"x"},"modifiers":[{"name":"frame","value":{"maxWidth":300,"minHeight":44}},{"name":"offset","value":{"x":2,"y":3}}],"children":[]}]}"#
        );
    }

    #[test]
    fn non_finite_frame_bound_serializes_as_json_valid_sentinel() {
        // Deferred `.infinity` must never produce invalid JSON (issue #189).
        let json = render_json(r#"Text("x").frame(maxWidth: Double.infinity)"#);
        assert_eq!(
            json,
            r#"{"id":"0","kind":"Text","args":{"verbatim":"x"},"modifiers":[{"name":"frame","value":{"maxWidth":{"$":"infinity"}}}],"children":[]}"#
        );
    }

    #[test]
    fn compositing_modifiers_serialize_nested_view_and_alignment() {
        // `.background(view)` lowers to a `0`-rooted nested node; `.overlay(view,
        // alignment:)` wraps it with the alignment token; a color background
        // stays the bare token (C0 backward compatibility) — issue #204.
        let json = render_json(
            r#"Text("Hi").background(Circle().fill(.blue)).overlay(Text("X"), alignment: .topTrailing).background(.yellow)"#,
        );
        assert_eq!(
            json,
            r#"{"id":"0","kind":"Text","args":{"verbatim":"Hi"},"modifiers":[{"name":"background","value":{"id":"0","kind":"Circle","args":{},"modifiers":[{"name":"fill","value":{"$":"color","name":"blue"}}],"children":[]}},{"name":"overlay","value":{"value":{"id":"0","kind":"Text","args":{"verbatim":"X"},"modifiers":[],"children":[]},"alignment":{"$":"align","name":"topTrailing"}}},{"name":"background","value":{"$":"color","name":"yellow"}}],"children":[]}"#
        );
    }

    #[test]
    fn progress_view_label_serializes_as_arg() {
        // `ProgressView("…")`'s title becomes a `label` arg (issue #206); a
        // value-only ProgressView carries no `label`.
        let titled = render_json(r#"ProgressView("Loading", value: 0.4)"#);
        assert_eq!(
            titled,
            r#"{"id":"0","kind":"ProgressView","args":{"label":"Loading","value":0.4},"modifiers":[],"children":[]}"#
        );
        let bare = render_json(r#"ProgressView(value: 0.4)"#);
        assert_eq!(
            bare,
            r#"{"id":"0","kind":"ProgressView","args":{"value":0.4},"modifiers":[],"children":[]}"#
        );
    }

    #[test]
    fn c6_lazy_grid_serializes_griditem_array() {
        // `[GridItem]` serializes as a JSON array of `{kind,value,spacing?}`
        // objects (issue #205). `.flexible()`/`.fixed(_)`/`.adaptive(minimum:)`
        // resolve against `GridItem` via the typed `columns:` signature.
        let json = render_json(
            r#"LazyVGrid(columns: [.flexible(), .fixed(80), .adaptive(minimum: 50)], spacing: 12) { Text("a") }"#,
        );
        assert_eq!(
            json,
            r#"{"id":"0","kind":"LazyVGrid","args":{"columns":[{"kind":"flexible","value":10.0},{"kind":"fixed","value":80.0},{"kind":"adaptive","value":50.0}],"spacing":12},"modifiers":[],"children":[{"id":"0.0","kind":"Text","args":{"verbatim":"a"},"modifiers":[],"children":[]}]}"#
        );
    }

    #[test]
    fn c3_structural_containers_serialize() {
        let json = render_json(r#"ScrollView(.horizontal) { Group { Text("a"); Divider() } }"#);
        assert_eq!(
            json,
            r#"{"id":"0","kind":"ScrollView","args":{"axes":{"$":"axis","name":"horizontal"}},"modifiers":[],"children":[{"id":"0.0","kind":"Group","args":{},"modifiers":[],"children":[{"id":"0.0.0","kind":"Text","args":{"verbatim":"a"},"modifiers":[],"children":[]},{"id":"0.0.1","kind":"Divider","args":{},"modifiers":[],"children":[]}]}]}"#
        );
    }

    #[test]
    fn c4_decoration_modifiers_serialize() {
        let json = render_json(
            r#"Text("x").border(.red, width: 2).shadow(color: .gray, radius: 4, x: 0, y: 2).clipShape(Circle()).clipped()"#,
        );
        assert_eq!(
            json,
            r#"{"id":"0","kind":"Text","args":{"verbatim":"x"},"modifiers":[{"name":"border","value":{"value":{"$":"color","name":"red"},"width":2}},{"name":"shadow","value":{"color":{"$":"color","name":"gray"},"radius":4,"x":0,"y":2}},{"name":"clipShape","value":{"id":"0","kind":"Circle","args":{},"modifiers":[],"children":[]}},{"name":"clipped","value":null}],"children":[]}"#
        );
    }

    #[test]
    fn c5_content_views_serialize() {
        let json = render_json(
            r#"VStack { Label("Home", systemImage: "house.fill"); Image(systemName: "star.fill"); Image("photo"); ProgressView(value: 0.4) }"#,
        );
        assert_eq!(
            json,
            r#"{"id":"0","kind":"VStack","args":{},"modifiers":[],"children":[{"id":"0.0","kind":"Label","args":{"title":"Home","systemImage":"house.fill"},"modifiers":[],"children":[]},{"id":"0.1","kind":"Image","args":{"systemName":"star.fill"},"modifiers":[],"children":[]},{"id":"0.2","kind":"Image","args":{"name":"photo"},"modifiers":[],"children":[]},{"id":"0.3","kind":"ProgressView","args":{"value":0.4},"modifiers":[],"children":[]}]}"#
        );
    }

    #[test]
    fn c6_grids_and_lazy_stacks_serialize() {
        let json = render_json(
            r#"Form { LazyVStack(spacing: 4) { Text("a") }; Grid { GridRow { Text("x"); Text("y") } } }"#,
        );
        assert_eq!(
            json,
            r#"{"id":"0","kind":"Form","args":{},"modifiers":[],"children":[{"id":"0.0","kind":"LazyVStack","args":{"spacing":4},"modifiers":[],"children":[{"id":"0.0.0","kind":"Text","args":{"verbatim":"a"},"modifiers":[],"children":[]}]},{"id":"0.1","kind":"Grid","args":{},"modifiers":[],"children":[{"id":"0.1.0","kind":"GridRow","args":{},"modifiers":[],"children":[{"id":"0.1.0.0","kind":"Text","args":{"verbatim":"x"},"modifiers":[],"children":[]},{"id":"0.1.0.1","kind":"Text","args":{"verbatim":"y"},"modifiers":[],"children":[]}]}]}]}"#
        );
    }

    #[test]
    fn c7_control_styling_and_accessibility_serialize() {
        let json = render_json(
            r#"Button("Save") { }.buttonStyle(.borderedProminent).disabled(true).accessibilityLabel("save button")"#,
        );
        assert_eq!(
            json,
            r#"{"id":"0","kind":"Button","args":{"title":"Save"},"modifiers":[{"name":"buttonStyle","value":{"$":"style","name":"borderedProminent"}},{"name":"disabled","value":true},{"name":"accessibilityLabel","value":"save button"}],"children":[]}"#
        );
    }

    #[test]
    fn tier2_scale_aspect_layout_modifiers_serialize() {
        // scaledToFit / scaledToFill (no-arg markers).
        let json = render_json(r#"Image("photo").resizable().scaledToFit().scaledToFill()"#);
        assert_eq!(
            json,
            r#"{"id":"0","kind":"Image","args":{"name":"photo"},"modifiers":[{"name":"resizable","value":null},{"name":"scaledToFit","value":null},{"name":"scaledToFill","value":null}],"children":[]}"#
        );
        // aspectRatio with ContentMode token.
        let json2 = render_json(r#"Rectangle().aspectRatio(1.777, contentMode: .fit)"#);
        assert_eq!(
            json2,
            r#"{"id":"0","kind":"Rectangle","args":{},"modifiers":[{"name":"aspectRatio","value":{"value":1.777,"contentMode":{"$":"contentMode","name":"fit"}}}],"children":[]}"#
        );
        // fixedSize no-arg vs horizontal/vertical.
        let json3 =
            render_json(r#"Text("hi").fixedSize().fixedSize(horizontal: true, vertical: false)"#);
        assert_eq!(
            json3,
            r#"{"id":"0","kind":"Text","args":{"verbatim":"hi"},"modifiers":[{"name":"fixedSize","value":null},{"name":"fixedSize","value":{"horizontal":true,"vertical":false}}],"children":[]}"#
        );
        // layoutPriority, zIndex, navigationTitle.
        let json4 =
            render_json(r#"Text("x").layoutPriority(1.0).zIndex(2.0).navigationTitle("MyTitle")"#);
        assert_eq!(
            json4,
            r#"{"id":"0","kind":"Text","args":{"verbatim":"x"},"modifiers":[{"name":"layoutPriority","value":1.0},{"name":"zIndex","value":2.0},{"name":"navigationTitle","value":"MyTitle"}],"children":[]}"#
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
