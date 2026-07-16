//! View modifiers — the `_Modifier` record scheme, the shared modifier table
//! ([`MODIFIER_FNS`]), nested-view modifiers (`background`/`overlay`/
//! `tabItem`), environment injection, and the event/gesture/watch handlers
//! (ADR-0013 §3).

use std::rc::Rc;

use tswift_core::{Arg, StdContext, StdResult, StructMethodFn, StructObj, SwiftValue};

use crate::navigation::modifier_navigation_destination;
use crate::{
    container_value, expand_into, token_of, type_error, ENV_FIELD, HANDLERS_FIELD, HANDLERS_TYPE,
    MODIFIERS_FIELD, MODIFIER_TYPE, WATCH_FIELD, WATCH_TYPE,
};

/// Define a view-modifier intrinsic that appends a named `_Modifier` record to
/// the receiver view (copy-on-write). All v1 modifiers share this shape; the
/// host interprets the recorded name + args.
macro_rules! modifier {
    ($fn_name:ident, $swift_name:literal) => {
        pub(crate) fn $fn_name(
            _ctx: &mut dyn StdContext,
            recv: SwiftValue,
            args: Vec<Arg>,
        ) -> StdResult {
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
// `background`/`overlay` are not plain `modifier!`s: their argument may be an
// arbitrary nested view (issue #204), so they evaluate a trailing closure and
// thread an optional `alignment:` — see `compose_modifier`.
modifier!(modifier_fill, "fill");
modifier!(modifier_tag, "tag");
// `.tabItem { ... }` is not a plain `modifier!`: its argument is a nested label
// view (a `Label`, or a `Text`/`Image` pair) built from a trailing
// `@ViewBuilder` closure, serialized like `background`/`overlay` — see
// `modifier_tab_item`.
// C1 — text & universal styling modifiers (no new node kinds).
modifier!(modifier_bold, "bold");
modifier!(modifier_italic, "italic");
modifier!(modifier_underline, "underline");
modifier!(modifier_strikethrough, "strikethrough");
// Text typography adjustments (Text -> Text). `kerning`/`tracking`/
// `baselineOffset` carry a CGFloat; `monospaced`/`monospacedDigit` are Bool
// toggles (default true); `fontDesign`/`fontWidth` carry a token.
modifier!(modifier_kerning, "kerning");
modifier!(modifier_tracking, "tracking");
modifier!(modifier_baseline_offset, "baselineOffset");
modifier!(modifier_monospaced, "monospaced");
modifier!(modifier_monospaced_digit, "monospacedDigit");
modifier!(modifier_font_design, "fontDesign");
modifier!(modifier_font_width, "fontWidth");
// Graphic/visual-effect modifiers (Core Animation-style filters). Each records
// a scalar, Bool, token, `Color`, or `Angle` value the host applies.
modifier!(modifier_blur, "blur");
modifier!(modifier_brightness, "brightness");
modifier!(modifier_contrast, "contrast");
modifier!(modifier_saturation, "saturation");
modifier!(modifier_grayscale, "grayscale");
modifier!(modifier_hue_rotation, "hueRotation");
modifier!(modifier_color_invert, "colorInvert");
modifier!(modifier_color_multiply, "colorMultiply");
modifier!(modifier_scale_effect, "scaleEffect");
modifier!(modifier_rotation_effect, "rotationEffect");
modifier!(modifier_hidden, "hidden");
modifier!(modifier_allows_hit_testing, "allowsHitTesting");
modifier!(modifier_line_spacing, "lineSpacing");
modifier!(modifier_minimum_scale_factor, "minimumScaleFactor");
modifier!(modifier_allows_tightening, "allowsTightening");
modifier!(modifier_labels_hidden, "labelsHidden");
modifier!(modifier_help, "help");
modifier!(modifier_scroll_disabled, "scrollDisabled");
// List & scroll styling. No-arg render hints (compositingGroup/drawingGroup/
// unredacted), Bool toggles (scrollClipDisabled/interactiveDismissDisabled/
// accessibilityHidden/flipsForRightToLeftLayoutDirection), Visibility tokens
// (listRowSeparator/listSectionSeparator/scrollContentBackground/
// scrollIndicators), and `Color` tints for separator lines.
modifier!(modifier_compositing_group, "compositingGroup");
modifier!(modifier_drawing_group, "drawingGroup");
modifier!(modifier_unredacted, "unredacted");
modifier!(modifier_scroll_clip_disabled, "scrollClipDisabled");
modifier!(
    modifier_interactive_dismiss_disabled,
    "interactiveDismissDisabled"
);
modifier!(modifier_accessibility_hidden, "accessibilityHidden");
modifier!(modifier_flips_for_rtl, "flipsForRightToLeftLayoutDirection");
modifier!(modifier_list_row_separator, "listRowSeparator");
modifier!(modifier_list_section_separator, "listSectionSeparator");
modifier!(modifier_list_row_separator_tint, "listRowSeparatorTint");
modifier!(
    modifier_list_section_separator_tint,
    "listSectionSeparatorTint"
);
modifier!(
    modifier_scroll_content_background,
    "scrollContentBackground"
);
modifier!(modifier_scroll_indicators, "scrollIndicators");
// Token-enum modifiers: blend/size/rendering/redaction tokens.
modifier!(modifier_blend_mode, "blendMode");
modifier!(modifier_control_size, "controlSize");
modifier!(modifier_symbol_rendering_mode, "symbolRenderingMode");
modifier!(modifier_redacted, "redacted");
modifier!(modifier_truncation_mode, "truncationMode");
modifier!(modifier_opacity, "opacity");
modifier!(modifier_foreground_style, "foregroundStyle");
modifier!(modifier_tint, "tint");
modifier!(modifier_line_limit, "lineLimit");
modifier!(modifier_multiline_text_alignment, "multilineTextAlignment");
modifier!(modifier_text_case, "textCase");
// C2 — layout. `.offset(x:y:)` shifts a view by a fixed translation.
modifier!(modifier_offset, "offset");
// C4 — visual decoration. `clipShape` carries a nested shape descriptor
// (a view value); `border`/`shadow` carry color tokens + numeric lengths.
modifier!(modifier_clipped, "clipped");
modifier!(modifier_clip_shape, "clipShape");
modifier!(modifier_border, "border");
modifier!(modifier_shadow, "shadow");
// C7 — control styling (token-valued) + `disabled` (Bool).
modifier!(modifier_button_style, "buttonStyle");
modifier!(modifier_list_style, "listStyle");
modifier!(modifier_picker_style, "pickerStyle");
modifier!(modifier_text_field_style, "textFieldStyle");
modifier!(modifier_disabled, "disabled");
// Additional container/control style modifiers. Each carries a leading-dot
// `_ControlStyle` token (`.automatic`, `.grouped`, `.page`, `.accessoryCircular`
// …); the host disambiguates by modifier name, exactly like buttonStyle/
// listStyle/pickerStyle above.
modifier!(modifier_toggle_style, "toggleStyle");
modifier!(modifier_menu_style, "menuStyle");
modifier!(modifier_gauge_style, "gaugeStyle");
modifier!(modifier_form_style, "formStyle");
modifier!(modifier_group_box_style, "groupBoxStyle");
modifier!(modifier_labeled_content_style, "labeledContentStyle");
modifier!(modifier_index_view_style, "indexViewStyle");
modifier!(modifier_tab_view_style, "tabViewStyle");
modifier!(modifier_date_picker_style, "datePickerStyle");
modifier!(modifier_disclosure_group_style, "disclosureGroupStyle");
modifier!(modifier_control_group_style, "controlGroupStyle");
// Text-input modifiers. `submitLabel` carries a `SubmitLabel` token;
// `textInputAutocapitalization` a `TextInputAutocapitalization`.
// `autocorrectionDisabled`/`focusable`/`disableAutocorrection` are Bool toggles
// (default true). (`colorScheme`/`preferredColorScheme` are deferred: their
// `.light` token collides with `FontWeight.light` and needs contextual typing.)
modifier!(modifier_submit_label, "submitLabel");
modifier!(
    modifier_text_input_autocapitalization,
    "textInputAutocapitalization"
);
modifier!(modifier_autocorrection_disabled, "autocorrectionDisabled");
modifier!(modifier_disable_autocorrection, "disableAutocorrection");
modifier!(modifier_focusable, "focusable");
// C7 — accessibility no-ops: accepted-and-recorded so snippets using them still
// render; the hosts ignore them (no visual effect).
modifier!(modifier_accessibility_label, "accessibilityLabel");
modifier!(modifier_accessibility_hint, "accessibilityHint");
modifier!(modifier_accessibility_value, "accessibilityValue");
modifier!(modifier_accessibility_identifier, "accessibilityIdentifier");
// Accessibility metadata modifiers: recorded on the view node so the serialized
// UIIR carries the semantic data (there is no on-device assistive tech in a
// headless runtime). Token-valued ones carry an `AccessibilityTraits` /
// `AccessibilityHeadingLevel` / `AccessibilityChildBehavior` leading-dot token;
// the rest carry a scalar, `[String]`, or `Bool`.
modifier!(modifier_accessibility_add_traits, "accessibilityAddTraits");
modifier!(
    modifier_accessibility_remove_traits,
    "accessibilityRemoveTraits"
);
modifier!(
    modifier_accessibility_sort_priority,
    "accessibilitySortPriority"
);
modifier!(modifier_accessibility_heading, "accessibilityHeading");
modifier!(
    modifier_accessibility_input_labels,
    "accessibilityInputLabels"
);
modifier!(modifier_accessibility_element, "accessibilityElement");
modifier!(
    modifier_accessibility_ignores_invert_colors,
    "accessibilityIgnoresInvertColors"
);
modifier!(
    modifier_accessibility_responds_to_user_interaction,
    "accessibilityRespondsToUserInteraction"
);
modifier!(
    modifier_accessibility_direct_touch,
    "accessibilityDirectTouch"
);
modifier!(
    modifier_accessibility_shows_large_content_viewer,
    "accessibilityShowsLargeContentViewer"
);
// List-editing & row-layout + misc identity modifiers. All carry a scalar,
// Bool, String, or passthrough value — no leading-dot token — so they record
// straight onto the view node.
modifier!(modifier_delete_disabled, "deleteDisabled");
modifier!(modifier_move_disabled, "moveDisabled");
modifier!(modifier_selection_disabled, "selectionDisabled");
modifier!(modifier_list_row_spacing, "listRowSpacing");
modifier!(modifier_list_section_spacing, "listSectionSpacing");
modifier!(modifier_badge, "badge");
modifier!(modifier_id, "id");
modifier!(modifier_geometry_group, "geometryGroup");
modifier!(modifier_invalidatable_content, "invalidatableContent");
modifier!(
    modifier_interaction_activity_tracking_tag,
    "interactionActivityTrackingTag"
);
// Tier 2 — scale/aspect/layout modifiers.
modifier!(modifier_scaled_to_fit, "scaledToFit");
modifier!(modifier_scaled_to_fill, "scaledToFill");
modifier!(modifier_aspect_ratio, "aspectRatio");
modifier!(modifier_fixed_size, "fixedSize");
modifier!(modifier_layout_priority, "layoutPriority");
modifier!(modifier_z_index, "zIndex");
modifier!(modifier_navigation_title, "navigationTitle");
modifier!(modifier_resizable, "resizable");
// Slice 3 — `.transition(_:)` records an `AnyTransition` for insert/remove.
modifier!(modifier_transition, "transition");

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

/// `.background(_ content, alignment:)` / `.overlay(_ content, alignment:)`.
/// `content` is either a `ShapeStyle`/`Color` token (the C0 behavior) or an
/// arbitrary nested view — given directly (`.background(Circle())`) or via a
/// trailing `@ViewBuilder` closure (`.overlay { Circle() }`). A nested view is
/// serialized as its own `0`-rooted subtree (`write_value` already lowers a view
/// value to a node); the host renders it as a detached layer behind
/// (background) or in front of (overlay) the receiver, honoring `alignment:`
/// (issue #204).
fn compose_modifier(
    ctx: &mut dyn StdContext,
    recv: SwiftValue,
    name: &str,
    args: Vec<Arg>,
) -> StdResult {
    let mut alignment: Option<SwiftValue> = None;
    let mut content: Option<SwiftValue> = None;
    for arg in args {
        match arg.label.as_deref() {
            Some("alignment") => alignment = Some(arg.value),
            // The content: a `ShapeStyle`/`Color` token (the C0 color path), a
            // direct view value (builtin `Circle()` or a custom `Badge()`), or a
            // trailing `@ViewBuilder` closure. Views — including custom views
            // collapsed to their `body` — are resolved via `expand_into`;
            // multiple statements compose as an implicit `ZStack` (SwiftUI groups
            // them back-to-front). A token stays a token.
            _ => match arg.value {
                token if token_of(&token).is_some() => content = Some(token),
                view_or_closure => {
                    let mut views = Vec::new();
                    match view_or_closure {
                        SwiftValue::Closure(id) => {
                            let block = ctx.eval_block_values(id)?;
                            expand_into(ctx, block, &mut views, 0, &[])?;
                        }
                        other => expand_into(ctx, other, &mut views, 0, &[])?,
                    }
                    content = match views.len() {
                        0 => None, // an unsupported/empty content value
                        1 => Some(views.into_iter().next().expect("len checked")),
                        _ => Some(container_value("ZStack", views)),
                    };
                }
            },
        }
    }
    let mut margs: Vec<Arg> = Vec::new();
    if let Some(content) = content {
        margs.push(Arg {
            label: None,
            value: content,

            static_ty: None,
        });
    }
    if let Some(alignment) = alignment {
        margs.push(Arg {
            label: Some("alignment".into()),
            value: alignment,

            static_ty: None,
        });
    }
    append_modifier(recv, make_modifier(name, margs))
}

/// `.animation(_ animation: Animation?, value:)` (modern) and the deprecated
/// `.animation(_ animation: Animation?)` (no `value:`). Records an `animation`
/// modifier whose serialized value is an object with `animation` (the curve, or
/// JSON `null` to disable) plus, for the modern form, `value` — the current
/// observed operand the host diffs across renders to know when to animate.
/// Mirrors https://developer.apple.com/documentation/swiftui/view/animation(_:value:).
pub(crate) fn modifier_animation(
    _ctx: &mut dyn StdContext,
    recv: SwiftValue,
    args: Vec<Arg>,
) -> StdResult {
    let mut animation: SwiftValue = SwiftValue::Nil;
    let mut observed: Option<SwiftValue> = None;
    for arg in args {
        match arg.label.as_deref() {
            Some("value") => observed = Some(arg.value),
            // The leading positional is the `Animation?` (possibly `nil`).
            _ => animation = arg.value,
        }
    }
    let mut margs = vec![Arg {
        label: Some("animation".into()),
        value: animation,

        static_ty: None,
    }];
    if let Some(value) = observed {
        margs.push(Arg {
            label: Some("value".into()),
            value,

            static_ty: None,
        });
    }
    append_modifier(recv, make_modifier("animation", margs))
}

pub(crate) fn modifier_background(
    ctx: &mut dyn StdContext,
    recv: SwiftValue,
    args: Vec<Arg>,
) -> StdResult {
    compose_modifier(ctx, recv, "background", args)
}

pub(crate) fn modifier_overlay(
    ctx: &mut dyn StdContext,
    recv: SwiftValue,
    args: Vec<Arg>,
) -> StdResult {
    compose_modifier(ctx, recv, "overlay", args)
}

/// View modifiers registered as generic struct methods, by Swift name. Drives
/// both [`install`] and the `View.<name>` coverage keys in [`registered_keys`].
pub(crate) const MODIFIER_FNS: &[(&str, StructMethodFn)] = &[
    ("frame", modifier_frame),
    ("padding", modifier_padding),
    ("cornerRadius", modifier_corner_radius),
    ("font", modifier_font),
    ("fontWeight", modifier_font_weight),
    ("foregroundColor", modifier_foreground_color),
    ("background", modifier_background),
    ("overlay", modifier_overlay),
    ("fill", modifier_fill),
    ("tag", modifier_tag),
    ("tabItem", modifier_tab_item),
    ("bold", modifier_bold),
    ("italic", modifier_italic),
    ("underline", modifier_underline),
    ("strikethrough", modifier_strikethrough),
    ("kerning", modifier_kerning),
    ("tracking", modifier_tracking),
    ("baselineOffset", modifier_baseline_offset),
    ("monospaced", modifier_monospaced),
    ("monospacedDigit", modifier_monospaced_digit),
    ("fontDesign", modifier_font_design),
    ("fontWidth", modifier_font_width),
    ("blur", modifier_blur),
    ("brightness", modifier_brightness),
    ("contrast", modifier_contrast),
    ("saturation", modifier_saturation),
    ("grayscale", modifier_grayscale),
    ("hueRotation", modifier_hue_rotation),
    ("colorInvert", modifier_color_invert),
    ("colorMultiply", modifier_color_multiply),
    ("scaleEffect", modifier_scale_effect),
    ("rotationEffect", modifier_rotation_effect),
    ("hidden", modifier_hidden),
    ("allowsHitTesting", modifier_allows_hit_testing),
    ("lineSpacing", modifier_line_spacing),
    ("minimumScaleFactor", modifier_minimum_scale_factor),
    ("allowsTightening", modifier_allows_tightening),
    ("labelsHidden", modifier_labels_hidden),
    ("help", modifier_help),
    ("scrollDisabled", modifier_scroll_disabled),
    ("compositingGroup", modifier_compositing_group),
    ("drawingGroup", modifier_drawing_group),
    ("unredacted", modifier_unredacted),
    ("scrollClipDisabled", modifier_scroll_clip_disabled),
    (
        "interactiveDismissDisabled",
        modifier_interactive_dismiss_disabled,
    ),
    ("accessibilityHidden", modifier_accessibility_hidden),
    ("flipsForRightToLeftLayoutDirection", modifier_flips_for_rtl),
    ("listRowSeparator", modifier_list_row_separator),
    ("listSectionSeparator", modifier_list_section_separator),
    ("listRowSeparatorTint", modifier_list_row_separator_tint),
    (
        "listSectionSeparatorTint",
        modifier_list_section_separator_tint,
    ),
    (
        "scrollContentBackground",
        modifier_scroll_content_background,
    ),
    ("scrollIndicators", modifier_scroll_indicators),
    ("blendMode", modifier_blend_mode),
    ("controlSize", modifier_control_size),
    ("symbolRenderingMode", modifier_symbol_rendering_mode),
    ("redacted", modifier_redacted),
    ("truncationMode", modifier_truncation_mode),
    ("opacity", modifier_opacity),
    ("foregroundStyle", modifier_foreground_style),
    ("tint", modifier_tint),
    ("lineLimit", modifier_line_limit),
    ("multilineTextAlignment", modifier_multiline_text_alignment),
    ("textCase", modifier_text_case),
    ("offset", modifier_offset),
    ("clipped", modifier_clipped),
    ("clipShape", modifier_clip_shape),
    ("border", modifier_border),
    ("shadow", modifier_shadow),
    ("buttonStyle", modifier_button_style),
    ("listStyle", modifier_list_style),
    ("pickerStyle", modifier_picker_style),
    ("textFieldStyle", modifier_text_field_style),
    ("toggleStyle", modifier_toggle_style),
    ("menuStyle", modifier_menu_style),
    ("gaugeStyle", modifier_gauge_style),
    ("formStyle", modifier_form_style),
    ("groupBoxStyle", modifier_group_box_style),
    ("labeledContentStyle", modifier_labeled_content_style),
    ("indexViewStyle", modifier_index_view_style),
    ("tabViewStyle", modifier_tab_view_style),
    ("datePickerStyle", modifier_date_picker_style),
    ("disclosureGroupStyle", modifier_disclosure_group_style),
    ("controlGroupStyle", modifier_control_group_style),
    ("submitLabel", modifier_submit_label),
    (
        "textInputAutocapitalization",
        modifier_text_input_autocapitalization,
    ),
    ("autocorrectionDisabled", modifier_autocorrection_disabled),
    ("disableAutocorrection", modifier_disable_autocorrection),
    ("focusable", modifier_focusable),
    ("disabled", modifier_disabled),
    ("accessibilityLabel", modifier_accessibility_label),
    ("accessibilityHint", modifier_accessibility_hint),
    ("accessibilityValue", modifier_accessibility_value),
    ("accessibilityIdentifier", modifier_accessibility_identifier),
    ("accessibilityAddTraits", modifier_accessibility_add_traits),
    (
        "accessibilityRemoveTraits",
        modifier_accessibility_remove_traits,
    ),
    (
        "accessibilitySortPriority",
        modifier_accessibility_sort_priority,
    ),
    ("accessibilityHeading", modifier_accessibility_heading),
    (
        "accessibilityInputLabels",
        modifier_accessibility_input_labels,
    ),
    ("accessibilityElement", modifier_accessibility_element),
    (
        "accessibilityIgnoresInvertColors",
        modifier_accessibility_ignores_invert_colors,
    ),
    (
        "accessibilityRespondsToUserInteraction",
        modifier_accessibility_responds_to_user_interaction,
    ),
    (
        "accessibilityDirectTouch",
        modifier_accessibility_direct_touch,
    ),
    (
        "accessibilityShowsLargeContentViewer",
        modifier_accessibility_shows_large_content_viewer,
    ),
    ("deleteDisabled", modifier_delete_disabled),
    ("moveDisabled", modifier_move_disabled),
    ("selectionDisabled", modifier_selection_disabled),
    ("listRowSpacing", modifier_list_row_spacing),
    ("listSectionSpacing", modifier_list_section_spacing),
    ("badge", modifier_badge),
    ("id", modifier_id),
    ("geometryGroup", modifier_geometry_group),
    ("invalidatableContent", modifier_invalidatable_content),
    (
        "interactionActivityTrackingTag",
        modifier_interaction_activity_tracking_tag,
    ),
    ("environmentObject", modifier_environment_object),
    // Tier 2 — scale / aspect / layout / z-order / navigation modifiers.
    ("scaledToFit", modifier_scaled_to_fit),
    ("scaledToFill", modifier_scaled_to_fill),
    ("aspectRatio", modifier_aspect_ratio),
    ("fixedSize", modifier_fixed_size),
    ("layoutPriority", modifier_layout_priority),
    ("zIndex", modifier_z_index),
    ("navigationTitle", modifier_navigation_title),
    ("navigationDestination", modifier_navigation_destination),
    ("resizable", modifier_resizable),
    // Lifecycle / gesture / submit event handlers (ADR-0013 §3).
    ("onTapGesture", modifier_on_tap_gesture),
    ("onLongPressGesture", modifier_on_long_press_gesture),
    ("onSubmit", modifier_on_submit),
    ("onAppear", modifier_on_appear),
    ("task", modifier_task),
    ("onDisappear", modifier_on_disappear),
    ("onChange", modifier_on_change),
    // Gesture composition: `.gesture(TapGesture().onEnded { })` lowers to the
    // same marker+handler route as `.onTapGesture`/`.onLongPressGesture`.
    ("gesture", modifier_gesture),
    // `.animation(_:value:)` / deprecated `.animation(_:)` (Slice 2).
    ("animation", modifier_animation),
    // `.transition(_:)` — records an `AnyTransition` (Slice 3).
    ("transition", modifier_transition),
];

/// `.tabItem { Label/Text/Image }` — record a tab's bar label (ADR-0013 §2).
/// The trailing `@ViewBuilder` produces the label subtree (classic API: a
/// `Label`, or a `Text` + `Image` pair); it is serialized as the modifier's
/// value like other nested-view modifiers (cf. `background`/`overlay`), so the
/// host builds the tab-bar item from this marker. (The iOS 18 `Tab` struct API
/// is out of scope.)
fn modifier_tab_item(ctx: &mut dyn StdContext, recv: SwiftValue, args: Vec<Arg>) -> StdResult {
    let mut views = Vec::new();
    for arg in args {
        match arg.value {
            SwiftValue::Closure(id) => {
                let block = ctx.eval_block_values(id)?;
                expand_into(ctx, block, &mut views, 0, &[])?;
            }
            other => expand_into(ctx, other, &mut views, 0, &[])?,
        }
    }
    // A single label view is stored directly; a `Text` + `Image` pair composes
    // as a `Group` the host renders as the tab item (icon + title).
    let content = match views.len() {
        0 => None,
        1 => Some(views.into_iter().next().expect("len checked")),
        _ => Some(container_value("Group", views)),
    };
    let margs = match content {
        Some(view) => vec![Arg {
            label: None,
            value: view,

            static_ty: None,
        }],
        None => Vec::new(),
    };
    append_modifier(recv, make_modifier("tabItem", margs))
}

/// Build a [`HANDLERS_TYPE`] record from `(event, closure)` pairs. Only closure
/// values are kept (a missing/`nil` handler is dropped), so the map is empty
/// exactly when nothing is bound.
pub(crate) fn handlers_map(entries: Vec<(&str, SwiftValue)>) -> SwiftValue {
    let fields = entries
        .into_iter()
        .filter(|(_, v)| matches!(v, SwiftValue::Closure(_)))
        .map(|(k, v)| (k.to_string(), v))
        .collect();
    SwiftValue::Struct(Rc::new(StructObj {
        type_name: HANDLERS_TYPE.into(),
        fields,
    }))
}

/// Whether `view` already binds a closure for `event` in its [`HANDLERS_FIELD`]
/// map (e.g. a `Button` owns `"tap"` via its action).
fn has_handler(view: &SwiftValue, event: &str) -> bool {
    let SwiftValue::Struct(obj) = view else {
        return false;
    };
    matches!(
        obj.get(HANDLERS_FIELD),
        Some(SwiftValue::Struct(h)) if matches!(h.get(event), Some(SwiftValue::Closure(_)))
    )
}

/// Merge an event handler into a view's [`HANDLERS_FIELD`] map (copy-on-write),
/// creating the map if absent. A non-closure handler is a no-op (the caller may
/// pass an optional `perform:` that was omitted).
fn set_handler(view: SwiftValue, event: &str, closure: Option<SwiftValue>) -> StdResult {
    let Some(closure @ SwiftValue::Closure(_)) = closure else {
        return Ok(view);
    };
    let SwiftValue::Struct(obj) = &view else {
        return Err(type_error(format!(
            "event handler applied to non-view value `{}`",
            view.type_name()
        )));
    };
    let mut fields = obj.fields.clone();
    if !fields.iter().any(|(k, _)| k == HANDLERS_FIELD) {
        fields.push((HANDLERS_FIELD.into(), handlers_map(Vec::new())));
    }
    let slot = fields
        .iter_mut()
        .find(|(k, _)| k == HANDLERS_FIELD)
        .map(|(_, v)| v)
        .expect("_handlers slot ensured above");
    let mut map = match slot {
        SwiftValue::Struct(h) => (**h).clone(),
        _ => StructObj {
            type_name: HANDLERS_TYPE.into(),
            fields: Vec::new(),
        },
    };
    map.fields.retain(|(k, _)| k != event);
    map.fields.push((event.to_string(), closure));
    *slot = SwiftValue::Struct(Rc::new(map));
    Ok(SwiftValue::Struct(Rc::new(StructObj {
        type_name: obj.type_name.clone(),
        fields,
    })))
}

/// Attach a lifecycle/gesture/submit event to a view: append the marker
/// modifier (so hosts know which listener to wire) and register the handler
/// closure under `event` (ADR-0013 §3). Closures never serialize — only the
/// marker reaches the UIIR.
fn attach_event(
    recv: SwiftValue,
    marker: &str,
    event: &str,
    marker_args: Vec<Arg>,
    closure: Option<SwiftValue>,
) -> StdResult {
    let recv = append_modifier(recv, make_modifier(marker, marker_args))?;
    set_handler(recv, event, closure)
}

/// `.onTapGesture(count:perform:)` — fire `perform` on a tap (ADR-0013 §3).
/// Emits an `onTapGesture` marker (carrying `count` when > 1) and binds the
/// action under the `"tap"` event.
fn modifier_on_tap_gesture(
    _ctx: &mut dyn StdContext,
    recv: SwiftValue,
    args: Vec<Arg>,
) -> StdResult {
    let mut count: Option<SwiftValue> = None;
    let mut action: Option<SwiftValue> = None;
    for arg in args {
        match arg.label.as_deref() {
            Some("count") => count = Some(arg.value),
            Some("perform") => action = Some(arg.value),
            _ => match arg.value {
                v @ SwiftValue::Closure(_) => action = Some(v),
                v @ SwiftValue::Int(_) if count.is_none() => count = Some(v),
                _ => {}
            },
        }
    }
    // A `Button` already owns the `tap` event through its action. Adding
    // `.onTapGesture` to a Button must not clobber that action via the shared
    // `tap` key, nor add a second marker that would make hosts double-emit
    // `tap` (Button click + gesture listener). Keep the Button action
    // authoritative and drop the gesture — matching SwiftUI, where the Button
    // intercepts the tap before a `.onTapGesture` sees it.
    if has_handler(&recv, "tap") {
        return Ok(recv);
    }
    // A default single-tap emits a bare marker; a multi-tap records its count.
    let marker_args = match count {
        Some(SwiftValue::Int(i)) if i.raw != 1 => vec![Arg {
            label: Some("count".into()),
            value: SwiftValue::Int(i),

            static_ty: None,
        }],
        _ => Vec::new(),
    };
    attach_event(recv, "onTapGesture", "tap", marker_args, action)
}

/// `.onLongPressGesture(minimumDuration:maximumDistance:perform:onPressingChanged:)`
/// — fire `perform` on a long press. The optional `onPressingChanged` callback
/// is out of scope (no host press-state stream); `minimumDuration`/
/// `maximumDistance` are recorded on the marker when non-default so hosts can
/// tune the gesture.
fn modifier_on_long_press_gesture(
    _ctx: &mut dyn StdContext,
    recv: SwiftValue,
    args: Vec<Arg>,
) -> StdResult {
    let mut action: Option<SwiftValue> = None;
    let mut marker_args: Vec<Arg> = Vec::new();
    for arg in args {
        match arg.label.as_deref() {
            Some("perform") => action = Some(arg.value),
            Some("minimumDuration") => marker_args.push(Arg {
                label: Some("minimumDuration".into()),
                value: arg.value,

                static_ty: None,
            }),
            Some("maximumDistance") => marker_args.push(Arg {
                label: Some("maximumDistance".into()),
                value: arg.value,

                static_ty: None,
            }),
            // The `onPressingChanged:` callback (a `(Bool) -> Void`) has no host
            // press-state event; accept it but drop it.
            Some("onPressingChanged") => {}
            _ => {
                if let v @ SwiftValue::Closure(_) = arg.value {
                    if action.is_none() {
                        action = Some(v);
                    }
                }
            }
        }
    }
    attach_event(recv, "onLongPressGesture", "longPress", marker_args, action)
}

/// `.onSubmit(of:_:)` — fire `action` when the user submits a text field. The
/// `of:` `SubmitTriggers` token is out of scope (all submits route the same);
/// binds the action under the `"submit"` event.
fn modifier_on_submit(_ctx: &mut dyn StdContext, recv: SwiftValue, args: Vec<Arg>) -> StdResult {
    let action = args
        .into_iter()
        .find_map(|a| matches!(a.value, SwiftValue::Closure(_)).then_some(a.value));
    attach_event(recv, "onSubmit", "submit", Vec::new(), action)
}

/// `.onAppear(perform:)` — the host fires an `appear` event on mount; binds the
/// (optional) action under the `"appear"` event.
fn modifier_on_appear(_ctx: &mut dyn StdContext, recv: SwiftValue, args: Vec<Arg>) -> StdResult {
    let action = args
        .into_iter()
        .find_map(|a| matches!(a.value, SwiftValue::Closure(_)).then_some(a.value));
    attach_event(recv, "onAppear", "appear", Vec::new(), action)
}

/// `.task(priority:_:)` — SwiftUI runs the async action when the view appears
/// and cancels it on disappear. The runtime's cooperative executor has no
/// mid-flight cancellation, so v1 fires the action inline (any `await` inside
/// runs to completion) when the host calls `run_mount_tasks` after mount. The
/// optional `priority:` label is parsed and dropped (one signature covers all
/// priorities). Emits a `task` marker modifier and binds the action under the
/// `"task"` event; coexists with `.onAppear` (distinct handler keys).
fn modifier_task(_ctx: &mut dyn StdContext, recv: SwiftValue, args: Vec<Arg>) -> StdResult {
    let action = args
        .into_iter()
        .find_map(|a| matches!(a.value, SwiftValue::Closure(_)).then_some(a.value));
    attach_event(recv, "task", "task", Vec::new(), action)
}

/// `.onDisappear(perform:)` — the host fires a `disappear` event on unmount;
/// binds the (optional) action under the `"disappear"` event.
fn modifier_on_disappear(_ctx: &mut dyn StdContext, recv: SwiftValue, args: Vec<Arg>) -> StdResult {
    let action = args
        .into_iter()
        .find_map(|a| matches!(a.value, SwiftValue::Closure(_)).then_some(a.value));
    attach_event(recv, "onDisappear", "disappear", Vec::new(), action)
}

/// `.onChange(of:initial:_:)` — runtime-internal (ADR-0013 §3): record the
/// watched value plus the action into the view's [`WATCH_FIELD`] list. The
/// session compares the watched value across renders and invokes the action
/// with `(oldValue, newValue)`; no host involvement and no serialized modifier.
fn modifier_on_change(_ctx: &mut dyn StdContext, recv: SwiftValue, args: Vec<Arg>) -> StdResult {
    let mut value: Option<SwiftValue> = None;
    let mut action: Option<SwiftValue> = None;
    for arg in args {
        match arg.label.as_deref() {
            // `initial:` (fire once on appear) is out of scope; accept + drop.
            Some("initial") => {}
            Some("of") => value = Some(arg.value),
            _ => match arg.value {
                v @ SwiftValue::Closure(_) => action = Some(v),
                v if value.is_none() => value = Some(v),
                _ => {}
            },
        }
    }
    match (value, action) {
        (Some(value), Some(action)) => add_watch(recv, value, action),
        _ => Ok(recv),
    }
}

// ── Gesture value types (TapGesture / LongPressGesture) ────────────────────

/// `TapGesture(count:)` — constructs a tap-gesture value that can chain
/// `.onEnded { _ in … }` before being passed to `.gesture(_:)`. Mirrors
/// https://developer.apple.com/documentation/swiftui/tapgesture/init(count:).
pub(crate) fn tap_gesture_init(_ctx: &mut dyn StdContext, args: Vec<Arg>) -> StdResult {
    let count = args.into_iter().find_map(|a| match a.label.as_deref() {
        Some("count") | None => Some(a.value),
        _ => None,
    });
    let mut fields = Vec::new();
    if let Some(c) = count {
        fields.push(("count".into(), c));
    }
    Ok(SwiftValue::Struct(Rc::new(StructObj {
        type_name: "TapGesture".into(),
        fields,
    })))
}

/// `LongPressGesture(minimumDuration:)` — constructs a long-press gesture value
/// that can chain `.onEnded { _ in … }` before being passed to `.gesture(_:)`.
/// Mirrors https://developer.apple.com/documentation/swiftui/longpressgesture.
pub(crate) fn long_press_gesture_init(_ctx: &mut dyn StdContext, args: Vec<Arg>) -> StdResult {
    let mut fields = Vec::new();
    for arg in args {
        match arg.label.as_deref() {
            Some("minimumDuration") | None => {
                fields.push(("minimumDuration".into(), arg.value));
            }
            _ => {}
        }
    }
    Ok(SwiftValue::Struct(Rc::new(StructObj {
        type_name: "LongPressGesture".into(),
        fields,
    })))
}

/// `.onEnded(_:)` on `TapGesture` or `LongPressGesture` — attach the action
/// closure and return the modified gesture value. The value passed to `perform`
/// in real SwiftUI is the gesture's `Value` type (`Void` for `TapGesture`,
/// `Bool` for `LongPressGesture`); the runtime supplies `()` (unit) for both —
/// document this as an accepted v1 simplification in notes.md.
/// Mirrors https://developer.apple.com/documentation/swiftui/gesture/onended(_:).
pub(crate) fn gesture_on_ended(
    _ctx: &mut dyn StdContext,
    recv: SwiftValue,
    args: Vec<Arg>,
) -> StdResult {
    let action = args
        .into_iter()
        .find_map(|a| matches!(a.value, SwiftValue::Closure(_)).then_some(a.value));
    let Some(action) = action else {
        return Ok(recv);
    };
    let SwiftValue::Struct(obj) = &recv else {
        return Err(type_error(format!(
            "onEnded applied to non-gesture value `{}`",
            recv.type_name()
        )));
    };
    let mut fields = obj.fields.clone();
    fields.retain(|(k, _)| k != "_action");
    fields.push(("_action".into(), action));
    Ok(SwiftValue::Struct(Rc::new(StructObj {
        type_name: obj.type_name.clone(),
        fields,
    })))
}

/// `.gesture(_:)` View modifier — accepts a `TapGesture` or `LongPressGesture`
/// built via `.onEnded { }` and lowers it to the **same** marker + handler route
/// as `.onTapGesture`/`.onLongPressGesture` (ADR-0013 §3). Hosts need no new
/// code: the same `onTapGesture`/`onLongPressGesture` markers and `tap`/
/// `longPress` handler keys are emitted.
/// Mirrors https://developer.apple.com/documentation/swiftui/view/gesture(_:including:).
pub(crate) fn modifier_gesture(
    _ctx: &mut dyn StdContext,
    recv: SwiftValue,
    args: Vec<Arg>,
) -> StdResult {
    let gesture = args
        .into_iter()
        .find_map(|a| matches!(a.value, SwiftValue::Struct(_)).then_some(a.value));
    let Some(SwiftValue::Struct(ref g)) = gesture else {
        return Ok(recv);
    };
    let action = g.get("_action").cloned();
    match g.type_name.as_str() {
        "TapGesture" => {
            // Honour the same Button-priority rule as `.onTapGesture`.
            if has_handler(&recv, "tap") {
                return Ok(recv);
            }
            let marker_args = match g.get("count") {
                Some(SwiftValue::Int(i)) if i.raw != 1 => vec![Arg {
                    label: Some("count".into()),
                    value: SwiftValue::Int(*i),

                    static_ty: None,
                }],
                _ => Vec::new(),
            };
            attach_event(recv, "onTapGesture", "tap", marker_args, action)
        }
        "LongPressGesture" => {
            let mut marker_args = Vec::new();
            if let Some(v) = g.get("minimumDuration") {
                marker_args.push(Arg {
                    label: Some("minimumDuration".into()),
                    value: v.clone(),

                    static_ty: None,
                });
            }
            attach_event(recv, "onLongPressGesture", "longPress", marker_args, action)
        }
        _ => Ok(recv), // Unknown gesture type — silently ignored.
    }
}

/// Append an `onChange` watcher (`_Watch { value, action }`) to a view's
/// [`WATCH_FIELD`] list (copy-on-write).
fn add_watch(view: SwiftValue, value: SwiftValue, action: SwiftValue) -> StdResult {
    let SwiftValue::Struct(obj) = &view else {
        return Err(type_error(format!(
            "onChange applied to non-view value `{}`",
            view.type_name()
        )));
    };
    let mut fields = obj.fields.clone();
    if !fields.iter().any(|(k, _)| k == WATCH_FIELD) {
        fields.push((WATCH_FIELD.into(), SwiftValue::Array(Rc::new(Vec::new()))));
    }
    let slot = fields
        .iter_mut()
        .find(|(k, _)| k == WATCH_FIELD)
        .map(|(_, v)| v)
        .expect("_watch slot ensured above");
    let mut list = match slot {
        SwiftValue::Array(items) => (**items).clone(),
        _ => Vec::new(),
    };
    list.push(SwiftValue::Struct(Rc::new(StructObj {
        type_name: WATCH_TYPE.into(),
        fields: vec![("value".into(), value), ("action".into(), action)],
    })));
    *slot = SwiftValue::Array(Rc::new(list));
    Ok(SwiftValue::Struct(Rc::new(StructObj {
        type_name: obj.type_name.clone(),
        fields,
    })))
}

/// Build a `_Modifier` record: a struct carrying `name` plus each call argument
/// as a field keyed by its label (positional args use `value`, `value1`, …).
///
/// Public so sibling render-host frameworks (e.g. Charts mark modifiers) can
/// append the same `_Modifier` shape without reimplementing the record layout.
pub fn make_modifier(name: &str, args: Vec<Arg>) -> SwiftValue {
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
///
/// Public so sibling render-host frameworks can share the COW append path used
/// by SwiftUI view modifiers (a mark is the same view-value shape).
pub fn append_modifier(view: SwiftValue, modifier: SwiftValue) -> StdResult {
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
