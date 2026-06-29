import SwiftUI

/// Semantic-token resolution tables. These mirror the *names* in
/// `web/swiftui-canvas/src/modifier-css.ts` but map to real SwiftUI values
/// instead of CSS — this is the iOS half of the §3.1 token contract. Drift
/// between this file and `modifier-css.ts` is exactly what Layer D catches.
public enum Tokens {
    /// `.font(.largeTitle)` text styles -> `Font`.
    public static func font(_ name: String) -> Font {
        switch name {
        case "largeTitle": return .largeTitle
        case "title": return .title
        case "title2": return .title2
        case "title3": return .title3
        case "headline": return .headline
        case "subheadline": return .subheadline
        case "body": return .body
        case "callout": return .callout
        case "caption": return .caption
        case "caption2": return .caption2
        case "footnote": return .footnote
        default: return .body
        }
    }

    /// `.fontWeight(.bold)` weights -> `Font.Weight`.
    public static func weight(_ name: String) -> Font.Weight {
        switch name {
        case "ultraLight": return .ultraLight
        case "thin": return .thin
        case "light": return .light
        case "regular": return .regular
        case "medium": return .medium
        case "semibold": return .semibold
        case "bold": return .bold
        case "heavy": return .heavy
        case "black": return .black
        default: return .regular
        }
    }

    /// Named SwiftUI colors -> `Color`.
    public static func color(_ name: String) -> Color {
        switch name {
        case "primary": return .primary
        case "secondary": return .secondary
        case "white": return .white
        case "black": return .black
        case "red": return .red
        case "orange": return .orange
        case "yellow": return .yellow
        case "green": return .green
        case "mint": return .mint
        case "teal": return .teal
        case "cyan": return .cyan
        case "blue": return .blue
        case "indigo": return .indigo
        case "purple": return .purple
        case "pink": return .pink
        case "brown": return .brown
        case "gray": return .gray
        case "clear": return .clear
        default: return .primary
        }
    }
}

extension UiirValue {
    /// Resolve a `{ "$": "color", "name": ... }` token to a `Color`.
    var asColor: Color? {
        if case let .token(tag, name) = self, tag == "color" { return Tokens.color(name) }
        return nil
    }

    /// Resolve a length (numeric px) to a `CGFloat`.
    var asLength: CGFloat? {
        if case let .number(n) = self { return CGFloat(n) }
        return nil
    }

    /// Resolve a frame length, honoring the non-finite `{ "$": "infinity" }`
    /// sentinel (`.frame(maxWidth: .infinity)`, issue #203). The sentinel has a
    /// `$` but no `name`, so it decodes as an `.object`, not a `.token`.
    var asFrameLength: CGFloat? {
        if case let .number(n) = self { return CGFloat(n) }
        if case let .object(o) = self, case let .string(s)? = o["$"] {
            switch s {
            case "infinity": return .infinity
            case "-infinity": return -.infinity
            default: return nil
            }
        }
        return nil
    }

    /// Resolve a `{ "$": "align", "name": ... }` token to a 2-D `Alignment`.
    var asAlignment: Alignment? {
        guard case let .token(tag, name) = self, tag == "align" else { return nil }
        switch name {
        case "center": return .center
        case "leading": return .leading
        case "trailing": return .trailing
        case "top": return .top
        case "bottom": return .bottom
        case "topLeading": return .topLeading
        case "topTrailing": return .topTrailing
        case "bottomLeading": return .bottomLeading
        case "bottomTrailing": return .bottomTrailing
        case "leadingFirstTextBaseline": return .leadingFirstTextBaseline
        case "centerFirstTextBaseline": return .centerFirstTextBaseline
        case "trailingFirstTextBaseline": return .trailingFirstTextBaseline
        default: return .center
        }
    }

    /// Interpret this value as a nested UIIR view node — a `.background(view)` /
    /// `.overlay(view)` subtree with its own `0`-rooted id space (#204). Returns
    /// nil when it is not a node object (e.g. a color token).
    var asNode: UiirNode? {
        guard case let .object(o) = self, case let .string(kind)? = o["kind"] else { return nil }
        let id = o["id"]?.stringValue ?? "0"
        var args: [String: UiirValue] = [:]
        if case let .object(a)? = o["args"] { args = a }
        var modifiers: [UiirModifier] = []
        if case let .array(ms)? = o["modifiers"] {
            modifiers = ms.compactMap { m in
                guard case let .object(mo) = m, case let .string(name)? = mo["name"] else {
                    return nil
                }
                return UiirModifier(name: name, value: mo["value"] ?? .null)
            }
        }
        var children: [UiirNode] = []
        if case let .array(cs)? = o["children"] { children = cs.compactMap { $0.asNode } }
        return UiirNode(id: id, kind: kind, args: args, modifiers: modifiers, children: children)
    }

    /// The composite content of a `background`/`overlay` modifier value: either a
    /// bare nested node, or `{ value: <node>, alignment: <token> }` (#204).
    var asComposite: (node: UiirNode, alignment: Alignment)? {
        if let node = asNode { return (node, .center) }
        if case let .object(o) = self, let inner = o["value"]?.asNode {
            return (inner, o["alignment"]?.asAlignment ?? .center)
        }
        return nil
    }

    /// Decode a `[GridItem]` arg (a JSON array of `{ kind, value, spacing? }`)
    /// into SwiftUI `GridItem` track sizers (issue #205).
    var asGridItems: [GridItem]? {
        guard case let .array(items) = self else { return nil }
        return items.map { item in
            let kind = item.member("kind")?.stringValue ?? "flexible"
            let value = CGFloat(item.member("value")?.doubleValue ?? 0)
            let spacing = item.member("spacing")?.doubleValue.map { CGFloat($0) }
            // A finite `max` is honored; its absence means unbounded (.infinity).
            let upper = item.member("max")?.doubleValue.map { CGFloat($0) } ?? .infinity
            let size: GridItem.Size
            switch kind {
            case "fixed": size = .fixed(value)
            case "adaptive": size = .adaptive(minimum: max(value, 1), maximum: upper)
            default: size = .flexible(minimum: value, maximum: upper)
            }
            return GridItem(size, spacing: spacing)
        }
    }

    /// Resolve a `{ "$": "edge", "name": ... }` token to an `Edge.Set`.
    var asEdgeSet: Edge.Set? {
        guard case let .token(tag, name) = self, tag == "edge" else { return nil }
        switch name {
        case "top": return .top
        case "bottom": return .bottom
        case "leading": return .leading
        case "trailing": return .trailing
        case "horizontal": return .horizontal
        case "vertical": return .vertical
        case "all": return .all
        default: return .all
        }
    }

    /// Resolve a `{ "$": "textAlign", "name": ... }` token to a `TextAlignment`.
    var asTextAlignment: TextAlignment? {
        guard case let .token(tag, name) = self, tag == "textAlign" else { return nil }
        switch name {
        case "leading": return .leading
        case "center": return .center
        case "trailing": return .trailing
        default: return nil
        }
    }

    /// Resolve a nested shape descriptor (a UIIR node value carried by
    /// `.clipShape(...)`) to a type-erased `AnyShape`.
    var asClipShape: AnyShape? {
        guard case let .object(o) = self, case let .string(kind)? = o["kind"] else { return nil }
        switch kind {
        case "Circle": return AnyShape(Circle())
        case "Ellipse": return AnyShape(Ellipse())
        case "Capsule": return AnyShape(Capsule())
        case "Rectangle": return AnyShape(Rectangle())
        case "RoundedRectangle":
            let r = o["args"]?.member("cornerRadius")?.doubleValue ?? 8
            return AnyShape(RoundedRectangle(cornerRadius: CGFloat(r)))
        default: return nil
        }
    }

    /// Resolve a `{ "$": "textCase", "name": ... }` token to a `Text.Case`.
    var asTextCase: Text.Case? {
        guard case let .token(tag, name) = self, tag == "textCase" else { return nil }
        switch name {
        case "uppercase": return .uppercase
        case "lowercase": return .lowercase
        default: return nil
        }
    }
}
