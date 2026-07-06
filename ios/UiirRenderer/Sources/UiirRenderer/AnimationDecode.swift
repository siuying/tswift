import SwiftUI

/// Decodes UIIR `.animation` / `.transition` payloads into real SwiftUI
/// `Animation` and `AnyTransition` values. See `anim-uiir-schema.md` for the
/// wire shape the runtime emits.
enum AnimationDecode {
    /// Decode an `ANIM` object (`{ "$":"animation","kind":…,… }`) into a
    /// SwiftUI `Animation`. Returns nil for JSON `null` (`.animation(nil,…)`
    /// disables animation) or an unrecognized payload.
    static func animation(_ value: UiirValue) -> Animation? {
        guard case let .object(o) = value else { return nil }
        guard case let .string(kind)? = o["kind"] else { return nil }

        let duration = o["duration"]?.doubleValue

        var anim: Animation
        switch kind {
        case "easeIn":
            anim = duration.map { .easeIn(duration: $0) } ?? .easeIn
        case "easeOut":
            anim = duration.map { .easeOut(duration: $0) } ?? .easeOut
        case "easeInOut":
            anim = duration.map { .easeInOut(duration: $0) } ?? .easeInOut
        case "linear":
            anim = duration.map { .linear(duration: $0) } ?? .linear
        case "spring":
            let response = o["response"]?.doubleValue
            let damping = o["dampingFraction"]?.doubleValue
            if let response, let damping {
                anim = .spring(response: response, dampingFraction: damping)
            } else {
                anim = .spring()
            }
        case "bouncy":
            anim = duration.map { .bouncy(duration: $0) } ?? .bouncy
        case "smooth":
            anim = duration.map { .smooth(duration: $0) } ?? .smooth
        case "snappy":
            anim = duration.map { .snappy(duration: $0) } ?? .snappy
        default:
            anim = duration.map { .easeInOut(duration: $0) } ?? .easeInOut
        }

        // Chained transforms — order is not observable but mirror Swift's.
        if let delay = o["delay"]?.doubleValue {
            anim = anim.delay(delay)
        }
        if let speed = o["speed"]?.doubleValue {
            anim = anim.speed(speed)
        }
        // `repeat` is either the string "forever" or an integer count; the
        // `autoreverses` flag only accompanies a repeat.
        if let rep = o["repeat"] {
            let autoreverses = o["autoreverses"]?.boolValue ?? true
            if case let .string(s) = rep, s == "forever" {
                anim = anim.repeatForever(autoreverses: autoreverses)
            } else if let count = rep.doubleValue {
                anim = anim.repeatCount(Int(count), autoreverses: autoreverses)
            }
        }
        return anim
    }

    /// Decode a `TRANS` object recursively into an `AnyTransition`. Falls back
    /// to `.identity` for unknown/missing payloads (never crashes).
    static func transition(_ value: UiirValue) -> AnyTransition {
        guard case let .object(o) = value, case let .string(type)? = o["type"] else {
            return .identity
        }
        switch type {
        case "opacity": return .opacity
        case "identity": return .identity
        case "slide": return .slide
        case "scale":
            if let s = o["scale"]?.doubleValue {
                if let anchor = o["anchor"]?.stringValue.flatMap(unitPoint) {
                    return .scale(scale: s, anchor: anchor)
                }
                return .scale(scale: s)
            }
            return .scale
        case "move":
            if let edge = o["edge"]?.stringValue.flatMap(edge) {
                return .move(edge: edge)
            }
            return .identity
        case "push":
            if let edge = o["edge"]?.stringValue.flatMap(edge) {
                return .push(from: edge)
            }
            return .identity
        case "offset":
            let x = o["x"]?.doubleValue ?? 0
            let y = o["y"]?.doubleValue ?? 0
            return .offset(x: x, y: y)
        case "combined":
            guard case let .array(items)? = o["transitions"], !items.isEmpty else {
                return .identity
            }
            return items.dropFirst().reduce(transition(items[0])) { acc, t in
                acc.combined(with: transition(t))
            }
        case "asymmetric":
            let insertion = o["insertion"].map(transition) ?? .identity
            let removal = o["removal"].map(transition) ?? .identity
            return .asymmetric(insertion: insertion, removal: removal)
        default:
            return .identity
        }
    }

    /// Map an `Edge` token string to a SwiftUI `Edge`.
    private static func edge(_ name: String) -> Edge? {
        switch name {
        case "leading": return .leading
        case "trailing": return .trailing
        case "top": return .top
        case "bottom": return .bottom
        default: return nil
        }
    }

    /// Map a `UnitPoint` token string to a SwiftUI `UnitPoint`.
    private static func unitPoint(_ name: String) -> UnitPoint? {
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
        default: return nil
        }
    }
}

extension UiirValue {
    /// A `Hashable` erasure of a scalar observed value, so `.animation(_:value:)`
    /// recomposes when the watched state changes. Non-scalars hash as a stable
    /// sentinel (they cannot animate meaningfully but must stay Hashable).
    var asObservedHashable: AnyHashable {
        switch self {
        case let .bool(b): return AnyHashable(b)
        case let .number(n): return AnyHashable(n)
        case let .string(s): return AnyHashable(s)
        case let .token(tag, name): return AnyHashable("\(tag):\(name)")
        case .null: return AnyHashable("__null__")
        default: return AnyHashable("__unhashable__")
        }
    }
}
