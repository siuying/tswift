//! ChartContent mark modifiers — chainable methods on mark view values.
//!
//! Marks are ordinary view values with a `_modifiers` list (same shape as
//! SwiftUI). Each modifier appends a `_Modifier { name, <args> }` record via
//! the shared COW path in `tswift_swiftui::{append_modifier, make_modifier}`.
//!
//! Shared names (`foregroundStyle`, `cornerRadius`, `opacity`, `offset`) are
//! re-registered here so Charts can attach typed parameter hints (`by:` →
//! `PlottableValue`, etc.) without changing the append semantics SwiftUI uses.
//! Charts-only names (`symbol`, `lineStyle`, …) are registered only here.

use tswift_core::{
    Arg, BuiltinParam, Interpreter, StdContext, StdResult, StructMethodFn, SwiftValue,
};
use tswift_swiftui::{append_modifier, collect_children, container_value, make_modifier};

/// Plain mark modifier: store every call arg on a `_Modifier` and append.
macro_rules! mark_modifier {
    ($fn_name:ident, $swift_name:literal) => {
        fn $fn_name(_ctx: &mut dyn StdContext, recv: SwiftValue, args: Vec<Arg>) -> StdResult {
            append_modifier(recv, make_modifier($swift_name, args))
        }
    };
}

mark_modifier!(modifier_foreground_style, "foregroundStyle");
mark_modifier!(modifier_symbol, "symbol");
mark_modifier!(modifier_symbol_size, "symbolSize");
mark_modifier!(modifier_line_style, "lineStyle");
mark_modifier!(modifier_interpolation_method, "interpolationMethod");
mark_modifier!(modifier_corner_radius, "cornerRadius");
mark_modifier!(modifier_opacity, "opacity");
mark_modifier!(modifier_offset, "offset");
mark_modifier!(modifier_position, "position");

/// `.annotation(position:…, …) { content }` — like SwiftUI `overlay`/`background`:
/// evaluate the `@ViewBuilder` content into a child view value stored on the
/// modifier record (under `value` when unlabeled, or `content` when labeled).
fn modifier_annotation(ctx: &mut dyn StdContext, recv: SwiftValue, args: Vec<Arg>) -> StdResult {
    let mut meta: Vec<Arg> = Vec::new();
    let mut content_args: Vec<Arg> = Vec::new();
    for arg in args {
        match arg.label.as_deref() {
            Some("position") | Some("alignment") | Some("spacing") | Some("overflowResolution") => {
                meta.push(arg)
            }
            Some("content") => content_args.push(Arg {
                label: None,
                value: arg.value,
                static_ty: None,
            }),
            // Trailing `@ViewBuilder` closure / direct view (unlabeled).
            None => content_args.push(arg),
            // Unknown labels (forward-compat) stay on the record as-is.
            _ => meta.push(arg),
        }
    }
    let views = collect_children(ctx, content_args)?;
    let content = match views.len() {
        0 => None,
        1 => Some(views.into_iter().next().expect("len checked")),
        _ => Some(container_value("ZStack", views)),
    };
    if let Some(content) = content {
        meta.push(Arg {
            label: None,
            value: content,
            static_ty: None,
        });
    }
    append_modifier(recv, make_modifier("annotation", meta))
}

/// Mark modifiers registered as generic struct methods (any mark view value).
/// Coverage keys are `ChartContent.<name>` (inventory owning protocol).
pub(crate) const MARK_MODIFIER_FNS: &[(&str, StructMethodFn)] = &[
    ("foregroundStyle", modifier_foreground_style),
    ("symbol", modifier_symbol),
    ("symbolSize", modifier_symbol_size),
    ("lineStyle", modifier_line_style),
    ("interpolationMethod", modifier_interpolation_method),
    ("annotation", modifier_annotation),
    ("cornerRadius", modifier_corner_radius),
    ("opacity", modifier_opacity),
    ("offset", modifier_offset),
    ("position", modifier_position),
];

/// Register mark modifiers + typed parameter hints for leading-dot resolution.
pub(crate) fn install(interp: &mut Interpreter<'_>) {
    for (name, func) in MARK_MODIFIER_FNS {
        interp.register_struct_method(name, *func);
    }

    // `.foregroundStyle(by: .value(...))` and positional ShapeStyle/Color.
    interp.register_struct_method_typed(
        "foregroundStyle",
        modifier_foreground_style,
        vec![
            BuiltinParam::positional("Color"),
            BuiltinParam::labeled("by", "PlottableValue"),
        ],
    );
    // `.symbol(.circle)` / `.symbol(by: .value(...))`.
    interp.register_struct_method_typed(
        "symbol",
        modifier_symbol,
        vec![
            BuiltinParam::positional("ChartSymbolShape"),
            BuiltinParam::labeled("by", "PlottableValue"),
        ],
    );
    // `.interpolationMethod(.catmullRom)` — disambiguate from `Animation.linear`.
    interp.register_struct_method_typed(
        "interpolationMethod",
        modifier_interpolation_method,
        vec![BuiltinParam::positional("InterpolationMethod")],
    );
    // `.annotation(position: .top) { … }` — disambiguate from `Alignment.top`.
    interp.register_struct_method_typed(
        "annotation",
        modifier_annotation,
        vec![
            BuiltinParam::labeled("position", "AnnotationPosition"),
            BuiltinParam::labeled("alignment", "Alignment"),
            BuiltinParam::labeled("spacing", "CGFloat"),
        ],
    );
    // `.position(by: .value(...))` for grouped positioning.
    interp.register_struct_method_typed(
        "position",
        modifier_position,
        vec![BuiltinParam::labeled("by", "PlottableValue")],
    );
}
