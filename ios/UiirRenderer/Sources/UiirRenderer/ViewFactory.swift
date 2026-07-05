import Foundation
import SwiftUI

/// Builds a real `SwiftUI.View` (type-erased) from a UIIR node. The kind table
/// mirrors `element()` in `web/swiftui-canvas/src/apply-patch.ts`, but lowers to
/// SwiftUI primitives instead of DOM. Interactive controls forward their
/// interactions to an injected `UiirEventSink`; the default no-op sink keeps
/// static snapshots inert (identical to the pre-seam renderer).
@MainActor
public enum ViewFactory {
    public static func render(
        _ node: UiirNode,
        eventSink: any UiirEventSink = NoopEventSink()
    ) -> AnyView {
        let base = build(node, eventSink)
        return ModifierApply.apply(node.modifiers, to: base, nodeId: node.id, sink: eventSink)
    }

    private static func children(_ node: UiirNode) -> [UiirNode] { node.children }

    private static func arg(_ node: UiirNode, _ key: String) -> UiirValue? {
        node.args[key]
    }

    private static func str(_ node: UiirNode, _ key: String, _ fallback: String = "") -> String {
        arg(node, key)?.stringValue ?? fallback
    }

    private static func num(_ node: UiirNode, _ key: String, _ fallback: Double = 0) -> Double {
        arg(node, key)?.doubleValue ?? fallback
    }

    /// An optional length arg (e.g. stack `spacing:`, `Spacer(minLength:)`) ->
    /// `CGFloat?`, preserving SwiftUI's default when the arg is absent.
    private static func optLength(_ node: UiirNode, _ key: String) -> CGFloat? {
        arg(node, key)?.doubleValue.map { CGFloat($0) }
    }

    /// A `VStack`'s `alignment:` (a `HorizontalAlignment` token); default center.
    private static func hAlignment(_ node: UiirNode) -> HorizontalAlignment {
        guard case let .token(tag, name)? = arg(node, "alignment"), tag == "hAlign" else {
            return .center
        }
        switch name {
        case "leading": return .leading
        case "trailing": return .trailing
        default: return .center
        }
    }

    /// An `HStack`'s `alignment:` (a `VerticalAlignment` token); default center.
    private static func vAlignment(_ node: UiirNode) -> VerticalAlignment {
        guard case let .token(tag, name)? = arg(node, "alignment"), tag == "vAlign" else {
            return .center
        }
        switch name {
        case "top": return .top
        case "bottom": return .bottom
        case "firstTextBaseline": return .firstTextBaseline
        case "lastTextBaseline": return .lastTextBaseline
        default: return .center
        }
    }

    /// A `ZStack`'s 2-D `alignment:` token; default center.
    private static func zAlignment(_ node: UiirNode) -> Alignment {
        arg(node, "alignment")?.asAlignment ?? .center
    }

    /// `ScrollView` scroll axes from its `axes` token arg; default vertical.
    private static func scrollAxes(_ node: UiirNode) -> Axis.Set {
        if case let .token(tag, name)? = arg(node, "axes"), tag == "axis", name == "horizontal" {
            return .horizontal
        }
        return .vertical
    }

    private static func bool(_ node: UiirNode, _ key: String, _ fallback: Bool = false) -> Bool {
        arg(node, key)?.boolValue ?? fallback
    }

    @ViewBuilder
    private static func renderChildren(_ node: UiirNode, _ sink: any UiirEventSink) -> some View {
        ForEach(node.children, id: \.id) { child in
            render(child, eventSink: sink)
        }
    }

    /// The tap action a `Button` node forwards. Internal so it can be tested
    /// without driving SwiftUI.
    static func tapAction(_ node: UiirNode, _ sink: any UiirEventSink) -> () -> Void {
        { sink.send(.tap(node.id)) }
    }

    /// A `Binding` whose getter reflects the node's current value (so rendering
    /// is identical to the old `.constant`) and whose setter forwards a `set`
    /// event carrying `encode(newValue)` as a raw JSON scalar.
    static func controlBinding<T>(
        _ node: UiirNode,
        _ sink: any UiirEventSink,
        value: T,
        encode: @escaping (T) -> String
    ) -> Binding<T> {
        Binding(
            get: { value },
            set: { newValue in sink.send(.set(node.id, encode(newValue))) }
        )
    }

    /// Encode `s` as a JSON string scalar (properly escaping quotes,
    /// backslashes, and control characters such as newline/tab).
    private static func jsonString(_ s: String) -> String {
        if let data = try? JSONSerialization.data(withJSONObject: s, options: [.fragmentsAllowed]),
           let encoded = String(data: data, encoding: .utf8) {
            return encoded
        }
        return "\"\""
    }

    private static func build(_ node: UiirNode, _ sink: any UiirEventSink) -> AnyView {
        switch node.kind {
        case "Text":
            return AnyView(Text(str(node, "verbatim")))

        case "Button":
            return AnyView(Button(str(node, "title"), action: tapAction(node, sink)))

        case "Toggle":
            return AnyView(
                Toggle(str(node, "title", str(node, "label")),
                       isOn: controlBinding(node, sink, value: bool(node, "isOn"),
                                            encode: { $0 ? "true" : "false" }))
            )

        case "Slider":
            let lo = num(node, "lowerBound", 0)
            let hi = num(node, "upperBound", 1)
            let step = num(node, "step", 0)
            let value = num(node, "value", lo)
            let binding = controlBinding(node, sink, value: value, encode: { String($0) })
            if step > 0 {
                return AnyView(Slider(value: binding, in: lo...hi, step: step))
            }
            return AnyView(Slider(value: binding, in: lo...hi))

        case "Stepper":
            let lo = num(node, "lowerBound", 0)
            let hi = num(node, "upperBound", 100)
            let step = num(node, "step", 1)
            let value = num(node, "value", lo)
            let title = str(node, "title")
            return AnyView(
                Stepper("\(title): \(Int(value))",
                        value: controlBinding(node, sink, value: value, encode: { String($0) }),
                        in: lo...hi, step: step)
            )

        case "TextField":
            return AnyView(
                TextField(str(node, "title"),
                          text: controlBinding(node, sink, value: str(node, "text"),
                                               encode: jsonString))
                    .textFieldStyle(.roundedBorder)
            )

        case "SecureField":
            return AnyView(
                SecureField(str(node, "title"),
                            text: controlBinding(node, sink, value: str(node, "text"),
                                                 encode: jsonString))
                    .textFieldStyle(.roundedBorder)
            )

        case "Picker":
            return AnyView(
                Picker(str(node, "title"),
                       selection: controlBinding(node, sink, value: str(node, "selection"),
                                                 encode: jsonString)) {
                    ForEach(node.children, id: \.id) { child in
                        Text(child.args["verbatim"]?.stringValue ?? "")
                            .tag(tagValue(child))
                    }
                }
            )

        case "TabView":
            return AnyView(tabView(node, sink))

        case "VStack":
            return AnyView(
                VStack(alignment: hAlignment(node), spacing: optLength(node, "spacing")) {
                    renderChildren(node, sink)
                })
        case "HStack":
            return AnyView(
                HStack(alignment: vAlignment(node), spacing: optLength(node, "spacing")) {
                    renderChildren(node, sink)
                })
        case "ZStack":
            return AnyView(ZStack(alignment: zAlignment(node)) { renderChildren(node, sink) })
        case "Spacer":
            return AnyView(Spacer(minLength: optLength(node, "minLength")))
        case "Group":
            return AnyView(Group { renderChildren(node, sink) })
        case "Divider":
            return AnyView(Divider())
        case "ScrollView":
            return AnyView(ScrollView(scrollAxes(node)) { renderChildren(node, sink) })
        case "Label":
            return AnyView(Label(str(node, "title"), systemImage: str(node, "systemImage")))
        case "Image":
            if case let .string(systemName)? = arg(node, "systemName") {
                return AnyView(Image(systemName: systemName))
            }
            return AnyView(Image(str(node, "name")))
        case "ProgressView":
            // The optional title label uses the string-title initializers (#206).
            let progressLabel = arg(node, "label")?.stringValue
            if case let .number(value)? = arg(node, "value") {
                let total = arg(node, "total")?.doubleValue ?? 1
                if let progressLabel {
                    return AnyView(ProgressView(progressLabel, value: value, total: total))
                }
                return AnyView(ProgressView(value: value, total: total))
            }
            if let progressLabel {
                return AnyView(ProgressView(progressLabel))
            }
            return AnyView(ProgressView())
        // C6 — lazy stacks, grids, Form.
        case "LazyVStack":
            return AnyView(
                LazyVStack(alignment: hAlignment(node), spacing: optLength(node, "spacing")) {
                    renderChildren(node, sink)
                })
        case "LazyHStack":
            return AnyView(
                LazyHStack(alignment: vAlignment(node), spacing: optLength(node, "spacing")) {
                    renderChildren(node, sink)
                })
        case "LazyVGrid":
            return AnyView(
                LazyVGrid(
                    columns: arg(node, "columns")?.asGridItems ?? [GridItem(.flexible())],
                    alignment: hAlignment(node),
                    spacing: optLength(node, "spacing")
                ) { renderChildren(node, sink) })
        case "LazyHGrid":
            return AnyView(
                LazyHGrid(
                    rows: arg(node, "rows")?.asGridItems ?? [GridItem(.flexible())],
                    alignment: vAlignment(node),
                    spacing: optLength(node, "spacing")
                ) { renderChildren(node, sink) })
        case "Grid":
            return AnyView(Grid { renderChildren(node, sink) })
        case "GridRow":
            return AnyView(GridRow { renderChildren(node, sink) })
        case "Form":
            return AnyView(Form { renderChildren(node, sink) })

        case "ForEach":
            return AnyView(renderChildren(node, sink))

        case "List":
            return AnyView(List { renderChildren(node, sink) })

        case "Section":
            let header = str(node, "header")
            return AnyView(
                Section(header: Text(header)) { renderChildren(node, sink) }
            )

        case "Circle":
            return AnyView(fillShape(node, Circle()))
        case "Ellipse":
            return AnyView(fillShape(node, Ellipse()))
        case "Rectangle":
            return AnyView(fillShape(node, Rectangle()))
        case "Capsule":
            return AnyView(fillShape(node, Capsule()))
        case "RoundedRectangle":
            let r = num(node, "cornerRadius", 0)
            return AnyView(fillShape(node, RoundedRectangle(cornerRadius: r)))

        default:
            // Unknown kind: render its children transparently.
            return AnyView(renderChildren(node, sink))
        }
    }

    /// A shape, tinted by its `fill` modifier if present (the ShapeStyle overload
    /// that a chained `.foregroundColor` can't express on a bare `Shape`).
    private static func fillShape<S: Shape>(_ node: UiirNode, _ shape: S) -> AnyView {
        if let fill = node.modifiers.first(where: { $0.name == "fill" })?.value.asColor {
            return AnyView(shape.fill(fill))
        }
        return AnyView(shape)
    }

    /// A native `TabView` driven by the children + their `tabItem` labels
    /// (ADR-0013 §2). Selection is runtime-owned: the binding reflects the
    /// node's `selection` arg and forwards a `select` event on change. Tabs are
    /// tagged with a canonical `type:value` key so a `String` selection binding
    /// matches whether the runtime tag is a string or a number/index.
    private static func tabView(_ node: UiirNode, _ sink: any UiirEventSink) -> some View {
        let currentKey = tabKey(node.args["selection"] ?? .null)
        let selection = Binding<String>(
            get: { currentKey },
            set: { newKey in sink.send(.select(node.id, decodeTabKey(newKey))) }
        )
        return TabView(selection: selection) {
            ForEach(Array(node.children.enumerated()), id: \.offset) { index, child in
                render(child, eventSink: sink)
                    .tabItem { tabItemLabel(child) }
                    .tag(tabChildKey(child, index))
            }
        }
    }

    /// A tab's bar label from its `.tabItem { … }` marker (a nested view node).
    @ViewBuilder
    private static func tabItemLabel(_ child: UiirNode) -> some View {
        if let marker = child.modifiers.first(where: { $0.name == "tabItem" }),
           let node = marker.value.asNode {
            render(node, eventSink: NoopEventSink())
        } else {
            EmptyView()
        }
    }

    /// A canonical `type:value` selection key for a tab identity value, so a
    /// `String`-typed native selection matches whether the runtime carries a
    /// string tag or a number/index (`s:home`, `n:1.0`).
    private static func tabKey(_ v: UiirValue) -> String {
        switch v {
        case let .string(s): return "s:" + s
        case let .number(n): return "n:" + String(n)
        default: return ""
        }
    }

    /// A tab child's selection key: its `.tag(_)` value if present, else its
    /// index (as a number key, matching the runtime's index selection default).
    private static func tabChildKey(_ child: UiirNode, _ index: Int) -> String {
        if let tag = child.modifiers.first(where: { $0.name == "tag" }) {
            let key = tabKey(tag.value)
            if !key.isEmpty { return key }
        }
        return "n:" + String(Double(index))
    }

    /// Decode a tab key back into the raw JSON scalar the runtime expects for a
    /// `select` event: a string tag as a quoted JSON string, a number/index as
    /// its bare numeric literal.
    private static func decodeTabKey(_ key: String) -> String {
        if key.hasPrefix("s:") { return jsonString(String(key.dropFirst(2))) }
        if key.hasPrefix("n:") { return String(key.dropFirst(2)) }
        return "\"\""
    }

    /// A Picker option's tag value (string from the `tag` modifier).
    private static func tagValue(_ child: UiirNode) -> String {
        if let tag = child.modifiers.first(where: { $0.name == "tag" }) {
            switch tag.value {
            case let .string(s): return s
            case let .number(n): return String(n)
            default: return ""
            }
        }
        return ""
    }
}
