//! tswift-swiftui — SwiftUI view primitives as runtime builtins.
//!
//! SwiftUI is a render-host framework, not value semantics: the interpreter
//! evaluates a `View`'s `body` into a tree of *view values* (the host-neutral
//! UIIR), a Rust diff engine turns successive trees into a keyed patch stream,
//! and a thin host applies the patches. See `docs/adr/0006-swiftui-render-host.md`
//! and `docs/plan/swiftui-support.md`.
//!
//! This crate mirrors the `tswift-foundation` registry seam: [`install`] wires
//! the view constructors into an interpreter, and [`registered_keys`] exposes
//! the live registry to the framework-inventory coverage tooling (Layer A).
//!
//! A *view value* is a [`SwiftValue::Struct`] carrying the SwiftUI type name and
//! a flat, ordered `_modifiers` field (appended copy-on-write by `.font(_:)`
//! &c.). Container views additionally carry a `_children` field. The view-value
//! tree *is* the UIIR — no `tswift-core` change is needed for view values.

use std::rc::Rc;

pub mod session;
pub mod uiir;

use tswift_core::{
    Arg, EvalError, Interpreter, StdContext, StdError, StdResult, StructMethodFn, StructObj,
    SwiftValue,
};

/// Field name holding a view's ordered modifier list.
pub const MODIFIERS_FIELD: &str = "_modifiers";
/// Field name holding a container view's ordered child views.
pub const CHILDREN_FIELD: &str = "_children";
/// Field name holding a view's primary action closure (`Button`'s `action`).
pub const ACTION_FIELD: &str = "_action";
/// Type name of an appended modifier record (`_Modifier { name, <args> }`).
pub const MODIFIER_TYPE: &str = "_Modifier";

/// Define a view-modifier intrinsic that appends a named `_Modifier` record to
/// the receiver view (copy-on-write). All v1 modifiers share this shape; the
/// host interprets the recorded name + args.
macro_rules! modifier {
    ($fn_name:ident, $swift_name:literal) => {
        fn $fn_name(_ctx: &mut dyn StdContext, recv: SwiftValue, args: Vec<Arg>) -> StdResult {
            append_modifier(recv, make_modifier($swift_name, args))
        }
    };
}

modifier!(modifier_frame, "frame");
modifier!(modifier_padding, "padding");
modifier!(modifier_corner_radius, "cornerRadius");
modifier!(modifier_font, "font");
modifier!(modifier_font_weight, "fontWeight");
modifier!(modifier_foreground_color, "foregroundColor");
modifier!(modifier_background, "background");

/// View modifiers registered as generic struct methods, by Swift name. Drives
/// both [`install`] and the `View.<name>` coverage keys in [`registered_keys`].
const MODIFIER_FNS: &[(&str, StructMethodFn)] = &[
    ("frame", modifier_frame),
    ("padding", modifier_padding),
    ("cornerRadius", modifier_corner_radius),
    ("font", modifier_font),
    ("fontWeight", modifier_font_weight),
    ("foregroundColor", modifier_foreground_color),
    ("background", modifier_background),
];

/// SwiftUI token namespaces, defined in Swift so `Color.blue` / `.largeTitle` /
/// `.bold` resolve to lightweight token structs the host later interprets
/// (the semantic-token value encoding from the plan, §3.1). Each token is a
/// `static let` carrying a single `token` string; leading-dot forms resolve via
/// the runtime's unique-static lookup. Prepended to user source before running.
///
/// Note: a leading-dot token shared by two namespaces (e.g. `.black` is both a
/// `Color` and a `FontWeight`) is ambiguous without contextual typing; write
/// the qualified form (`Color.black`) in that case.
pub const PRELUDE: &str = r#"
class _StateBox<Value> {
    var value: Value
    init(_ v: Value) { value = v }
}
@propertyWrapper
struct State<Value> {
    let box: _StateBox<Value>
    var wrappedValue: Value {
        get { box.value }
        set { box.value = newValue }
    }
    init(wrappedValue: Value) { box = _StateBox(wrappedValue) }
}
struct Color {
    let token: String
    static let primary = Color(token: "primary")
    static let secondary = Color(token: "secondary")
    static let white = Color(token: "white")
    static let black = Color(token: "black")
    static let red = Color(token: "red")
    static let orange = Color(token: "orange")
    static let yellow = Color(token: "yellow")
    static let green = Color(token: "green")
    static let mint = Color(token: "mint")
    static let teal = Color(token: "teal")
    static let cyan = Color(token: "cyan")
    static let blue = Color(token: "blue")
    static let indigo = Color(token: "indigo")
    static let purple = Color(token: "purple")
    static let pink = Color(token: "pink")
    static let brown = Color(token: "brown")
    static let gray = Color(token: "gray")
    static let clear = Color(token: "clear")
}
struct Font {
    let token: String
    static let largeTitle = Font(token: "largeTitle")
    static let title = Font(token: "title")
    static let title2 = Font(token: "title2")
    static let title3 = Font(token: "title3")
    static let headline = Font(token: "headline")
    static let subheadline = Font(token: "subheadline")
    static let body = Font(token: "body")
    static let callout = Font(token: "callout")
    static let caption = Font(token: "caption")
    static let caption2 = Font(token: "caption2")
    static let footnote = Font(token: "footnote")
}
struct FontWeight {
    let token: String
    static let ultraLight = FontWeight(token: "ultraLight")
    static let thin = FontWeight(token: "thin")
    static let light = FontWeight(token: "light")
    static let regular = FontWeight(token: "regular")
    static let medium = FontWeight(token: "medium")
    static let semibold = FontWeight(token: "semibold")
    static let bold = FontWeight(token: "bold")
    static let heavy = FontWeight(token: "heavy")
    static let black = FontWeight(token: "black")
}
"#;

/// The token string carried by a prelude token struct (`Color`/`Font`/
/// `FontWeight`), if `value` is one.
pub fn token_of(value: &SwiftValue) -> Option<(&str, &str)> {
    let SwiftValue::Struct(obj) = value else {
        return None;
    };
    if !matches!(obj.type_name.as_str(), "Color" | "Font" | "FontWeight") {
        return None;
    }
    match obj.get("token") {
        Some(SwiftValue::Str(s)) => Some((obj.type_name.as_str(), s.as_str())),
        _ => None,
    }
}

/// Register every currently-supported SwiftUI view constructor and modifier
/// into `interp`.
pub fn install(interp: &mut Interpreter<'_>) {
    interp.register_free_fn("Text", text_init);
    interp.register_free_fn("VStack", vstack_init);
    interp.register_free_fn("HStack", hstack_init);
    interp.register_free_fn("Spacer", spacer_init);
    interp.register_free_fn("Button", button_init);

    for (name, func) in MODIFIER_FNS {
        interp.register_struct_method(name, *func);
    }
}

/// Render `root_type`'s `body` into a view-value tree (the UIIR root). The
/// interpreter must already have run the program so `root_type` is declared.
pub fn render_root(interp: &mut Interpreter<'_>, root_type: &str) -> Result<SwiftValue, EvalError> {
    let view = interp.make_struct(root_type, &[])?;
    interp.get_member(&view, "body")
}

/// Every SwiftUI entry registered by [`install`], as coverage keys
/// (`Type.member`, matching `tools/framework-inventory/coverage.py`).
pub fn registered_keys() -> Vec<String> {
    let mut sink = std::io::sink();
    let mut interp = Interpreter::new(&mut sink);
    install(&mut interp);
    let mut keys: Vec<String> = interp
        .registered_keys()
        .into_iter()
        .filter_map(|key| match key.as_str() {
            "Text" | "VStack" | "HStack" | "Spacer" | "Button" => Some(format!("{key}.init")),
            _ => None,
        })
        .collect();
    // Modifiers are members of `View` for coverage purposes.
    keys.extend(MODIFIER_FNS.iter().map(|(m, _)| format!("View.{m}")));
    keys.sort();
    keys.dedup();
    keys
}

/// Build a view value: a struct carrying `type_name` plus any constructor
/// fields, an empty ordered `_modifiers` list, and (for containers) `_children`.
fn view_value(type_name: &str, mut fields: Vec<(String, SwiftValue)>) -> SwiftValue {
    fields.push((
        MODIFIERS_FIELD.into(),
        SwiftValue::Array(Rc::new(Vec::new())),
    ));
    SwiftValue::Struct(Rc::new(StructObj {
        type_name: type_name.into(),
        fields,
    }))
}

/// Build a container view value with an ordered `_children` list.
fn container_value(type_name: &str, children: Vec<SwiftValue>) -> SwiftValue {
    view_value(
        type_name,
        vec![(CHILDREN_FIELD.into(), SwiftValue::Array(Rc::new(children)))],
    )
}

/// `Text(_ verbatim: String)` — the leaf text view.
fn text_init(_ctx: &mut dyn StdContext, args: Vec<Arg>) -> StdResult {
    let verbatim = match args.into_iter().next() {
        Some(arg) => match arg.value {
            SwiftValue::Str(s) => s,
            other => other.to_string(),
        },
        None => String::new(),
    };
    Ok(view_value(
        "Text",
        vec![("verbatim".into(), SwiftValue::Str(verbatim))],
    ))
}

/// `VStack { ... }` — vertical container. Children arrive via the `@ViewBuilder`
/// shim: the trailing closure is evaluated as a result-builder block and each
/// view-valued statement becomes a child.
fn vstack_init(ctx: &mut dyn StdContext, args: Vec<Arg>) -> StdResult {
    Ok(container_value("VStack", collect_children(ctx, args)?))
}

/// `HStack { ... }` — horizontal container.
fn hstack_init(ctx: &mut dyn StdContext, args: Vec<Arg>) -> StdResult {
    Ok(container_value("HStack", collect_children(ctx, args)?))
}

/// `Spacer()` — flexible empty space.
fn spacer_init(_ctx: &mut dyn StdContext, _args: Vec<Arg>) -> StdResult {
    Ok(view_value("Spacer", Vec::new()))
}

/// `Button(_ title) { action }` — a titled button. The leading positional is
/// the title string; the trailing closure is the tap action, stored as an
/// `_action` closure value the dispatch loop invokes on a `tap` event.
fn button_init(_ctx: &mut dyn StdContext, args: Vec<Arg>) -> StdResult {
    let mut title = String::new();
    let mut action: Option<SwiftValue> = None;
    for arg in args {
        match arg.value {
            SwiftValue::Closure(_) => action = Some(arg.value),
            SwiftValue::Str(s) if action.is_none() => title = s,
            other if title.is_empty() && action.is_none() => title = other.to_string(),
            _ => {}
        }
    }
    let mut fields = vec![("title".into(), SwiftValue::Str(title))];
    if let Some(action) = action {
        fields.push((ACTION_FIELD.into(), action));
    }
    Ok(view_value("Button", fields))
}

/// Resolve a container's `@ViewBuilder` content into an ordered child list.
/// Each argument is either the content closure (evaluated as a result-builder
/// block) or an already-built view; non-view statement values are dropped.
fn collect_children(ctx: &mut dyn StdContext, args: Vec<Arg>) -> Result<Vec<SwiftValue>, StdError> {
    let mut out = Vec::new();
    for arg in args {
        match arg.value {
            SwiftValue::Closure(id) => {
                let block = ctx.eval_block_values(id)?;
                push_views(&block, &mut out);
            }
            other => push_views(&other, &mut out),
        }
    }
    Ok(out)
}

/// Append every view value reachable in `value` (flattening nested arrays a
/// builder shim produces) to `out`, dropping non-view results.
fn push_views(value: &SwiftValue, out: &mut Vec<SwiftValue>) {
    match value {
        SwiftValue::Array(items) => {
            for item in items.iter() {
                push_views(item, out);
            }
        }
        v if view_type_name(v).is_some() => out.push(v.clone()),
        _ => {}
    }
}

/// Build a `_Modifier` record: a struct carrying `name` plus each call argument
/// as a field keyed by its label (positional args use `value`, `value1`, …).
fn make_modifier(name: &str, args: Vec<Arg>) -> SwiftValue {
    let mut fields: Vec<(String, SwiftValue)> = vec![("name".into(), SwiftValue::Str(name.into()))];
    let mut positional = 0usize;
    for arg in args {
        let key = match arg.label {
            Some(label) => label,
            None => {
                let key = if positional == 0 {
                    "value".to_string()
                } else {
                    format!("value{positional}")
                };
                positional += 1;
                key
            }
        };
        fields.push((key, arg.value));
    }
    SwiftValue::Struct(Rc::new(StructObj {
        type_name: MODIFIER_TYPE.into(),
        fields,
    }))
}

/// Append `modifier` to `view`'s ordered `_modifiers` list, returning a new view
/// value (copy-on-write; the original is untouched).
fn append_modifier(view: SwiftValue, modifier: SwiftValue) -> StdResult {
    let SwiftValue::Struct(obj) = &view else {
        return Err(type_error(format!(
            "view modifier applied to non-view value `{}`",
            view.type_name()
        )));
    };
    let mut fields = obj.fields.clone();
    let slot = fields
        .iter_mut()
        .find(|(k, _)| k == MODIFIERS_FIELD)
        .map(|(_, v)| v)
        .ok_or_else(|| type_error("view value is missing its `_modifiers` field"))?;
    let mut mods = match slot {
        SwiftValue::Array(items) => (**items).clone(),
        _ => Vec::new(),
    };
    mods.push(modifier);
    *slot = SwiftValue::Array(Rc::new(mods));
    Ok(SwiftValue::Struct(Rc::new(StructObj {
        type_name: obj.type_name.clone(),
        fields,
    })))
}

fn type_error(message: impl Into<String>) -> StdError {
    StdError::Error(EvalError::Type(message.into()))
}

/// Returns the SwiftUI type name of a view value, if it is one.
pub fn view_type_name(value: &SwiftValue) -> Option<&str> {
    match value {
        SwiftValue::Struct(obj) if obj.fields.iter().any(|(k, _)| k == MODIFIERS_FIELD) => {
            Some(obj.type_name.as_str())
        }
        _ => None,
    }
}

#[cfg(test)]
mod coverage_dump {
    #[test]
    fn dump_registered_keys() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let path = root.join("frameworks/swiftui/registered_keys.txt");
        let body = super::registered_keys().join("\n") + "\n";
        std::fs::write(&path, body).expect("write registered_keys.txt");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registered_keys_cover_v1_constructors() {
        let keys = registered_keys();
        assert_eq!(
            keys,
            vec![
                "Button.init",
                "HStack.init",
                "Spacer.init",
                "Text.init",
                "VStack.init",
                "View.background",
                "View.cornerRadius",
                "View.font",
                "View.fontWeight",
                "View.foregroundColor",
                "View.frame",
                "View.padding",
            ]
        );
    }

    #[test]
    fn render_root_captures_button_title_and_action() {
        let src = r#"
struct V: View {
    var body: some View {
        Button("Increment") { }
    }
}
"#;
        let view = render_to_string(src, "V");
        assert_eq!(view_type_name(&view), Some("Button"));
        let SwiftValue::Struct(obj) = &view else {
            panic!("expected struct");
        };
        assert_eq!(obj.get("title"), Some(&SwiftValue::Str("Increment".into())));
        assert!(
            matches!(obj.get(ACTION_FIELD), Some(SwiftValue::Closure(_))),
            "button should capture its action closure"
        );
    }

    #[test]
    fn render_root_interpolates_text() {
        let src = r#"
struct V: View {
    var body: some View {
        Text("count: \(1 + 1)")
    }
}
"#;
        let view = render_to_string(src, "V");
        let SwiftValue::Struct(obj) = &view else {
            panic!("expected struct");
        };
        assert_eq!(
            obj.get("verbatim"),
            Some(&SwiftValue::Str("count: 2".into()))
        );
    }

    #[test]
    fn render_root_captures_color_and_font_tokens() {
        let src = r#"
struct V: View {
    var body: some View {
        Text("x")
            .font(.largeTitle)
            .fontWeight(.bold)
            .foregroundColor(.white)
            .background(Color.blue)
    }
}
"#;
        let view = render_to_string(src, "V");
        let mods = modifiers_of(&view);
        let tokens: Vec<(String, String)> = mods
            .iter()
            .filter_map(|m| match m {
                SwiftValue::Struct(o) => o
                    .get("value")
                    .and_then(token_of)
                    .map(|(t, n)| (t.to_string(), n.to_string())),
                _ => None,
            })
            .collect();
        assert_eq!(
            tokens,
            vec![
                ("Font".to_string(), "largeTitle".to_string()),
                ("FontWeight".to_string(), "bold".to_string()),
                ("Color".to_string(), "white".to_string()),
                ("Color".to_string(), "blue".to_string()),
            ]
        );
    }

    #[test]
    fn render_root_collects_vstack_children() {
        let src = r#"
struct V: View {
    var body: some View {
        VStack {
            Text("a")
            Text("b")
        }
    }
}
"#;
        let view = render_to_string(src, "V");
        assert_eq!(view_type_name(&view), Some("VStack"));
        let SwiftValue::Struct(obj) = &view else {
            panic!("expected struct");
        };
        let Some(SwiftValue::Array(children)) = obj.get(CHILDREN_FIELD) else {
            panic!("expected children array");
        };
        assert_eq!(children.len(), 2);
        assert_eq!(view_type_name(&children[0]), Some("Text"));
        assert_eq!(view_type_name(&children[1]), Some("Text"));
    }

    #[test]
    fn render_root_chains_modifiers_in_order() {
        let src = r#"
struct V: View {
    var body: some View {
        Text("x").padding().cornerRadius(8)
    }
}
"#;
        let view = render_to_string(src, "V");
        let mods = modifiers_of(&view);
        let names: Vec<String> = mods
            .iter()
            .filter_map(|m| match m {
                SwiftValue::Struct(o) => match o.get("name") {
                    Some(SwiftValue::Str(s)) => Some(s.clone()),
                    _ => None,
                },
                _ => None,
            })
            .collect();
        assert_eq!(names, vec!["padding", "cornerRadius"]);
        // cornerRadius carries its numeric value positionally.
        let SwiftValue::Struct(corner) = &mods[1] else {
            panic!("expected struct");
        };
        assert_eq!(corner.get("value"), Some(&SwiftValue::int(8)));
    }

    #[test]
    fn render_root_applies_frame_modifier() {
        let src = r#"
struct V: View {
    var body: some View {
        Text("hi").frame(width: 56, height: 56)
    }
}
"#;
        let view = render_to_string(src, "V");
        // Text leaf carrying one `frame` modifier with numeric width/height.
        assert_eq!(view_type_name(&view), Some("Text"));
        let mods = modifiers_of(&view);
        assert_eq!(mods.len(), 1);
        let SwiftValue::Struct(m) = &mods[0] else {
            panic!("modifier should be a struct");
        };
        assert_eq!(m.type_name, "_Modifier");
        assert_eq!(m.get("name"), Some(&SwiftValue::Str("frame".into())));
    }

    /// Render `root_type`'s `body` from `src` for assertions, with the token
    /// prelude prepended (as the render CLI will do).
    fn render_to_string(src: &str, root_type: &str) -> SwiftValue {
        let program = format!("{PRELUDE}\n{src}");
        let analysis = tswift_frontend::Analysis::analyze(&program, "test.swift").expect("analyze");
        let analysis: &'static tswift_frontend::Analysis = Box::leak(Box::new(analysis));
        let mut sink = std::io::sink();
        let mut interp = Interpreter::new(&mut sink);
        install(&mut interp);
        interp.run(analysis).expect("run");
        render_root(&mut interp, root_type).expect("render")
    }

    fn modifiers_of(view: &SwiftValue) -> Vec<SwiftValue> {
        let SwiftValue::Struct(obj) = view else {
            return Vec::new();
        };
        match obj.get(MODIFIERS_FIELD) {
            Some(SwiftValue::Array(items)) => items.iter().cloned().collect(),
            _ => Vec::new(),
        }
    }

    #[test]
    fn text_value_carries_verbatim_and_modifiers() {
        let mut sink = std::io::sink();
        let mut interp = Interpreter::new(&mut sink);
        install(&mut interp);
        let v = text_init(
            &mut interp,
            vec![Arg::positional(SwiftValue::Str("hi".into()))],
        )
        .unwrap();
        assert_eq!(view_type_name(&v), Some("Text"));
        let SwiftValue::Struct(obj) = &v else {
            panic!("expected struct");
        };
        assert_eq!(obj.fields[0].0, "verbatim");
        assert_eq!(obj.fields[1].0, MODIFIERS_FIELD);
    }
}
