import SwiftUI

/// Applies an ordered UIIR modifier list onto a view, mirroring the order and
/// token resolution of `applyModifiers` in
/// `web/swiftui-canvas/src/modifier-css.ts` — but onto real SwiftUI modifiers.
///
/// Order matters (`.padding().background()` != `.background().padding()`), so we
/// fold the list left-to-right, type-erasing each step.
@MainActor
enum ModifierApply {
    static func apply(
        _ modifiers: [UiirModifier],
        to view: AnyView,
        nodeId: String = "",
        sink: any UiirEventSink = NoopEventSink()
    ) -> AnyView {
        var out = view
        for mod in modifiers {
            out = applyOne(mod, to: out, nodeId: nodeId, sink: sink)
        }
        return out
    }

    private static func applyOne(
        _ mod: UiirModifier,
        to view: AnyView,
        nodeId: String,
        sink: any UiirEventSink
    ) -> AnyView {
        switch mod.name {
        // Lifecycle / gesture / submit events (ADR-0013 §3): the runtime carries
        // only these markers; the captured closures live in its handler map, so
        // the host wires a real SwiftUI modifier that reports the event by name.
        // (`onChange` is runtime-internal and never reaches the UIIR.)
        case "onTapGesture":
            let count = Int(mod.value.member("count")?.doubleValue ?? 1)
            return AnyView(view.onTapGesture(count: max(1, count)) {
                sink.send(.tap(nodeId))
            })
        case "onLongPressGesture":
            let minDuration = mod.value.member("minimumDuration")?.doubleValue ?? 0.5
            return AnyView(view.onLongPressGesture(minimumDuration: minDuration) {
                sink.send(.named(nodeId, "longPress"))
            })
        case "onSubmit":
            return AnyView(view.onSubmit(of: .text) {
                sink.send(.named(nodeId, "submit"))
            })
        case "onAppear":
            return AnyView(view.onAppear { sink.send(.named(nodeId, "appear")) })
        case "onDisappear":
            return AnyView(view.onDisappear { sink.send(.named(nodeId, "disappear")) })
        case "font":
            if case let .token(tag, name) = mod.value, tag == "textStyle" {
                return AnyView(view.font(Tokens.font(name)))
            }
        case "fontWeight":
            if case let .token(tag, name) = mod.value, tag == "weight" {
                return AnyView(view.fontWeight(Tokens.weight(name)))
            }
        case "foregroundColor":
            if let c = mod.value.asColor {
                return AnyView(view.foregroundColor(c))
            }
        case "background":
            // An arbitrary nested view composites behind the receiver (#204);
            // a color/token stays the C0 fast path.
            if let comp = mod.value.asComposite {
                let content = ViewFactory.render(comp.node, eventSink: sink)
                return AnyView(view.background(alignment: comp.alignment) { content })
            }
            if let c = mod.value.asColor {
                return AnyView(view.background(c))
            }
        case "overlay":
            // An arbitrary nested view composites in front of the receiver (#204).
            if let comp = mod.value.asComposite {
                let content = ViewFactory.render(comp.node, eventSink: sink)
                return AnyView(view.overlay(alignment: comp.alignment) { content })
            }
            if let c = mod.value.asColor {
                return AnyView(view.overlay(c))
            }
        case "cornerRadius":
            if let r = mod.value.asLength {
                return AnyView(view.cornerRadius(r))
            }
        case "padding":
            // Directional `.padding(.horizontal, 8)` -> `{ value: <edge token>,
            // value1: <length?> }`; `.padding(.all)` -> a bare edge token;
            // `.padding(8)` -> a bare length; `.padding()` -> null (issue #203).
            if let edge = mod.value.asEdgeSet {
                return AnyView(view.padding(edge))
            }
            if let edge = mod.value.member("value")?.asEdgeSet {
                return AnyView(view.padding(edge, mod.value.member("value1")?.asLength))
            }
            if let p = mod.value.asLength ?? mod.value.member("value")?.asLength {
                return AnyView(view.padding(p))
            }
            return AnyView(view.padding())
        case "frame":
            // Content alignment (issue #203) threads through both overloads.
            let alignment = mod.value.member("alignment")?.asAlignment ?? .center
            // Flexible bounds may be non-finite (`.infinity`); fixed width/height
            // are always finite lengths.
            let minW = mod.value.member("minWidth")?.asFrameLength
            let maxW = mod.value.member("maxWidth")?.asFrameLength
            let minH = mod.value.member("minHeight")?.asFrameLength
            let maxH = mod.value.member("maxHeight")?.asFrameLength
            if minW != nil || maxW != nil || minH != nil || maxH != nil {
                return AnyView(view.frame(
                    minWidth: minW,
                    maxWidth: maxW,
                    minHeight: minH,
                    maxHeight: maxH,
                    alignment: alignment
                ))
            }
            let w = mod.value.member("width")?.asLength
            let h = mod.value.member("height")?.asLength
            if w != nil || h != nil {
                return AnyView(view.frame(width: w, height: h, alignment: alignment))
            }
            return view
        case "offset":
            return AnyView(view.offset(
                x: mod.value.member("x")?.asLength ?? 0,
                y: mod.value.member("y")?.asLength ?? 0
            ))
        // C4 — visual decoration.
        case "clipped":
            return AnyView(view.clipped())
        case "clipShape":
            if let shape = mod.value.asClipShape {
                return AnyView(view.clipShape(shape))
            }
        case "border":
            // `{ value: <color token>, width: n }` (positional color + width).
            let c = (mod.value.member("value") ?? mod.value).asColor ?? .primary
            let w = mod.value.member("width")?.asLength ?? 1
            return AnyView(view.border(c, width: w))
        case "shadow":
            let radius = mod.value.member("radius")?.asLength ?? 0
            let x = mod.value.member("x")?.asLength ?? 0
            let y = mod.value.member("y")?.asLength ?? 0
            if let c = mod.value.member("color")?.asColor {
                return AnyView(view.shadow(color: c, radius: radius, x: x, y: y))
            }
            return AnyView(view.shadow(radius: radius, x: x, y: y))
        case "fill":
            // `.fill` on a shape is handled in the ViewFactory (shapes need the
            // ShapeStyle overload). As a chained modifier we approximate with
            // foreground color so non-shape receivers still tint.
            if let c = mod.value.asColor {
                return AnyView(view.foregroundColor(c))
            }
        case "tag":
            // Picker option identity — applied via `.tag(...)` in the factory,
            // not as a visual modifier; ignore here.
            break
        // C7 — control styling + disabled. Accessibility modifiers are accepted
        // and ignored (no-op) so snippets using them still render.
        case "buttonStyle":
            if case let .token(tag, name) = mod.value, tag == "style" {
                switch name {
                case "borderedProminent": return AnyView(view.buttonStyle(.borderedProminent))
                case "bordered": return AnyView(view.buttonStyle(.bordered))
                case "borderless": return AnyView(view.buttonStyle(.borderless))
                case "plain": return AnyView(view.buttonStyle(.plain))
                default: return AnyView(view.buttonStyle(.automatic))
                }
            }
        case "listStyle":
            if case let .token(tag, name) = mod.value, tag == "style" {
                switch name {
                case "plain": return AnyView(view.listStyle(.plain))
                case "inset": return AnyView(view.listStyle(.inset))
                case "sidebar": return AnyView(view.listStyle(.sidebar))
                #if os(iOS)
                    case "grouped": return AnyView(view.listStyle(.grouped))
                    case "insetGrouped": return AnyView(view.listStyle(.insetGrouped))
                #endif
                default: return AnyView(view.listStyle(.automatic))
                }
            }
        case "pickerStyle":
            if case let .token(tag, name) = mod.value, tag == "style" {
                switch name {
                case "segmented": return AnyView(view.pickerStyle(.segmented))
                case "menu": return AnyView(view.pickerStyle(.menu))
                case "inline": return AnyView(view.pickerStyle(.inline))
                #if os(iOS)
                    case "wheel": return AnyView(view.pickerStyle(.wheel))
                #endif
                default: return AnyView(view.pickerStyle(.automatic))
                }
            }
        case "textFieldStyle":
            if case let .token(tag, name) = mod.value, tag == "style" {
                switch name {
                case "roundedBorder": return AnyView(view.textFieldStyle(.roundedBorder))
                case "plain": return AnyView(view.textFieldStyle(.plain))
                default: return AnyView(view.textFieldStyle(.automatic))
                }
            }
        case "disabled":
            if case let .bool(flag) = mod.value {
                return AnyView(view.disabled(flag))
            }
        // C1 — text & universal styling modifiers.
        case "bold":
            return AnyView(view.bold())
        case "italic":
            return AnyView(view.italic())
        case "underline":
            return AnyView(view.underline())
        case "strikethrough":
            return AnyView(view.strikethrough())
        case "opacity":
            if case let .number(n) = mod.value {
                return AnyView(view.opacity(n))
            }
        case "foregroundStyle":
            // v1 supports color foreground styles; gradients/materials deferred.
            if let c = mod.value.asColor {
                return AnyView(view.foregroundStyle(c))
            }
        case "tint":
            if let c = mod.value.asColor {
                return AnyView(view.tint(c))
            }
        case "lineLimit":
            if case let .number(n) = mod.value {
                return AnyView(view.lineLimit(Int(n)))
            }
        case "multilineTextAlignment":
            if let a = mod.value.asTextAlignment {
                return AnyView(view.multilineTextAlignment(a))
            }
        case "textCase":
            if let c = mod.value.asTextCase {
                return AnyView(view.textCase(c))
            }
        default:
            break
        }
        return view
    }
}
