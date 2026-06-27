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
}
