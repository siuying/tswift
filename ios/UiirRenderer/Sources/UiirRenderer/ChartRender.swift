import Charts
import Foundation
import SwiftUI

/// Lowers Charts UIIR (`Chart` + mark children + mark/chart modifiers) to native
/// SwiftUI Charts. Mark trees become `@ChartContentBuilder` content; chart-level
/// modifiers are applied on the `Chart` view via `ModifierApply`.
@MainActor
enum ChartRender {
    // MARK: - Chart view

    static func chart(_ node: UiirNode) -> some View {
        Chart {
            ForEach(node.children, id: \.id) { child in
                markContent(child)
            }
        }
    }

    // MARK: - Mark content

    /// One UIIR mark node → `some ChartContent`, with mark `_modifiers` applied.
    @ChartContentBuilder
    static func markContent(_ node: UiirNode) -> some ChartContent {
        rawMark(node)
            .uiirApplyMarkModifiers(node.modifiers)
    }

    @ChartContentBuilder
    private static func rawMark(_ node: UiirNode) -> some ChartContent {
        switch node.kind {
        case "BarMark":
            xyBarMark(node)
        case "LineMark":
            xyLineMark(node)
        case "PointMark":
            xyPointMark(node)
        case "AreaMark":
            xyAreaMark(node)
        case "RuleMark":
            ruleMark(node)
        case "RectangleMark":
            rectangleMark(node)
        case "SectorMark":
            sectorMark(node)
        default:
            // Unknown / axis leaves under Chart: no plot geometry.
            emptyMark()
        }
    }

    /// Empty chart content (no geometry) for unsupported kinds / incomplete args.
    @ChartContentBuilder
    private static func emptyMark() -> some ChartContent {}

    // MARK: - 2-D marks (x/y PlottableValue)

    @ChartContentBuilder
    private static func xyBarMark(_ node: UiirNode) -> some ChartContent {
        if let x = plotArg(node, "x"), let y = plotArg(node, "y") {
            switch (x.value, y.value) {
            case (.string(let xs), .double(let yd)):
                BarMark(x: .value(x.label, xs), y: .value(y.label, yd))
            case (.double(let xd), .double(let yd)):
                BarMark(x: .value(x.label, xd), y: .value(y.label, yd))
            case (.string(let xs), .string(let ys)):
                BarMark(x: .value(x.label, xs), y: .value(y.label, ys))
            case (.double(let xd), .string(let ys)):
                BarMark(x: .value(x.label, xd), y: .value(y.label, ys))
            }
        }
    }

    @ChartContentBuilder
    private static func xyLineMark(_ node: UiirNode) -> some ChartContent {
        if let x = plotArg(node, "x"), let y = plotArg(node, "y") {
            switch (x.value, y.value) {
            case (.string(let xs), .double(let yd)):
                LineMark(x: .value(x.label, xs), y: .value(y.label, yd))
            case (.double(let xd), .double(let yd)):
                LineMark(x: .value(x.label, xd), y: .value(y.label, yd))
            case (.string(let xs), .string(let ys)):
                LineMark(x: .value(x.label, xs), y: .value(y.label, ys))
            case (.double(let xd), .string(let ys)):
                LineMark(x: .value(x.label, xd), y: .value(y.label, ys))
            }
        }
    }

    @ChartContentBuilder
    private static func xyPointMark(_ node: UiirNode) -> some ChartContent {
        if let x = plotArg(node, "x"), let y = plotArg(node, "y") {
            switch (x.value, y.value) {
            case (.string(let xs), .double(let yd)):
                PointMark(x: .value(x.label, xs), y: .value(y.label, yd))
            case (.double(let xd), .double(let yd)):
                PointMark(x: .value(x.label, xd), y: .value(y.label, yd))
            case (.string(let xs), .string(let ys)):
                PointMark(x: .value(x.label, xs), y: .value(y.label, ys))
            case (.double(let xd), .string(let ys)):
                PointMark(x: .value(x.label, xd), y: .value(y.label, ys))
            }
        }
    }

    @ChartContentBuilder
    private static func xyAreaMark(_ node: UiirNode) -> some ChartContent {
        if let x = plotArg(node, "x"), let y = plotArg(node, "y") {
            switch (x.value, y.value) {
            case (.string(let xs), .double(let yd)):
                AreaMark(x: .value(x.label, xs), y: .value(y.label, yd))
            case (.double(let xd), .double(let yd)):
                AreaMark(x: .value(x.label, xd), y: .value(y.label, yd))
            case (.string(let xs), .string(let ys)):
                AreaMark(x: .value(x.label, xs), y: .value(y.label, ys))
            case (.double(let xd), .string(let ys)):
                AreaMark(x: .value(x.label, xd), y: .value(y.label, ys))
            }
        }
    }

    // MARK: - RuleMark (variable label sets)

    @ChartContentBuilder
    private static func ruleMark(_ node: UiirNode) -> some ChartContent {
        if let x = plotArg(node, "x"), plotArg(node, "yStart") == nil, plotArg(node, "yEnd") == nil {
            // Vertical rule at x (optionally full height).
            plotRuleX(x)
        } else if let y = plotArg(node, "y"), plotArg(node, "xStart") == nil, plotArg(node, "xEnd") == nil {
            // Horizontal rule at y.
            plotRuleY(y)
        } else if let xStart = plotArg(node, "xStart"), let xEnd = plotArg(node, "xEnd") {
            // Horizontal segment (and optional y position).
            if let y = plotArg(node, "y") {
                plotRuleXRangeAtY(xStart, xEnd, y)
            } else {
                plotRuleXRange(xStart, xEnd)
            }
        } else if let yStart = plotArg(node, "yStart"), let yEnd = plotArg(node, "yEnd") {
            // Vertical segment (and optional x position).
            if let x = plotArg(node, "x") {
                plotRuleYRangeAtX(yStart, yEnd, x)
            } else {
                plotRuleYRange(yStart, yEnd)
            }
        }
    }

    @ChartContentBuilder
    private static func plotRuleX(_ x: PlottableArg) -> some ChartContent {
        switch x.value {
        case .string(let s): RuleMark(x: .value(x.label, s))
        case .double(let d): RuleMark(x: .value(x.label, d))
        }
    }

    @ChartContentBuilder
    private static func plotRuleY(_ y: PlottableArg) -> some ChartContent {
        switch y.value {
        case .string(let s): RuleMark(y: .value(y.label, s))
        case .double(let d): RuleMark(y: .value(y.label, d))
        }
    }

    @ChartContentBuilder
    private static func plotRuleXRange(_ start: PlottableArg, _ end: PlottableArg) -> some ChartContent {
        switch (start.value, end.value) {
        case (.double(let a), .double(let b)):
            RuleMark(xStart: .value(start.label, a), xEnd: .value(end.label, b))
        case (.string(let a), .string(let b)):
            RuleMark(xStart: .value(start.label, a), xEnd: .value(end.label, b))
        default:
            emptyMark()
        }
    }

    @ChartContentBuilder
    private static func plotRuleYRange(_ start: PlottableArg, _ end: PlottableArg) -> some ChartContent {
        switch (start.value, end.value) {
        case (.double(let a), .double(let b)):
            RuleMark(yStart: .value(start.label, a), yEnd: .value(end.label, b))
        case (.string(let a), .string(let b)):
            RuleMark(yStart: .value(start.label, a), yEnd: .value(end.label, b))
        default:
            emptyMark()
        }
    }

    /// `RuleMark(xStart:xEnd:y:)` — horizontal rule segment at a y position.
    @ChartContentBuilder
    private static func plotRuleXRangeAtY(
        _ start: PlottableArg,
        _ end: PlottableArg,
        _ y: PlottableArg
    ) -> some ChartContent {
        switch (start.value, end.value, y.value) {
        case (.double(let a), .double(let b), .double(let yd)):
            RuleMark(
                xStart: .value(start.label, a),
                xEnd: .value(end.label, b),
                y: .value(y.label, yd)
            )
        case (.double(let a), .double(let b), .string(let ys)):
            RuleMark(
                xStart: .value(start.label, a),
                xEnd: .value(end.label, b),
                y: .value(y.label, ys)
            )
        case (.string(let a), .string(let b), .double(let yd)):
            RuleMark(
                xStart: .value(start.label, a),
                xEnd: .value(end.label, b),
                y: .value(y.label, yd)
            )
        case (.string(let a), .string(let b), .string(let ys)):
            RuleMark(
                xStart: .value(start.label, a),
                xEnd: .value(end.label, b),
                y: .value(y.label, ys)
            )
        default:
            emptyMark()
        }
    }

    /// `RuleMark(yStart:yEnd:x:)` — vertical rule segment at an x position.
    @ChartContentBuilder
    private static func plotRuleYRangeAtX(
        _ start: PlottableArg,
        _ end: PlottableArg,
        _ x: PlottableArg
    ) -> some ChartContent {
        switch (start.value, end.value, x.value) {
        case (.double(let a), .double(let b), .double(let xd)):
            RuleMark(
                x: .value(x.label, xd),
                yStart: .value(start.label, a),
                yEnd: .value(end.label, b)
            )
        case (.double(let a), .double(let b), .string(let xs)):
            RuleMark(
                x: .value(x.label, xs),
                yStart: .value(start.label, a),
                yEnd: .value(end.label, b)
            )
        case (.string(let a), .string(let b), .double(let xd)):
            RuleMark(
                x: .value(x.label, xd),
                yStart: .value(start.label, a),
                yEnd: .value(end.label, b)
            )
        case (.string(let a), .string(let b), .string(let xs)):
            RuleMark(
                x: .value(x.label, xs),
                yStart: .value(start.label, a),
                yEnd: .value(end.label, b)
            )
        default:
            emptyMark()
        }
    }

    // MARK: - RectangleMark / SectorMark

    @ChartContentBuilder
    private static func rectangleMark(_ node: UiirNode) -> some ChartContent {
        let width = node.args["width"]?.doubleValue.map { CGFloat($0) }
        let height = node.args["height"]?.doubleValue.map { CGFloat($0) }
        if let x = plotArg(node, "x"), let y = plotArg(node, "y") {
            switch (x.value, y.value) {
            case (.string(let xs), .double(let yd)):
                rectMark(
                    x: .value(x.label, xs), y: .value(y.label, yd),
                    width: width, height: height
                )
            case (.double(let xd), .double(let yd)):
                rectMark(
                    x: .value(x.label, xd), y: .value(y.label, yd),
                    width: width, height: height
                )
            case (.string(let xs), .string(let ys)):
                rectMark(
                    x: .value(x.label, xs), y: .value(y.label, ys),
                    width: width, height: height
                )
            case (.double(let xd), .string(let ys)):
                rectMark(
                    x: .value(x.label, xd), y: .value(y.label, ys),
                    width: width, height: height
                )
            }
        } else if let xStart = plotArg(node, "xStart"), let xEnd = plotArg(node, "xEnd"),
                  let yStart = plotArg(node, "yStart"), let yEnd = plotArg(node, "yEnd")
        {
            // Domain-interval form (both ends numeric or both categorical).
            switch (xStart.value, xEnd.value, yStart.value, yEnd.value) {
            case (.double(let xs), .double(let xe), .double(let ys), .double(let ye)):
                RectangleMark(
                    xStart: .value(xStart.label, xs),
                    xEnd: .value(xEnd.label, xe),
                    yStart: .value(yStart.label, ys),
                    yEnd: .value(yEnd.label, ye)
                )
            default:
                emptyMark()
            }
        }
    }

    @ChartContentBuilder
    private static func rectMark<X: Plottable, Y: Plottable>(
        x: PlottableValue<X>,
        y: PlottableValue<Y>,
        width: CGFloat?,
        height: CGFloat?
    ) -> some ChartContent {
        if let width, let height {
            RectangleMark(
                x: x, y: y,
                width: .fixed(width), height: .fixed(height)
            )
        } else if let width {
            RectangleMark(x: x, y: y, width: .fixed(width))
        } else if let height {
            RectangleMark(x: x, y: y, height: .fixed(height))
        } else {
            RectangleMark(x: x, y: y)
        }
    }

    @ChartContentBuilder
    private static func sectorMark(_ node: UiirNode) -> some ChartContent {
        // SectorMark is iOS 17+ / macOS 14+; package floor is iOS 16.
        if #available(iOS 17.0, macOS 14.0, *) {
            if let angle = plotArg(node, "angle") {
                let inner = node.args["innerRadius"]?.doubleValue.map { CGFloat($0) }
                let inset = node.args["angularInset"]?.doubleValue.map { CGFloat($0) }
                switch angle.value {
                case .double(let d):
                    sectorWithAngle(.value(angle.label, d), inner: inner, inset: inset)
                case .string(let s):
                    sectorWithAngle(.value(angle.label, s), inner: inner, inset: inset)
                }
            }
        }
    }

    @available(iOS 17.0, macOS 14.0, *)
    @ChartContentBuilder
    private static func sectorWithAngle<V: Plottable>(
        _ angle: PlottableValue<V>,
        inner: CGFloat?,
        inset: CGFloat?
    ) -> some ChartContent {
        if let inner, let inset {
            SectorMark(angle: angle, innerRadius: .fixed(inner), angularInset: inset)
        } else if let inner {
            SectorMark(angle: angle, innerRadius: .fixed(inner))
        } else if let inset {
            SectorMark(angle: angle, angularInset: inset)
        } else {
            SectorMark(angle: angle)
        }
    }

    // MARK: - PlottableValue parsing

    /// Parsed `PlottableValue(label: L, value: V)` from UIIR args.
    ///
    /// Preferred wire form is the structured object
    /// `{"$":"plottable","label":…,"value":…}` so JSON string vs number preserves
    /// the declared Plottable type (String `"3"` stays categorical). Legacy
    /// Display strings remain accepted for older fixtures.
    struct PlottableArg {
        let label: String
        let value: Scalar

        enum Scalar: Hashable {
            case string(String)
            case double(Double)
        }

        static func parse(_ value: UiirValue) -> PlottableArg? {
            // Structured form: {"$":"plottable","label":L,"value":V}
            if case let .object(o) = value,
               case let .string(tag)? = o["$"], tag == "plottable"
            {
                let label = o["label"]?.stringValue ?? ""
                switch o["value"] {
                case .number(let n)?:
                    return PlottableArg(label: label, value: .double(n))
                case .string(let s)?:
                    // Declared String stays String even when numeric-looking.
                    return PlottableArg(label: label, value: .string(s))
                case .bool(let b)?:
                    return PlottableArg(label: label, value: .string(b ? "true" : "false"))
                case .null?, .none:
                    return PlottableArg(label: label, value: .string(""))
                default:
                    return nil
                }
            }
            if case let .string(s) = value { return parseDisplay(s) }
            return nil
        }

        /// Legacy Display string `PlottableValue(label: Name, value: A)`.
        static func parseDisplay(_ raw: String) -> PlottableArg? {
            guard raw.hasPrefix("PlottableValue(label:") else { return nil }
            let inner = raw.dropFirst("PlottableValue(label:".count)
            guard let valueSep = inner.range(of: ", value:") else { return nil }
            let label = inner[..<valueSep.lowerBound]
                .trimmingCharacters(in: .whitespacesAndNewlines)
            var valuePart = inner[valueSep.upperBound...]
                .trimmingCharacters(in: .whitespacesAndNewlines)
            if valuePart.hasSuffix(")") {
                valuePart = valuePart.dropLast()
                    .trimmingCharacters(in: .whitespacesAndNewlines)
            }
            let valueStr = String(valuePart)

            // Quoted string → always categorical String (preserves `"3"`).
            if valueStr.count >= 2, valueStr.hasPrefix("\""), valueStr.hasSuffix("\"") {
                let unquoted = String(valueStr.dropFirst().dropLast())
                return PlottableArg(label: label, value: .string(unescapeDisplayString(unquoted)))
            }

            // Unquoted number token → Double (Int/Double Display has no quotes).
            // Unquoted non-numeric text → String category.
            // degraded: legacy Display cannot distinguish String("3") from Int(3)
            // when the string was emitted unquoted; prefer structured `$":"plottable"`.
            if let d = Double(valueStr),
               valueStr != "",
               valueStr.utf8.allSatisfy({
                   ($0 >= 48 && $0 <= 57) || $0 == 46 || $0 == 45 || $0 == 43 || $0 == 101
                       || $0 == 69
               })
            {
                return PlottableArg(label: label, value: .double(d))
            }
            return PlottableArg(label: label, value: .string(valueStr))
        }

        private static func unescapeDisplayString(_ s: String) -> String {
            var out = String()
            out.reserveCapacity(s.count)
            var i = s.startIndex
            while i < s.endIndex {
                if s[i] == "\\", s.index(after: i) < s.endIndex {
                    let n = s[s.index(after: i)]
                    switch n {
                    case "\"", "\\": out.append(n)
                    case "n": out.append("\n")
                    case "t": out.append("\t")
                    default: out.append(n)
                    }
                    i = s.index(i, offsetBy: 2)
                } else {
                    out.append(s[i])
                    i = s.index(after: i)
                }
            }
            return out
        }
    }

    private static func plotArg(_ node: UiirNode, _ key: String) -> PlottableArg? {
        guard let v = node.args[key] else { return nil }
        return PlottableArg.parse(v)
    }
}

// MARK: - Mark modifiers (ChartContent)

extension ChartContent {
    /// Apply UIIR mark `_modifiers` in **original order**, including repeats.
    ///
    /// Peels one modifier per nested `UiirMarkModList` frame so the opaque
    /// `some ChartContent` type stays finite at each level while still applying
    /// **every** modifier (no fixed-arity cap). Same-method recursion on
    /// `some ChartContent` fails Swift opaque inference; this list peel does not.
    func uiirApplyMarkModifiers(_ modifiers: [UiirModifier]) -> some ChartContent {
        UiirMarkModList(content: self, modifiers: modifiers)
    }
}

/// Peel-and-apply over the full modifier list (arbitrary length, original order).
///
/// Each non-empty frame wraps `content` in one `UiirMarkStep` then nests another
/// `UiirMarkModList` specialized on that step type for the tail. Explicit
/// `UiirMarkModList<UiirMarkStep<Content>>` avoids the compiler fixing `Content`
/// to the outer specialization (which would reject the stepped content).
private struct UiirMarkModList<Content: ChartContent>: ChartContent {
    let content: Content
    let modifiers: [UiirModifier]

    @ChartContentBuilder
    var body: some ChartContent {
        if modifiers.isEmpty {
            content
        } else {
            UiirMarkModList<UiirMarkStep<Content>>(
                content: UiirMarkStep(content: content, mod: modifiers[0]),
                modifiers: Array(modifiers.dropFirst())
            )
        }
    }
}

/// One mark-modifier step as a `ChartContent` wrapper so ordered/repeated
/// modifiers do not require a recursive opaque return on the same method.
private struct UiirMarkStep<Content: ChartContent>: ChartContent {
    let content: Content
    let mod: UiirModifier

    @ChartContentBuilder
    var body: some ChartContent {
        uiirApplyNamed(mod)
    }

    @ChartContentBuilder
    private func uiirApplyNamed(_ mod: UiirModifier) -> some ChartContent {
        if mod.name == "foregroundStyle", let color = mod.value.asColor {
            content.foregroundStyle(color)
        } else if mod.name == "foregroundStyle",
                  let by = mod.value.member("by").flatMap(ChartRender.PlottableArg.parse)
        {
            switch by.value {
            case .string(let s):
                content.foregroundStyle(by: .value(by.label, s))
            case .double(let d):
                content.foregroundStyle(by: .value(by.label, d))
            }
        } else if mod.name == "opacity", let n = mod.value.doubleValue {
            content.opacity(n)
        } else if mod.name == "symbolSize", let n = mod.value.doubleValue {
            content.symbolSize(n)
        } else if mod.name == "lineStyle", let style = StrokeStyleParse.parse(mod.value) {
            content.lineStyle(style)
        } else if mod.name == "interpolationMethod",
                  let method = InterpolationParse.parse(mod.value)
        {
            content.interpolationMethod(method)
        } else if mod.name == "cornerRadius", let n = mod.value.doubleValue {
            content.cornerRadius(n)
        } else if mod.name == "symbol",
                  let by = mod.value.member("by").flatMap(ChartRender.PlottableArg.parse)
        {
            switch by.value {
            case .string(let s):
                content.symbol(by: .value(by.label, s))
            case .double(let d):
                content.symbol(by: .value(by.label, d))
            }
        } else if mod.name == "symbol", let name = SymbolParse.shapeName(mod.value) {
            content.symbol(SymbolParse.symbol(name))
        } else if mod.name == "position",
                  let by = mod.value.member("by").flatMap(ChartRender.PlottableArg.parse)
        {
            switch by.value {
            case .string(let s):
                content.position(by: .value(by.label, s))
            case .double(let d):
                content.position(by: .value(by.label, d))
            }
        } else if mod.name == "annotation",
                  let contentNode = mod.value.asNode ?? mod.value.member("value")?.asNode
        {
            let position = AnnotationParse.position(mod.value.member("position") ?? .null)
            content.annotation(position: position) {
                ViewFactory.render(contentNode)
            }
        } else {
            // offset / unknown: no ChartContent form.
            content
        }
    }
}

// MARK: - Token parsers for mark modifiers

private enum StrokeStyleParse {
    /// `"StrokeStyle(lineWidth: 2.0)"` Display string, or `{ lineWidth: n }`.
    static func parse(_ value: UiirValue) -> StrokeStyle? {
        if case let .number(n) = value {
            return StrokeStyle(lineWidth: n)
        }
        if let w = value.member("lineWidth")?.doubleValue {
            return StrokeStyle(lineWidth: w)
        }
        if case let .string(s) = value {
            if let r = s.range(of: "lineWidth:") {
                let rest = s[r.upperBound...]
                    .trimmingCharacters(in: .whitespacesAndNewlines)
                let num = rest.prefix(while: { $0.isNumber || $0 == "." || $0 == "-" })
                if let w = Double(num) {
                    return StrokeStyle(lineWidth: w)
                }
            }
        }
        return nil
    }
}

private enum InterpolationParse {
    static func parse(_ value: UiirValue) -> InterpolationMethod? {
        let name: String?
        if case let .string(s) = value {
            // `InterpolationMethod(token: catmullRom)` or bare `catmullRom`.
            if let r = s.range(of: "token:") {
                name = s[r.upperBound...]
                    .trimmingCharacters(in: CharacterSet(charactersIn: " )"))
                    .trimmingCharacters(in: .whitespacesAndNewlines)
            } else {
                name = s
            }
        } else if case let .token(_, n) = value {
            name = n
        } else if case let .object(o) = value, case let .string(n)? = o["token"] {
            name = n
        } else {
            name = nil
        }
        guard let name else { return nil }
        switch name {
        case "linear": return .linear
        case "catmullRom": return .catmullRom
        case "monotone": return .monotone
        case "cardinal": return .cardinal
        case "stepCenter": return .stepCenter
        case "stepStart": return .stepStart
        case "stepEnd": return .stepEnd
        default: return nil
        }
    }
}

private enum SymbolParse {
    static func shapeName(_ value: UiirValue) -> String? {
        if case let .string(s) = value {
            if let r = s.range(of: "token:") {
                return s[r.upperBound...]
                    .trimmingCharacters(in: CharacterSet(charactersIn: " )"))
                    .trimmingCharacters(in: .whitespacesAndNewlines)
            }
            return s
        }
        if case let .token(_, n) = value { return n }
        if case let .object(o) = value, case let .string(n)? = o["token"] { return n }
        return nil
    }

    static func symbol(_ name: String) -> BasicChartSymbolShape {
        switch name {
        case "circle": return .circle
        case "square": return .square
        case "diamond": return .diamond
        case "triangle": return .triangle
        case "pentagon": return .pentagon
        case "plus": return .plus
        case "cross": return .cross
        case "asterisk": return .asterisk
        default: return .circle
        }
    }
}

private enum AnnotationParse {
    static func position(_ value: UiirValue) -> AnnotationPosition {
        let name: String?
        if case let .string(s) = value {
            if let r = s.range(of: "token:") {
                name = s[r.upperBound...]
                    .trimmingCharacters(in: CharacterSet(charactersIn: " )"))
                    .trimmingCharacters(in: .whitespacesAndNewlines)
            } else {
                name = s
            }
        } else if case let .token(_, n) = value {
            name = n
        } else if case let .object(o) = value, case let .string(n)? = o["token"] {
            name = n
        } else {
            name = nil
        }
        switch name {
        case "top": return .top
        case "bottom": return .bottom
        case "leading", "left": return .leading
        case "trailing", "right": return .trailing
        case "overlay": return .overlay
        default: return .automatic
        }
    }
}

// MARK: - Chart-level view modifiers (AnyView)

extension ModifierApply {
    /// Chart-only modifiers applied on the `Chart` view (and harmless no-ops
    /// elsewhere). Called from the main `applyOne` switch.
    @MainActor
    static func applyChartModifier(
        _ mod: UiirModifier,
        to view: AnyView,
        nodeId: String,
        sink: any UiirEventSink
    ) -> AnyView? {
        switch mod.name {
        case "chartXAxis":
            return chartAxis(mod.value, axis: .x, view: view)
        case "chartYAxis":
            return chartAxis(mod.value, axis: .y, view: view)
        case "chartXAxisLabel":
            if let s = chartLabelString(mod.value) {
                return AnyView(view.chartXAxisLabel(s))
            }
            return view
        case "chartYAxisLabel":
            if let s = chartLabelString(mod.value) {
                return AnyView(view.chartYAxisLabel(s))
            }
            return view
        case "chartLegend":
            if isVisibilityHidden(mod.value) {
                return AnyView(view.chartLegend(.hidden))
            }
            // Builder / position forms: degrade (keep default legend).
            return view
        case "chartXScale":
            return chartScale(mod.value, axis: .x, view: view)
        case "chartYScale":
            return chartScale(mod.value, axis: .y, view: view)
        case "chartForegroundStyleScale":
            return chartForegroundStyleScale(mod.value, view: view)
        case "chartPlotStyle":
            return chartPlotStyle(mod.value, view: view, nodeId: nodeId, sink: sink)
        case "chartXSelection":
            return chartXSelection(mod.value, view: view)
        default:
            return nil
        }
    }

    private enum ChartAxisKind { case x, y }

    @MainActor
    private static func chartAxis(
        _ value: UiirValue,
        axis: ChartAxisKind,
        view: AnyView
    ) -> AnyView {
        if isVisibilityHidden(value) {
            switch axis {
            case .x: return AnyView(view.chartXAxis(.hidden))
            case .y: return AnyView(view.chartYAxis(.hidden))
            }
        }
        // Builder form (`AxisMarks { … }`) → leave default axis for v1.
        // Could later map nested AxisGridLine/AxisTick/AxisValueLabel.
        return view
    }

    @MainActor
    private static func chartScale(
        _ value: UiirValue,
        axis: ChartAxisKind,
        view: AnyView
    ) -> AnyView {
        // `.chartXScale(domain: ["A","B"])` or `domain: [0, 100]` stored as object
        // with `domain` array, or bare array.
        let domain = value.member("domain") ?? value
        if case let .array(items) = domain {
            let strings = items.compactMap(\.stringValue)
            if strings.count == items.count, !strings.isEmpty {
                switch axis {
                case .x: return AnyView(view.chartXScale(domain: strings))
                case .y: return AnyView(view.chartYScale(domain: strings))
                }
            }
            let nums = items.compactMap(\.doubleValue)
            if nums.count == items.count, nums.count >= 2 {
                let lo = nums.min() ?? 0
                let hi = nums.max() ?? 1
                switch axis {
                case .x: return AnyView(view.chartXScale(domain: lo...hi))
                case .y: return AnyView(view.chartYScale(domain: lo...hi))
                }
            }
        }
        return view
    }

    /// `.chartForegroundStyleScale(["A": Color.red, …])` domain→color mapping.
    @MainActor
    private static func chartForegroundStyleScale(_ value: UiirValue, view: AnyView) -> AnyView {
        if let pairs = parseForegroundStyleMapping(value), !pairs.isEmpty {
            // KeyValuePairs / dictionary of series → Color.
            // Build via domain + range arrays (heterogeneous KeyValuePairs literals
            // cannot be formed dynamically).
            let domain = pairs.map(\.0)
            let range = pairs.map(\.1)
            return AnyView(view.chartForegroundStyleScale(domain: domain, range: range))
        }
        // degraded: mapping/closure forms we cannot decode stay default palette.
        return view
    }

    /// Parse series→Color pairs from Display dict string or structured object.
    private static func parseForegroundStyleMapping(_ value: UiirValue) -> [(String, Color)]? {
        // Structured object `{ "A": {"$":"color","name":"red"}, … }` (if ever emitted).
        if case let .object(o) = value {
            var pairs: [(String, Color)] = []
            for (k, v) in o.sorted(by: { $0.key < $1.key }) {
                if k == "$" { continue }
                if let c = v.asColor {
                    pairs.append((k, c))
                }
            }
            if !pairs.isEmpty { return pairs }
        }
        // Display string from Dict: `["A": Color(token: red), "B": Color(token: blue)]`
        if case let .string(s) = value {
            return parseForegroundStyleMappingDisplay(s)
        }
        return nil
    }

    private static func parseForegroundStyleMappingDisplay(_ s: String) -> [(String, Color)]? {
        var pairs: [(String, Color)] = []
        // Match `"Key": Color(token: name)` or `"Key": Color(name: …)` fragments.
        let pattern = #"\"((?:\\.|[^\"])*)\"\s*:\s*Color\(\s*token:\s*([A-Za-z0-9_]+)\s*\)"#
        guard let regex = try? NSRegularExpression(pattern: pattern) else { return nil }
        let ns = s as NSString
        let full = NSRange(location: 0, length: ns.length)
        regex.enumerateMatches(in: s, options: [], range: full) { match, _, _ in
            guard let match, match.numberOfRanges >= 3 else { return }
            let key = ns.substring(with: match.range(at: 1))
            let colorName = ns.substring(with: match.range(at: 2))
            pairs.append((key, Tokens.color(colorName)))
        }
        return pairs.isEmpty ? nil : pairs
    }

    /// `.chartPlotStyle { plotArea in … }` — apply collected plot-area modifiers.
    @MainActor
    private static func chartPlotStyle(
        _ value: UiirValue,
        view: AnyView,
        nodeId: String,
        sink: any UiirEventSink
    ) -> AnyView {
        // Runtime stores expanded ChartPlotContent (+ modifiers) or a marker leaf.
        if let node = value.asNode {
            if node.kind == "ChartPlotStyleContent" {
                // degraded: param-closure could not expand; no plot-area styling.
                return view
            }
            // Apply the collected plot-area modifiers onto the real ChartPlotContent.
            return AnyView(view.chartPlotStyle { plotArea in
                ModifierApply.apply(
                    node.modifiers,
                    to: AnyView(plotArea),
                    nodeId: nodeId,
                    sink: sink
                )
            })
        }
        // degraded: unrecognized plot-style payload; leave default plot area.
        return view
    }

    /// `.chartXSelection(value: $binding)` — best-effort constant selection.
    @MainActor
    private static func chartXSelection(_ value: UiirValue, view: AnyView) -> AnyView {
        // degraded: UIIR carries a snapshot Binding(_StateBox) Display string, not a
        // live round-trip channel to the runtime. Apply .constant from the boxed
        // initial value so the modifier is present (selection gestures will not
        // write back to @State).
        if #available(iOS 17.0, macOS 14.0, *) {
            let initial = parseBindingInitialString(value)
            return AnyView(view.chartXSelection(value: .constant(initial)))
        }
        // degraded: chartXSelection requires iOS 17+; package floor is iOS 16.
        return view
    }

    /// Extract optional string from `Binding(box: _StateBox(value: …))` Display.
    private static func parseBindingInitialString(_ value: UiirValue) -> String? {
        if case let .string(s) = value {
            // nil / Optional.none
            if s.contains("value: nil") { return nil }
            // Quoted string value inside _StateBox.
            if let r = s.range(of: "value: \"") {
                let rest = s[r.upperBound...]
                if let end = rest.firstIndex(of: "\"") {
                    return String(rest[..<end])
                }
            }
            // Bare token / unquoted (e.g. value: A) — uncommon for String?.
            if let r = s.range(of: "value: ") {
                var rest = String(
                    s[r.upperBound...].trimmingCharacters(in: .whitespacesAndNewlines)
                )
                if let cut = rest.firstIndex(where: { $0 == ")" || $0 == "," }) {
                    rest = String(rest[..<cut])
                }
                let token = rest.trimmingCharacters(in: .whitespacesAndNewlines)
                if token == "nil" || token.isEmpty { return nil }
                return token
            }
            return nil
        }
        // Structured Binding object if ever emitted.
        if case let .object(o) = value {
            if let box = o["box"] {
                if case let .object(b) = box {
                    if case .null? = b["value"] { return nil }
                    return b["value"]?.stringValue
                }
            }
            return o["value"]?.stringValue
        }
        return nil
    }

    private static func isVisibilityHidden(_ value: UiirValue) -> Bool {
        if case let .string(s) = value {
            if s == "hidden" { return true }
            // `Visibility(token: hidden)`
            if s.range(of: "token:\\s*hidden", options: .regularExpression) != nil {
                return true
            }
            if s.contains("hidden") && s.contains("Visibility") { return true }
            return false
        }
        if case let .token(_, name) = value, name == "hidden" { return true }
        if case let .object(o) = value {
            if case let .string(n)? = o["token"], n == "hidden" { return true }
            if case let .string(n)? = o["name"], n == "hidden" { return true }
        }
        return false
    }

    private static func chartLabelString(_ value: UiirValue) -> String? {
        if case let .string(s) = value { return s }
        // Builder form may nest a Text node.
        if let node = value.asNode, node.kind == "Text" {
            return node.args["verbatim"]?.stringValue
        }
        if case let .object(o) = value {
            if case let .string(s)? = o["verbatim"] { return s }
            if let inner = o["value"]?.asNode, inner.kind == "Text" {
                return inner.args["verbatim"]?.stringValue
            }
        }
        return nil
    }
}
