//! View constructors — every builtin SwiftUI view init registered by
//! [`crate::install`]: text, stacks, keyed `ForEach` rows, lists, pickers,
//! `TabView`, shapes, images, grids, controls, and the shared `@ViewBuilder`
//! child-collection helpers.

use std::rc::Rc;

use tswift_core::{Arg, StdContext, StdError, StdResult, SwiftValue};

use crate::values::{key_string, number_f64, range_bounds, sequence_items, with_key};
use crate::{
    container_value, expand_into, handlers_map, token_of, type_error, view_type_name, view_value,
    BINDING_FIELD, CHILDREN_FIELD, HANDLERS_FIELD, MODIFIERS_FIELD,
};

/// `Text(_ verbatim: String)` — the leaf text view.
pub(crate) fn text_init(_ctx: &mut dyn StdContext, args: Vec<Arg>) -> StdResult {
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
pub(crate) fn vstack_init(ctx: &mut dyn StdContext, args: Vec<Arg>) -> StdResult {
    stack_init("VStack", ctx, args)
}

/// `HStack { ... }` — horizontal container.
pub(crate) fn hstack_init(ctx: &mut dyn StdContext, args: Vec<Arg>) -> StdResult {
    stack_init("HStack", ctx, args)
}

/// `ZStack { ... }` — depth (overlay) container; children stack back-to-front.
pub(crate) fn zstack_init(ctx: &mut dyn StdContext, args: Vec<Arg>) -> StdResult {
    stack_init("ZStack", ctx, args)
}

/// Shared `VStack`/`HStack`/`ZStack` builder: capture `spacing:` (a CGFloat gap
/// between children) and `alignment:` (a `HorizontalAlignment`/`VerticalAlignment`/
/// `Alignment` token, resolved via the typed stack signatures from issue #203)
/// as constructor fields, then collect the children from the trailing
/// `@ViewBuilder` closure. The host applies `alignment` on the stack's cross
/// axis (issue #189).
fn stack_init(type_name: &str, ctx: &mut dyn StdContext, args: Vec<Arg>) -> StdResult {
    let mut spacing: Option<SwiftValue> = None;
    let mut alignment: Option<SwiftValue> = None;
    let mut rest: Vec<Arg> = Vec::new();
    for arg in args {
        match arg.label.as_deref() {
            Some("spacing") => spacing = Some(arg.value),
            Some("alignment") => alignment = Some(arg.value),
            _ => rest.push(arg),
        }
    }
    let children = collect_children(ctx, rest)?;
    let mut fields: Vec<(String, SwiftValue)> = Vec::new();
    if let Some(spacing) = spacing {
        fields.push(("spacing".into(), spacing));
    }
    if let Some(alignment) = alignment {
        fields.push(("alignment".into(), alignment));
    }
    fields.push((CHILDREN_FIELD.into(), SwiftValue::Array(Rc::new(children))));
    Ok(view_value(type_name, fields))
}

/// `ForEach(_ data, id:, content:)` — a keyed sequence of views. Each element
/// of `data` is passed to the `content` builder; the produced view(s) are
/// tagged with a stable identity key so the diff can `move` reordered rows
/// rather than rebuild them. The key comes from the `id:` key-path argument
/// (e.g. `\.self` or `\.name`), else the element's `id` member (an
/// `Identifiable` model), else the element's display string.
pub(crate) fn foreach_init(ctx: &mut dyn StdContext, args: Vec<Arg>) -> StdResult {
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
pub(crate) fn list_init(ctx: &mut dyn StdContext, args: Vec<Arg>) -> StdResult {
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
pub(crate) fn section_init(ctx: &mut dyn StdContext, args: Vec<Arg>) -> StdResult {
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
pub(crate) fn picker_init(ctx: &mut dyn StdContext, args: Vec<Arg>) -> StdResult {
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

/// Expand any transparent container (`ForEach`, `Group`) among a Picker's
/// content into its rows, recursively, so each tagged option becomes a direct
/// child of the `Picker` (the host option lowerers expect flat option views).
fn flatten_picker_options(children: Vec<SwiftValue>) -> Vec<SwiftValue> {
    let mut out = Vec::new();
    for child in children {
        if matches!(view_type_name(&child), Some("ForEach") | Some("Group")) {
            if let SwiftValue::Struct(obj) = &child {
                if let Some(SwiftValue::Array(rows)) = obj.get(CHILDREN_FIELD) {
                    out.extend(flatten_picker_options(rows.iter().cloned().collect()));
                    continue;
                }
            }
        }
        out.push(child);
    }
    out
}

/// `TabView { ... }` / `TabView(selection: $binding) { ... }` — a tabbed
/// container (ADR-0013 §2). Every tab renders eagerly as a child; each child
/// carries a `.tabItem { … }` bar label and an optional `.tag(_)`. The runtime
/// owns the selection: with a `selection:` binding it reads the bound value
/// (and the `select` dispatch writes it back through the binding, reusing the
/// `set` binding route); without one the session keeps per-node selection
/// state. The current selection is serialized as a `selection` arg so a change
/// flows through `setArgs`; the host shows only the selected child and builds
/// the tab bar from the children's `tabItem` markers. Selection value: a
/// child's `.tag(_)` when present, else its index.
///
/// This models the classic `.tabItem` API; the iOS 18 `Tab { }` struct API is
/// out of scope.
pub(crate) fn tabview_init(ctx: &mut dyn StdContext, args: Vec<Arg>) -> StdResult {
    let mut binding: Option<SwiftValue> = None;
    let mut content_args: Vec<Arg> = Vec::new();
    for arg in args {
        match arg.label.as_deref() {
            Some("selection") => binding = Some(arg.value),
            _ => content_args.push(arg),
        }
    }
    let children = collect_children(ctx, content_args)?;
    // The active selection: the bound value when a `selection:` binding is
    // given, else the first tab's identity (its `.tag(_)` or index 0). Without
    // a binding the session overrides this from its per-node state after each
    // render (mirrors NavigationStack per-stack state).
    let selection = match &binding {
        Some(b) => ctx.get_member(b, "wrappedValue")?,
        None => default_tab_selection(&children),
    };
    let mut fields = vec![
        ("selection".into(), selection),
        (CHILDREN_FIELD.into(), SwiftValue::Array(Rc::new(children))),
    ];
    if let Some(b) = binding {
        fields.push((BINDING_FIELD.into(), b));
    }
    Ok(view_value("TabView", fields))
}

/// The default selection identity for a `TabView` without a `selection:`
/// binding: the first tab's `.tag(_)` value if present, else index `0`.
fn default_tab_selection(children: &[SwiftValue]) -> SwiftValue {
    match children.first().and_then(child_tag) {
        Some(tag) => tag,
        None => SwiftValue::int(0),
    }
}

/// A tab child's `.tag(_)` modifier value, if it carries one (the identity a
/// `TabView` selection matches against).
fn child_tag(child: &SwiftValue) -> Option<SwiftValue> {
    let SwiftValue::Struct(obj) = child else {
        return None;
    };
    let Some(SwiftValue::Array(mods)) = obj.get(MODIFIERS_FIELD) else {
        return None;
    };
    mods.iter().rev().find_map(|m| {
        let SwiftValue::Struct(mo) = m else {
            return None;
        };
        let is_tag = matches!(mo.get("name"), Some(SwiftValue::Str(n)) if n == "tag");
        is_tag.then(|| mo.get("value").cloned()).flatten()
    })
}

/// `Slider(value: Binding<Double>, in: range, step:)` — a continuous value
/// control. The current value (read from the binding) plus the range bounds and
/// optional step are serialized as args so the host can render an `<input
/// type=range>`; a `set` event writes the new double through the binding.
pub(crate) fn slider_init(ctx: &mut dyn StdContext, args: Vec<Arg>) -> StdResult {
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
pub(crate) fn stepper_init(ctx: &mut dyn StdContext, args: Vec<Arg>) -> StdResult {
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

/// `Circle()` — a circular shape leaf.
pub(crate) fn circle_init(_ctx: &mut dyn StdContext, _args: Vec<Arg>) -> StdResult {
    Ok(view_value("Circle", Vec::new()))
}

/// `Rectangle()` — a rectangular shape leaf.
pub(crate) fn rectangle_init(_ctx: &mut dyn StdContext, _args: Vec<Arg>) -> StdResult {
    Ok(view_value("Rectangle", Vec::new()))
}

/// `RoundedRectangle(cornerRadius:)` — a rounded-rectangle shape leaf carrying
/// its corner radius for the host. Accepts the labelled `cornerRadius:` form or
/// a single positional radius; an unrelated `style:` argument is ignored.
pub(crate) fn rounded_rectangle_init(_ctx: &mut dyn StdContext, args: Vec<Arg>) -> StdResult {
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
pub(crate) fn capsule_init(_ctx: &mut dyn StdContext, _args: Vec<Arg>) -> StdResult {
    Ok(view_value("Capsule", Vec::new()))
}

/// `Ellipse()` — an elliptical shape leaf.
pub(crate) fn ellipse_init(_ctx: &mut dyn StdContext, _args: Vec<Arg>) -> StdResult {
    Ok(view_value("Ellipse", Vec::new()))
}

/// `Label(_ title, systemImage:)` — a title paired with an SF Symbol icon.
pub(crate) fn label_init(_ctx: &mut dyn StdContext, args: Vec<Arg>) -> StdResult {
    let mut title = String::new();
    let mut system_image = String::new();
    for arg in args {
        match arg.label.as_deref() {
            Some("systemImage") => system_image = arg.value.to_string(),
            None if matches!(arg.value, SwiftValue::Str(_)) => title = arg.value.to_string(),
            _ => {}
        }
    }
    Ok(view_value(
        "Label",
        vec![
            ("title".into(), SwiftValue::Str(title)),
            ("systemImage".into(), SwiftValue::Str(system_image)),
        ],
    ))
}

/// `Image(systemName:)` (an SF Symbol) or `Image(_ name)` (a bundle asset).
pub(crate) fn image_init(_ctx: &mut dyn StdContext, args: Vec<Arg>) -> StdResult {
    let mut system_name: Option<String> = None;
    let mut name: Option<String> = None;
    for arg in args {
        match arg.label.as_deref() {
            Some("systemName") => system_name = Some(arg.value.to_string()),
            None if matches!(arg.value, SwiftValue::Str(_)) => name = Some(arg.value.to_string()),
            _ => {}
        }
    }
    let fields = match system_name {
        Some(system_name) => vec![("systemName".into(), SwiftValue::Str(system_name))],
        None => vec![("name".into(), SwiftValue::Str(name.unwrap_or_default()))],
    };
    Ok(view_value("Image", fields))
}

/// `ProgressView()` (indeterminate) or `ProgressView(value:total:)` (determinate),
/// optionally with a leading title label (`ProgressView("Loading", value:)`) that
/// becomes a `label` arg — the host wraps the bar with a label row (issue #206).
pub(crate) fn progress_view_init(_ctx: &mut dyn StdContext, args: Vec<Arg>) -> StdResult {
    let mut value: Option<SwiftValue> = None;
    let mut total: Option<SwiftValue> = None;
    let mut label: Option<SwiftValue> = None;
    for arg in args {
        match arg.label.as_deref() {
            Some("value") => value = Some(arg.value),
            Some("total") => total = Some(arg.value),
            // The leading unlabeled `ProgressView("Loading", …)` title string
            // becomes the `label` arg the host renders alongside the bar (#206).
            // A trailing `@ViewBuilder` label closure is not modelled here.
            None if matches!(arg.value, SwiftValue::Str(_)) && label.is_none() => {
                label = Some(arg.value)
            }
            _ => {}
        }
    }
    let mut fields: Vec<(String, SwiftValue)> = Vec::new();
    if let Some(label) = label {
        fields.push(("label".into(), label));
    }
    if let Some(value) = value {
        fields.push(("value".into(), value));
    }
    if let Some(total) = total {
        fields.push(("total".into(), total));
    }
    Ok(view_value("ProgressView", fields))
}

/// `Group { ... }` — a transparent container: it groups views for shared
/// modifiers without adding layout, laying its children out as if inline.
pub(crate) fn group_init(ctx: &mut dyn StdContext, args: Vec<Arg>) -> StdResult {
    Ok(container_value("Group", collect_children(ctx, args)?))
}

/// `LazyVStack(spacing:) { ... }` — a vertical stack that renders lazily; for the
/// UIIR it lays out exactly like `VStack` (the host owns lazy materialization).
pub(crate) fn lazy_vstack_init(ctx: &mut dyn StdContext, args: Vec<Arg>) -> StdResult {
    stack_init("LazyVStack", ctx, args)
}

/// `LazyHStack(spacing:) { ... }` — the horizontal lazy stack.
pub(crate) fn lazy_hstack_init(ctx: &mut dyn StdContext, args: Vec<Arg>) -> StdResult {
    stack_init("LazyHStack", ctx, args)
}

/// Collect only the `@ViewBuilder` closure children of a container, dropping any
/// labeled scalar args (e.g. `Grid(horizontalSpacing:)`) which are deferred.
fn collect_closure_children(
    who: &str,
    ctx: &mut dyn StdContext,
    args: Vec<Arg>,
) -> Result<Vec<SwiftValue>, StdError> {
    let mut out = Vec::new();
    for arg in args {
        match arg.value {
            SwiftValue::Closure(id) => {
                let block = ctx.eval_block_values(id)?;
                expand_into(ctx, block, &mut out, 0, &[])?;
            }
            // Any non-closure arg (e.g. `Grid(horizontalSpacing:)`/`alignment:`)
            // is a deferred layout option; error explicitly rather than silently
            // dropping it (mirrors the stack `alignment:` deferral, issue #193).
            _ => {
                let what = arg.label.as_deref().unwrap_or("an argument");
                return Err(type_error(format!(
                    "{who}({what}:) is not yet supported (deferred, issue #193); omit it"
                )));
            }
        }
    }
    Ok(out)
}

/// `Grid { GridRow { ... } ... }` — a 2-D grid (SwiftUI's iOS 16 `Grid`, distinct
/// from the `GridItem`-driven `LazyVGrid`). Spacing/alignment args are deferred.
pub(crate) fn grid_init(ctx: &mut dyn StdContext, args: Vec<Arg>) -> StdResult {
    Ok(container_value(
        "Grid",
        collect_closure_children("Grid", ctx, args)?,
    ))
}

/// `LazyVGrid(columns: [GridItem], alignment:, spacing:) { ... }` — a lazy grid
/// whose `columns` array sizes the cross-axis tracks. The host turns the
/// `GridItem` array into a CSS-grid template (web) or a native `LazyVGrid`
/// (iOS). `LazyHGrid` is the same with `rows:` (issue #205).
pub(crate) fn lazy_vgrid_init(ctx: &mut dyn StdContext, args: Vec<Arg>) -> StdResult {
    grid_tracks_init("LazyVGrid", "columns", ctx, args)
}

/// `LazyHGrid(rows: [GridItem], …) { ... }` — the horizontal counterpart.
pub(crate) fn lazy_hgrid_init(ctx: &mut dyn StdContext, args: Vec<Arg>) -> StdResult {
    grid_tracks_init("LazyHGrid", "rows", ctx, args)
}

/// Shared `LazyVGrid`/`LazyHGrid` builder: capture the track array
/// (`columns:`/`rows:`), optional `spacing:`/`alignment:`, then collect the
/// content children from the trailing `@ViewBuilder` closure.
fn grid_tracks_init(
    type_name: &str,
    axis_label: &str,
    ctx: &mut dyn StdContext,
    args: Vec<Arg>,
) -> StdResult {
    let mut tracks: Option<SwiftValue> = None;
    let mut spacing: Option<SwiftValue> = None;
    let mut alignment: Option<SwiftValue> = None;
    let mut rest: Vec<Arg> = Vec::new();
    for arg in args {
        match arg.label.as_deref() {
            Some(label) if label == axis_label => tracks = Some(arg.value),
            Some("spacing") => spacing = Some(arg.value),
            Some("alignment") => alignment = Some(arg.value),
            Some("pinnedViews") => {} // visual-only; ignored for now
            _ => rest.push(arg),
        }
    }
    let children = collect_children(ctx, rest)?;
    let mut fields: Vec<(String, SwiftValue)> = Vec::new();
    if let Some(tracks) = tracks {
        fields.push((axis_label.into(), tracks));
    }
    if let Some(spacing) = spacing {
        fields.push(("spacing".into(), spacing));
    }
    if let Some(alignment) = alignment {
        fields.push(("alignment".into(), alignment));
    }
    fields.push((CHILDREN_FIELD.into(), SwiftValue::Array(Rc::new(children))));
    Ok(view_value(type_name, fields))
}

/// `GridRow { ... }` — one row of a `Grid`; its children are the row's cells.
pub(crate) fn grid_row_init(ctx: &mut dyn StdContext, args: Vec<Arg>) -> StdResult {
    Ok(container_value(
        "GridRow",
        collect_closure_children("GridRow", ctx, args)?,
    ))
}

/// `Form { ... }` — a grouped, list-styled container for settings-style content.
pub(crate) fn form_init(ctx: &mut dyn StdContext, args: Vec<Arg>) -> StdResult {
    Ok(container_value(
        "Form",
        collect_closure_children("Form", ctx, args)?,
    ))
}

/// `Divider()` — a thin rule separating content along the container's axis.
pub(crate) fn divider_init(_ctx: &mut dyn StdContext, _args: Vec<Arg>) -> StdResult {
    Ok(view_value("Divider", Vec::new()))
}

/// `ScrollView(_ axes:) { ... }` — a scrollable container. Captures an optional
/// leading `Axis` token (`.horizontal`/`.vertical`; default vertical) as the
/// `axes` field; `showsIndicators:` is parsed-and-dropped.
pub(crate) fn scrollview_init(ctx: &mut dyn StdContext, args: Vec<Arg>) -> StdResult {
    let mut axes: Option<SwiftValue> = None;
    let mut rest: Vec<Arg> = Vec::new();
    for arg in args {
        match arg.label.as_deref() {
            Some("showsIndicators") => {} // visual-only; ignored for now
            _ if matches!(token_of(&arg.value), Some(("Axis", _))) => axes = Some(arg.value),
            _ => rest.push(arg),
        }
    }
    let children = collect_children(ctx, rest)?;
    let mut fields: Vec<(String, SwiftValue)> = Vec::new();
    if let Some(axes) = axes {
        fields.push(("axes".into(), axes));
    }
    fields.push((CHILDREN_FIELD.into(), SwiftValue::Array(Rc::new(children))));
    Ok(view_value("ScrollView", fields))
}

/// `Spacer(minLength:)` — flexible empty space with an optional minimum length
/// along the stack's axis.
pub(crate) fn spacer_init(_ctx: &mut dyn StdContext, args: Vec<Arg>) -> StdResult {
    let mut fields: Vec<(String, SwiftValue)> = Vec::new();
    for arg in args {
        if arg.label.as_deref() == Some("minLength") {
            fields.push(("minLength".into(), arg.value));
        }
    }
    Ok(view_value("Spacer", fields))
}
/// `Toggle(_ title: String, isOn: Binding<Bool>)` — a labelled on/off control.
/// The current `isOn` bool is read from the binding for rendering; the binding
/// itself is stashed internally so the dispatch loop can write a new value
/// through it (`set` event) to drive the bound `@State`.
pub(crate) fn toggle_init(ctx: &mut dyn StdContext, args: Vec<Arg>) -> StdResult {
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
pub(crate) fn text_field_init(ctx: &mut dyn StdContext, args: Vec<Arg>) -> StdResult {
    input_field_init(ctx, args, "TextField")
}

/// `SecureField(_ title, text: Binding<String>)` — a masked text input. Same
/// shape as `TextField`; the host renders the value obscured.
pub(crate) fn secure_field_init(ctx: &mut dyn StdContext, args: Vec<Arg>) -> StdResult {
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
/// the title string; the trailing closure is the tap action, stored under the
/// `"tap"` key of the view's [`HANDLERS_FIELD`] map (ADR-0013 §3) which the
/// dispatch loop invokes on a `tap` event.
pub(crate) fn button_init(_ctx: &mut dyn StdContext, args: Vec<Arg>) -> StdResult {
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
        fields.push((HANDLERS_FIELD.into(), handlers_map(vec![("tap", action)])));
    }
    Ok(view_value("Button", fields))
}

/// Resolve a container's `@ViewBuilder` content into an ordered child list.
/// Each argument is either the content closure (evaluated as a result-builder
/// block) or an already-built view; non-view statement values are dropped and
/// composed custom `View`s are expanded into their `body`.
pub(crate) fn collect_children(
    ctx: &mut dyn StdContext,
    args: Vec<Arg>,
) -> Result<Vec<SwiftValue>, StdError> {
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
