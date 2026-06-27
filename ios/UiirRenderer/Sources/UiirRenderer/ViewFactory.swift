import SwiftUI

/// Builds a real `SwiftUI.View` (type-erased) from a UIIR node. The kind table
/// mirrors `element()` in `web/swiftui-canvas/src/apply-patch.ts`, but lowers to
/// SwiftUI primitives instead of DOM. Actions are no-ops — snapshots are static.
@MainActor
public enum ViewFactory {
    public static func render(_ node: UiirNode) -> AnyView {
        let base = build(node)
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

    private static func bool(_ node: UiirNode, _ key: String, _ fallback: Bool = false) -> Bool {
        arg(node, key)?.boolValue ?? fallback
    }

    @ViewBuilder
    private static func renderChildren(_ node: UiirNode) -> some View {
        ForEach(node.children, id: \.id) { child in
            render(child)
        }
    }

    private static func build(_ node: UiirNode) -> AnyView {
        switch node.kind {
        case "Text":
            return AnyView(Text(str(node, "verbatim")))

        case "Button":
            return AnyView(Button(str(node, "title")) {})

        case "Toggle":
            return AnyView(
                Toggle(str(node, "title", str(node, "label")),
                       isOn: .constant(bool(node, "isOn")))
            )

        case "Slider":
            let lo = num(node, "lowerBound", 0)
            let hi = num(node, "upperBound", 1)
            let step = num(node, "step", 0)
            let value = num(node, "value", lo)
            if step > 0 {
                return AnyView(Slider(value: .constant(value), in: lo...hi, step: step))
            }
            return AnyView(Slider(value: .constant(value), in: lo...hi))

        case "Stepper":
            let lo = num(node, "lowerBound", 0)
            let hi = num(node, "upperBound", 100)
            let step = num(node, "step", 1)
            let value = num(node, "value", lo)
            let title = str(node, "title")
            return AnyView(
                Stepper("\(title): \(Int(value))",
                        value: .constant(value), in: lo...hi, step: step)
            )

        case "TextField":
            return AnyView(
                TextField(str(node, "title"), text: .constant(str(node, "text")))
                    .textFieldStyle(.roundedBorder)
            )

        case "SecureField":
            return AnyView(
                SecureField(str(node, "title"), text: .constant(str(node, "text")))
                    .textFieldStyle(.roundedBorder)
            )

        case "Picker":
            return AnyView(
                Picker(str(node, "title"), selection: .constant(str(node, "selection"))) {
                    ForEach(node.children, id: \.id) { child in
                        Text(child.args["verbatim"]?.stringValue ?? "")
                            .tag(tagValue(child))
                    }
                }
            )

        case "VStack":
            return AnyView(VStack { renderChildren(node) })
        case "HStack":
            return AnyView(HStack { renderChildren(node) })
        case "ZStack":
            return AnyView(ZStack { renderChildren(node) })
        case "Spacer":
            return AnyView(Spacer())

        case "ForEach":
            return AnyView(renderChildren(node))

        case "List":
            return AnyView(List { renderChildren(node) })

        case "Section":
            let header = str(node, "header")
            return AnyView(
                Section(header: Text(header)) { renderChildren(node) }
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
            return AnyView(renderChildren(node))
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
