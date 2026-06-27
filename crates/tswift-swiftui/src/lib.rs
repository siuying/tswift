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

use tswift_core::{
    Arg, EvalError, Interpreter, StdContext, StdError, StdResult, StructObj, SwiftValue,
};

/// Field name holding a view's ordered modifier list.
pub const MODIFIERS_FIELD: &str = "_modifiers";
/// Field name holding a container view's ordered child views.
pub const CHILDREN_FIELD: &str = "_children";
/// Type name of an appended modifier record (`_Modifier { name, <args> }`).
pub const MODIFIER_TYPE: &str = "_Modifier";

/// View modifiers registered as generic struct methods. Each appends a
/// `_Modifier` record to the receiver view's `_modifiers` field (copy-on-write)
/// and returns the new view. The list also drives the `View.<name>` coverage
/// keys in [`registered_keys`].
const MODIFIERS: &[&str] = &["frame"];

/// Register every currently-supported SwiftUI view constructor and modifier
/// into `interp`.
pub fn install(interp: &mut Interpreter<'_>) {
    interp.register_free_fn("Text", text_init);
    interp.register_free_fn("VStack", vstack_init);
    interp.register_free_fn("HStack", hstack_init);
    interp.register_free_fn("Spacer", spacer_init);
    interp.register_free_fn("Button", button_init);

    interp.register_struct_method("frame", modifier_frame);
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
    keys.extend(MODIFIERS.iter().map(|m| format!("View.{m}")));
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

/// `Button(_ title) { action }` (scaffold: title only; the action closure and
/// `@ViewBuilder` label come with the interaction slice).
fn button_init(_ctx: &mut dyn StdContext, args: Vec<Arg>) -> StdResult {
    let mut iter = args.into_iter();
    let title = match iter.next() {
        Some(arg) => match arg.value {
            SwiftValue::Str(s) => s,
            other => other.to_string(),
        },
        None => String::new(),
    };
    Ok(view_value(
        "Button",
        vec![("title".into(), SwiftValue::Str(title))],
    ))
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

/// `.frame(width:height:)` — fixed-size frame around the view.
fn modifier_frame(_ctx: &mut dyn StdContext, recv: SwiftValue, args: Vec<Arg>) -> StdResult {
    append_modifier(recv, make_modifier("frame", args))
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
                "View.frame",
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

    /// Render `root_type`'s `body` from `src` for assertions.
    fn render_to_string(src: &str, root_type: &str) -> SwiftValue {
        let analysis = tswift_frontend::Analysis::analyze(src, "test.swift").expect("analyze");
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
