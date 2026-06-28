import SwiftUI

/// Applies an ordered UIIR modifier list onto a view, mirroring the order and
/// token resolution of `applyModifiers` in
/// `web/swiftui-canvas/src/modifier-css.ts` — but onto real SwiftUI modifiers.
///
/// Order matters (`.padding().background()` != `.background().padding()`), so we
/// fold the list left-to-right, type-erasing each step.
@MainActor
enum ModifierApply {
    static func apply(_ modifiers: [UiirModifier], to view: AnyView) -> AnyView {
        var out = view
        for mod in modifiers {
            out = applyOne(mod, to: out)
        }
        return out
    }

    private static func applyOne(_ mod: UiirModifier, to view: AnyView) -> AnyView {
        switch mod.name {
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
            if let c = mod.value.asColor {
                return AnyView(view.background(c))
            }
        case "cornerRadius":
            if let r = mod.value.asLength {
                return AnyView(view.cornerRadius(r))
            }
        case "padding":
            // `.padding` -> null means default padding.
            if let p = mod.value.asLength {
                return AnyView(view.padding(p))
            }
            return AnyView(view.padding())
        case "frame":
            let w = mod.value.member("width")?.asLength
            let h = mod.value.member("height")?.asLength
            return AnyView(view.frame(width: w, height: h))
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
