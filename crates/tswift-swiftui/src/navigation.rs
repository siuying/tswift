//! Navigation — `NavigationStack`/`NavigationLink` constructors, the
//! `.navigationDestination(for:)` registration modifier, and the path-binding
//! helpers the session uses to realize pushed screens (ADR-0013 §1).

use std::rc::Rc;

use tswift_core::{Arg, Interpreter, StdContext, StdError, StdResult, StructObj, SwiftValue};

use crate::values::set_or_push_field;
use crate::{
    collect_children, container_value, expand_into, type_error, view_value, BINDING_FIELD,
    CHILDREN_FIELD, NAV_DESTINATIONS_FIELD, NAV_DESTINATIONS_TYPE, NAV_DESTINATION_FIELD,
    NAV_VALUE_FIELD, PUSHED_VALUE_TYPE,
};

/// `NavigationStack { root }` — a runtime-owned navigation container (ADR-0013
/// §1). The trailing `@ViewBuilder` is the root screen; every screen in the
/// stack renders as an ordinary child (root first, topmost last). Pushed screens
/// are appended by the session from per-stack state, so the base node here holds
/// just the root content.
///
/// With a `path:` binding (`NavigationStack(path: $path)`, ADR-0013 §1
/// value-based navigation) the binding is captured in [`BINDING_FIELD`] and
/// becomes the session's source of truth: the stack's depth (and each pushed
/// screen) is derived from the path's items (a `NavigationPath` or a typed
/// array), matched to `.navigationDestination(for:)` registrations. Pushes/pops
/// mutate the bound path; external path mutation re-renders to the new depth.
pub(crate) fn navigation_stack_init(ctx: &mut dyn StdContext, args: Vec<Arg>) -> StdResult {
    let mut path_binding: Option<SwiftValue> = None;
    let mut content_args: Vec<Arg> = Vec::new();
    for arg in args {
        match arg.label.as_deref() {
            Some("path") => path_binding = Some(arg.value),
            _ => content_args.push(arg),
        }
    }
    let view = container_value("NavigationStack", collect_children(ctx, content_args)?);
    match path_binding {
        Some(binding @ SwiftValue::Struct(_)) => {
            let SwiftValue::Struct(obj) = &view else {
                return Ok(view);
            };
            let mut fields = obj.fields.clone();
            fields.push((BINDING_FIELD.into(), binding));
            Ok(SwiftValue::Struct(Rc::new(StructObj {
                type_name: obj.type_name.clone(),
                fields,
            })))
        }
        _ => Ok(view),
    }
}

/// `NavigationLink("title") { destination }` / `NavigationLink(destination:
/// label:)` — a tappable link serialized *without* its destination subtree
/// (ADR-0013 §1). The destination is captured in [`NAV_DESTINATION_FIELD`] (a
/// `@ViewBuilder` closure — re-evaluated fresh on every render so the pushed
/// screen stays live against `@State` — or an eagerly-built view for the
/// `destination: SomeView()` form) and never serialized. The link's own label is
/// its `title` arg (title form) or its `label:`/trailing `@ViewBuilder`
/// children. A tap routes to the session, which pushes the destination onto the
/// enclosing stack.
///
/// A value-based link — `NavigationLink("title", value: v)` /
/// `NavigationLink(value: v) { label }` (ADR-0013 §1) — captures its `value:`
/// payload in [`NAV_VALUE_FIELD`] instead of a destination. A tap resolves the
/// nearest enclosing `.navigationDestination(for:)` whose type matches the
/// value, evaluates that closure with the value, and pushes the result.
pub(crate) fn navigation_link_init(ctx: &mut dyn StdContext, args: Vec<Arg>) -> StdResult {
    let mut title: Option<String> = None;
    let mut destination: Option<SwiftValue> = None;
    let mut value: Option<SwiftValue> = None;
    let mut label_closure: Option<SwiftValue> = None;
    let mut trailing: Option<SwiftValue> = None;
    for arg in args {
        match arg.label.as_deref() {
            Some("destination") => destination = Some(arg.value),
            Some("value") => value = Some(arg.value),
            Some("label") => label_closure = Some(arg.value),
            None => match arg.value {
                SwiftValue::Str(s) if title.is_none() => title = Some(s),
                v @ SwiftValue::Closure(_) => trailing = Some(v),
                _ => {}
            },
            _ => {}
        }
    }
    // Disambiguate the trailing `@ViewBuilder`. With an explicit `destination:`
    // or a `value:`, the trailing closure is the link's *label*; otherwise it is
    // the destination itself (the `NavigationLink("title") { destination }`
    // form). A `value:` link carries no destination subtree.
    let destination = if value.is_some() {
        if label_closure.is_none() {
            label_closure = trailing;
        }
        None
    } else {
        match (destination, trailing) {
            (Some(dest), trailing) => {
                if label_closure.is_none() {
                    label_closure = trailing;
                }
                Some(dest)
            }
            (None, trailing) => trailing,
        }
    };
    let children = match label_closure {
        Some(closure) => collect_children(
            ctx,
            vec![Arg {
                label: None,
                value: closure,
            }],
        )?,
        None => Vec::new(),
    };
    let mut fields: Vec<(String, SwiftValue)> = Vec::new();
    if let Some(title) = title {
        fields.push(("title".into(), SwiftValue::Str(title)));
    }
    fields.push((CHILDREN_FIELD.into(), SwiftValue::Array(Rc::new(children))));
    if let Some(destination) = destination {
        fields.push((NAV_DESTINATION_FIELD.into(), destination));
    }
    if let Some(value) = value {
        fields.push((NAV_VALUE_FIELD.into(), value));
    }
    Ok(view_value("NavigationLink", fields))
}

/// `.navigationDestination(for: T.self) { value in destination }` — register a
/// value→destination mapping for the enclosing `NavigationStack` (ADR-0013 §1).
/// The `for:` metatype's spelled type name keys the `@ViewBuilder` `(T) ->
/// Content` closure in the view's [`NAV_DESTINATIONS_FIELD`] map. The runtime
/// resolves a pushed value against these registrations (nearest enclosing first,
/// then the stack's screens) and evaluates the matching closure with the value.
/// Never serialized — the runtime owns navigation, so hosts see only the
/// realized screen appended as an ordinary child.
pub(crate) fn modifier_navigation_destination(
    _ctx: &mut dyn StdContext,
    recv: SwiftValue,
    args: Vec<Arg>,
) -> StdResult {
    let mut type_name: Option<String> = None;
    let mut closure: Option<SwiftValue> = None;
    for arg in args {
        match arg.label.as_deref() {
            Some("for") => {
                if let SwiftValue::Metatype(name) = arg.value {
                    type_name = Some(name);
                }
            }
            Some("destination") => closure = Some(arg.value),
            None => match arg.value {
                SwiftValue::Metatype(name) if type_name.is_none() => type_name = Some(name),
                v @ SwiftValue::Closure(_) => closure = Some(v),
                _ => {}
            },
            _ => {}
        }
    }
    let (Some(type_name), Some(closure @ SwiftValue::Closure(_))) = (type_name, closure) else {
        // A malformed registration (missing type or closure) is dropped rather
        // than trapping — the link simply won't resolve.
        return Ok(recv);
    };
    let SwiftValue::Struct(obj) = &recv else {
        return Err(type_error(format!(
            "navigationDestination applied to non-view value `{}`",
            recv.type_name()
        )));
    };
    let mut fields = obj.fields.clone();
    if !fields.iter().any(|(k, _)| k == NAV_DESTINATIONS_FIELD) {
        fields.push((
            NAV_DESTINATIONS_FIELD.into(),
            SwiftValue::Struct(Rc::new(StructObj {
                type_name: NAV_DESTINATIONS_TYPE.into(),
                fields: Vec::new(),
            })),
        ));
    }
    let slot = fields
        .iter_mut()
        .find(|(k, _)| k == NAV_DESTINATIONS_FIELD)
        .map(|(_, v)| v)
        .expect("_navDestinations slot ensured above");
    let mut map = match slot {
        SwiftValue::Struct(m) => (**m).clone(),
        _ => StructObj {
            type_name: NAV_DESTINATIONS_TYPE.into(),
            fields: Vec::new(),
        },
    };
    map.fields.retain(|(k, _)| k != &type_name);
    map.fields.push((type_name, closure));
    *slot = SwiftValue::Struct(Rc::new(map));
    Ok(SwiftValue::Struct(Rc::new(StructObj {
        type_name: obj.type_name.clone(),
        fields,
    })))
}

/// Realize a `NavigationLink` destination captured in [`NAV_DESTINATION_FIELD`]
/// into a single screen view value, expanding a custom `View` into its `body`
/// (ADR-0013 §1). A `@ViewBuilder` closure destination is evaluated fresh (so
/// the pushed screen re-reads `@State` on every render); an eagerly-built view
/// destination is expanded as-is. Multiple produced views compose as a `Group`.
///
/// A [`PUSHED_VALUE_TYPE`] record (a value-based push, ADR-0013 §1) invokes its
/// captured `.navigationDestination(for:)` closure with the stored value — also
/// re-evaluated fresh each render so the screen stays live against `@State`.
pub fn realize_pushed_screen(
    ctx: &mut dyn StdContext,
    destination: &SwiftValue,
) -> Result<Option<SwiftValue>, StdError> {
    let mut out = Vec::new();
    match destination {
        // A value-based push: invoke the destination closure with the value.
        SwiftValue::Struct(rec) if rec.type_name == PUSHED_VALUE_TYPE => {
            match (rec.get("destination"), rec.get("value")) {
                (Some(SwiftValue::Closure(id)), Some(value)) => {
                    let produced = ctx.eval_block_values_with_args(*id, vec![value.clone()])?;
                    expand_into(ctx, produced, &mut out, 0, &[])?;
                }
                _ => return Ok(None),
            }
        }
        SwiftValue::Closure(id) => {
            let block = ctx.eval_block_values(*id)?;
            expand_into(ctx, block, &mut out, 0, &[])?;
        }
        other => expand_into(ctx, other.clone(), &mut out, 0, &[])?,
    }
    Ok(match out.len() {
        0 => None,
        1 => Some(out.into_iter().next().expect("len checked")),
        _ => Some(container_value("Group", out)),
    })
}

/// Build a session-mode value-based push record ([`PUSHED_VALUE_TYPE`]): the
/// resolved `.navigationDestination(for:)` closure paired with the pushed value,
/// realized fresh each render by [`realize_pushed_screen`].
pub fn pushed_value(destination: SwiftValue, value: SwiftValue) -> SwiftValue {
    SwiftValue::Struct(Rc::new(StructObj {
        type_name: PUSHED_VALUE_TYPE.into(),
        fields: vec![("destination".into(), destination), ("value".into(), value)],
    }))
}

/// The `NavigationPath` internal item storage field (`_items`).
pub const NAV_PATH_ITEMS_FIELD: &str = "_items";

/// Read a `NavigationStack(path:)` binding's items as a plain value list
/// (ADR-0013 §1): a `NavigationPath`'s `_items`, or a typed array binding's
/// elements directly. Returns `None` when the binding is absent or holds neither
/// shape.
pub fn read_path_items(
    ctx: &mut Interpreter,
    binding: &SwiftValue,
) -> Result<Option<Vec<SwiftValue>>, StdError> {
    let wrapped = ctx.get_member(binding, "wrappedValue")?;
    Ok(match wrapped {
        SwiftValue::Struct(obj) if obj.type_name == "NavigationPath" => {
            match obj.get(NAV_PATH_ITEMS_FIELD) {
                Some(SwiftValue::Array(items)) => Some(items.iter().cloned().collect()),
                _ => Some(Vec::new()),
            }
        }
        SwiftValue::Array(items) => Some(items.iter().cloned().collect()),
        _ => None,
    })
}

/// Append `value` to a `NavigationStack(path:)` binding, writing back through it
/// (ADR-0013 §1). Handles both a `NavigationPath` (append to `_items`) and a
/// typed array binding. Returns `true` when the push landed.
pub fn path_append(
    ctx: &mut Interpreter,
    binding: &SwiftValue,
    value: SwiftValue,
) -> Result<bool, StdError> {
    let wrapped = ctx.get_member(binding, "wrappedValue")?;
    let updated = match wrapped {
        SwiftValue::Struct(obj) if obj.type_name == "NavigationPath" => {
            let mut obj = (*obj).clone();
            let mut items = match obj.get(NAV_PATH_ITEMS_FIELD) {
                Some(SwiftValue::Array(items)) => items.iter().cloned().collect::<Vec<_>>(),
                _ => Vec::new(),
            };
            items.push(value);
            set_or_push_field(
                &mut obj,
                NAV_PATH_ITEMS_FIELD,
                SwiftValue::Array(Rc::new(items)),
            );
            SwiftValue::Struct(Rc::new(obj))
        }
        SwiftValue::Array(items) => {
            let mut items = items.iter().cloned().collect::<Vec<_>>();
            items.push(value);
            SwiftValue::Array(Rc::new(items))
        }
        _ => return Ok(false),
    };
    ctx.set_member(binding, "wrappedValue", updated)?;
    Ok(true)
}

/// Drop the last item of a `NavigationStack(path:)` binding, writing back
/// through it (ADR-0013 §1). A no-op on an empty path. Returns `true` when the
/// binding was a recognised path shape (even if already empty).
pub fn path_remove_last(ctx: &mut Interpreter, binding: &SwiftValue) -> Result<bool, StdError> {
    let wrapped = ctx.get_member(binding, "wrappedValue")?;
    let updated = match wrapped {
        SwiftValue::Struct(obj) if obj.type_name == "NavigationPath" => {
            let mut obj = (*obj).clone();
            let mut items = match obj.get(NAV_PATH_ITEMS_FIELD) {
                Some(SwiftValue::Array(items)) => items.iter().cloned().collect::<Vec<_>>(),
                _ => Vec::new(),
            };
            items.pop();
            set_or_push_field(
                &mut obj,
                NAV_PATH_ITEMS_FIELD,
                SwiftValue::Array(Rc::new(items)),
            );
            SwiftValue::Struct(Rc::new(obj))
        }
        SwiftValue::Array(items) => {
            let mut items = items.iter().cloned().collect::<Vec<_>>();
            items.pop();
            SwiftValue::Array(Rc::new(items))
        }
        _ => return Ok(false),
    };
    ctx.set_member(binding, "wrappedValue", updated)?;
    Ok(true)
}
