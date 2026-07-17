//! Embedded Swift prelude for Charts value types (leading-dot statics).

/// Swift prelude for Charts value types that need leading-dot static methods.
///
/// `PlottableValue.value(_:_:)` is declared in Swift (like SwiftUI's
/// `GridItem.flexible`) so `.value("Label", datum)` resolves via the
/// interpreter's implicit-static-method path when `BarMark(x:y:)` pushes a
/// `PlottableValue` contextual type. Prepend this ahead of user source the
/// same way hosts prepend `tswift_swiftui::PRELUDE`.
///
/// Token namespaces (`InterpolationMethod`, `ChartSymbolShape`,
/// `AnnotationPosition`) mirror SwiftUI's `Color`/`Font` pattern so leading-dot
/// forms resolve under typed mark-modifier parameter hints.
pub const PRELUDE: &str = r#"
// `PlottableValue` — label + plottable datum pair used as mark x/y args
// (`BarMark(x: .value("Name", "A"), y: .value("Count", 3))`). Real Charts is
// generic over `Plottable`; v1 stores the datum as `Any`.
struct PlottableValue {
    let label: String
    let value: Any
    static func value(_ label: String, _ value: Any) -> PlottableValue {
        PlottableValue(label: label, value: value)
    }
}
// A mark's along-axis size. `.automatic` (scale-driven) plus the explicit
// `.fixed(_)`/`.ratio(_)`/`.inset(_)` builders; serialized as
// `{"$":"markDimension","kind":…,"value":…}`.
struct MarkDimension {
    let kind: String
    let value: Double
    static let automatic = MarkDimension(kind: "automatic", value: 0)
    static func fixed(_ value: Double) -> MarkDimension { MarkDimension(kind: "fixed", value: value) }
    static func ratio(_ value: Double) -> MarkDimension { MarkDimension(kind: "ratio", value: value) }
    static func inset(_ value: Double) -> MarkDimension { MarkDimension(kind: "inset", value: value) }
}
// How overlapping marks in the same x-position combine. Pure token (the
// parameterized cases are out of scope v1).
struct MarkStackingMethod {
    let token: String
    static let standard = MarkStackingMethod(token: "standard")
    static let center = MarkStackingMethod(token: "center")
    static let normalized = MarkStackingMethod(token: "normalized")
    static let unstacked = MarkStackingMethod(token: "unstacked")
}
// Line/area interpolation token (`.interpolationMethod(.catmullRom)`).
struct InterpolationMethod {
    let token: String
    static let linear = InterpolationMethod(token: "linear")
    static let catmullRom = InterpolationMethod(token: "catmullRom")
    static let monotone = InterpolationMethod(token: "monotone")
    static let cardinal = InterpolationMethod(token: "cardinal")
    static let stepStart = InterpolationMethod(token: "stepStart")
    static let stepCenter = InterpolationMethod(token: "stepCenter")
    static let stepEnd = InterpolationMethod(token: "stepEnd")
}
// Point symbol shape token (`.symbol(.circle)`).
struct ChartSymbolShape {
    let token: String
    static let circle = ChartSymbolShape(token: "circle")
    static let square = ChartSymbolShape(token: "square")
    static let diamond = ChartSymbolShape(token: "diamond")
    static let triangle = ChartSymbolShape(token: "triangle")
    static let cross = ChartSymbolShape(token: "cross")
    static let plus = ChartSymbolShape(token: "plus")
    static let asterisk = ChartSymbolShape(token: "asterisk")
    static let pentagon = ChartSymbolShape(token: "pentagon")
}
// Annotation placement token (`.annotation(position: .top) { … }`).
struct AnnotationPosition {
    let token: String
    static let automatic = AnnotationPosition(token: "automatic")
    static let top = AnnotationPosition(token: "top")
    static let bottom = AnnotationPosition(token: "bottom")
    static let leading = AnnotationPosition(token: "leading")
    static let trailing = AnnotationPosition(token: "trailing")
    static let overlay = AnnotationPosition(token: "overlay")
    static let topLeading = AnnotationPosition(token: "topLeading")
    static let topTrailing = AnnotationPosition(token: "topTrailing")
    static let bottomLeading = AnnotationPosition(token: "bottomLeading")
    static let bottomTrailing = AnnotationPosition(token: "bottomTrailing")
}
// Minimal `StrokeStyle` so `.lineStyle(StrokeStyle(lineWidth: 2))` stores args.
struct StrokeStyle {
    let lineWidth: Double
    init(lineWidth: Double = 1.0) { self.lineWidth = lineWidth }
}
// Axis / legend visibility token (`.chartXAxis(.hidden)`, `.chartLegend(.visible)`).
// Real SwiftUI `Visibility` lives in SwiftUICore; Charts reuses it. v1 is a
// Charts-local token so leading-dot resolves under chart-modifier type hints.
struct Visibility {
    let token: String
    static let automatic = Visibility(token: "automatic")
    static let visible = Visibility(token: "visible")
    static let hidden = Visibility(token: "hidden")
}
"#;
