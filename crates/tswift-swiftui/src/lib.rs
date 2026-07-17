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

pub(crate) mod async_image;
pub mod charts;
pub mod diff;
pub(crate) mod modifiers;
pub(crate) mod navigation;
pub mod session;
pub(crate) mod tree;
pub mod uiir;
pub(crate) mod values;
pub(crate) mod views;

pub use views::collect_children;
pub(crate) use views::{
    button_init, capsule_init, circle_init, divider_init, ellipse_init, foreach_init, form_init,
    grid_init, grid_row_init, group_init, hstack_init, image_init, label_init, lazy_hgrid_init,
    lazy_hstack_init, lazy_vgrid_init, lazy_vstack_init, list_init, picker_init,
    progress_view_init, rectangle_init, rounded_rectangle_init, scrollview_init, section_init,
    secure_field_init, slider_init, spacer_init, stepper_init, tabview_init, text_field_init,
    text_init, toggle_init, vstack_init, zstack_init,
};

pub(crate) use async_image::async_image_init;
pub use async_image::{async_image_url_image, has_async_image_closures, realize_async_image_child};
pub use modifiers::{append_modifier, make_modifier};
pub(crate) use modifiers::{
    gesture_on_ended, handlers_map, long_press_gesture_init, modifier_animation,
    modifier_aspect_ratio, modifier_background, modifier_frame, modifier_multiline_text_alignment,
    modifier_overlay, modifier_padding, modifier_transition, tap_gesture_init, MODIFIER_FNS,
};
pub(crate) use navigation::{navigation_link_init, navigation_stack_init};
pub use navigation::{
    path_append, path_remove_last, pushed_value, read_path_items, realize_pushed_screen,
    NAV_PATH_ITEMS_FIELD,
};
pub(crate) use values::type_error;
pub use values::{child_id, container_value, key_of, token_of, view_type_name, view_value};

use tswift_core::{BuiltinParam, EvalError, Interpreter, StdContext, StdError, SwiftValue};
use tswift_frontend::{Analysis, Node, NodeKind};

/// Field name holding a view's ordered modifier list.
pub const MODIFIERS_FIELD: &str = "_modifiers";
/// Field name holding a container view's ordered child views.
pub const CHILDREN_FIELD: &str = "_children";
/// Field name holding a view's primary action closure (`Button`'s `action`).
/// Retained as the canonical event key `"tap"` inside [`HANDLERS_FIELD`].
pub const ACTION_FIELD: &str = "_action";
/// Field name holding a view's event-handler map (event name → captured
/// closure): the generalization of `Button`'s `_action` (ADR-0013 §3). A
/// `Button` stores its action under `"tap"`; gesture/lifecycle/submit modifiers
/// add `"tap"`/`"longPress"`/`"appear"`/`"disappear"`/`"submit"`. Never
/// serialized (leading `_`); hosts learn which listeners to attach from the
/// marker modifiers instead.
pub const HANDLERS_FIELD: &str = "_handlers";
/// Type name of the [`HANDLERS_FIELD`] record (a bare event→closure map).
pub const HANDLERS_TYPE: &str = "_Handlers";
/// Field name holding a view's runtime-internal `onChange(of:)` watchers: an
/// ordered list of `_Watch { value, action }` records. Compared across renders
/// by the session (ADR-0013 §3); never serialized and never host-visible.
pub const WATCH_FIELD: &str = "_watch";
/// Type name of a [`WATCH_FIELD`] record (`{ value, action }`).
pub const WATCH_TYPE: &str = "_Watch";
/// Type name of an appended modifier record (`_Modifier { name, <args> }`).
pub const MODIFIER_TYPE: &str = "_Modifier";
/// Field name holding a `NavigationLink`'s captured destination (ADR-0013 §1):
/// either a `@ViewBuilder` destination `Closure` (re-evaluated fresh on every
/// render so a pushed screen stays live against `@State`) or an eagerly-built
/// destination view value (the `destination:` view form). Never serialized
/// (leading `_`); the session pushes it onto the enclosing stack's state when
/// the link is tapped.
pub const NAV_DESTINATION_FIELD: &str = "_destination";
/// Field name holding a value-based `NavigationLink`'s captured `value:`
/// payload (ADR-0013 §1, value-based navigation). Present instead of
/// [`NAV_DESTINATION_FIELD`] for `NavigationLink("t", value: v)`. A tap resolves
/// the nearest enclosing `.navigationDestination(for:)` whose type matches the
/// value, then pushes the realized screen. Never serialized (leading `_`).
pub const NAV_VALUE_FIELD: &str = "_navValue";
/// Field name holding a node's `.navigationDestination(for:destination:)`
/// registrations: a map from a metatype's spelled type name (e.g. `"Int"`,
/// `"String"`, a struct name) to the `@ViewBuilder` `(T) -> Content` closure.
/// Walked up from a value link (and across the stack's screens) to match a
/// pushed value to its destination. Never serialized (leading `_`).
pub const NAV_DESTINATIONS_FIELD: &str = "_navDestinations";
/// Type name of the [`NAV_DESTINATIONS_FIELD`] record (a bare type→closure map).
pub const NAV_DESTINATIONS_TYPE: &str = "_NavDestinations";
/// Type name of a session-mode pushed value-link screen: a `{ destination,
/// value }` record realized by invoking the captured destination closure with
/// the value (re-evaluated fresh each render for `@State` liveness).
pub const PUSHED_VALUE_TYPE: &str = "_PushedValue";
/// Field name holding a view's captured presentation modifiers (`.sheet`,
/// `.fullScreenCover`, `.popover`, …): an ordered list of [`PRESENTATION_TYPE`]
/// records, each carrying a gating `Binding`, a `@ViewBuilder` content closure,
/// an optional `onDismiss` closure, and a `style` tag. Never serialized
/// (leading `_`); the session realizes each open presentation into a
/// `Presentation` child node (ADR-0019).
pub const PRESENTATIONS_FIELD: &str = "_presentations";
/// Type name of a [`PRESENTATIONS_FIELD`] record. Fields: `style` (Str),
/// `_binding` (the gating `Binding<Bool>`/`Binding<Item?>`), `_content` (the
/// `@ViewBuilder` closure), and optional `_onDismiss` (a closure).
pub const PRESENTATION_TYPE: &str = "_Presentation";
/// UIIR node kind emitted for an open presentation (ADR-0019). The host renders
/// it on a top layer (sheet/popover/alert/…) keyed by its `style` arg, never in
/// flow, and emits a `dismiss` event that writes `false`/`nil` through the
/// gating binding.
pub const PRESENTATION_NODE_KIND: &str = "Presentation";
/// Field name holding a `ForEach`-generated child's stable identity key. When
/// present, the child's UIIR id is `{parent}.{key}` (not `{parent}.{index}`) so
/// the keyed diff can emit `move` instead of replacing reordered rows.
pub const KEY_FIELD: &str = "_key";
/// Field name holding an `AsyncImage` node's `content` closure — a
/// `@ViewBuilder (Image) -> Content` invoked with the remote-image node on
/// success (ADR-0013 §4). Never serialized (leading `_`).
pub const ASYNC_IMAGE_CONTENT_FIELD: &str = "_asyncContent";
/// Field name holding an `AsyncImage` node's `placeholder` closure — a
/// `@ViewBuilder () -> Placeholder` invoked while the image is loading or if
/// it fails (ADR-0013 §4). Never serialized (leading `_`).
pub const ASYNC_IMAGE_PLACEHOLDER_FIELD: &str = "_asyncPlaceholder";
/// Field name holding an `AsyncImage` node's phase closure — a
/// `@ViewBuilder (AsyncImagePhase) -> Content` invoked with the phase value
/// (ADR-0013 §4). Present for the single-trailing-closure phase form; mutually
/// exclusive with `ASYNC_IMAGE_CONTENT_FIELD`. Never serialized (leading `_`).
pub const ASYNC_IMAGE_PHASE_FIELD: &str = "_asyncPhaseContent";

/// Field holding the `ObservableObject`s a view provides to its subtree via
/// `.environmentObject(_)`. Unlike a visual modifier this never reaches the
/// UIIR — it is consumed by the renderer to inject `@EnvironmentObject` slots.
/// Stored separately from `_modifiers` so a custom `View` (which has no
/// `_modifiers`) can still carry it without looking like a builtin view value.
pub const ENV_FIELD: &str = "_env";

/// Internal field on a `Toggle`: the `Binding<Bool>` its `set` event writes to.
pub const BINDING_FIELD: &str = "_binding";

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
    // A binding projects to itself, which lets `$binding` be passed through to
    // controls in exactly the same way as `$state`.
    var projectedValue: Binding<Value> { self }
    // A constant binding deliberately retains a private box. Controls can
    // write their event payload into it, but no external state is observed.
    static func constant(_ value: Value) -> Binding<Value> {
        Binding(box: _StateBox(value))
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
    // Named system colors remain semantic tokens for host-specific resolution.
    // Explicit RGB values instead cross the UIIR boundary as RGBA components.
    let token: String?
    let red: Double?
    let green: Double?
    let blue: Double?
    let opacity: Double?
    init(token: String) {
        self.token = token
        self.red = nil
        self.green = nil
        self.blue = nil
        self.opacity = nil
    }
    init(token: String, opacity: Double) {
        self.token = token
        self.red = nil
        self.green = nil
        self.blue = nil
        self.opacity = opacity
    }
    init(red: Double, green: Double, blue: Double, opacity: Double = 1) {
        self.token = nil
        self.red = red
        self.green = green
        self.blue = blue
        self.opacity = opacity
    }
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
    static let accentColor = Color(token: "accentColor")

    // `.opacity(_:)` — multiply the color's alpha. A named token keeps its
    // semantic name plus the applied opacity; an explicit RGB color adjusts
    // its alpha component directly.
    func opacity(_ opacity: Double) -> Color {
        if let token = token {
            return Color(token: token, opacity: opacity)
        }
        return Color(red: red ?? 0, green: green ?? 0, blue: blue ?? 0, opacity: opacity)
    }
}
// `Visibility` — the show/hide token for list separators, scroll indicators and
// scroll content background (`.visible`/`.hidden`/`.automatic`). Serialized as a
// leading-dot token like other SwiftUI token namespaces.
struct Visibility {
    let token: String
    static let automatic = Visibility(token: "automatic")
    static let visible = Visibility(token: "visible")
    static let hidden = Visibility(token: "hidden")
}
// `BlendMode` — the compositing blend for `.blendMode(_:)` (Core Graphics blend
// modes). Leading-dot token like the other SwiftUI token namespaces.
struct BlendMode {
    let token: String
    static let normal = BlendMode(token: "normal")
    static let multiply = BlendMode(token: "multiply")
    static let screen = BlendMode(token: "screen")
    static let overlay = BlendMode(token: "overlay")
    static let darken = BlendMode(token: "darken")
    static let lighten = BlendMode(token: "lighten")
    static let colorDodge = BlendMode(token: "colorDodge")
    static let colorBurn = BlendMode(token: "colorBurn")
    static let softLight = BlendMode(token: "softLight")
    static let hardLight = BlendMode(token: "hardLight")
    static let difference = BlendMode(token: "difference")
    static let exclusion = BlendMode(token: "exclusion")
    static let hue = BlendMode(token: "hue")
    static let saturation = BlendMode(token: "saturation")
    static let color = BlendMode(token: "color")
    static let luminosity = BlendMode(token: "luminosity")
    static let plusDarker = BlendMode(token: "plusDarker")
    static let plusLighter = BlendMode(token: "plusLighter")
}
// `ControlSize` — the size class for controls via `.controlSize(_:)`.
struct ControlSize {
    let token: String
    static let mini = ControlSize(token: "mini")
    static let small = ControlSize(token: "small")
    static let large = ControlSize(token: "large")
    static let extraLarge = ControlSize(token: "extraLarge")
}
// `SymbolRenderingMode` — SF Symbol rendering mode via `.symbolRenderingMode(_:)`.
struct SymbolRenderingMode {
    let token: String
    static let monochrome = SymbolRenderingMode(token: "monochrome")
    static let hierarchical = SymbolRenderingMode(token: "hierarchical")
    static let multicolor = SymbolRenderingMode(token: "multicolor")
    static let palette = SymbolRenderingMode(token: "palette")
}
// `RedactionReasons` — the reason set for `.redacted(reason:)`.
struct RedactionReasons {
    let token: String
    static let placeholder = RedactionReasons(token: "placeholder")
    static let privacy = RedactionReasons(token: "privacy")
    static let invalidated = RedactionReasons(token: "invalidated")
}
// `TruncationMode` — where a line is truncated via `.truncationMode(_:)`.
struct TruncationMode {
    let token: String
    static let head = TruncationMode(token: "head")
    static let tail = TruncationMode(token: "tail")
    static let middle = TruncationMode(token: "middle")
}
// `AccessibilityTraits` — the trait set for `.accessibilityAddTraits(_:)` /
// `.accessibilityRemoveTraits(_:)`. Modelled as leading-dot tokens (Swift's
// real type is an OptionSet; a `[.isButton, .isHeader]` array is accepted too).
struct AccessibilityTraits {
    let token: String
    static let isButton = AccessibilityTraits(token: "isButton")
    static let isHeader = AccessibilityTraits(token: "isHeader")
    static let isSelected = AccessibilityTraits(token: "isSelected")
    static let isLink = AccessibilityTraits(token: "isLink")
    static let isSearchField = AccessibilityTraits(token: "isSearchField")
    static let isImage = AccessibilityTraits(token: "isImage")
    static let playsSound = AccessibilityTraits(token: "playsSound")
    static let isKeyboardKey = AccessibilityTraits(token: "isKeyboardKey")
    static let isStaticText = AccessibilityTraits(token: "isStaticText")
    static let isSummaryElement = AccessibilityTraits(token: "isSummaryElement")
    static let updatesFrequently = AccessibilityTraits(token: "updatesFrequently")
    static let startsMediaSession = AccessibilityTraits(token: "startsMediaSession")
    static let allowsDirectInteraction = AccessibilityTraits(token: "allowsDirectInteraction")
    static let causesPageTurn = AccessibilityTraits(token: "causesPageTurn")
    static let isModal = AccessibilityTraits(token: "isModal")
    static let isToggle = AccessibilityTraits(token: "isToggle")
}
// `AccessibilityHeadingLevel` — the heading rank for `.accessibilityHeading(_:)`.
struct AccessibilityHeadingLevel {
    let token: String
    static let unspecified = AccessibilityHeadingLevel(token: "unspecified")
    static let h1 = AccessibilityHeadingLevel(token: "h1")
    static let h2 = AccessibilityHeadingLevel(token: "h2")
    static let h3 = AccessibilityHeadingLevel(token: "h3")
    static let h4 = AccessibilityHeadingLevel(token: "h4")
    static let h5 = AccessibilityHeadingLevel(token: "h5")
    static let h6 = AccessibilityHeadingLevel(token: "h6")
}
// `AccessibilityTextContentType` — the semantic text category for
// `.accessibilityTextContentType(_:)`. Leading-dot token namespace.
struct AccessibilityTextContentType {
    let token: String
    static let plain = AccessibilityTextContentType(token: "plain")
    static let console = AccessibilityTextContentType(token: "console")
    static let fileSystem = AccessibilityTextContentType(token: "fileSystem")
    static let messaging = AccessibilityTextContentType(token: "messaging")
    static let narrative = AccessibilityTextContentType(token: "narrative")
    static let sourceCode = AccessibilityTextContentType(token: "sourceCode")
    static let spreadsheet = AccessibilityTextContentType(token: "spreadsheet")
    static let wordProcessing = AccessibilityTextContentType(token: "wordProcessing")
}
// `ContainerBackgroundPlacement` — the container scope for
// `.containerBackground(_:for:)`. Leading-dot token namespace.
struct ContainerBackgroundPlacement {
    let token: String
    static let navigation = ContainerBackgroundPlacement(token: "navigation")
    static let navigationSplitView = ContainerBackgroundPlacement(token: "navigationSplitView")
    static let tabView = ContainerBackgroundPlacement(token: "tabView")
    static let window = ContainerBackgroundPlacement(token: "window")
}
// `AccessibilityChildBehavior` — how `.accessibilityElement(children:)` folds
// descendant accessibility elements.
struct AccessibilityChildBehavior {
    let token: String
    static let ignore = AccessibilityChildBehavior(token: "ignore")
    static let combine = AccessibilityChildBehavior(token: "combine")
    static let contain = AccessibilityChildBehavior(token: "contain")
}
// `Angle` — a rotation quantity for `.rotationEffect`/`.hueRotation`. Stored in
// degrees (the canonical UIIR unit); `.radians(_:)` converts on the way in.
// Serialized as `{"$":"angle","degrees":…}`.
struct Angle {
    var degrees: Double
    var radians: Double { degrees * 3.141592653589793 / 180.0 }
    init(degrees: Double) { self.degrees = degrees }
    init(radians: Double) { self.degrees = radians * 180.0 / 3.141592653589793 }
    static func degrees(_ degrees: Double) -> Angle { Angle(degrees: degrees) }
    static func radians(_ radians: Double) -> Angle { Angle(radians: radians) }
    static let zero = Angle(degrees: 0)
}
// `EdgeInsets` — per-edge padding amounts for `.listRowInsets`, `.padding`,
// safe-area insets, etc. Serialized as `{"$":"edgeInsets","top":…,…}`.
struct EdgeInsets {
    var top: Double
    var leading: Double
    var bottom: Double
    var trailing: Double
    init(top: Double, leading: Double, bottom: Double, trailing: Double) {
        self.top = top
        self.leading = leading
        self.bottom = bottom
        self.trailing = trailing
    }
    init() {
        self.top = 0
        self.leading = 0
        self.bottom = 0
        self.trailing = 0
    }
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
// `.multilineTextAlignment(.center)` — text alignment token namespace.
struct TextAlignment {
    let token: String
    static let leading = TextAlignment(token: "leading")
    static let center = TextAlignment(token: "center")
    static let trailing = TextAlignment(token: "trailing")
}
// `.textCase(.uppercase)` — text-case token namespace (Swift's `Text.Case`).
struct TextCase {
    let token: String
    static let uppercase = TextCase(token: "uppercase")
    static let lowercase = TextCase(token: "lowercase")
}
// `ScrollView(.horizontal)` — scroll-axis token namespace (Swift's `Axis.Set`).
struct Axis {
    let token: String
    static let horizontal = Axis(token: "horizontal")
    static let vertical = Axis(token: "vertical")
}
// `.frame(alignment:)` / `ZStack(alignment:)` — 2-D alignment token namespace.
// Leading-dot forms (`.center`, `.leading`, `.top`, …) collide with the
// 1-D alignment and edge namespaces below; they resolve by the modifier's
// declared parameter type (the typed-token mechanism, issue #203).
struct Alignment {
    let token: String
    static let center = Alignment(token: "center")
    static let leading = Alignment(token: "leading")
    static let trailing = Alignment(token: "trailing")
    static let top = Alignment(token: "top")
    static let bottom = Alignment(token: "bottom")
    static let topLeading = Alignment(token: "topLeading")
    static let topTrailing = Alignment(token: "topTrailing")
    static let bottomLeading = Alignment(token: "bottomLeading")
    static let bottomTrailing = Alignment(token: "bottomTrailing")
    static let leadingFirstTextBaseline = Alignment(token: "leadingFirstTextBaseline")
    static let centerFirstTextBaseline = Alignment(token: "centerFirstTextBaseline")
    static let trailingFirstTextBaseline = Alignment(token: "trailingFirstTextBaseline")
}
// `VStack(alignment:)` — horizontal-alignment token namespace (1-D).
struct HorizontalAlignment {
    let token: String
    static let leading = HorizontalAlignment(token: "leading")
    static let center = HorizontalAlignment(token: "center")
    static let trailing = HorizontalAlignment(token: "trailing")
}
// `HStack(alignment:)` — vertical-alignment token namespace (1-D).
struct VerticalAlignment {
    let token: String
    static let top = VerticalAlignment(token: "top")
    static let center = VerticalAlignment(token: "center")
    static let bottom = VerticalAlignment(token: "bottom")
    static let firstTextBaseline = VerticalAlignment(token: "firstTextBaseline")
    static let lastTextBaseline = VerticalAlignment(token: "lastTextBaseline")
}
// `.padding(.horizontal, _)` — edge-set token namespace (Swift's `Edge.Set`).
// `.horizontal`/`.vertical` collide with `Axis`; `.leading`/`.trailing` collide
// with the alignment namespaces. Resolved by the modifier's parameter type.
struct Edge {
    let token: String
    static let top = Edge(token: "top")
    static let leading = Edge(token: "leading")
    static let bottom = Edge(token: "bottom")
    static let trailing = Edge(token: "trailing")
    static let horizontal = Edge(token: "horizontal")
    static let vertical = Edge(token: "vertical")
    static let all = Edge(token: "all")
}
// `.aspectRatio(_:contentMode:)` — content-mode token namespace.
struct ContentMode {
    let token: String
    static let fit = ContentMode(token: "fit")
    static let fill = ContentMode(token: "fill")
}
// Typed token namespaces for token-valued View modifiers. Each is resolved
// contextually via `register_struct_method_typed`, so shared leading-dot names
// (`.automatic`, `.fill`, `.circle`) resolve against the declared parameter
// type instead of colliding in the global uniqueness namespace.
struct ColorScheme {
    let token: String
    static let light = ColorScheme(token: "light")
    static let dark = ColorScheme(token: "dark")
}
struct SymbolVariants {
    let token: String
    static let none = SymbolVariants(token: "none")
    static let circle = SymbolVariants(token: "circle")
    static let square = SymbolVariants(token: "square")
    static let rectangle = SymbolVariants(token: "rectangle")
    static let fill = SymbolVariants(token: "fill")
    static let slash = SymbolVariants(token: "slash")
}
struct HoverEffect {
    let token: String
    static let automatic = HoverEffect(token: "automatic")
    static let highlight = HoverEffect(token: "highlight")
    static let lift = HoverEffect(token: "lift")
}
struct MenuOrder {
    let token: String
    static let automatic = MenuOrder(token: "automatic")
    static let priority = MenuOrder(token: "priority")
    static let fixed = MenuOrder(token: "fixed")
}
struct ContentTransition {
    let token: String
    static let identity = ContentTransition(token: "identity")
    static let opacity = ContentTransition(token: "opacity")
    static let interpolate = ContentTransition(token: "interpolate")
    static let numericText = ContentTransition(token: "numericText")
}
struct ScrollBounceBehavior {
    let token: String
    static let automatic = ScrollBounceBehavior(token: "automatic")
    static let always = ScrollBounceBehavior(token: "always")
    static let basedOnSize = ScrollBounceBehavior(token: "basedOnSize")
}
struct ScrollDismissesKeyboardMode {
    let token: String
    static let automatic = ScrollDismissesKeyboardMode(token: "automatic")
    static let immediately = ScrollDismissesKeyboardMode(token: "immediately")
    static let interactively = ScrollDismissesKeyboardMode(token: "interactively")
    static let never = ScrollDismissesKeyboardMode(token: "never")
}
struct ImageScale {
    let token: String
    static let small = ImageScale(token: "small")
    static let medium = ImageScale(token: "medium")
    static let large = ImageScale(token: "large")
}
struct KeyboardType {
    let token: String
    static let `default` = KeyboardType(token: "default")
    static let asciiCapable = KeyboardType(token: "asciiCapable")
    static let numbersAndPunctuation = KeyboardType(token: "numbersAndPunctuation")
    static let URL = KeyboardType(token: "URL")
    static let numberPad = KeyboardType(token: "numberPad")
    static let phonePad = KeyboardType(token: "phonePad")
    static let namePhonePad = KeyboardType(token: "namePhonePad")
    static let emailAddress = KeyboardType(token: "emailAddress")
    static let decimalPad = KeyboardType(token: "decimalPad")
    static let twitter = KeyboardType(token: "twitter")
    static let webSearch = KeyboardType(token: "webSearch")
    static let asciiCapableNumberPad = KeyboardType(token: "asciiCapableNumberPad")
}
struct UITextAutocapitalizationType {
    let token: String
    static let none = UITextAutocapitalizationType(token: "none")
    static let words = UITextAutocapitalizationType(token: "words")
    static let sentences = UITextAutocapitalizationType(token: "sentences")
    static let allCharacters = UITextAutocapitalizationType(token: "allCharacters")
}
struct MenuActionDismissBehavior {
    let token: String
    static let automatic = MenuActionDismissBehavior(token: "automatic")
    static let enabled = MenuActionDismissBehavior(token: "enabled")
    static let disabled = MenuActionDismissBehavior(token: "disabled")
}
struct ButtonRepeatBehavior {
    let token: String
    static let automatic = ButtonRepeatBehavior(token: "automatic")
    static let enabled = ButtonRepeatBehavior(token: "enabled")
    static let disabled = ButtonRepeatBehavior(token: "disabled")
}
struct TextScale {
    let token: String
    static let `default` = TextScale(token: "default")
    static let secondary = TextScale(token: "secondary")
}
struct WritingToolsBehavior {
    let token: String
    static let automatic = WritingToolsBehavior(token: "automatic")
    static let complete = WritingToolsBehavior(token: "complete")
    static let limited = WritingToolsBehavior(token: "limited")
    static let disabled = WritingToolsBehavior(token: "disabled")
}
struct ImageDynamicRange {
    let token: String
    static let standard = ImageDynamicRange(token: "standard")
    static let constrainedHigh = ImageDynamicRange(token: "constrainedHigh")
    static let high = ImageDynamicRange(token: "high")
}
struct TextSelectionAffinity {
    let token: String
    static let automatic = TextSelectionAffinity(token: "automatic")
    static let downstream = TextSelectionAffinity(token: "downstream")
    static let upstream = TextSelectionAffinity(token: "upstream")
}
struct ScrollInputBehavior {
    let token: String
    static let automatic = ScrollInputBehavior(token: "automatic")
    static let enabled = ScrollInputBehavior(token: "enabled")
    static let disabled = ScrollInputBehavior(token: "disabled")
}
struct DialogSeverity {
    let token: String
    static let automatic = DialogSeverity(token: "automatic")
    static let standard = DialogSeverity(token: "standard")
    static let critical = DialogSeverity(token: "critical")
}
struct WindowInteractionBehavior {
    let token: String
    static let automatic = WindowInteractionBehavior(token: "automatic")
    static let enabled = WindowInteractionBehavior(token: "enabled")
    static let disabled = WindowInteractionBehavior(token: "disabled")
}
struct ToolbarPlacement {
    let token: String
    static let automatic = ToolbarPlacement(token: "automatic")
    static let navigationBar = ToolbarPlacement(token: "navigationBar")
    static let tabBar = ToolbarPlacement(token: "tabBar")
    static let bottomBar = ToolbarPlacement(token: "bottomBar")
    static let windowToolbar = ToolbarPlacement(token: "windowToolbar")
}
struct NavigationBarItemTitleDisplayMode {
    let token: String
    static let automatic = NavigationBarItemTitleDisplayMode(token: "automatic")
    static let inline = NavigationBarItemTitleDisplayMode(token: "inline")
    static let large = NavigationBarItemTitleDisplayMode(token: "large")
}
struct ToolbarTitleDisplayMode {
    let token: String
    static let automatic = ToolbarTitleDisplayMode(token: "automatic")
    static let inline = ToolbarTitleDisplayMode(token: "inline")
    static let inlineLarge = ToolbarTitleDisplayMode(token: "inlineLarge")
    static let large = ToolbarTitleDisplayMode(token: "large")
}
struct ToolbarRole {
    let token: String
    static let automatic = ToolbarRole(token: "automatic")
    static let navigationStack = ToolbarRole(token: "navigationStack")
    static let browser = ToolbarRole(token: "browser")
    static let editor = ToolbarRole(token: "editor")
}
struct SpringLoadingBehavior {
    let token: String
    static let automatic = SpringLoadingBehavior(token: "automatic")
    static let enabled = SpringLoadingBehavior(token: "enabled")
    static let disabled = SpringLoadingBehavior(token: "disabled")
}
struct LayoutDirectionBehavior {
    let token: String
    static let fixed = LayoutDirectionBehavior(token: "fixed")
    static let mirrors = LayoutDirectionBehavior(token: "mirrors")
}
struct TextSelectability {
    let token: String
    static let enabled = TextSelectability(token: "enabled")
    static let disabled = TextSelectability(token: "disabled")
}
struct PreviewLayout {
    let token: String
    static let device = PreviewLayout(token: "device")
    static let sizeThatFits = PreviewLayout(token: "sizeThatFits")
}
struct InterfaceOrientation {
    let token: String
    static let portrait = InterfaceOrientation(token: "portrait")
    static let portraitUpsideDown = InterfaceOrientation(token: "portraitUpsideDown")
    static let landscapeLeft = InterfaceOrientation(token: "landscapeLeft")
    static let landscapeRight = InterfaceOrientation(token: "landscapeRight")
}
struct SymbolColorRenderingMode {
    let token: String
    static let flat = SymbolColorRenderingMode(token: "flat")
    static let gradient = SymbolColorRenderingMode(token: "gradient")
}
struct SymbolVariableValueMode {
    let token: String
    static let color = SymbolVariableValueMode(token: "color")
    static let draw = SymbolVariableValueMode(token: "draw")
    static let reveal = SymbolVariableValueMode(token: "reveal")
}
struct UnitPoint {
    let token: String
    static let zero = UnitPoint(token: "zero")
    static let center = UnitPoint(token: "center")
    static let leading = UnitPoint(token: "leading")
    static let trailing = UnitPoint(token: "trailing")
    static let top = UnitPoint(token: "top")
    static let bottom = UnitPoint(token: "bottom")
    static let topLeading = UnitPoint(token: "topLeading")
    static let topTrailing = UnitPoint(token: "topTrailing")
    static let bottomLeading = UnitPoint(token: "bottomLeading")
    static let bottomTrailing = UnitPoint(token: "bottomTrailing")
}
struct PresentationBackgroundInteraction {
    let token: String
    static let automatic = PresentationBackgroundInteraction(token: "automatic")
    static let enabled = PresentationBackgroundInteraction(token: "enabled")
    static let disabled = PresentationBackgroundInteraction(token: "disabled")
}
struct PresentationAdaptation {
    let token: String
    static let automatic = PresentationAdaptation(token: "automatic")
    static let none = PresentationAdaptation(token: "none")
    static let popover = PresentationAdaptation(token: "popover")
    static let sheet = PresentationAdaptation(token: "sheet")
    static let fullScreenCover = PresentationAdaptation(token: "fullScreenCover")
}
struct ScrollTargetBehavior {
    let token: String
    static let viewAligned = ScrollTargetBehavior(token: "viewAligned")
    static let paging = ScrollTargetBehavior(token: "paging")
}
struct MaterialActiveAppearance {
    let token: String
    static let automatic = MaterialActiveAppearance(token: "automatic")
    static let active = MaterialActiveAppearance(token: "active")
    static let inactive = MaterialActiveAppearance(token: "inactive")
}
struct PaletteSelectionEffect {
    let token: String
    static let automatic = PaletteSelectionEffect(token: "automatic")
    static let symbolVariant = PaletteSelectionEffect(token: "symbolVariant")
    static let custom = PaletteSelectionEffect(token: "custom")
}
struct AlternatingRowBackgroundBehavior {
    let token: String
    static let automatic = AlternatingRowBackgroundBehavior(token: "automatic")
    static let enabled = AlternatingRowBackgroundBehavior(token: "enabled")
    static let disabled = AlternatingRowBackgroundBehavior(token: "disabled")
}
struct ButtonSizing {
    let token: String
    static let automatic = ButtonSizing(token: "automatic")
    static let fitted = ButtonSizing(token: "fitted")
    static let flexible = ButtonSizing(token: "flexible")
}
struct AdaptableTabBarPlacement {
    let token: String
    static let automatic = AdaptableTabBarPlacement(token: "automatic")
    static let tabBarOnly = AdaptableTabBarPlacement(token: "tabBarOnly")
    static let sidebarAdaptable = AdaptableTabBarPlacement(token: "sidebarAdaptable")
}
struct TabBarMinimizeBehavior {
    let token: String
    static let automatic = TabBarMinimizeBehavior(token: "automatic")
    static let onScrollDown = TabBarMinimizeBehavior(token: "onScrollDown")
    static let onScrollUp = TabBarMinimizeBehavior(token: "onScrollUp")
    static let never = TabBarMinimizeBehavior(token: "never")
}
struct SearchPresentationToolbarBehavior {
    let token: String
    static let automatic = SearchPresentationToolbarBehavior(token: "automatic")
    static let avoidHidingContent = SearchPresentationToolbarBehavior(token: "avoidHidingContent")
}
struct SearchToolbarBehavior {
    let token: String
    static let automatic = SearchToolbarBehavior(token: "automatic")
    static let minimize = SearchToolbarBehavior(token: "minimize")
}
struct SearchFieldPlacement {
    let token: String
    static let automatic = SearchFieldPlacement(token: "automatic")
    static let toolbar = SearchFieldPlacement(token: "toolbar")
    static let sidebar = SearchFieldPlacement(token: "sidebar")
    static let navigationBarDrawer = SearchFieldPlacement(token: "navigationBarDrawer")
}
struct SearchScopeActivation {
    let token: String
    static let automatic = SearchScopeActivation(token: "automatic")
    static let onSearchPresentation = SearchScopeActivation(token: "onSearchPresentation")
    static let onTextEntry = SearchScopeActivation(token: "onTextEntry")
}
struct HandGestureShortcut {
    let token: String
    static let primaryAction = HandGestureShortcut(token: "primaryAction")
}
struct ScrollEdgeEffectStyle {
    let token: String
    static let automatic = ScrollEdgeEffectStyle(token: "automatic")
    static let soft = ScrollEdgeEffectStyle(token: "soft")
    static let hard = ScrollEdgeEffectStyle(token: "hard")
}
struct PresentationContentInteraction {
    let token: String
    static let automatic = PresentationContentInteraction(token: "automatic")
    static let resizes = PresentationContentInteraction(token: "resizes")
    static let scrolls = PresentationContentInteraction(token: "scrolls")
}
struct PresentationSizing {
    let token: String
    static let automatic = PresentationSizing(token: "automatic")
    static let fitted = PresentationSizing(token: "fitted")
    static let form = PresentationSizing(token: "form")
    static let page = PresentationSizing(token: "page")
}
struct TextInputDictationBehavior {
    let token: String
    static let automatic = TextInputDictationBehavior(token: "automatic")
    static let inactive = TextInputDictationBehavior(token: "inactive")
}
struct Glass {
    let token: String
    static let regular = Glass(token: "regular")
    static let clear = Glass(token: "clear")
    static let identity = Glass(token: "identity")
}
struct NavigationTransition {
    let token: String
    static let automatic = NavigationTransition(token: "automatic")
    static let slide = NavigationTransition(token: "slide")
}
struct WindowStyle {
    let token: String
    static let automatic = WindowStyle(token: "automatic")
    static let plain = WindowStyle(token: "plain")
    static let hinted = WindowStyle(token: "hinted")
    static let volumetric = WindowStyle(token: "volumetric")
}
struct WindowToolbarStyle {
    let token: String
    static let automatic = WindowToolbarStyle(token: "automatic")
    static let expanded = WindowToolbarStyle(token: "expanded")
    static let unified = WindowToolbarStyle(token: "unified")
    static let unifiedCompact = WindowToolbarStyle(token: "unifiedCompact")
}
struct TypesettingLanguage {
    let token: String
    static let automatic = TypesettingLanguage(token: "automatic")
}
struct WindowToolbarFullScreenVisibility {
    let token: String
    static let automatic = WindowToolbarFullScreenVisibility(token: "automatic")
    static let onHover = WindowToolbarFullScreenVisibility(token: "onHover")
}
struct UITextContentType {
    let token: String
    static let name = UITextContentType(token: "name")
    static let givenName = UITextContentType(token: "givenName")
    static let familyName = UITextContentType(token: "familyName")
    static let nickname = UITextContentType(token: "nickname")
    static let jobTitle = UITextContentType(token: "jobTitle")
    static let organizationName = UITextContentType(token: "organizationName")
    static let location = UITextContentType(token: "location")
    static let fullStreetAddress = UITextContentType(token: "fullStreetAddress")
    static let addressCity = UITextContentType(token: "addressCity")
    static let postalCode = UITextContentType(token: "postalCode")
    static let telephoneNumber = UITextContentType(token: "telephoneNumber")
    static let emailAddress = UITextContentType(token: "emailAddress")
    static let URL = UITextContentType(token: "URL")
    static let creditCardNumber = UITextContentType(token: "creditCardNumber")
    static let username = UITextContentType(token: "username")
    static let password = UITextContentType(token: "password")
    static let newPassword = UITextContentType(token: "newPassword")
    static let oneTimeCode = UITextContentType(token: "oneTimeCode")
}
struct DynamicTypeSize {
    let token: String
    static let xSmall = DynamicTypeSize(token: "xSmall")
    static let small = DynamicTypeSize(token: "small")
    static let medium = DynamicTypeSize(token: "medium")
    static let large = DynamicTypeSize(token: "large")
    static let xLarge = DynamicTypeSize(token: "xLarge")
    static let xxLarge = DynamicTypeSize(token: "xxLarge")
    static let xxxLarge = DynamicTypeSize(token: "xxxLarge")
    static let accessibility1 = DynamicTypeSize(token: "accessibility1")
    static let accessibility2 = DynamicTypeSize(token: "accessibility2")
    static let accessibility3 = DynamicTypeSize(token: "accessibility3")
    static let accessibility4 = DynamicTypeSize(token: "accessibility4")
    static let accessibility5 = DynamicTypeSize(token: "accessibility5")
}
// `LazyVGrid(columns: [.flexible(), .fixed(80)])` — a grid track sizer. Declared
// as a Swift type so `.flexible()`/`.fixed(_)`/`.adaptive(minimum:)` resolve and
// carry their parameters; serialized as `{kind,value,spacing?}` (issue #205).
struct GridItem {
    let kind: String
    let value: Double
    let maximum: Double
    let spacing: Double?
    static func flexible(minimum: Double = 10, maximum: Double = Double.infinity, spacing: Double? = nil) -> GridItem {
        GridItem(kind: "flexible", value: minimum, maximum: maximum, spacing: spacing)
    }
    static func fixed(_ size: Double, spacing: Double? = nil) -> GridItem {
        GridItem(kind: "fixed", value: size, maximum: size, spacing: spacing)
    }
    static func adaptive(minimum: Double, maximum: Double = Double.infinity, spacing: Double? = nil) -> GridItem {
        GridItem(kind: "adaptive", value: minimum, maximum: maximum, spacing: spacing)
    }
}
// Control-style tokens for `.buttonStyle`/`.listStyle`/`.pickerStyle`/
// `.textFieldStyle`. SwiftUI uses several distinct style types that share
// leading-dot names (`.plain`, `.automatic`); the runtime resolves leading-dot
// members by uniqueness, so they live in ONE namespace (each name once) and the
// host disambiguates by the *modifier* name (button vs list vs picker).
struct _ControlStyle {
    let token: String
    static let automatic = _ControlStyle(token: "automatic")
    static let plain = _ControlStyle(token: "plain")
    static let bordered = _ControlStyle(token: "bordered")
    static let borderedProminent = _ControlStyle(token: "borderedProminent")
    static let borderless = _ControlStyle(token: "borderless")
    static let grouped = _ControlStyle(token: "grouped")
    static let insetGrouped = _ControlStyle(token: "insetGrouped")
    static let inset = _ControlStyle(token: "inset")
    static let sidebar = _ControlStyle(token: "sidebar")
    static let menu = _ControlStyle(token: "menu")
    static let segmented = _ControlStyle(token: "segmented")
    static let wheel = _ControlStyle(token: "wheel")
    static let inline = _ControlStyle(token: "inline")
    static let roundedBorder = _ControlStyle(token: "roundedBorder")
    // Toggle / menu / gauge / form / tab / index / disclosure / control-group
    // styles. Names stay unique across the shared namespace; the host resolves
    // meaning from the modifier name.
    static let button = _ControlStyle(token: "button")
    static let borderlessButton = _ControlStyle(token: "borderlessButton")
    static let checkbox = _ControlStyle(token: "checkbox")
    static let columns = _ControlStyle(token: "columns")
    static let page = _ControlStyle(token: "page")
    static let card = _ControlStyle(token: "card")
    static let navigationLink = _ControlStyle(token: "navigationLink")
    static let accessoryCircular = _ControlStyle(token: "accessoryCircular")
    static let accessoryLinear = _ControlStyle(token: "accessoryLinear")
    static let linearCapacity = _ControlStyle(token: "linearCapacity")
    static let accessoryCircularCapacity = _ControlStyle(token: "accessoryCircularCapacity")
    static let accessoryLinearCapacity = _ControlStyle(token: "accessoryLinearCapacity")
    static let graphical = _ControlStyle(token: "graphical")
    static let compact = _ControlStyle(token: "compact")
    static let field = _ControlStyle(token: "field")
    static let stepper = _ControlStyle(token: "stepper")
    // Label / progress-view / table / navigation-view / navigation-split-view
    // styles. Names stay unique across the shared namespace (`.linear` is
    // intentionally omitted from progressViewStyle — it collides with
    // `Animation.linear` — so fixtures exercise `.circular` instead).
    static let iconOnly = _ControlStyle(token: "iconOnly")
    static let titleOnly = _ControlStyle(token: "titleOnly")
    static let titleAndIcon = _ControlStyle(token: "titleAndIcon")
    static let circular = _ControlStyle(token: "circular")
    static let stack = _ControlStyle(token: "stack")
    static let balanced = _ControlStyle(token: "balanced")
    static let prominentDetail = _ControlStyle(token: "prominentDetail")
    // Prominence (headerProminence/badgeProminence) + button-border shapes.
    static let increased = _ControlStyle(token: "increased")
    static let standard = _ControlStyle(token: "standard")
    static let decreased = _ControlStyle(token: "decreased")
    static let roundedRectangle = _ControlStyle(token: "roundedRectangle")
    static let capsule = _ControlStyle(token: "capsule")
    static let circle = _ControlStyle(token: "circle")
}
// `SubmitLabel` — the keyboard return-key label for `.submitLabel(_:)`.
struct SubmitLabel {
    let token: String
    static let done = SubmitLabel(token: "done")
    static let go = SubmitLabel(token: "go")
    static let send = SubmitLabel(token: "send")
    static let join = SubmitLabel(token: "join")
    static let route = SubmitLabel(token: "route")
    static let search = SubmitLabel(token: "search")
    static let `return` = SubmitLabel(token: "return")
    static let next = SubmitLabel(token: "next")
    static let `continue` = SubmitLabel(token: "continue")
}
// `TextInputAutocapitalization` — the autocapitalization policy for
// `.textInputAutocapitalization(_:)`.
struct TextInputAutocapitalization {
    let token: String
    static let never = TextInputAutocapitalization(token: "never")
    static let words = TextInputAutocapitalization(token: "words")
    static let sentences = TextInputAutocapitalization(token: "sentences")
    static let characters = TextInputAutocapitalization(token: "characters")
}
// `NavigationPath` — a type-erased list of navigation values driving a
// `NavigationStack(path:)` (ADR-0013 §1). The runtime derives the stack's depth
// from `_items`, matching each item to a `.navigationDestination(for:)`. In real
// SwiftUI the storage is opaque; `_items` is a tswift-internal detail the
// runtime reads.
struct NavigationPath {
    var _items: [Any] = []
    var count: Int { _items.count }
    var isEmpty: Bool { _items.isEmpty }
    init() {}
    mutating func append(_ value: Any) { _items.append(value) }
    mutating func removeLast(_ k: Int = 1) {
        for _ in 0 ..< k { _items.removeLast() }
    }
}
// `AsyncImagePhase` — a lightweight enum-like struct for the single-trailing-
// closure phase form of `AsyncImage` (ADR-0013 §4). Simplification vs real
// SwiftUI: no `Error` associated value on `.failure`; `.image` is absent (the
// phase closure receives the phase value but not the loaded `Image` — use the
// content+placeholder form when the loaded image is needed with modifiers).
// See notes.md for documented simplifications.
struct AsyncImagePhase {
    var phaseCase: String
    var phaseUrl: String
    func checkCase(_ c: String) -> Bool { return phaseCase == c }
    var isEmpty: Bool { return checkCase("empty") }
    var isSuccess: Bool { return checkCase("success") }
    var isFailure: Bool { return checkCase("failure") }
}
// `Animation` — a timing/spring curve value for `.animation`/`withAnimation`
// (SwiftUI's `Animation`). Modeled as a struct carrying a `kind` token plus the
// optional numeric params of each curve family and the chainable modifiers
// (`delay`/`speed`/`repeat`). Serialized as a tagged object
// `{"$":"animation","kind":…,…}` via a dedicated `write_value` branch (only the
// set fields are emitted, in a fixed order). See notes.md for the full schema.
struct Animation {
    var kind: String
    var duration: Double? = nil
    var response: Double? = nil
    var dampingFraction: Double? = nil
    var blendDuration: Double? = nil
    var bounce: Double? = nil
    var extraBounce: Double? = nil
    var delayValue: Double? = nil
    var speedValue: Double? = nil
    var repeatKind: String? = nil
    var repeatCountValue: Int? = nil
    var autoreversesValue: Bool? = nil
    // `timingCurve` cubic Bézier control points (p1 = c0x/c0y, p2 = c1x/c1y).
    var c0x: Double? = nil
    var c0y: Double? = nil
    var c1x: Double? = nil
    var c1y: Double? = nil
    // `interpolatingSpring` physical parameters.
    var mass: Double? = nil
    var stiffness: Double? = nil
    var damping: Double? = nil
    var initialVelocity: Double? = nil

    // `Animation.default` — the standard implicit curve. Reachable now that the
    // lexer escapes reserved words as identifiers (`` `default` ``).
    static let `default` = Animation(kind: "default")

    static let linear = Animation(kind: "linear")
    static func linear(duration: Double) -> Animation {
        Animation(kind: "linear", duration: duration)
    }

    static let easeIn = Animation(kind: "easeIn")
    static func easeIn(duration: Double) -> Animation {
        Animation(kind: "easeIn", duration: duration)
    }

    static let easeOut = Animation(kind: "easeOut")
    static func easeOut(duration: Double) -> Animation {
        Animation(kind: "easeOut", duration: duration)
    }

    static let easeInOut = Animation(kind: "easeInOut")
    static func easeInOut(duration: Double) -> Animation {
        Animation(kind: "easeInOut", duration: duration)
    }

    static let spring = Animation(kind: "spring")
    static func spring(response: Double = 0.5, dampingFraction: Double = 0.825, blendDuration: Double = 0) -> Animation {
        Animation(kind: "spring", response: response, dampingFraction: dampingFraction, blendDuration: blendDuration)
    }
    // `duration:` is required (no default) so the bare `.spring()` stays
    // unambiguously the response/dampingFraction overload above; `bounce`/
    // `blendDuration` default so `.spring(duration: 0.4)` compiles.
    static func spring(duration: Double, bounce: Double = 0.0, blendDuration: Double = 0.0) -> Animation {
        Animation(kind: "spring", duration: duration, bounce: bounce)
    }

    static let bouncy = Animation(kind: "bouncy")
    static func bouncy(duration: Double = 0.5, extraBounce: Double = 0.0) -> Animation {
        Animation(kind: "bouncy", duration: duration, extraBounce: extraBounce)
    }

    static let smooth = Animation(kind: "smooth")
    static func smooth(duration: Double = 0.5, extraBounce: Double = 0.0) -> Animation {
        Animation(kind: "smooth", duration: duration)
    }

    static let snappy = Animation(kind: "snappy")
    static func snappy(duration: Double = 0.5, extraBounce: Double = 0.0) -> Animation {
        Animation(kind: "snappy", duration: duration)
    }

    // `timingCurve(_:_:_:_:duration:)` — a custom cubic Bézier curve defined by
    // two control points, matching SwiftUI's parameter order.
    static func timingCurve(_ c0x: Double, _ c0y: Double, _ c1x: Double, _ c1y: Double, duration: Double = 0.35) -> Animation {
        Animation(kind: "timingCurve", duration: duration, c0x: c0x, c0y: c0y, c1x: c1x, c1y: c1y)
    }

    // `interpolatingSpring(mass:stiffness:damping:initialVelocity:)` — a spring
    // driven by physical constants (additive across concurrent animations).
    static func interpolatingSpring(mass: Double = 1.0, stiffness: Double, damping: Double, initialVelocity: Double = 0.0) -> Animation {
        Animation(kind: "interpolatingSpring", mass: mass, stiffness: stiffness, damping: damping, initialVelocity: initialVelocity)
    }
    // Modern `interpolatingSpring(duration:bounce:initialVelocity:)` form.
    static func interpolatingSpring(duration: Double = 0.5, bounce: Double = 0.0, initialVelocity: Double = 0.0) -> Animation {
        Animation(kind: "interpolatingSpring", duration: duration, bounce: bounce, initialVelocity: initialVelocity)
    }

    // `interactiveSpring(response:dampingFraction:blendDuration:)` — a lower-
    // duration spring tuned for gesture-tracking interactions.
    static func interactiveSpring(response: Double = 0.15, dampingFraction: Double = 0.86, blendDuration: Double = 0.25) -> Animation {
        Animation(kind: "interactiveSpring", response: response, dampingFraction: dampingFraction, blendDuration: blendDuration)
    }

    func delay(_ delay: Double) -> Animation {
        var a = self
        a.delayValue = delay
        return a
    }
    func speed(_ speed: Double) -> Animation {
        var a = self
        a.speedValue = speed
        return a
    }
    func repeatCount(_ count: Int, autoreverses: Bool = true) -> Animation {
        var a = self
        a.repeatKind = "count"
        a.repeatCountValue = count
        a.autoreversesValue = autoreverses
        return a
    }
    func repeatForever(autoreverses: Bool = true) -> Animation {
        var a = self
        a.repeatKind = "forever"
        a.autoreversesValue = autoreverses
        return a
    }
}
// `AnyTransition` — the insert/remove transition for `.transition(_:)`
// (SwiftUI's `AnyTransition`). Modeled as a struct carrying a `transitionType`
// token plus the optional params of each transition and the recursive
// combinators (`combined`/`asymmetric`). The recursive slots are typed `Any`
// (not `AnyTransition`) so a value type may hold its own kind without a
// recursive-size error; the serializer reads the nested `AnyTransition` structs
// regardless. Serialized as a tagged object `{"$":"transition","type":…,…}` via
// a dedicated `write_value` branch. See notes.md for the full schema.
struct AnyTransition {
    var transitionType: String
    var scaleValue: Double? = nil
    var anchor: String? = nil
    var edge: String? = nil
    var offsetX: Double? = nil
    var offsetY: Double? = nil
    var transitions: [Any]? = nil
    var insertion: Any? = nil
    var removal: Any? = nil
    // Animation curve attached via `.animation(_:)` (typed `Any` to avoid a
    // recursive-size dependency on `Animation`; the serializer reads it back).
    var animationValue: Any? = nil

    static let opacity = AnyTransition(transitionType: "opacity")
    static let identity = AnyTransition(transitionType: "identity")
    static let slide = AnyTransition(transitionType: "slide")
    static let scale = AnyTransition(transitionType: "scale")

    static func scale(scale: Double, anchor: Alignment? = nil) -> AnyTransition {
        var t = AnyTransition(transitionType: "scale", scaleValue: scale)
        if let a = anchor { t.anchor = a.token }
        return t
    }
    static func move(edge: Edge) -> AnyTransition {
        AnyTransition(transitionType: "move", edge: edge.token)
    }
    static func offset(x: Double, y: Double = 0) -> AnyTransition {
        AnyTransition(transitionType: "offset", offsetX: x, offsetY: y)
    }
    static func push(from edge: Edge) -> AnyTransition {
        AnyTransition(transitionType: "push", edge: edge.token)
    }

    func combined(with other: AnyTransition) -> AnyTransition {
        AnyTransition(transitionType: "combined", transitions: [self, other])
    }
    static func asymmetric(insertion: AnyTransition, removal: AnyTransition) -> AnyTransition {
        AnyTransition(transitionType: "asymmetric", insertion: insertion, removal: removal)
    }

    // `.animation(_:)` — attach an animation curve to this transition so the
    // insert/remove runs with the given timing. Returns a copy carrying the
    // curve; a `nil` curve clears any attached animation.
    func animation(_ animation: Animation?) -> AnyTransition {
        var t = self
        t.animationValue = animation
        return t
    }
}
// `.symbolEffect(_:)` — SF Symbol content-transition/animation token. Only the
// discrete leading-dot presets are modelled (the `.repeat`/`options:` builders
// are out of scope v1); the host reads the token and animates the symbol.
struct SymbolEffect {
    let token: String
    static let bounce = SymbolEffect(token: "bounce")
    static let pulse = SymbolEffect(token: "pulse")
    static let variableColor = SymbolEffect(token: "variableColor")
    static let scale = SymbolEffect(token: "scale")
    static let appear = SymbolEffect(token: "appear")
    static let disappear = SymbolEffect(token: "disappear")
    static let wiggle = SymbolEffect(token: "wiggle")
    static let rotate = SymbolEffect(token: "rotate")
    static let breathe = SymbolEffect(token: "breathe")
}
// `.sensoryFeedback(_:trigger:)` — a haptic/audio feedback token fired when the
// `trigger:` value changes. The impact/weight builders are out of scope; the
// discrete presets are modelled as tokens.
struct SensoryFeedback {
    let token: String
    static let success = SensoryFeedback(token: "success")
    static let warning = SensoryFeedback(token: "warning")
    static let error = SensoryFeedback(token: "error")
    static let selection = SensoryFeedback(token: "selection")
    static let increase = SensoryFeedback(token: "increase")
    static let decrease = SensoryFeedback(token: "decrease")
    static let start = SensoryFeedback(token: "start")
    static let stop = SensoryFeedback(token: "stop")
    static let alignment = SensoryFeedback(token: "alignment")
    static let levelChange = SensoryFeedback(token: "levelChange")
    static let impact = SensoryFeedback(token: "impact")
}
// `.presentationDetents([...])` — the resting heights a sheet may snap to.
// Only the discrete `.medium`/`.large` presets are modelled (the
// `.fraction(_)`/`.height(_)`/`.custom` builders are out of scope v1).
struct PresentationDetent {
    let token: String
    static let medium = PresentationDetent(token: "medium")
    static let large = PresentationDetent(token: "large")
}
// ── Swift Charts (ADR-0020) ────────────────────────────────────────────────
// A labelled data point mapped onto a chart's x/y/angle scale. `.value(_:_:)`
// is the only factory modelled; the value is stored dynamically (Double / Int /
// String / Date all flow through unchanged) and serialized as
// `{"$":"plottable","label":…,"value":…}`.
struct PlottableValue<Value> {
    let label: String
    let value: Value
    static func value(_ label: String, _ value: Value) -> PlottableValue<Value> {
        PlottableValue(label: label, value: value)
    }
}
// A mark's along-axis size. `.automatic` (scale-driven) plus the explicit
// `.fixed(_)`/`.ratio(_)`/`.inset(_)` builders; serialized as
// `{"$":"markDimension","kind":…,"value":…}` (issue #205 GridItem precedent).
struct MarkDimension {
    let kind: String
    let value: Double
    static let automatic = MarkDimension(kind: "automatic", value: 0)
    static func fixed(_ value: Double) -> MarkDimension { MarkDimension(kind: "fixed", value: value) }
    static func ratio(_ value: Double) -> MarkDimension { MarkDimension(kind: "ratio", value: value) }
    static func inset(_ value: Double) -> MarkDimension { MarkDimension(kind: "inset", value: value) }
}
// How overlapping marks in the same x-position combine. Pure token (the
// `cardinal(tension:)`-style parameterized cases are out of scope v1).
struct MarkStackingMethod {
    let token: String
    static let standard = MarkStackingMethod(token: "standard")
    static let center = MarkStackingMethod(token: "center")
    static let normalized = MarkStackingMethod(token: "normalized")
    static let unstacked = MarkStackingMethod(token: "unstacked")
}
// The curve fit connecting LineMark/AreaMark points. Only the bare presets are
// modelled (`.cardinal(tension:)`/`.catmullRom(alpha:)` builders are v1 gaps).
struct InterpolationMethod {
    let token: String
    static let linear = InterpolationMethod(token: "linear")
    static let monotone = InterpolationMethod(token: "monotone")
    static let cardinal = InterpolationMethod(token: "cardinal")
    static let catmullRom = InterpolationMethod(token: "catmullRom")
    static let stepStart = InterpolationMethod(token: "stepStart")
    static let stepCenter = InterpolationMethod(token: "stepCenter")
    static let stepEnd = InterpolationMethod(token: "stepEnd")
}
"#;

/// Register every currently-supported SwiftUI view constructor and modifier
/// into `interp`.
pub fn install(interp: &mut Interpreter<'_>) {
    interp.module("SwiftUI", |interp| {
        install_inner(interp);
    });
}

fn install_inner(interp: &mut Interpreter<'_>) {
    interp.register_free_fn("Text", text_init);
    // Stacks carry a typed `alignment:` so its leading-dot token resolves
    // against the right 1-D/2-D namespace (`VStack` → `HorizontalAlignment`,
    // `HStack` → `VerticalAlignment`, `ZStack` → `Alignment`) instead of
    // colliding with `TextAlignment`/`Edge` (issue #203). `stack_init`
    // serializes the resolved token and the hosts apply it on the cross axis
    // (issue #189).
    interp.register_free_fn_typed(
        "VStack",
        vstack_init,
        vec![
            BuiltinParam::labeled("alignment", "HorizontalAlignment"),
            BuiltinParam::labeled("spacing", "CGFloat"),
        ],
    );
    interp.register_free_fn_typed(
        "HStack",
        hstack_init,
        vec![
            BuiltinParam::labeled("alignment", "VerticalAlignment"),
            BuiltinParam::labeled("spacing", "CGFloat"),
        ],
    );
    interp.register_free_fn_typed(
        "ZStack",
        zstack_init,
        vec![BuiltinParam::labeled("alignment", "Alignment")],
    );
    interp.register_free_fn("ForEach", foreach_init);
    interp.register_free_fn("List", list_init);
    interp.register_free_fn("Section", section_init);
    interp.register_free_fn("Label", label_init);
    interp.register_free_fn("Image", image_init);
    interp.register_free_fn("AsyncImage", async_image_init);
    interp.register_free_fn("ProgressView", progress_view_init);
    interp.register_free_fn("Group", group_init);
    interp.register_free_fn("Divider", divider_init);
    // `ScrollView(_ axes: Axis.Set)` — typed so the leading-dot axis
    // (`.horizontal`/`.vertical`) resolves against `Axis` rather than colliding
    // with the new `Edge` namespace (issue #203).
    interp.register_free_fn_typed(
        "ScrollView",
        scrollview_init,
        vec![BuiltinParam::positional("Axis")],
    );
    // Lazy stacks share the stacks' typed `alignment:` so their leading-dot
    // tokens resolve against the right 1-D namespace (issue #189/#203).
    interp.register_free_fn_typed(
        "LazyVStack",
        lazy_vstack_init,
        vec![
            BuiltinParam::labeled("alignment", "HorizontalAlignment"),
            BuiltinParam::labeled("spacing", "CGFloat"),
        ],
    );
    interp.register_free_fn_typed(
        "LazyHStack",
        lazy_hstack_init,
        vec![
            BuiltinParam::labeled("alignment", "VerticalAlignment"),
            BuiltinParam::labeled("spacing", "CGFloat"),
        ],
    );
    interp.register_free_fn("Grid", grid_init);
    interp.register_free_fn("GridRow", grid_row_init);
    // Lazy grids: `columns:`/`rows:` is `[GridItem]` so leading-dot sizers
    // (`.flexible()`/`.fixed(_)`/`.adaptive(minimum:)`) resolve against
    // `GridItem` (issue #205).
    interp.register_free_fn_typed(
        "LazyVGrid",
        lazy_vgrid_init,
        vec![
            BuiltinParam::labeled("columns", "[GridItem]"),
            BuiltinParam::labeled("alignment", "HorizontalAlignment"),
            BuiltinParam::labeled("spacing", "CGFloat"),
        ],
    );
    interp.register_free_fn_typed(
        "LazyHGrid",
        lazy_hgrid_init,
        vec![
            BuiltinParam::labeled("rows", "[GridItem]"),
            BuiltinParam::labeled("alignment", "VerticalAlignment"),
            BuiltinParam::labeled("spacing", "CGFloat"),
        ],
    );
    interp.register_free_fn("Form", form_init);
    interp.register_free_fn("Spacer", spacer_init);
    interp.register_free_fn("Button", button_init);
    interp.register_free_fn("Toggle", toggle_init);
    interp.register_free_fn("TextField", text_field_init);
    interp.register_free_fn("SecureField", secure_field_init);
    interp.register_free_fn("Slider", slider_init);
    interp.register_free_fn("Stepper", stepper_init);
    interp.register_free_fn("Picker", picker_init);
    interp.register_free_fn("TabView", tabview_init);
    interp.register_free_fn("NavigationStack", navigation_stack_init);
    interp.register_free_fn("NavigationLink", navigation_link_init);
    interp.register_free_fn("Circle", circle_init);
    interp.register_free_fn("Rectangle", rectangle_init);
    interp.register_free_fn("RoundedRectangle", rounded_rectangle_init);
    interp.register_free_fn("Capsule", capsule_init);
    interp.register_free_fn("Ellipse", ellipse_init);
    // Gesture value types — not Views, but their `.onEnded` method needs to be
    // registered so the interpreter can chain it on the gesture struct before
    // passing the result to `.gesture(_:)`.
    interp.register_free_fn("TapGesture", tap_gesture_init);
    interp.register_free_fn("LongPressGesture", long_press_gesture_init);
    interp.register_struct_method("onEnded", gesture_on_ended);

    for (name, func) in MODIFIER_FNS {
        interp.register_struct_method(name, *func);
    }

    // Typed modifier signatures (issue #203). Re-registering with a declared
    // parameter type lets a leading-dot member argument resolve against that
    // type instead of failing on cross-namespace collisions. `frame`'s length
    // params are `CGFloat` so `.infinity` resolves; `alignment` is `Alignment`;
    // directional `padding` takes an `Edge.Set`; `multilineTextAlignment` keeps
    // resolving its `.center`/`.leading`/`.trailing` against `TextAlignment`
    // even though those names now also live in the alignment namespaces.
    interp.register_struct_method_typed(
        "frame",
        modifier_frame,
        vec![
            BuiltinParam::labeled("width", "CGFloat"),
            BuiltinParam::labeled("height", "CGFloat"),
            BuiltinParam::labeled("minWidth", "CGFloat"),
            BuiltinParam::labeled("maxWidth", "CGFloat"),
            BuiltinParam::labeled("minHeight", "CGFloat"),
            BuiltinParam::labeled("maxHeight", "CGFloat"),
            BuiltinParam::labeled("idealWidth", "CGFloat"),
            BuiltinParam::labeled("idealHeight", "CGFloat"),
            BuiltinParam::labeled("alignment", "Alignment"),
        ],
    );
    interp.register_struct_method_typed(
        "padding",
        modifier_padding,
        vec![
            BuiltinParam::positional("Edge.Set"),
            BuiltinParam::positional("CGFloat"),
        ],
    );
    interp.register_struct_method_typed(
        "multilineTextAlignment",
        modifier_multiline_text_alignment,
        vec![BuiltinParam::positional("TextAlignment")],
    );
    // Compositing modifiers: a positional content view + a labeled `alignment:`
    // (`Alignment`) so `.overlay(_, alignment: .topTrailing)` resolves (#204).
    interp.register_struct_method_typed(
        "background",
        modifier_background,
        vec![
            BuiltinParam::positional("View"),
            BuiltinParam::labeled("alignment", "Alignment"),
        ],
    );
    interp.register_struct_method_typed(
        "overlay",
        modifier_overlay,
        vec![
            BuiltinParam::positional("View"),
            BuiltinParam::labeled("alignment", "Alignment"),
        ],
    );
    // Tier 2 — `aspectRatio(_:contentMode:)` typed so `.fit`/`.fill` resolve
    // against `ContentMode` (issue #203).
    interp.register_struct_method_typed(
        "aspectRatio",
        modifier_aspect_ratio,
        vec![
            BuiltinParam::positional("CGFloat"),
            BuiltinParam::labeled("contentMode", "ContentMode"),
        ],
    );
    // `.animation(_:value:)` — the positional curve is typed `Animation` so a
    // leading-dot factory (`.easeInOut(…)`, `.linear`) resolves against it. The
    // `value:` operand is any Equatable and carries its own type, so it needs no
    // contextual hint (hence no declared parameter for it).
    interp.register_struct_method_typed(
        "animation",
        modifier_animation,
        vec![BuiltinParam::positional("Animation")],
    );
    // `.transition(_:)` — the positional arg is typed `AnyTransition` so a
    // leading-dot factory/static (`.opacity`, `.move(edge:)`, `.asymmetric(…)`)
    // resolves against it.
    interp.register_struct_method_typed(
        "transition",
        modifier_transition,
        vec![BuiltinParam::positional("AnyTransition")],
    );
    // Accessibility trait/heading/element tokens: typed so their leading-dot
    // members resolve against the token namespace even when a name is shared.
    interp.register_struct_method_typed(
        "accessibilityAddTraits",
        modifiers::modifier_accessibility_add_traits,
        vec![BuiltinParam::positional("AccessibilityTraits")],
    );
    interp.register_struct_method_typed(
        "accessibilityRemoveTraits",
        modifiers::modifier_accessibility_remove_traits,
        vec![BuiltinParam::positional("AccessibilityTraits")],
    );
    interp.register_struct_method_typed(
        "accessibilityHeading",
        modifiers::modifier_accessibility_heading,
        vec![BuiltinParam::positional("AccessibilityHeadingLevel")],
    );
    interp.register_struct_method_typed(
        "accessibilityElement",
        modifiers::modifier_accessibility_element,
        vec![BuiltinParam::labeled(
            "children",
            "AccessibilityChildBehavior",
        )],
    );
    interp.register_struct_method_typed(
        "accessibilityActivationPoint",
        modifiers::modifier_accessibility_activation_point,
        vec![BuiltinParam::positional("UnitPoint")],
    );
    interp.register_struct_method_typed(
        "accessibilityTextContentType",
        modifiers::modifier_accessibility_text_content_type,
        vec![BuiltinParam::positional("AccessibilityTextContentType")],
    );
    interp.register_struct_method_typed(
        "containerBackground",
        modifiers::modifier_container_background,
        vec![
            BuiltinParam::positional("View"),
            BuiltinParam::labeled("for", "ContainerBackgroundPlacement"),
            BuiltinParam::labeled("alignment", "Alignment"),
        ],
    );
    // Token-valued modifiers whose leading-dot arg resolves against a dedicated
    // parameter type. Contextual typing is what lets shared tokens like
    // `.automatic`/`.fill`/`.circle`/`.small`/`.light`/`.medium` resolve per
    // modifier instead of by global uniqueness. Any modifier whose token names
    // overlap another type's must be typed, so `controlSize`/`fontWeight`/
    // `buttonBorderShape` join the newer namespaces below.
    interp.register_struct_method_typed(
        "controlSize",
        modifiers::modifier_control_size,
        vec![BuiltinParam::positional("ControlSize")],
    );
    interp.register_struct_method_typed(
        "fontWeight",
        modifiers::modifier_font_weight,
        vec![BuiltinParam::positional("FontWeight")],
    );
    interp.register_struct_method_typed(
        "buttonBorderShape",
        modifiers::modifier_button_border_shape,
        vec![BuiltinParam::positional("_ControlStyle")],
    );
    interp.register_struct_method_typed(
        "colorScheme",
        modifiers::modifier_color_scheme,
        vec![BuiltinParam::positional("ColorScheme")],
    );
    interp.register_struct_method_typed(
        "preferredColorScheme",
        modifiers::modifier_preferred_color_scheme,
        vec![BuiltinParam::positional("ColorScheme")],
    );
    interp.register_struct_method_typed(
        "symbolVariant",
        modifiers::modifier_symbol_variant,
        vec![BuiltinParam::positional("SymbolVariants")],
    );
    interp.register_struct_method_typed(
        "hoverEffect",
        modifiers::modifier_hover_effect,
        vec![BuiltinParam::positional("HoverEffect")],
    );
    interp.register_struct_method_typed(
        "menuOrder",
        modifiers::modifier_menu_order,
        vec![BuiltinParam::positional("MenuOrder")],
    );
    interp.register_struct_method_typed(
        "contentTransition",
        modifiers::modifier_content_transition,
        vec![BuiltinParam::positional("ContentTransition")],
    );
    interp.register_struct_method_typed(
        "scrollBounceBehavior",
        modifiers::modifier_scroll_bounce_behavior,
        vec![BuiltinParam::positional("ScrollBounceBehavior")],
    );
    interp.register_struct_method_typed(
        "scrollDismissesKeyboard",
        modifiers::modifier_scroll_dismisses_keyboard,
        vec![BuiltinParam::positional("ScrollDismissesKeyboardMode")],
    );
    interp.register_struct_method_typed(
        "dynamicTypeSize",
        modifiers::modifier_dynamic_type_size,
        vec![BuiltinParam::positional("DynamicTypeSize")],
    );
    interp.register_struct_method_typed(
        "imageScale",
        modifiers::modifier_image_scale,
        vec![BuiltinParam::positional("ImageScale")],
    );
    interp.register_struct_method_typed(
        "keyboardType",
        modifiers::modifier_keyboard_type,
        vec![BuiltinParam::positional("KeyboardType")],
    );
    interp.register_struct_method_typed(
        "autocapitalization",
        modifiers::modifier_autocapitalization,
        vec![BuiltinParam::positional("UITextAutocapitalizationType")],
    );
    interp.register_struct_method_typed(
        "menuActionDismissBehavior",
        modifiers::modifier_menu_action_dismiss_behavior,
        vec![BuiltinParam::positional("MenuActionDismissBehavior")],
    );
    interp.register_struct_method_typed(
        "buttonRepeatBehavior",
        modifiers::modifier_button_repeat_behavior,
        vec![BuiltinParam::positional("ButtonRepeatBehavior")],
    );
    interp.register_struct_method_typed(
        "textScale",
        modifiers::modifier_text_scale,
        vec![BuiltinParam::positional("TextScale")],
    );
    interp.register_struct_method_typed(
        "writingToolsBehavior",
        modifiers::modifier_writing_tools_behavior,
        vec![BuiltinParam::positional("WritingToolsBehavior")],
    );
    interp.register_struct_method_typed(
        "allowedDynamicRange",
        modifiers::modifier_allowed_dynamic_range,
        vec![BuiltinParam::positional("ImageDynamicRange")],
    );
    interp.register_struct_method_typed(
        "labelsVisibility",
        modifiers::modifier_labels_visibility,
        vec![BuiltinParam::positional("Visibility")],
    );
    // Convert the color and prominence/autocap modifiers to the typed seam too:
    // the new namespaces reuse `.secondary` (Color), `.words`/`.sentences`
    // (TextInputAutocapitalization) and `.standard` (_ControlStyle), so these
    // must resolve contextually rather than by (now-broken) uniqueness.
    interp.register_struct_method_typed(
        "foregroundColor",
        modifiers::modifier_foreground_color,
        vec![BuiltinParam::positional("Color")],
    );
    interp.register_struct_method_typed(
        "accentColor",
        modifiers::modifier_accent_color,
        vec![BuiltinParam::positional("Color")],
    );
    interp.register_struct_method_typed(
        "tint",
        modifiers::modifier_tint,
        vec![BuiltinParam::positional("Color")],
    );
    interp.register_struct_method_typed(
        "foregroundStyle",
        modifiers::modifier_foreground_style,
        vec![BuiltinParam::positional("Color")],
    );
    interp.register_struct_method_typed(
        "textContentType",
        modifiers::modifier_text_content_type,
        vec![BuiltinParam::positional("UITextContentType")],
    );
    interp.register_struct_method_typed(
        "textSelectionAffinity",
        modifiers::modifier_text_selection_affinity,
        vec![BuiltinParam::positional("TextSelectionAffinity")],
    );
    interp.register_struct_method_typed(
        "scrollInputBehavior",
        modifiers::modifier_scroll_input_behavior,
        vec![BuiltinParam::positional("ScrollInputBehavior")],
    );
    interp.register_struct_method_typed(
        "dialogSeverity",
        modifiers::modifier_dialog_severity,
        vec![BuiltinParam::positional("DialogSeverity")],
    );
    interp.register_struct_method_typed(
        "defaultHoverEffect",
        modifiers::modifier_default_hover_effect,
        vec![BuiltinParam::positional("HoverEffect")],
    );
    interp.register_struct_method_typed(
        "presentationDragIndicator",
        modifiers::modifier_presentation_drag_indicator,
        vec![BuiltinParam::positional("Visibility")],
    );
    // `listRowHoverEffect(_:)` / `sliderThumbVisibility(_:)` reuse the existing
    // HoverEffect / Visibility token namespaces so their leading-dot args
    // resolve contextually.
    interp.register_struct_method_typed(
        "listRowHoverEffect",
        modifiers::modifier_list_row_hover_effect,
        vec![BuiltinParam::positional("HoverEffect")],
    );
    interp.register_struct_method_typed(
        "sliderThumbVisibility",
        modifiers::modifier_slider_thumb_visibility,
        vec![BuiltinParam::positional("Visibility")],
    );
    // Edge.Set / Axis.Set / Visibility token modifiers reusing existing
    // namespaces, plus the four window-interaction behaviors typed against the
    // new WindowInteractionBehavior namespace.
    interp.register_struct_method_typed(
        "scenePadding",
        modifiers::modifier_scene_padding,
        vec![BuiltinParam::positional("Edge.Set")],
    );
    interp.register_struct_method_typed(
        "containerRelativeFrame",
        modifiers::modifier_container_relative_frame,
        vec![BuiltinParam::positional("Axis.Set")],
    );
    interp.register_struct_method_typed(
        "defersSystemGestures",
        modifiers::modifier_defers_system_gestures,
        vec![BuiltinParam::labeled("edges", "Edge.Set")],
    );
    interp.register_struct_method_typed(
        "pointerVisibility",
        modifiers::modifier_pointer_visibility,
        vec![BuiltinParam::positional("Visibility")],
    );
    interp.register_struct_method_typed(
        "windowResizeBehavior",
        modifiers::modifier_window_resize_behavior,
        vec![BuiltinParam::positional("WindowInteractionBehavior")],
    );
    interp.register_struct_method_typed(
        "windowMinimizeBehavior",
        modifiers::modifier_window_minimize_behavior,
        vec![BuiltinParam::positional("WindowInteractionBehavior")],
    );
    interp.register_struct_method_typed(
        "windowDismissBehavior",
        modifiers::modifier_window_dismiss_behavior,
        vec![BuiltinParam::positional("WindowInteractionBehavior")],
    );
    interp.register_struct_method_typed(
        "windowFullScreenBehavior",
        modifiers::modifier_window_full_screen_behavior,
        vec![BuiltinParam::positional("WindowInteractionBehavior")],
    );
    // Toolbar bar-targeted modifiers: a leading token (Visibility / ColorScheme)
    // plus a `for:` ToolbarPlacement bar selector, both typed so their
    // leading-dot args resolve contextually.
    interp.register_struct_method_typed(
        "toolbarBackground",
        modifiers::modifier_toolbar_background,
        vec![
            BuiltinParam::positional("Visibility"),
            BuiltinParam::labeled("for", "ToolbarPlacement"),
        ],
    );
    interp.register_struct_method_typed(
        "toolbarBackgroundVisibility",
        modifiers::modifier_toolbar_background_visibility,
        vec![
            BuiltinParam::positional("Visibility"),
            BuiltinParam::labeled("for", "ToolbarPlacement"),
        ],
    );
    interp.register_struct_method_typed(
        "toolbarColorScheme",
        modifiers::modifier_toolbar_color_scheme,
        vec![
            BuiltinParam::positional("ColorScheme"),
            BuiltinParam::labeled("for", "ToolbarPlacement"),
        ],
    );
    interp.register_struct_method_typed(
        "toolbarVisibility",
        modifiers::modifier_toolbar_visibility,
        vec![
            BuiltinParam::positional("Visibility"),
            BuiltinParam::labeled("for", "ToolbarPlacement"),
        ],
    );
    interp.register_struct_method_typed(
        "textInputAutocapitalization",
        modifiers::modifier_text_input_autocapitalization,
        vec![BuiltinParam::positional("TextInputAutocapitalization")],
    );
    // Token modifiers introducing dedicated namespaces (typed so their
    // leading-dot member arg resolves against the declared parameter type).
    interp.register_struct_method_typed(
        "navigationBarTitleDisplayMode",
        modifiers::modifier_navigation_bar_title_display_mode,
        vec![BuiltinParam::positional(
            "NavigationBarItemTitleDisplayMode",
        )],
    );
    interp.register_struct_method_typed(
        "toolbarTitleDisplayMode",
        modifiers::modifier_toolbar_title_display_mode,
        vec![BuiltinParam::positional("ToolbarTitleDisplayMode")],
    );
    interp.register_struct_method_typed(
        "toolbarRole",
        modifiers::modifier_toolbar_role,
        vec![BuiltinParam::positional("ToolbarRole")],
    );
    interp.register_struct_method_typed(
        "springLoadingBehavior",
        modifiers::modifier_spring_loading_behavior,
        vec![BuiltinParam::positional("SpringLoadingBehavior")],
    );
    interp.register_struct_method_typed(
        "layoutDirectionBehavior",
        modifiers::modifier_layout_direction_behavior,
        vec![BuiltinParam::positional("LayoutDirectionBehavior")],
    );
    interp.register_struct_method_typed(
        "textSelection",
        modifiers::modifier_text_selection,
        vec![BuiltinParam::positional("TextSelectability")],
    );
    interp.register_struct_method_typed(
        "previewLayout",
        modifiers::modifier_preview_layout,
        vec![BuiltinParam::positional("PreviewLayout")],
    );
    interp.register_struct_method_typed(
        "previewInterfaceOrientation",
        modifiers::modifier_preview_interface_orientation,
        vec![BuiltinParam::positional("InterfaceOrientation")],
    );
    interp.register_struct_method_typed(
        "symbolColorRenderingMode",
        modifiers::modifier_symbol_color_rendering_mode,
        vec![BuiltinParam::positional("SymbolColorRenderingMode")],
    );
    interp.register_struct_method_typed(
        "symbolVariableValueMode",
        modifiers::modifier_symbol_variable_value_mode,
        vec![BuiltinParam::positional("SymbolVariableValueMode")],
    );
    interp.register_struct_method_typed(
        "edgesIgnoringSafeArea",
        modifiers::modifier_edges_ignoring_safe_area,
        vec![BuiltinParam::positional("Edge.Set")],
    );
    // `safeAreaInset(edge:)` / `safeAreaBar(edge:)` — the `edge:` token
    // (`.top`/`.bottom`/`.leading`/`.trailing`) resolves against `Edge`.
    interp.register_struct_method_typed(
        "safeAreaInset",
        modifiers::modifier_safe_area_inset,
        vec![BuiltinParam::labeled("edge", "Edge")],
    );
    interp.register_struct_method_typed(
        "safeAreaBar",
        modifiers::modifier_safe_area_bar,
        vec![BuiltinParam::labeled("edge", "Edge")],
    );
    // `listSectionMargins(_ edges: Edge.Set = .all, _ length: CGFloat?)` — the
    // leading `.horizontal`/`.all` token resolves against `Edge.Set`.
    interp.register_struct_method_typed(
        "listSectionMargins",
        modifiers::modifier_list_section_margins,
        vec![
            BuiltinParam::positional("Edge.Set"),
            BuiltinParam::positional("CGFloat"),
        ],
    );
    // `glassEffect(_ glass: Glass = .regular, in shape: some Shape)` — the
    // leading `.regular`/`.clear` token resolves against `Glass`; `in:` is a
    // nested shape view recorded on the node.
    interp.register_struct_method_typed(
        "glassEffect",
        modifiers::modifier_glass_effect,
        vec![
            BuiltinParam::positional("Glass"),
            BuiltinParam::labeled("in", "View"),
        ],
    );
    // `containerCornerOffset(_ edges: Edge.Set, sizeToFit: Bool = false)` — the
    // leading `.horizontal`/`.all` token resolves against `Edge.Set`.
    interp.register_struct_method_typed(
        "containerCornerOffset",
        modifiers::modifier_container_corner_offset,
        vec![
            BuiltinParam::positional("Edge.Set"),
            BuiltinParam::labeled("sizeToFit", "Bool"),
        ],
    );
    // `navigationTransition(_ style: some NavigationTransition)` — the leading
    // `.automatic`/`.slide` token resolves against `NavigationTransition`.
    interp.register_struct_method_typed(
        "navigationTransition",
        modifiers::modifier_navigation_transition,
        vec![BuiltinParam::positional("NavigationTransition")],
    );
    // Grid/scroll/presentation/material/palette token modifiers. Grid layout
    // reuses UnitPoint / HorizontalAlignment / Axis.Set;
    // writingToolsAffordanceVisibility reuses Visibility; the rest introduce
    // dedicated namespaces.
    interp.register_struct_method_typed(
        "defaultScrollAnchor",
        modifiers::modifier_default_scroll_anchor,
        vec![BuiltinParam::positional("UnitPoint")],
    );
    interp.register_struct_method_typed(
        "gridCellAnchor",
        modifiers::modifier_grid_cell_anchor,
        vec![BuiltinParam::positional("UnitPoint")],
    );
    interp.register_struct_method_typed(
        "gridColumnAlignment",
        modifiers::modifier_grid_column_alignment,
        vec![BuiltinParam::positional("HorizontalAlignment")],
    );
    interp.register_struct_method_typed(
        "gridCellUnsizedAxes",
        modifiers::modifier_grid_cell_unsized_axes,
        vec![BuiltinParam::positional("Axis.Set")],
    );
    interp.register_struct_method_typed(
        "writingToolsAffordanceVisibility",
        modifiers::modifier_writing_tools_affordance_visibility,
        vec![BuiltinParam::positional("Visibility")],
    );
    interp.register_struct_method_typed(
        "presentationBackgroundInteraction",
        modifiers::modifier_presentation_background_interaction,
        vec![BuiltinParam::positional(
            "PresentationBackgroundInteraction",
        )],
    );
    interp.register_struct_method_typed(
        "presentationCompactAdaptation",
        modifiers::modifier_presentation_compact_adaptation,
        vec![BuiltinParam::positional("PresentationAdaptation")],
    );
    interp.register_struct_method_typed(
        "scrollTargetBehavior",
        modifiers::modifier_scroll_target_behavior,
        vec![BuiltinParam::positional("ScrollTargetBehavior")],
    );
    interp.register_struct_method_typed(
        "materialActiveAppearance",
        modifiers::modifier_material_active_appearance,
        vec![BuiltinParam::positional("MaterialActiveAppearance")],
    );
    interp.register_struct_method_typed(
        "paletteSelectionEffect",
        modifiers::modifier_palette_selection_effect,
        vec![BuiltinParam::positional("PaletteSelectionEffect")],
    );
    // More dedicated-namespace token modifiers. scrollEdgeEffectStyle carries a
    // leading style token + a `for:` Edge.Set; toolbarForegroundStyle a Color +
    // a `for:` ToolbarPlacement. (horizontalRadioGroupLayout /
    // backgroundExtensionEffect are no-arg markers — no typed registration.)
    interp.register_struct_method_typed(
        "alternatingRowBackgrounds",
        modifiers::modifier_alternating_row_backgrounds,
        vec![BuiltinParam::positional("AlternatingRowBackgroundBehavior")],
    );
    interp.register_struct_method_typed(
        "buttonSizing",
        modifiers::modifier_button_sizing,
        vec![BuiltinParam::positional("ButtonSizing")],
    );
    interp.register_struct_method_typed(
        "defaultAdaptableTabBarPlacement",
        modifiers::modifier_default_adaptable_tab_bar_placement,
        vec![BuiltinParam::positional("AdaptableTabBarPlacement")],
    );
    interp.register_struct_method_typed(
        "tabBarMinimizeBehavior",
        modifiers::modifier_tab_bar_minimize_behavior,
        vec![BuiltinParam::positional("TabBarMinimizeBehavior")],
    );
    interp.register_struct_method_typed(
        "searchPresentationToolbarBehavior",
        modifiers::modifier_search_presentation_toolbar_behavior,
        vec![BuiltinParam::positional(
            "SearchPresentationToolbarBehavior",
        )],
    );
    interp.register_struct_method_typed(
        "searchToolbarBehavior",
        modifiers::modifier_search_toolbar_behavior,
        vec![BuiltinParam::positional("SearchToolbarBehavior")],
    );
    interp.register_struct_method_typed(
        "searchable",
        modifiers::modifier_searchable,
        vec![
            BuiltinParam::labeled("text", "Binding"),
            BuiltinParam::labeled("placement", "SearchFieldPlacement"),
            BuiltinParam::labeled("prompt", "String"),
        ],
    );
    interp.register_struct_method_typed(
        "searchScopes",
        modifiers::modifier_search_scopes,
        vec![
            BuiltinParam::positional("Binding"),
            BuiltinParam::labeled("activation", "SearchScopeActivation"),
            BuiltinParam::labeled("scopes", "AnyView"),
        ],
    );
    interp.register_struct_method_typed(
        "handGestureShortcut",
        modifiers::modifier_hand_gesture_shortcut,
        vec![BuiltinParam::positional("HandGestureShortcut")],
    );
    interp.register_struct_method_typed(
        "scrollEdgeEffectStyle",
        modifiers::modifier_scroll_edge_effect_style,
        vec![
            BuiltinParam::positional("ScrollEdgeEffectStyle"),
            BuiltinParam::labeled("for", "Edge.Set"),
        ],
    );
    interp.register_struct_method_typed(
        "toolbarForegroundStyle",
        modifiers::modifier_toolbar_foreground_style,
        vec![
            BuiltinParam::positional("Color"),
            BuiltinParam::labeled("for", "ToolbarPlacement"),
        ],
    );
    interp.register_struct_method_typed(
        "headerProminence",
        modifiers::modifier_header_prominence,
        vec![BuiltinParam::positional("_ControlStyle")],
    );
    interp.register_struct_method_typed(
        "badgeProminence",
        modifiers::modifier_badge_prominence,
        vec![BuiltinParam::positional("_ControlStyle")],
    );
    // Presentation/search/window token modifiers. Dedicated namespaces for
    // presentationContentInteraction/presentationSizing/searchDictationBehavior/
    // windowToolbarFullScreenVisibility; windowResizeAnchor reuses UnitPoint;
    // scrollEdgeEffectHidden takes a leading Bool + a `for:` Edge.Set.
    // `PresentationSizing.page` collides with `_ControlStyle.page`, so the
    // previously uniqueness-resolved style modifiers that accept `.page`
    // (tabViewStyle/indexViewStyle) must be typed against `_ControlStyle`.
    interp.register_struct_method_typed(
        "tabViewStyle",
        modifiers::modifier_tab_view_style,
        vec![BuiltinParam::positional("_ControlStyle")],
    );
    interp.register_struct_method_typed(
        "indexViewStyle",
        modifiers::modifier_index_view_style,
        vec![BuiltinParam::positional("_ControlStyle")],
    );
    interp.register_struct_method_typed(
        "presentationContentInteraction",
        modifiers::modifier_presentation_content_interaction,
        vec![BuiltinParam::positional("PresentationContentInteraction")],
    );
    interp.register_struct_method_typed(
        "presentationSizing",
        modifiers::modifier_presentation_sizing,
        vec![BuiltinParam::positional("PresentationSizing")],
    );
    interp.register_struct_method_typed(
        "searchDictationBehavior",
        modifiers::modifier_search_dictation_behavior,
        vec![BuiltinParam::positional("TextInputDictationBehavior")],
    );
    interp.register_struct_method_typed(
        "windowToolbarFullScreenVisibility",
        modifiers::modifier_window_toolbar_full_screen_visibility,
        vec![BuiltinParam::positional(
            "WindowToolbarFullScreenVisibility",
        )],
    );
    interp.register_struct_method_typed(
        "windowResizeAnchor",
        modifiers::modifier_window_resize_anchor,
        vec![BuiltinParam::positional("UnitPoint")],
    );
    interp.register_struct_method_typed(
        "scrollEdgeEffectHidden",
        modifiers::modifier_scroll_edge_effect_hidden,
        vec![
            BuiltinParam::positional("Bool"),
            BuiltinParam::labeled("for", "Edge.Set"),
        ],
    );
    // Symbol/haptic/detent token modifiers: leading-dot presets resolve against
    // the declared token type. `sensoryFeedback` also carries a `trigger:`
    // value (any Equatable, self-typed); `presentationDetents` takes an array
    // of `PresentationDetent` tokens (`[.medium, .large]`). Note: the
    // `.success`/`.failure` SensoryFeedback presets collide with the builtin
    // `Result` enum cases and degrade to a bare token string — the other
    // presets (`.selection`, `.warning`, …) tag cleanly.
    interp.register_struct_method_typed(
        "symbolEffect",
        modifiers::modifier_symbol_effect,
        vec![BuiltinParam::positional("SymbolEffect")],
    );
    interp.register_struct_method_typed(
        "sensoryFeedback",
        modifiers::modifier_sensory_feedback,
        vec![
            BuiltinParam::positional("SensoryFeedback"),
            BuiltinParam::labeled("trigger", "Equatable"),
        ],
    );
    interp.register_struct_method_typed(
        "presentationDetents",
        modifiers::modifier_presentation_detents,
        vec![BuiltinParam::positional("[PresentationDetent]")],
    );
    // Scene/window presentation-style token modifiers, each typed so its
    // leading-dot `.automatic`/`.plain`/… member resolves against a dedicated
    // namespace. `digitalCrownAccessory` reuses the shared `Visibility`
    // namespace; `touchBarItemPrincipal` is a plain `Bool` (no typed seam).
    interp.register_struct_method_typed(
        "presentedWindowStyle",
        modifiers::modifier_presented_window_style,
        vec![BuiltinParam::positional("WindowStyle")],
    );
    interp.register_struct_method_typed(
        "presentedWindowToolbarStyle",
        modifiers::modifier_presented_window_toolbar_style,
        vec![BuiltinParam::positional("WindowToolbarStyle")],
    );
    interp.register_struct_method_typed(
        "typesettingLanguage",
        modifiers::modifier_typesetting_language,
        vec![BuiltinParam::positional("TypesettingLanguage")],
    );
    interp.register_struct_method_typed(
        "digitalCrownAccessory",
        modifiers::modifier_digital_crown_accessory,
        vec![BuiltinParam::positional("Visibility")],
    );
    // Style modifiers whose `.plain` token now overlaps `WindowStyle.plain`
    // must be typed against `_ControlStyle` so their leading-dot member resolves
    // contextually instead of degrading to a bare string (issue #203 pattern).
    interp.register_struct_method_typed(
        "buttonStyle",
        modifiers::modifier_button_style,
        vec![BuiltinParam::positional("_ControlStyle")],
    );
    interp.register_struct_method_typed(
        "listStyle",
        modifiers::modifier_list_style,
        vec![BuiltinParam::positional("_ControlStyle")],
    );
    interp.register_struct_method_typed(
        "textFieldStyle",
        modifiers::modifier_text_field_style,
        vec![BuiltinParam::positional("_ControlStyle")],
    );
    interp.register_struct_method_typed(
        "textEditorStyle",
        modifiers::modifier_text_editor_style,
        vec![BuiltinParam::positional("_ControlStyle")],
    );
    // `withAnimation` — executes the trailing closure immediately and returns
    // its value.  The animation argument (if any) is accepted and dropped;
    // hosts that want to animate will read `.animation` modifiers and diff
    // state transitions themselves (v1 simplification).
    interp.register_free_fn_typed(
        "withAnimation",
        with_animation,
        vec![BuiltinParam::positional("Animation")],
    );
    // Swift Charts marks (ADR-0020) render through this same pipeline.
    charts::install(interp);
}

/// `withAnimation(_:_:)` — runs the trailing closure immediately, drops the
/// animation argument, and returns the closure's result value.  The runtime
/// has no animation transaction or clock; state mutations inside the body
/// take effect as usual and the next render reflects them (v1 simplification).
fn with_animation(ctx: &mut dyn StdContext, args: Vec<tswift_core::Arg>) -> tswift_core::StdResult {
    // The trailing closure is always the last arg (and may be the only one
    // when called as `withAnimation { … }` without an explicit animation).
    let closure_arg = args
        .into_iter()
        .rev()
        .find(|a| matches!(a.value, SwiftValue::Closure(_)));
    match closure_arg {
        Some(a) => {
            let SwiftValue::Closure(id) = a.value else {
                unreachable!()
            };
            ctx.call_closure(id, vec![])
        }
        None => Ok(SwiftValue::Void),
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
            | "Picker" | "Circle" | "Rectangle" | "RoundedRectangle" | "Capsule" | "Ellipse"
            | "Group" | "Divider" | "ScrollView" | "Label" | "Image" | "AsyncImage"
            | "ProgressView" | "LazyVStack" | "LazyHStack" | "Grid" | "GridRow" | "Form"
            | "LazyVGrid" | "LazyHGrid" | "TabView" | "NavigationStack" | "NavigationLink"
            | "TapGesture" | "LongPressGesture" => Some(format!("{key}.init")),
            _ => None,
        })
        .collect();
    // Modifiers are members of `View` for coverage purposes.
    keys.extend(MODIFIER_FNS.iter().map(|(m, _)| format!("View.{m}")));
    // Gesture method — not a View modifier, coverage key is per gesture type.
    keys.push("TapGesture.onEnded".into());
    keys.push("LongPressGesture.onEnded".into());
    // Free functions (no `.` → coverage's free-function section).
    keys.push("withAnimation".into());
    keys.sort();
    keys.dedup();
    keys
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
            // Bracket the view's `body` expansion with the generic render-scope
            // hooks so a modifier-carried subtree value (e.g. SwiftData's
            // `.modelContainer` context) is published for exactly this subtree
            // and restored after — nearest-ancestor wins, no sibling leakage.
            ctx.view_scope_enter(&v);
            let result = match ctx.get_member(&v, "body") {
                Ok(body) => expand_into(ctx, body, out, depth + 1, &child_env),
                Err(e) => Err(e),
            };
            ctx.view_scope_exit(&v);
            result?;
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
        // Bracket `body` evaluation (which eagerly builds the whole subtree)
        // with the render-scope hooks, mirroring `expand_into`.
        ctx.view_scope_enter(&injected);
        let body = ctx.get_member(&injected, "body");
        ctx.view_scope_exit(&injected);
        current = body?;
        depth += 1;
    }
    Ok(current)
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

#[cfg(test)]
mod coverage_dump {
    #[test]
    fn dump_registered_keys() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let path = root.join("frameworks/swiftui/registered_keys.txt");
        let body = super::registered_keys().join("\n") + "\n";
        std::fs::write(&path, body).expect("write registered_keys.txt");
    }

    #[test]
    fn dump_charts_registered_keys() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let path = root.join("frameworks/charts/registered_keys.txt");
        let body = super::charts::registered_keys().join("\n") + "\n";
        std::fs::write(&path, body).expect("write charts registered_keys.txt");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::values::key_string;
    use tswift_core::Arg;

    #[test]
    fn registered_keys_cover_v1_constructors() {
        let keys = registered_keys();
        assert_eq!(
            keys,
            vec![
                "AsyncImage.init",
                "Button.init",
                "Capsule.init",
                "Circle.init",
                "Divider.init",
                "Ellipse.init",
                "ForEach.init",
                "Form.init",
                "Grid.init",
                "GridRow.init",
                "Group.init",
                "HStack.init",
                "Image.init",
                "Label.init",
                "LazyHGrid.init",
                "LazyHStack.init",
                "LazyVGrid.init",
                "LazyVStack.init",
                "List.init",
                "LongPressGesture.init",
                "LongPressGesture.onEnded",
                "NavigationLink.init",
                "NavigationStack.init",
                "Picker.init",
                "ProgressView.init",
                "Rectangle.init",
                "RoundedRectangle.init",
                "ScrollView.init",
                "Section.init",
                "SecureField.init",
                "Slider.init",
                "Spacer.init",
                "Stepper.init",
                "TabView.init",
                "TapGesture.init",
                "TapGesture.onEnded",
                "Text.init",
                "TextField.init",
                "Toggle.init",
                "VStack.init",
                "View.accentColor",
                "View.accessibilityActions",
                "View.accessibilityActivationPoint",
                "View.accessibilityAddTraits",
                "View.accessibilityChartDescriptor",
                "View.accessibilityChildren",
                "View.accessibilityCustomContent",
                "View.accessibilityDirectTouch",
                "View.accessibilityElement",
                "View.accessibilityHeading",
                "View.accessibilityHidden",
                "View.accessibilityHint",
                "View.accessibilityIdentifier",
                "View.accessibilityIgnoresInvertColors",
                "View.accessibilityInputLabels",
                "View.accessibilityLabel",
                "View.accessibilityRemoveTraits",
                "View.accessibilityRepresentation",
                "View.accessibilityRespondsToUserInteraction",
                "View.accessibilityShowsLargeContentViewer",
                "View.accessibilitySortPriority",
                "View.accessibilityTextContentType",
                "View.accessibilityValue",
                "View.alert",
                "View.allowedDynamicRange",
                "View.allowsHitTesting",
                "View.allowsTightening",
                "View.allowsWindowActivationEvents",
                "View.alternatingRowBackgrounds",
                "View.animation",
                "View.aspectRatio",
                "View.assistiveAccessNavigationIcon",
                "View.autocapitalization",
                "View.autocorrectionDisabled",
                "View.background",
                "View.backgroundExtensionEffect",
                "View.backgroundStyle",
                "View.badge",
                "View.badgeProminence",
                "View.baselineOffset",
                "View.blendMode",
                "View.blur",
                "View.bold",
                "View.border",
                "View.brightness",
                "View.buttonBorderShape",
                "View.buttonRepeatBehavior",
                "View.buttonSizing",
                "View.buttonStyle",
                "View.clipShape",
                "View.clipped",
                "View.colorInvert",
                "View.colorMultiply",
                "View.colorScheme",
                "View.compositingGroup",
                "View.confirmationDialog",
                "View.containerBackground",
                "View.containerCornerOffset",
                "View.containerRelativeFrame",
                "View.containerShape",
                "View.contentCaptureProtected",
                "View.contentMargins",
                "View.contentShape",
                "View.contentTransition",
                "View.contextMenu",
                "View.contrast",
                "View.controlGroupStyle",
                "View.controlSize",
                "View.coordinateSpace",
                "View.cornerRadius",
                "View.datePickerStyle",
                "View.defaultAdaptableTabBarPlacement",
                "View.defaultHoverEffect",
                "View.defaultScrollAnchor",
                "View.defaultWheelPickerItemHeight",
                "View.defersSystemGestures",
                "View.deleteDisabled",
                "View.dialogIcon",
                "View.dialogPreventsAppTermination",
                "View.dialogSeverity",
                "View.digitalCrownAccessory",
                "View.disableAutocorrection",
                "View.disabled",
                "View.disclosureGroupStyle",
                "View.draggable",
                "View.drawingGroup",
                "View.dynamicTypeSize",
                "View.edgesIgnoringSafeArea",
                "View.environmentObject",
                "View.equatable",
                "View.fileDialogConfirmationLabel",
                "View.fileDialogCustomizationID",
                "View.fileDialogDefaultDirectory",
                "View.fileDialogImportsUnresolvedAliases",
                "View.fileDialogMessage",
                "View.fill",
                "View.findDisabled",
                "View.fixedSize",
                "View.flipsForRightToLeftLayoutDirection",
                "View.focusEffectDisabled",
                "View.focusSection",
                "View.focusable",
                "View.font",
                "View.fontDesign",
                "View.fontWeight",
                "View.fontWidth",
                "View.foregroundColor",
                "View.foregroundStyle",
                "View.formStyle",
                "View.frame",
                "View.fullScreenCover",
                "View.gaugeStyle",
                "View.geometryGroup",
                "View.gesture",
                "View.glassEffect",
                "View.grayscale",
                "View.gridCellAnchor",
                "View.gridCellColumns",
                "View.gridCellUnsizedAxes",
                "View.gridColumnAlignment",
                "View.groupBoxStyle",
                "View.handGestureShortcut",
                "View.handlesExternalEvents",
                "View.headerProminence",
                "View.help",
                "View.hidden",
                "View.horizontalRadioGroupLayout",
                "View.hoverEffect",
                "View.hoverEffectDisabled",
                "View.hoverEffectGroup",
                "View.hueRotation",
                "View.id",
                "View.ignoresSafeArea",
                "View.imageScale",
                "View.indexViewStyle",
                "View.inspector",
                "View.inspectorColumnWidth",
                "View.interactionActivityTrackingTag",
                "View.interactiveDismissDisabled",
                "View.invalidatableContent",
                "View.italic",
                "View.kerning",
                "View.keyboardShortcut",
                "View.keyboardType",
                "View.labelIconToTitleSpacing",
                "View.labelReservedIconWidth",
                "View.labelStyle",
                "View.labeledContentStyle",
                "View.labelsHidden",
                "View.labelsVisibility",
                "View.layoutDirectionBehavior",
                "View.layoutPriority",
                "View.layoutValue",
                "View.lineHeight",
                "View.lineLimit",
                "View.lineSpacing",
                "View.listItemTint",
                "View.listRowBackground",
                "View.listRowHoverEffect",
                "View.listRowHoverEffectDisabled",
                "View.listRowInsets",
                "View.listRowPlatterColor",
                "View.listRowSeparator",
                "View.listRowSeparatorTint",
                "View.listRowSpacing",
                "View.listSectionIndexVisibility",
                "View.listSectionMargins",
                "View.listSectionSeparator",
                "View.listSectionSeparatorTint",
                "View.listSectionSpacing",
                "View.listStyle",
                "View.luminanceToAlpha",
                "View.mask",
                "View.materialActiveAppearance",
                "View.menuActionDismissBehavior",
                "View.menuIndicator",
                "View.menuOrder",
                "View.menuStyle",
                "View.minimumScaleFactor",
                "View.monospaced",
                "View.monospacedDigit",
                "View.moveDisabled",
                "View.multilineTextAlignment",
                "View.navigationBarBackButtonHidden",
                "View.navigationBarHidden",
                "View.navigationBarItems",
                "View.navigationBarTitle",
                "View.navigationBarTitleDisplayMode",
                "View.navigationDestination",
                "View.navigationDocument",
                "View.navigationLinkIndicatorVisibility",
                "View.navigationSplitViewColumnWidth",
                "View.navigationSplitViewStyle",
                "View.navigationSubtitle",
                "View.navigationTitle",
                "View.navigationTransition",
                "View.navigationViewStyle",
                "View.offset",
                "View.onAppear",
                "View.onChange",
                "View.onCommand",
                "View.onContinuousHover",
                "View.onCopyCommand",
                "View.onCutCommand",
                "View.onDeleteCommand",
                "View.onDisappear",
                "View.onDrag",
                "View.onExitCommand",
                "View.onGeometryChange",
                "View.onHover",
                "View.onLongPressGesture",
                "View.onModifierKeysChanged",
                "View.onMoveCommand",
                "View.onOpenURL",
                "View.onPasteCommand",
                "View.onPencilDoubleTap",
                "View.onPencilSqueeze",
                "View.onPlayPauseCommand",
                "View.onPreferenceChange",
                "View.onScrollGeometryChange",
                "View.onScrollPhaseChange",
                "View.onScrollVisibilityChange",
                "View.onSubmit",
                "View.onTapGesture",
                "View.opacity",
                "View.overlay",
                "View.padding",
                "View.paletteSelectionEffect",
                "View.persistentSystemOverlays",
                "View.phaseAnimator",
                "View.pickerStyle",
                "View.pointerVisibility",
                "View.popover",
                "View.position",
                "View.preference",
                "View.preferredColorScheme",
                "View.presentationBackground",
                "View.presentationBackgroundInteraction",
                "View.presentationCompactAdaptation",
                "View.presentationContentInteraction",
                "View.presentationCornerRadius",
                "View.presentationDetents",
                "View.presentationDragIndicator",
                "View.presentationSizing",
                "View.presentedWindowStyle",
                "View.presentedWindowToolbarStyle",
                "View.previewContext",
                "View.previewDevice",
                "View.previewDisplayName",
                "View.previewInterfaceOrientation",
                "View.previewLayout",
                "View.privacySensitive",
                "View.progressViewStyle",
                "View.projectionEffect",
                "View.redacted",
                "View.refreshable",
                "View.replaceDisabled",
                "View.resizable",
                "View.rotation3DEffect",
                "View.rotationEffect",
                "View.safeAreaBar",
                "View.safeAreaInset",
                "View.safeAreaPadding",
                "View.saturation",
                "View.scaleEffect",
                "View.scaledToFill",
                "View.scaledToFit",
                "View.scenePadding",
                "View.scrollBounceBehavior",
                "View.scrollClipDisabled",
                "View.scrollContentBackground",
                "View.scrollDisabled",
                "View.scrollDismissesKeyboard",
                "View.scrollEdgeEffectHidden",
                "View.scrollEdgeEffectStyle",
                "View.scrollIndicators",
                "View.scrollIndicatorsFlash",
                "View.scrollInputBehavior",
                "View.scrollTargetBehavior",
                "View.scrollTargetLayout",
                "View.scrollTransition",
                "View.searchCompletion",
                "View.searchDictationBehavior",
                "View.searchFocused",
                "View.searchPresentationToolbarBehavior",
                "View.searchScopes",
                "View.searchSelection",
                "View.searchSuggestions",
                "View.searchToolbarBehavior",
                "View.searchable",
                "View.sectionIndexLabel",
                "View.selectionDisabled",
                "View.sensoryFeedback",
                "View.shadow",
                "View.sheet",
                "View.sliderThumbVisibility",
                "View.speechAdjustedPitch",
                "View.speechAlwaysIncludesPunctuation",
                "View.speechAnnouncementsQueued",
                "View.speechSpellsOutCharacters",
                "View.springLoadingBehavior",
                "View.statusBar",
                "View.statusBarHidden",
                "View.strikethrough",
                "View.submitLabel",
                "View.submitScope",
                "View.swipeActions",
                "View.symbolColorRenderingMode",
                "View.symbolEffect",
                "View.symbolEffectsRemoved",
                "View.symbolRenderingMode",
                "View.symbolVariableValueMode",
                "View.symbolVariant",
                "View.tabBarMinimizeBehavior",
                "View.tabItem",
                "View.tabViewStyle",
                "View.tableStyle",
                "View.tag",
                "View.task",
                "View.textCase",
                "View.textContentType",
                "View.textEditorStyle",
                "View.textFieldStyle",
                "View.textInputAutocapitalization",
                "View.textScale",
                "View.textSelection",
                "View.textSelectionAffinity",
                "View.tint",
                "View.toggleStyle",
                "View.toolbarBackground",
                "View.toolbarBackgroundVisibility",
                "View.toolbarColorScheme",
                "View.toolbarForegroundStyle",
                "View.toolbarItemHidden",
                "View.toolbarRole",
                "View.toolbarTitleDisplayMode",
                "View.toolbarVisibility",
                "View.touchBarItemPrincipal",
                "View.tracking",
                "View.transaction",
                "View.transformEffect",
                "View.transformEnvironment",
                "View.transformPreference",
                "View.transition",
                "View.truncationMode",
                "View.typeSelectEquivalent",
                "View.typesettingLanguage",
                "View.underline",
                "View.unredacted",
                "View.visualEffect",
                "View.windowDismissBehavior",
                "View.windowFullScreenBehavior",
                "View.windowMinimizeBehavior",
                "View.windowResizeAnchor",
                "View.windowResizeBehavior",
                "View.windowToolbarFullScreenVisibility",
                "View.writingToolsAffordanceVisibility",
                "View.writingToolsBehavior",
                "View.zIndex",
                "ZStack.init",
                "withAnimation",
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
        let Some(SwiftValue::Struct(handlers)) = obj.get(HANDLERS_FIELD) else {
            panic!("button should carry a _handlers map");
        };
        assert!(
            matches!(handlers.get("tap"), Some(SwiftValue::Closure(_))),
            "button should capture its action closure under the `tap` event"
        );
    }

    #[test]
    fn on_tap_gesture_on_button_keeps_action_and_emits_no_marker() {
        // `.onTapGesture` on a Button must not overwrite the Button's action
        // (shared `tap` key) nor add an `onTapGesture` marker (hosts would
        // otherwise double-emit `tap`). The Button action stays authoritative.
        let src = r#"
struct V: View {
    var body: some View {
        Button("Inc") { }.onTapGesture { }
    }
}
"#;
        let view = render_to_string(src, "V");
        let json = uiir::to_json(&view);
        assert!(
            !json.contains("onTapGesture"),
            "no gesture marker on a Button: {json}"
        );
        let SwiftValue::Struct(obj) = &view else {
            panic!("expected struct");
        };
        let Some(SwiftValue::Struct(handlers)) = obj.get(HANDLERS_FIELD) else {
            panic!("button should keep its _handlers map");
        };
        assert!(
            matches!(handlers.get("tap"), Some(SwiftValue::Closure(_))),
            "button action stays authoritative under `tap`"
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
        let program = format!("import SwiftUI\n{PRELUDE}\n{src}");
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
        let program = format!("import SwiftUI\n{PRELUDE}\n{src}");
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
        let program = format!("import SwiftUI\n{PRELUDE}\n{src}");
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
    fn picker_flattens_group_wrapped_options() {
        // A `Group` inside a `Picker` is transparent: its tagged children must
        // flatten into direct option views, not a single opaque container.
        let view = render_to_string(
            r#"struct V: View {
    @State private var sel = "a"
    var body: some View {
        Picker("Pick", selection: $sel) {
            Group { Text("A").tag("a"); Text("B").tag("b") }
        }
    }
}"#,
            "V",
        );
        let json = uiir::to_json(&view);
        assert!(!json.contains("Group"), "Group must be flattened: {json}");
        assert_eq!(
            json.matches(r#""kind":"Text""#).count(),
            2,
            "two options: {json}"
        );
    }

    #[test]
    fn grid_scalar_args_are_explicit_unsupported_errors() {
        let err = render_err(
            r#"struct V: View { var body: some View { Grid(horizontalSpacing: 8) { GridRow { Text("x") } } } }"#,
            "V",
        );
        assert!(err.contains("Grid"), "clear deferral error: {err}");
    }

    #[test]
    fn stack_alignment_resolves_and_is_stored_as_a_field() {
        // `VStack(alignment:)` resolves against `HorizontalAlignment` (issue
        // #203) and is captured as a constructor field the host honors (#189).
        let view = render_to_string(
            r#"struct V: View { var body: some View { VStack(alignment: .leading) { Text("x") } } }"#,
            "V",
        );
        let SwiftValue::Struct(obj) = &view else {
            panic!("expected a VStack struct, got {view:?}");
        };
        match obj.get("alignment") {
            Some(SwiftValue::Struct(tok)) => {
                assert_eq!(tok.type_name, "HorizontalAlignment");
                assert_eq!(tok.get("token"), Some(&SwiftValue::Str("leading".into())));
            }
            other => panic!("expected a HorizontalAlignment alignment field, got {other:?}"),
        }
    }

    #[test]
    fn gesture_tap_gesture_on_ended_lowers_to_on_tap_gesture_marker() {
        // `.gesture(TapGesture().onEnded { _ in })` must produce the same UIIR
        // marker (`onTapGesture`) and handler key (`tap`) as `.onTapGesture { }`.
        let src = r#"
struct V: View {
    @State private var taps = 0
    var body: some View {
        Text("tap me")
            .gesture(TapGesture().onEnded { _ in taps += 1 })
    }
}
"#;
        let view = render_to_string(src, "V");
        let json = uiir::to_json(&view);
        assert!(
            json.contains("onTapGesture"),
            "gesture(TapGesture) must emit onTapGesture marker: {json}"
        );
        let SwiftValue::Struct(obj) = &view else {
            panic!("expected struct");
        };
        let Some(SwiftValue::Struct(handlers)) = obj.get(HANDLERS_FIELD) else {
            panic!("expected _handlers map");
        };
        assert!(
            matches!(handlers.get("tap"), Some(SwiftValue::Closure(_))),
            "tap handler must be registered"
        );
    }

    #[test]
    fn gesture_long_press_gesture_on_ended_lowers_to_on_long_press_marker() {
        // `.gesture(LongPressGesture(minimumDuration: 1.0).onEnded { _ in })`
        // must produce `onLongPressGesture` marker + `longPress` handler key.
        let src = r#"
struct V: View {
    @State private var held = false
    var body: some View {
        Text("hold me")
            .gesture(LongPressGesture(minimumDuration: 1.0).onEnded { _ in held = true })
    }
}
"#;
        let view = render_to_string(src, "V");
        let json = uiir::to_json(&view);
        assert!(
            json.contains("onLongPressGesture"),
            "gesture(LongPressGesture) must emit onLongPressGesture marker: {json}"
        );
        assert!(
            json.contains("minimumDuration"),
            "minimumDuration must appear in the marker: {json}"
        );
        let SwiftValue::Struct(obj) = &view else {
            panic!("expected struct");
        };
        let Some(SwiftValue::Struct(handlers)) = obj.get(HANDLERS_FIELD) else {
            panic!("expected _handlers map");
        };
        assert!(
            matches!(handlers.get("longPress"), Some(SwiftValue::Closure(_))),
            "longPress handler must be registered"
        );
    }

    #[test]
    fn gesture_on_button_keeps_button_action() {
        // `.gesture(TapGesture().onEnded { })` on a Button must not clobber the
        // Button's action (same Button-priority rule as `.onTapGesture`).
        let src = r#"
struct V: View {
    var body: some View {
        Button("inc") { }.gesture(TapGesture().onEnded { _ in })
    }
}
"#;
        let view = render_to_string(src, "V");
        let json = uiir::to_json(&view);
        assert!(
            !json.contains("onTapGesture"),
            "no gesture marker added to Button: {json}"
        );
        let SwiftValue::Struct(obj) = &view else {
            panic!("expected struct");
        };
        let Some(SwiftValue::Struct(handlers)) = obj.get(HANDLERS_FIELD) else {
            panic!("button should keep its _handlers map");
        };
        assert!(
            matches!(handlers.get("tap"), Some(SwiftValue::Closure(_))),
            "button action stays authoritative"
        );
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

    // ----- withAnimation tests -----

    #[test]
    fn with_animation_no_arg_executes_body() {
        // `withAnimation { flag = true }` — no animation arg; body runs and
        // state change is reflected in next render.
        let view = render_to_string(
            r#"
struct V: View {
    @State var x = 0
    var body: some View {
        let _ = withAnimation { x = 99 }
        Text("\(x)")
    }
}
"#,
            "V",
        );
        let json = uiir::to_json(&view);
        assert!(
            json.contains(r#""verbatim":"99""#),
            "withAnimation body must have run: {json}"
        );
    }

    #[test]
    fn with_animation_with_linear_arg_executes_body() {
        // `withAnimation(.linear) { ... }` — animation arg present; body still runs.
        let view = render_to_string(
            r#"
struct V: View {
    @State var x = 0
    var body: some View {
        let _ = withAnimation(.linear) { x = 42 }
        Text("\(x)")
    }
}
"#,
            "V",
        );
        let json = uiir::to_json(&view);
        assert!(
            json.contains(r#""verbatim":"42""#),
            "withAnimation(.linear) body must have run: {json}"
        );
    }

    #[test]
    fn with_animation_easing_executes_body() {
        // `withAnimation(.easeInOut(duration:0.3)) { ... }` — real-world form.
        let view = render_to_string(
            r#"
struct V: View {
    @State var x = 0
    var body: some View {
        let _ = withAnimation(.easeInOut(duration: 0.3)) { x = 7 }
        Text("\(x)")
    }
}
"#,
            "V",
        );
        let json = uiir::to_json(&view);
        assert!(
            json.contains(r#""verbatim":"7""#),
            "withAnimation(.easeInOut) body must have run: {json}"
        );
    }
}
