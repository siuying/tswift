import Foundation

/// A UIIR modifier/arg value: the tagged-union wire encoding from
/// `docs/plan/swiftui-support.md` §3.1 and `web/swiftui-canvas/src/modifier-css.ts`.
///
///     null | number | string | bool | { "$": tag, "name": name } | { key: value }
public indirect enum UiirValue: Decodable, Equatable {
    case null
    case number(Double)
    case string(String)
    case bool(Bool)
    /// A semantic token: `{ "$": "color", "name": "indigo" }`.
    case token(tag: String, name: String)
    /// An arbitrary object, e.g. `.frame` -> `{ "width": 200, "height": 120 }`.
    case object([String: UiirValue])

    public init(from decoder: Decoder) throws {
        let container = try decoder.singleValueContainer()
        if container.decodeNil() {
            self = .null
        } else if let b = try? container.decode(Bool.self) {
            self = .bool(b)
        } else if let n = try? container.decode(Double.self) {
            self = .number(n)
        } else if let s = try? container.decode(String.self) {
            self = .string(s)
        } else if let dict = try? container.decode([String: UiirValue].self) {
            if case let .string(tag)? = dict["$"], case let .string(name)? = dict["name"] {
                self = .token(tag: tag, name: name)
            } else {
                self = .object(dict)
            }
        } else {
            throw DecodingError.dataCorruptedError(
                in: container,
                debugDescription: "Unrecognized UiirValue"
            )
        }
    }

    // MARK: - Convenience accessors

    public var stringValue: String? {
        if case let .string(s) = self { return s }
        return nil
    }

    public var doubleValue: Double? {
        if case let .number(n) = self { return n }
        return nil
    }

    public var boolValue: Bool? {
        if case let .bool(b) = self { return b }
        return nil
    }

    public func member(_ key: String) -> UiirValue? {
        if case let .object(o) = self { return o[key] }
        return nil
    }
}
