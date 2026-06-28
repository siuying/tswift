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
        return ModifierApply.apply(node.modifiers, to: base)
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

        case "VStack":
            return AnyView(VStack(spacing: optLength(node, "spacing")) { renderChildren(node, sink) })
        case "HStack":
            return AnyView(HStack(spacing: optLength(node, "spacing")) { renderChildren(node, sink) })
        case "ZStack":
            return AnyView(ZStack { renderChildren(node, sink) })
        case "Spacer":
            return AnyView(Spacer(minLength: optLength(node, "minLength")))
        case "Group":
            return AnyView(Group { renderChildren(node, sink) })
        case "Divider":
            return AnyView(Divider())
        case "ScrollView":
            return AnyView(ScrollView(scrollAxes(node)) { renderChildren(node, sink) })

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
