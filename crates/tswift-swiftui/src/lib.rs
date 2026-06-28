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

pub mod diff;
pub mod session;
pub mod uiir;

use tswift_core::{
    Arg, EvalError, Interpreter, StdContext, StdError, StdResult, StructMethodFn, StructObj,
    SwiftValue,
};
use tswift_frontend::{Analysis, Node, NodeKind};

/// Field name holding a view's ordered modifier list.
pub const MODIFIERS_FIELD: &str = "_modifiers";
/// Field name holding a container view's ordered child views.
pub const CHILDREN_FIELD: &str = "_children";
/// Field name holding a view's primary action closure (`Button`'s `action`).
pub const ACTION_FIELD: &str = "_action";
/// Type name of an appended modifier record (`_Modifier { name, <args> }`).
pub const MODIFIER_TYPE: &str = "_Modifier";
/// Field name holding a `ForEach`-generated child's stable identity key. When
/// present, the child's UIIR id is `{parent}.{key}` (not `{parent}.{index}`) so
/// the keyed diff can emit `move` instead of replacing reordered rows.
pub const KEY_FIELD: &str = "_key";

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
modifier!(modifier_fill, "fill");
modifier!(modifier_tag, "tag");

/// Field holding the `ObservableObject`s a view provides to its subtree via
/// `.environmentObject(_)`. Unlike a visual modifier this never reaches the
/// UIIR — it is consumed by the renderer to inject `@EnvironmentObject` slots.
/// Stored separately from `_modifiers` so a custom `View` (which has no
/// `_modifiers`) can still carry it without looking like a builtin view value.
pub const ENV_FIELD: &str = "_env";

/// `.environmentObject(_ object)` — provide an `ObservableObject` to this view
/// and its subtree. The object is appended to the view's `_env` list (not
/// `_modifiers`), to be injected into descendant `@EnvironmentObject` slots.
fn modifier_environment_object(
    _ctx: &mut dyn StdContext,
    recv: SwiftValue,
    args: Vec<Arg>,
) -> StdResult {
    let object = args
        .into_iter()
        .next()
        .map(|a| a.value)
        .unwrap_or(SwiftValue::Nil);
    let SwiftValue::Struct(obj) = &recv else {
        return Err(type_error(format!(
            "environmentObject applied to non-view value `{}`",
            recv.type_name()
        )));
    };
    let mut fields = obj.fields.clone();
    if !fields.iter().any(|(k, _)| k == ENV_FIELD) {
        fields.push((ENV_FIELD.into(), SwiftValue::Array(Rc::new(Vec::new()))));
    }
    let slot = fields
        .iter_mut()
        .find(|(k, _)| k == ENV_FIELD)
        .map(|(_, v)| v)
        .expect("_env slot ensured above");
    let mut list = match slot {
        SwiftValue::Array(items) => (**items).clone(),
        _ => Vec::new(),
    };
    list.push(object);
    *slot = SwiftValue::Array(Rc::new(list));
    Ok(SwiftValue::Struct(Rc::new(StructObj {
        type_name: obj.type_name.clone(),
        fields,
    })))
}

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
    ("fill", modifier_fill),
    ("tag", modifier_tag),
    ("environmentObject", modifier_environment_object),
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
// A two-way connection to a `_StateBox`. Its setter is `nonmutating` because it
// writes through the shared reference box, so a `let` binding (as stored inside
// a control) can still drive the source `@State`.
struct Binding<Value> {
    let box: _StateBox<Value>
    var wrappedValue: Value {
        get { box.value }
        nonmutating set { box.value = newValue }
    }
}
@propertyWrapper
struct State<Value> {
    let box: _StateBox<Value>
    var wrappedValue: Value {
        get { box.value }
        set { box.value = newValue }
    }
    // `$flag` yields a `Binding` onto the same box (two-way data flow).
    var projectedValue: Binding<Value> { Binding(box: box) }
    init(wrappedValue: Value) { box = _StateBox(wrappedValue) }
}
// Observation. The render host re-evaluates `body` after every event, so a
// mutated `@Published` property is reflected on the next render without any
// Combine publisher — these wrappers only need reference-stable storage.
protocol ObservableObject {}
@propertyWrapper
struct Published<Value> {
    var wrappedValue: Value
    init(wrappedValue: Value) { self.wrappedValue = wrappedValue }
}
// `@StateObject` owns its `ObservableObject`; `@ObservedObject` receives one it
// does not own. Both hold a class instance, so *interior* mutations through
// `wrappedValue` (`model.x = …`, `model.method()`) persist by reference and the
// next render sees them. v1 limits (see plan §4.1): the root view instance is
// the only one kept across renders, so a nested custom view's inline
// `@StateObject` is re-created each render; and reassigning the whole object
// (`model = Model()`) does not persist — mutate through the reference instead.
@propertyWrapper
struct StateObject<ObjectType> {
    var wrappedValue: ObjectType
    init(wrappedValue: ObjectType) { self.wrappedValue = wrappedValue }
}
@propertyWrapper
struct ObservedObject<ObjectType> {
    var wrappedValue: ObjectType
    init(wrappedValue: ObjectType) { self.wrappedValue = wrappedValue }
}
// `@EnvironmentObject var x: T` has no initializer — it is injected from an
// ancestor's `.environmentObject(_)`. The wrapper's no-argument `init()` lets
// the view be constructed with the slot empty; the render host fills `store`
// before evaluating `body`. Reading it before injection traps (force-unwrap),
// matching SwiftUI's "no ObservableObject of type … found" precondition.
@propertyWrapper
struct EnvironmentObject<ObjectType> {
    var store: ObjectType?
    var wrappedValue: ObjectType { store! }
    init() { store = nil }
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
    interp.register_free_fn("ZStack", zstack_init);
    interp.register_free_fn("ForEach", foreach_init);
    interp.register_free_fn("List", list_init);
    interp.register_free_fn("Section", section_init);
    interp.register_free_fn("Spacer", spacer_init);
    interp.register_free_fn("Button", button_init);
    interp.register_free_fn("Toggle", toggle_init);
    interp.register_free_fn("TextField", text_field_init);
    interp.register_free_fn("SecureField", secure_field_init);
    interp.register_free_fn("Slider", slider_init);
    interp.register_free_fn("Stepper", stepper_init);
    interp.register_free_fn("Picker", picker_init);
    interp.register_free_fn("Circle", circle_init);
    interp.register_free_fn("Rectangle", rectangle_init);
    interp.register_free_fn("RoundedRectangle", rounded_rectangle_init);
    interp.register_free_fn("Capsule", capsule_init);
    interp.register_free_fn("Ellipse", ellipse_init);

    for (name, func) in MODIFIER_FNS {
        interp.register_struct_method(name, *func);
    }
}

/// Render `root_type`'s `body` into a view-value tree (the UIIR root). The
/// interpreter must already have run the program so `root_type` is declared.
pub fn render_root(interp: &mut Interpreter<'_>, root_type: &str) -> Result<SwiftValue, EvalError> {
    let view = interp.make_struct(root_type, &[])?;
    let body = interp.get_member(&view, "body")?;
    resolve_root(interp, body).map_err(std_error_to_eval)
}

/// Collapse a [`StdError`] back to an [`EvalError`] for the render entry points.
fn std_error_to_eval(err: StdError) -> EvalError {
    match err {
        StdError::Error(e) => e,
        StdError::Throw(v) => EvalError::Type(format!("thrown: {}", v.type_name())),
    }
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
            "Text" | "VStack" | "HStack" | "ZStack" | "ForEach" | "List" | "Section" | "Spacer"
            | "Button" | "Toggle" | "TextField" | "SecureField" | "Slider" | "Stepper"
            | "Picker" | "Circle" | "Rectangle" | "RoundedRectangle" | "Capsule" | "Ellipse" => {
                Some(format!("{key}.init"))
            }
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

/// `ZStack { ... }` — depth (overlay) container; children stack back-to-front.
fn zstack_init(ctx: &mut dyn StdContext, args: Vec<Arg>) -> StdResult {
    Ok(container_value("ZStack", collect_children(ctx, args)?))
}

/// `ForEach(_ data, id:, content:)` — a keyed sequence of views. Each element
/// of `data` is passed to the `content` builder; the produced view(s) are
/// tagged with a stable identity key so the diff can `move` reordered rows
/// rather than rebuild them. The key comes from the `id:` key-path argument
/// (e.g. `\.self` or `\.name`), else the element's `id` member (an
/// `Identifiable` model), else the element's display string.
fn foreach_init(ctx: &mut dyn StdContext, args: Vec<Arg>) -> StdResult {
    let children = keyed_rows(ctx, args, "ForEach")?;
    Ok(container_value("ForEach", children))
}

/// Build the keyed child rows shared by `ForEach(_:id:content:)` and the
/// `List(_:id:rowContent:)` shorthand: materialize the data sequence, run the
/// content `@ViewBuilder` per element, and tag each produced view with a stable
/// identity key. `who` names the caller for error messages.
fn keyed_rows(
    ctx: &mut dyn StdContext,
    args: Vec<Arg>,
    who: &str,
) -> Result<Vec<SwiftValue>, StdError> {
    let mut data: Option<SwiftValue> = None;
    let mut id_keypath: Option<SwiftValue> = None;
    let mut content: Option<SwiftValue> = None;
    for arg in args {
        match arg.label.as_deref() {
            Some("id") => id_keypath = Some(arg.value),
            Some("content") | Some("rowContent") => content = Some(arg.value),
            _ => match arg.value {
                v @ SwiftValue::Closure(_) if content.is_none() => content = Some(v),
                v if data.is_none() => data = Some(v),
                _ => {}
            },
        }
    }
    let (Some(data), Some(SwiftValue::Closure(content))) = (data, content) else {
        return Err(type_error(format!(
            "{who} requires a data sequence and a content closure"
        )));
    };
    let items = sequence_items(&data)
        .ok_or_else(|| type_error(format!("{who} data is not a sequence (array or range)")))?;

    let mut children = Vec::new();
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    for item in items {
        let key = foreach_key(ctx, &item, id_keypath.as_ref())?;
        // The content closure is a `@ViewBuilder`: bind the element and collect
        // *every* produced sibling view, not just the last statement.
        let built = ctx.eval_block_values_with_args(content, vec![item])?;
        let mut rows = Vec::new();
        expand_into(ctx, built, &mut rows, 0, &[])?;
        // A single produced view takes the row key directly; multiple views
        // (a `Group`-like body) get an `_<j>` suffix so keys stay unique. The
        // separator is `_`, which `key_string` always escapes, so a suffixed
        // key can never collide with a single-view row's encoded key.
        let multi = rows.len() > 1;
        for (j, row) in rows.into_iter().enumerate() {
            let mut key = if multi {
                format!("{key}_{j}")
            } else {
                key.clone()
            };
            // Guarantee uniqueness even if the model yields duplicate ids.
            while !seen.insert(key.clone()) {
                key.push('\'');
            }
            children.push(with_key(row, key));
        }
    }
    Ok(children)
}

/// `List { ... }` — a vertically scrolling container. Two forms: a static
/// `@ViewBuilder` content closure, or the `List(_ data, id:, rowContent:)`
/// shorthand that is sugar for a `List` wrapping a keyed `ForEach`.
fn list_init(ctx: &mut dyn StdContext, args: Vec<Arg>) -> StdResult {
    // The data-driven shorthand has a leading non-closure positional argument.
    let data_driven = args
        .iter()
        .any(|a| a.label.is_none() && !matches!(a.value, SwiftValue::Closure(_)));
    let children = if data_driven {
        keyed_rows(ctx, args, "List")?
    } else {
        collect_children(ctx, args)?
    };
    Ok(container_value("List", children))
}

/// `Section { ... }` — a titled group within a `List`. Supports the bare
/// content form and `Section(_ title) { ... }`; the title is recorded as a
/// visible `header` arg the host renders above the rows.
fn section_init(ctx: &mut dyn StdContext, args: Vec<Arg>) -> StdResult {
    let mut header: Option<String> = None;
    let mut content_args: Vec<Arg> = Vec::new();
    for arg in args {
        match (&arg.label, &arg.value) {
            (Some(label), SwiftValue::Str(s)) if label == "header" => header = Some(s.clone()),
            (None, SwiftValue::Str(s)) if header.is_none() => header = Some(s.clone()),
            _ => content_args.push(arg),
        }
    }
    let children = collect_children(ctx, content_args)?;
    let mut fields = Vec::new();
    if let Some(title) = header {
        fields.push(("header".into(), SwiftValue::Str(title)));
    }
    fields.push((CHILDREN_FIELD.into(), SwiftValue::Array(Rc::new(children))));
    Ok(view_value("Section", fields))
}

/// `Picker(_ title, selection: Binding) { options }` — a choice control. Each
/// option view carries a `.tag(value)` modifier; the host renders a `<select>`
/// and emits `set` with the chosen tag. The current selection (read from the
/// binding) is serialized so the host marks the active option. v1 limitation:
/// the selection round-trips as a string, so string-tagged pickers are
/// supported; non-string tags are out of scope.
fn picker_init(ctx: &mut dyn StdContext, args: Vec<Arg>) -> StdResult {
    let mut title = String::new();
    let mut binding: Option<SwiftValue> = None;
    let mut content_args: Vec<Arg> = Vec::new();
    for arg in args {
        match arg.label.as_deref() {
            Some("selection") => binding = Some(arg.value),
            Some("content") => content_args.push(arg),
            _ => match arg.value {
                SwiftValue::Closure(_) => content_args.push(arg),
                SwiftValue::Str(ref s) if title.is_empty() => title = s.clone(),
                _ => {}
            },
        }
    }
    let Some(binding) = binding else {
        return Err(type_error("Picker requires a `selection:` binding"));
    };
    // Flatten `ForEach`-generated rows up into direct option views, so the
    // common `Picker { ForEach(data) { Text(..).tag(..) } }` pattern yields one
    // option per row instead of a single opaque container.
    let children = flatten_picker_options(collect_children(ctx, content_args)?);
    let selection = ctx.get_member(&binding, "wrappedValue")?;
    Ok(view_value(
        "Picker",
        vec![
            ("title".into(), SwiftValue::Str(title)),
            ("selection".into(), selection),
            (CHILDREN_FIELD.into(), SwiftValue::Array(Rc::new(children))),
            (BINDING_FIELD.into(), binding),
        ],
    ))
}

/// Expand any `ForEach` container among a Picker's content into its rows, so
/// each tagged option becomes a direct child of the `Picker`.
fn flatten_picker_options(children: Vec<SwiftValue>) -> Vec<SwiftValue> {
    let mut out = Vec::new();
    for child in children {
        if view_type_name(&child) == Some("ForEach") {
            if let SwiftValue::Struct(obj) = &child {
                if let Some(SwiftValue::Array(rows)) = obj.get(CHILDREN_FIELD) {
                    out.extend(rows.iter().cloned());
                    continue;
                }
            }
        }
        out.push(child);
    }
    out
}

/// `Slider(value: Binding<Double>, in: range, step:)` — a continuous value
/// control. The current value (read from the binding) plus the range bounds and
/// optional step are serialized as args so the host can render an `<input
/// type=range>`; a `set` event writes the new double through the binding.
fn slider_init(ctx: &mut dyn StdContext, args: Vec<Arg>) -> StdResult {
    let mut binding: Option<SwiftValue> = None;
    let mut range: Option<SwiftValue> = None;
    let mut step: Option<f64> = None;
    for arg in args {
        match arg.label.as_deref() {
            Some("value") => binding = Some(arg.value),
            Some("in") => range = Some(arg.value),
            Some("step") => step = number_f64(&arg.value),
            _ => {}
        }
    }
    let (lo, hi) = range_bounds(range.as_ref(), 0.0, 1.0);
    let value = match &binding {
        Some(b) => number_f64(&ctx.get_member(b, "wrappedValue")?).unwrap_or(lo),
        None => lo,
    };
    let mut fields = vec![
        ("value".into(), SwiftValue::Double(value)),
        ("lowerBound".into(), SwiftValue::Double(lo)),
        ("upperBound".into(), SwiftValue::Double(hi)),
    ];
    if let Some(step) = step {
        fields.push(("step".into(), SwiftValue::Double(step)));
    }
    if let Some(b) = binding {
        fields.push((BINDING_FIELD.into(), b));
    }
    Ok(view_value("Slider", fields))
}

/// `Stepper(_ title, value: Binding<Int>, in: range, step:)` — a +/- numeric
/// control. Current value (from the binding), bounds, and step are serialized
/// so the host computes the clamped next value and writes it back via `set`.
fn stepper_init(ctx: &mut dyn StdContext, args: Vec<Arg>) -> StdResult {
    let mut title = String::new();
    let mut binding: Option<SwiftValue> = None;
    let mut range: Option<SwiftValue> = None;
    let mut step: i128 = 1;
    for arg in args {
        match arg.label.as_deref() {
            Some("value") => binding = Some(arg.value),
            Some("in") => range = Some(arg.value),
            Some("step") => {
                if let SwiftValue::Int(i) = &arg.value {
                    step = i.raw;
                }
            }
            _ => {
                if let SwiftValue::Str(s) = &arg.value {
                    if title.is_empty() {
                        title = s.clone();
                    }
                }
            }
        }
    }
    let value = match &binding {
        Some(b) => match ctx.get_member(b, "wrappedValue")? {
            SwiftValue::Int(i) => i.raw,
            other => number_f64(&other).map(|d| d as i128).unwrap_or(0),
        },
        None => 0,
    };
    let mut fields = vec![
        ("title".into(), SwiftValue::Str(title)),
        ("value".into(), SwiftValue::int(value)),
        ("step".into(), SwiftValue::int(step)),
    ];
    // Bounds are optional for a `Stepper`; emit them only when given and
    // non-empty (an exclusive `0..<n` is normalized to a closed upper bound,
    // and a degenerate empty range is dropped rather than emitting lo > hi).
    if let Some(SwiftValue::Range { lo, hi, inclusive }) = &range {
        let upper = if *inclusive { *hi } else { *hi - 1 };
        if upper >= *lo {
            fields.push(("lowerBound".into(), SwiftValue::int(*lo)));
            fields.push(("upperBound".into(), SwiftValue::int(upper)));
        }
    }
    if let Some(b) = binding {
        fields.push((BINDING_FIELD.into(), b));
    }
    Ok(view_value("Stepper", fields))
}

/// Read a Swift numeric value as `f64` (int widened, double as-is).
fn number_f64(value: &SwiftValue) -> Option<f64> {
    match value {
        SwiftValue::Int(i) => Some(i.raw as f64),
        SwiftValue::Double(d) => Some(*d),
        _ => None,
    }
}

/// Resolve `(lower, upper)` bounds from an `in:` range argument, falling back to
/// the given defaults when absent or not a range. v1 limitation: the runtime
/// represents only integer ranges, so a `Slider` range is written as `0...1`
/// (not `0.0...1.0`); the integer endpoints are widened to `f64` here.
fn range_bounds(range: Option<&SwiftValue>, def_lo: f64, def_hi: f64) -> (f64, f64) {
    match range {
        Some(SwiftValue::Range { lo, hi, .. }) => (*lo as f64, *hi as f64),
        _ => (def_lo, def_hi),
    }
}

/// Materialize a ForEach data argument into an ordered element list. Supports
/// arrays and integer ranges (the two common `ForEach` sources).
fn sequence_items(data: &SwiftValue) -> Option<Vec<SwiftValue>> {
    match data {
        SwiftValue::Array(items) => Some(items.iter().cloned().collect()),
        SwiftValue::Range { lo, hi, inclusive } => {
            let end = if *inclusive { *hi + 1 } else { *hi };
            Some((*lo..end).map(SwiftValue::int).collect())
        }
        _ => None,
    }
}

/// Derive a ForEach row's identity key for `item`: apply the `id:` key path if
/// given, else read an `id` member, else fall back to the display string.
fn foreach_key(
    ctx: &mut dyn StdContext,
    item: &SwiftValue,
    id_keypath: Option<&SwiftValue>,
) -> Result<String, StdError> {
    let keyed = match id_keypath {
        Some(SwiftValue::Closure(kp)) => ctx.call_closure(*kp, vec![item.clone()])?,
        _ => match item {
            SwiftValue::Struct(_) | SwiftValue::Object(_) => {
                ctx.get_member(item, "id").unwrap_or_else(|_| item.clone())
            }
            _ => item.clone(),
        },
    };
    Ok(key_string(&keyed))
}

/// Stringify an identity value into a stable, id-safe key: an *injective* escape
/// so distinct identities never collapse to the same key (which would let the
/// keyed diff preserve the wrong row's state). ASCII alphanumerics and `-` pass
/// through; every other byte (including `_` and `.`) becomes `_<hex>`, so the
/// key is a reversible, `.`-free path segment.
fn key_string(value: &SwiftValue) -> String {
    let raw = match value {
        SwiftValue::Str(s) => s.clone(),
        other => other.to_string(),
    };
    let mut out = String::with_capacity(raw.len());
    for b in raw.bytes() {
        if b.is_ascii_alphanumeric() || b == b'-' {
            out.push(b as char);
        } else {
            out.push('_');
            out.push_str(&format!("{b:02x}"));
        }
    }
    out
}

/// Attach a stable identity [`KEY_FIELD`] to a view value (copy-on-write).
fn with_key(view: SwiftValue, key: String) -> SwiftValue {
    let SwiftValue::Struct(obj) = view else {
        return view;
    };
    let mut obj = (*obj).clone();
    obj.fields.retain(|(k, _)| k != KEY_FIELD);
    obj.fields.push((KEY_FIELD.into(), SwiftValue::Str(key)));
    SwiftValue::Struct(Rc::new(obj))
}

/// `Circle()` — a circular shape leaf.
fn circle_init(_ctx: &mut dyn StdContext, _args: Vec<Arg>) -> StdResult {
    Ok(view_value("Circle", Vec::new()))
}

/// `Rectangle()` — a rectangular shape leaf.
fn rectangle_init(_ctx: &mut dyn StdContext, _args: Vec<Arg>) -> StdResult {
    Ok(view_value("Rectangle", Vec::new()))
}

/// `RoundedRectangle(cornerRadius:)` — a rounded-rectangle shape leaf carrying
/// its corner radius for the host. Accepts the labelled `cornerRadius:` form or
/// a single positional radius; an unrelated `style:` argument is ignored.
fn rounded_rectangle_init(_ctx: &mut dyn StdContext, args: Vec<Arg>) -> StdResult {
    let radius = args
        .into_iter()
        .find(|a| a.label.as_deref() == Some("cornerRadius") || a.label.is_none())
        .map(|a| a.value)
        .unwrap_or(SwiftValue::int(0));
    Ok(view_value(
        "RoundedRectangle",
        vec![("cornerRadius".into(), radius)],
    ))
}

/// `Capsule()` — a capsule (stadium) shape leaf.
fn capsule_init(_ctx: &mut dyn StdContext, _args: Vec<Arg>) -> StdResult {
    Ok(view_value("Capsule", Vec::new()))
}

/// `Ellipse()` — an elliptical shape leaf.
fn ellipse_init(_ctx: &mut dyn StdContext, _args: Vec<Arg>) -> StdResult {
    Ok(view_value("Ellipse", Vec::new()))
}

/// `Spacer()` — flexible empty space.
fn spacer_init(_ctx: &mut dyn StdContext, _args: Vec<Arg>) -> StdResult {
    Ok(view_value("Spacer", Vec::new()))
}

/// Internal field on a `Toggle`: the `Binding<Bool>` its `set` event writes to.
pub const BINDING_FIELD: &str = "_binding";

/// `Toggle(_ title: String, isOn: Binding<Bool>)` — a labelled on/off control.
/// The current `isOn` bool is read from the binding for rendering; the binding
/// itself is stashed internally so the dispatch loop can write a new value
/// through it (`set` event) to drive the bound `@State`.
fn toggle_init(ctx: &mut dyn StdContext, args: Vec<Arg>) -> StdResult {
    let mut title = String::new();
    let mut binding: Option<SwiftValue> = None;
    for arg in args {
        match arg.label.as_deref() {
            Some("isOn") => binding = Some(arg.value),
            _ => {
                if let SwiftValue::Str(s) = &arg.value {
                    if title.is_empty() {
                        title = s.clone();
                    }
                }
            }
        }
    }
    let is_on = match &binding {
        Some(b) => matches!(ctx.get_member(b, "wrappedValue")?, SwiftValue::Bool(true)),
        None => false,
    };
    let mut fields = vec![
        ("title".into(), SwiftValue::Str(title)),
        ("isOn".into(), SwiftValue::Bool(is_on)),
    ];
    if let Some(b) = binding {
        fields.push((BINDING_FIELD.into(), b));
    }
    Ok(view_value("Toggle", fields))
}

/// `TextField(_ title, text: Binding<String>)` — a single-line text input. The
/// current string is read from the binding for rendering; the binding is stashed
/// internally so a `set` event writes the new text through it.
fn text_field_init(ctx: &mut dyn StdContext, args: Vec<Arg>) -> StdResult {
    input_field_init(ctx, args, "TextField")
}

/// `SecureField(_ title, text: Binding<String>)` — a masked text input. Same
/// shape as `TextField`; the host renders the value obscured.
fn secure_field_init(ctx: &mut dyn StdContext, args: Vec<Arg>) -> StdResult {
    input_field_init(ctx, args, "SecureField")
}

/// Shared builder for `TextField`/`SecureField`: a `title` placeholder, the
/// current `text` string (read from the binding), and the stashed binding.
fn input_field_init(ctx: &mut dyn StdContext, args: Vec<Arg>, kind: &str) -> StdResult {
    let mut title = String::new();
    let mut binding: Option<SwiftValue> = None;
    for arg in args {
        match arg.label.as_deref() {
            Some("text") => binding = Some(arg.value),
            _ => {
                if let SwiftValue::Str(s) = &arg.value {
                    if title.is_empty() {
                        title = s.clone();
                    }
                }
            }
        }
    }
    let text = match &binding {
        Some(b) => match ctx.get_member(b, "wrappedValue")? {
            SwiftValue::Str(s) => s,
            other => other.to_string(),
        },
        None => String::new(),
    };
    let mut fields = vec![
        ("title".into(), SwiftValue::Str(title)),
        ("text".into(), SwiftValue::Str(text)),
    ];
    if let Some(b) = binding {
        fields.push((BINDING_FIELD.into(), b));
    }
    Ok(view_value(kind, fields))
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
/// block) or an already-built view; non-view statement values are dropped and
/// composed custom `View`s are expanded into their `body`.
fn collect_children(ctx: &mut dyn StdContext, args: Vec<Arg>) -> Result<Vec<SwiftValue>, StdError> {
    let mut out = Vec::new();
    for arg in args {
        match arg.value {
            SwiftValue::Closure(id) => {
                let block = ctx.eval_block_values(id)?;
                expand_into(ctx, block, &mut out, 0, &[])?;
            }
            other => expand_into(ctx, other, &mut out, 0, &[])?,
        }
    }
    Ok(out)
}

/// The `ObservableObject`s a view provides to its subtree via
/// `.environmentObject(_)`, read from its `_env` list.
fn environment_objects(view: &SwiftValue) -> Vec<SwiftValue> {
    match view {
        SwiftValue::Struct(obj) => match obj.get(ENV_FIELD) {
            Some(SwiftValue::Array(objects)) => objects.iter().cloned().collect(),
            _ => Vec::new(),
        },
        _ => Vec::new(),
    }
}

/// Inject the accumulated environment into a custom view before its `body` is
/// evaluated: add the view's own `.environmentObject(_)` provisions, then fill
/// its `@EnvironmentObject` slots. Returns the (possibly updated) view and the
/// environment to pass down its subtree.
fn apply_environment(
    ctx: &mut dyn StdContext,
    view: SwiftValue,
    env: &[SwiftValue],
) -> Result<(SwiftValue, Vec<SwiftValue>), StdError> {
    let mut child_env = env.to_vec();
    child_env.extend(environment_objects(&view));
    let injected = if child_env.is_empty() {
        view
    } else {
        ctx.inject_environment_objects(&view, "EnvironmentObject", &child_env)?
    };
    Ok((injected, child_env))
}

/// Maximum custom-`View` composition depth before bailing. Bounds the `body`
/// recursion so a self- or mutually-recursive view can't hang the renderer.
const MAX_VIEW_DEPTH: usize = 256;

/// Append every view value reachable in `value` to `out`, flattening nested
/// arrays the builder shim produces, expanding composed custom `View`s into
/// their `body`, and dropping scalar/non-view results. `depth` bounds the
/// custom-view `body` recursion (array flattening does not count toward it).
fn expand_into(
    ctx: &mut dyn StdContext,
    value: SwiftValue,
    out: &mut Vec<SwiftValue>,
    depth: usize,
    env: &[SwiftValue],
) -> Result<(), StdError> {
    match value {
        SwiftValue::Array(items) => {
            for item in items.iter() {
                expand_into(ctx, item.clone(), out, depth, env)?;
            }
        }
        v if view_type_name(&v).is_some() => out.push(v),
        // Scalar / non-struct non-views are dropped; a struct-shaped candidate
        // must be a composed custom `View` (neither a builtin view value nor a
        // token), collapsed to its own `body`, recursively. The environment is
        // injected into the view before `body` runs and carried down its
        // subtree (`@EnvironmentObject` support).
        v @ SwiftValue::Struct(_) if is_custom_view(ctx, &v) => {
            if depth >= MAX_VIEW_DEPTH {
                return Err(recursion_error(&v));
            }
            let (v, child_env) = apply_environment(ctx, v, env)?;
            let body = ctx.get_member(&v, "body")?;
            expand_into(ctx, body, out, depth + 1, &child_env)?;
        }
        _ => {}
    }
    Ok(())
}

/// The error raised when custom-`View` composition exceeds [`MAX_VIEW_DEPTH`].
fn recursion_error(view: &SwiftValue) -> StdError {
    type_error(format!(
        "view composition exceeded depth {MAX_VIEW_DEPTH} (recursive `{}`?)",
        view.type_name()
    ))
}

/// Whether `value` is a user-defined `View` to expand: a struct that is not
/// already a builtin view value and not a prelude token (`Color`/`Font`/…).
fn is_custom_view(_ctx: &mut dyn StdContext, value: &SwiftValue) -> bool {
    matches!(value, SwiftValue::Struct(_))
        && view_type_name(value).is_none()
        && token_of(value).is_none()
}

/// Resolve a root `body` value to a single concrete view node, collapsing a
/// chain of composed custom `View`s (`body` returning another custom view)
/// down to the first builtin view value.
pub fn resolve_root(ctx: &mut dyn StdContext, value: SwiftValue) -> Result<SwiftValue, StdError> {
    let mut current = value;
    let mut env: Vec<SwiftValue> = Vec::new();
    let mut depth = 0;
    while is_custom_view(ctx, &current) {
        if depth >= MAX_VIEW_DEPTH {
            return Err(recursion_error(&current));
        }
        // Inject the environment provided so far (plus this view's own
        // `.environmentObject(_)`) before evaluating its `body`.
        let (injected, child_env) = apply_environment(ctx, current, &env)?;
        env = child_env;
        current = ctx.get_member(&injected, "body")?;
        depth += 1;
    }
    Ok(current)
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

/// Find the program's root `View` struct to render: the one no other view
/// *constructs* inside a view body.
///
/// In a composed scene every sub-view is referenced by a `CallExpr` whose callee
/// is an `IdentExpr` (`InfoRow(...)`), so the top-level screen is the View whose
/// name never appears as such a callee. This avoids picking a parameterised
/// child (which can't be instantiated with no arguments). Falls back to the
/// first View struct when the references are cyclic or there is only one.
///
/// The canonical home for this heuristic — the CLI, the wasm host, and the
/// native FFI host all pick the same top-level screen by calling here.
pub fn find_root_view(analysis: &Analysis) -> Option<String> {
    use std::collections::HashSet;
    let mut views: Vec<String> = Vec::new();
    let mut constructed: HashSet<String> = HashSet::new();

    fn callee_name(node: &Node<'_>) -> Option<String> {
        if node.kind() != NodeKind::CallExpr {
            return None;
        }
        let callee = node.children().next()?;
        if callee.kind() == NodeKind::IdentExpr {
            callee.text()
        } else {
            None
        }
    }

    fn walk(
        node: Node<'_>,
        in_view: bool,
        views: &mut Vec<String>,
        constructed: &mut HashSet<String>,
    ) {
        let mut child_in_view = in_view;
        if node.kind() == NodeKind::StructDecl {
            let conforms_view = node
                .children()
                .any(|c| c.kind() == NodeKind::TypeRef && c.text().as_deref() == Some("View"));
            if conforms_view {
                if let Some(name) = node.text() {
                    views.push(name);
                }
                child_in_view = true;
            }
        }
        if in_view {
            if let Some(name) = callee_name(&node) {
                constructed.insert(name);
            }
        }
        for child in node.children() {
            walk(child, child_in_view, views, constructed);
        }
    }

    walk(analysis.root(), false, &mut views, &mut constructed);
    views
        .iter()
        .find(|v| !constructed.contains(*v))
        .or_else(|| views.first())
        .cloned()
}

/// A node's stable identity key ([`KEY_FIELD`]), set on `ForEach`-generated
/// children so the diff and serializer agree on a position-independent id.
pub fn key_of(value: &SwiftValue) -> Option<&str> {
    match value {
        SwiftValue::Struct(obj) => match obj.get(KEY_FIELD) {
            Some(SwiftValue::Str(s)) => Some(s.as_str()),
            _ => None,
        },
        _ => None,
    }
}

/// The UIIR id of the `index`-th child of node `parent_id`. A keyed child
/// (`ForEach` row) uses its stable key so reorders preserve identity; every
/// other child uses its structural position.
pub fn child_id(parent_id: &str, index: usize, child: &SwiftValue) -> String {
    match key_of(child) {
        Some(key) => format!("{parent_id}.{key}"),
        None => format!("{parent_id}.{index}"),
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
                "Capsule.init",
                "Circle.init",
                "Ellipse.init",
                "ForEach.init",
                "HStack.init",
                "List.init",
                "Picker.init",
                "Rectangle.init",
                "RoundedRectangle.init",
                "Section.init",
                "SecureField.init",
                "Slider.init",
                "Spacer.init",
                "Stepper.init",
                "Text.init",
                "TextField.init",
                "Toggle.init",
                "VStack.init",
                "View.background",
                "View.cornerRadius",
                "View.environmentObject",
                "View.fill",
                "View.font",
                "View.fontWeight",
                "View.foregroundColor",
                "View.frame",
                "View.padding",
                "View.tag",
                "ZStack.init",
            ]
        );
    }

    #[test]
    fn qualified_token_resolves_when_leading_dot_is_ambiguous() {
        // `.black` is shared by `Color` and `FontWeight`, so the leading-dot
        // form is ambiguous without contextual typing; the qualified
        // `Color.black` resolves. This documents the accepted v1 limitation.
        let view = render_to_string(
            r#"struct V: View { var body: some View { Text("x").foregroundColor(Color.black) } }"#,
            "V",
        );
        let mods = modifiers_of(&view);
        assert_eq!(mods.len(), 1);
        let SwiftValue::Struct(m) = &mods[0] else {
            panic!("expected modifier struct");
        };
        assert_eq!(m.get("value").and_then(token_of), Some(("Color", "black")));
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
    fn render_root_expands_composed_sub_view() {
        // A custom `View` used inside a container expands into its own `body`,
        // with constructor parameters threaded down (Profile-tab composition).
        let src = r#"
struct Row: View {
    let label: String
    var body: some View { Text(label) }
}
struct V: View {
    var body: some View {
        VStack {
            Row(label: "a")
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
        // The composed Row collapses to its body (a Text carrying its param).
        assert_eq!(view_type_name(&children[0]), Some("Text"));
        let SwiftValue::Struct(text) = &children[0] else {
            panic!("expected text struct");
        };
        assert_eq!(text.get("verbatim"), Some(&SwiftValue::Str("a".into())));
        assert_eq!(view_type_name(&children[1]), Some("Text"));
    }

    #[test]
    fn render_root_expands_custom_view_at_root() {
        // A `body` that returns a custom view (not a builtin) resolves through
        // to that view's own body.
        let src = r#"
struct Inner: View {
    var body: some View { Text("inner") }
}
struct V: View {
    var body: some View { Inner() }
}
"#;
        let view = render_to_string(src, "V");
        assert_eq!(view_type_name(&view), Some("Text"));
        let SwiftValue::Struct(obj) = &view else {
            panic!("expected struct");
        };
        assert_eq!(obj.get("verbatim"), Some(&SwiftValue::Str("inner".into())));
    }

    #[test]
    fn recursive_custom_view_errors_instead_of_hanging() {
        // A view whose `body` returns itself must bail with a depth error,
        // not loop forever.
        let src = r#"
struct Loop: View {
    var body: some View { Loop() }
}
"#;
        let program = format!("{PRELUDE}\n{src}");
        let analysis = tswift_frontend::Analysis::analyze(&program, "test.swift").expect("analyze");
        let analysis: &'static tswift_frontend::Analysis = Box::leak(Box::new(analysis));
        let mut sink = std::io::sink();
        let mut interp = Interpreter::new(&mut sink);
        install(&mut interp);
        interp.run(analysis).expect("run");
        let err = render_root(&mut interp, "Loop").expect_err("recursive view must error");
        assert!(
            format!("{err:?}").contains("composition exceeded depth"),
            "unexpected error: {err:?}"
        );
    }

    #[test]
    fn render_root_builds_zstack_of_shapes() {
        let src = r#"
struct V: View {
    var body: some View {
        ZStack {
            Circle().fill(Color.blue)
            RoundedRectangle(cornerRadius: 12)
        }
    }
}
"#;
        let view = render_to_string(src, "V");
        assert_eq!(view_type_name(&view), Some("ZStack"));
        let SwiftValue::Struct(obj) = &view else {
            panic!("expected struct");
        };
        let Some(SwiftValue::Array(children)) = obj.get(CHILDREN_FIELD) else {
            panic!("expected children");
        };
        assert_eq!(children.len(), 2);
        assert_eq!(view_type_name(&children[0]), Some("Circle"));
        // The circle carries a `fill` modifier with a Color token.
        let fill = modifiers_of(&children[0]);
        assert_eq!(fill.len(), 1);
        let SwiftValue::Struct(m) = &fill[0] else {
            panic!("expected modifier struct");
        };
        assert_eq!(m.get("name"), Some(&SwiftValue::Str("fill".into())));
        assert_eq!(m.get("value").and_then(token_of), Some(("Color", "blue")));
        // The rounded rectangle keeps its corner radius as a visible arg.
        assert_eq!(view_type_name(&children[1]), Some("RoundedRectangle"));
        let SwiftValue::Struct(rr) = &children[1] else {
            panic!("expected struct");
        };
        assert_eq!(rr.get("cornerRadius"), Some(&SwiftValue::int(12)));
    }

    #[test]
    fn foreach_multi_view_row_keeps_every_sibling_with_suffixed_keys() {
        // A `@ViewBuilder` row emitting two views keeps both, keyed `{k}.0`/`{k}.1`.
        let src = r#"
struct V: View {
    var body: some View {
        ForEach(["a", "b"], id: \.self) { x in
            Text(x)
            Text(x)
        }
    }
}
"#;
        let view = render_to_string(src, "V");
        let SwiftValue::Struct(obj) = &view else {
            panic!("expected struct");
        };
        let Some(SwiftValue::Array(children)) = obj.get(CHILDREN_FIELD) else {
            panic!("expected children");
        };
        let keys: Vec<Option<&str>> = children.iter().map(key_of).collect();
        assert_eq!(
            keys,
            vec![Some("a_0"), Some("a_1"), Some("b_0"), Some("b_1")]
        );
    }

    #[test]
    fn picker_serializes_selection_and_tagged_options() {
        let src = r#"
struct V: View {
    @State private var flavor = "choc"
    var body: some View {
        Picker("Flavor", selection: $flavor) {
            Text("Vanilla").tag("van")
            Text("Chocolate").tag("choc")
        }
    }
}
"#;
        let view = render_to_string(src, "V");
        assert_eq!(view_type_name(&view), Some("Picker"));
        let SwiftValue::Struct(obj) = &view else {
            panic!("expected struct");
        };
        assert_eq!(obj.get("title"), Some(&SwiftValue::Str("Flavor".into())));
        assert_eq!(obj.get("selection"), Some(&SwiftValue::Str("choc".into())));
        let Some(SwiftValue::Array(options)) = obj.get(CHILDREN_FIELD) else {
            panic!("expected options");
        };
        assert_eq!(options.len(), 2);
        // Each option carries a `tag` modifier with its selection value.
        let tag1 = modifiers_of(&options[1]);
        let SwiftValue::Struct(m) = &tag1[0] else {
            panic!("expected tag modifier");
        };
        assert_eq!(m.get("name"), Some(&SwiftValue::Str("tag".into())));
        assert_eq!(m.get("value"), Some(&SwiftValue::Str("choc".into())));
        assert!(obj.get(BINDING_FIELD).is_some());
    }

    #[test]
    fn picker_flattens_foreach_options_into_direct_children() {
        let src = r#"
struct V: View {
    @State private var choice = "b"
    let opts = ["a", "b", "c"]
    var body: some View {
        Picker("Pick", selection: $choice) {
            ForEach(opts, id: \.self) { o in Text(o).tag(o) }
        }
    }
}
"#;
        let view = render_to_string(src, "V");
        let SwiftValue::Struct(obj) = &view else {
            panic!("expected struct");
        };
        let Some(SwiftValue::Array(options)) = obj.get(CHILDREN_FIELD) else {
            panic!("expected options");
        };
        // ForEach rows became three direct option views (not one container).
        assert_eq!(options.len(), 3);
        assert!(options.iter().all(|o| view_type_name(o) == Some("Text")));
        assert_eq!(key_of(&options[1]), Some("b"));
    }

    #[test]
    fn environment_object_read_without_injection_traps_cleanly() {
        // Rendering a view whose `@EnvironmentObject` was never injected (no
        // ancestor `.environmentObject`) surfaces a clean error — the wrapper's
        // force-unwrap precondition — rather than panicking the host.
        let src = r#"
class Settings: ObservableObject { @Published var theme = "dark" }
struct V: View {
    @EnvironmentObject var settings: Settings
    var body: some View { Text(settings.theme) }
}
"#;
        let err = render_err(src, "V");
        assert!(
            err.to_lowercase().contains("nil") || err.to_lowercase().contains("unwrap"),
            "expected a force-unwrap trap, got: {err}"
        );
    }

    #[test]
    fn picker_without_selection_is_an_error() {
        let src = r#"
struct V: View {
    var body: some View {
        Picker("Pick") { Text("x").tag("x") }
    }
}
"#;
        let err = render_err(src, "V");
        assert!(
            err.contains("selection"),
            "expected a selection-binding error, got: {err}"
        );
    }

    #[test]
    fn slider_serializes_value_and_bounds_from_binding() {
        let src = r#"
struct V: View {
    @State private var level = 0.25
    var body: some View {
        Slider(value: $level, in: 0...1, step: 0.05)
    }
}
"#;
        let view = render_to_string(src, "V");
        assert_eq!(view_type_name(&view), Some("Slider"));
        let SwiftValue::Struct(obj) = &view else {
            panic!("expected struct");
        };
        assert_eq!(obj.get("value"), Some(&SwiftValue::Double(0.25)));
        assert_eq!(obj.get("lowerBound"), Some(&SwiftValue::Double(0.0)));
        assert_eq!(obj.get("upperBound"), Some(&SwiftValue::Double(1.0)));
        assert_eq!(obj.get("step"), Some(&SwiftValue::Double(0.05)));
        assert!(obj.get(BINDING_FIELD).is_some());
    }

    #[test]
    fn stepper_serializes_value_step_and_bounds() {
        let src = r#"
struct V: View {
    @State private var count = 3
    var body: some View {
        Stepper("Count", value: $count, in: 0...10, step: 2)
    }
}
"#;
        let view = render_to_string(src, "V");
        assert_eq!(view_type_name(&view), Some("Stepper"));
        let SwiftValue::Struct(obj) = &view else {
            panic!("expected struct");
        };
        assert_eq!(obj.get("title"), Some(&SwiftValue::Str("Count".into())));
        assert_eq!(obj.get("value"), Some(&SwiftValue::int(3)));
        assert_eq!(obj.get("step"), Some(&SwiftValue::int(2)));
        assert_eq!(obj.get("lowerBound"), Some(&SwiftValue::int(0)));
        assert_eq!(obj.get("upperBound"), Some(&SwiftValue::int(10)));
    }

    #[test]
    fn textfield_reads_initial_text_from_binding() {
        let src = r#"
struct V: View {
    @State private var name = "Ada"
    var body: some View {
        TextField("Name", text: $name)
    }
}
"#;
        let view = render_to_string(src, "V");
        assert_eq!(view_type_name(&view), Some("TextField"));
        let SwiftValue::Struct(obj) = &view else {
            panic!("expected struct");
        };
        assert_eq!(obj.get("title"), Some(&SwiftValue::Str("Name".into())));
        assert_eq!(obj.get("text"), Some(&SwiftValue::Str("Ada".into())));
        // The binding is stashed internally (not a visible arg).
        assert!(obj.get(BINDING_FIELD).is_some());
    }

    #[test]
    fn list_with_sections_serializes_headers_and_children() {
        let src = r#"
struct V: View {
    var body: some View {
        List {
            Section("A") { Text("one") }
            Section("B") { Text("two") }
        }
    }
}
"#;
        let view = render_to_string(src, "V");
        assert_eq!(view_type_name(&view), Some("List"));
        let SwiftValue::Struct(obj) = &view else {
            panic!("expected struct");
        };
        let Some(SwiftValue::Array(sections)) = obj.get(CHILDREN_FIELD) else {
            panic!("expected children");
        };
        assert_eq!(sections.len(), 2);
        let SwiftValue::Struct(sec0) = &sections[0] else {
            panic!("expected section");
        };
        assert_eq!(sec0.type_name, "Section");
        assert_eq!(sec0.get("header"), Some(&SwiftValue::Str("A".into())));
    }

    #[test]
    fn list_data_shorthand_builds_keyed_rows() {
        let src = r#"
struct V: View {
    var body: some View {
        List(["x", "y"], id: \.self) { item in Text(item) }
    }
}
"#;
        let view = render_to_string(src, "V");
        assert_eq!(view_type_name(&view), Some("List"));
        let SwiftValue::Struct(obj) = &view else {
            panic!("expected struct");
        };
        let Some(SwiftValue::Array(rows)) = obj.get(CHILDREN_FIELD) else {
            panic!("expected children");
        };
        let keys: Vec<Option<&str>> = rows.iter().map(key_of).collect();
        assert_eq!(keys, vec![Some("x"), Some("y")]);
    }

    #[test]
    fn key_string_is_injective_across_separator_chars() {
        // Distinct ids that a lossy sanitizer would collapse must stay distinct.
        assert_ne!(
            key_string(&SwiftValue::Str("a.b".into())),
            key_string(&SwiftValue::Str("a_b".into()))
        );
        // And the encoding never contains the path separator.
        assert!(!key_string(&SwiftValue::Str("a.b.c".into())).contains('.'));
    }

    #[test]
    fn render_root_builds_remaining_shape_leaves() {
        // Rectangle/Capsule/Ellipse are parameterless leaves; positional
        // RoundedRectangle(8) is accepted too.
        let src = r#"
struct V: View {
    var body: some View {
        HStack {
            Rectangle()
            Capsule()
            Ellipse()
            RoundedRectangle(cornerRadius: 8)
        }
    }
}
"#;
        let view = render_to_string(src, "V");
        let SwiftValue::Struct(obj) = &view else {
            panic!("expected struct");
        };
        let Some(SwiftValue::Array(children)) = obj.get(CHILDREN_FIELD) else {
            panic!("expected children");
        };
        let kinds: Vec<Option<&str>> = children.iter().map(view_type_name).collect();
        assert_eq!(
            kinds,
            vec![
                Some("Rectangle"),
                Some("Capsule"),
                Some("Ellipse"),
                Some("RoundedRectangle"),
            ]
        );
        let SwiftValue::Struct(rr) = &children[3] else {
            panic!("expected struct");
        };
        assert_eq!(rr.get("cornerRadius"), Some(&SwiftValue::int(8)));
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

    /// Render `root_type` expecting a failure, returning the error message.
    fn render_err(src: &str, root_type: &str) -> String {
        let program = format!("{PRELUDE}\n{src}");
        let analysis = tswift_frontend::Analysis::analyze(&program, "test.swift").expect("analyze");
        let analysis: &'static tswift_frontend::Analysis = Box::leak(Box::new(analysis));
        let mut sink = std::io::sink();
        let mut interp = Interpreter::new(&mut sink);
        install(&mut interp);
        interp.run(analysis).expect("run");
        match render_root(&mut interp, root_type) {
            Ok(v) => panic!("expected a render error, got {v:?}"),
            Err(e) => format!("{e:?}"),
        }
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
