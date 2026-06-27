import Foundation

/// One UIIR modifier: an ordered `{ name, value }` pair (`docs/plan/swiftui-support.md` §3.1).
public struct UiirModifier: Decodable, Equatable {
    public let name: String
    public let value: UiirValue

    public init(name: String, value: UiirValue) {
        self.name = name
        self.value = value
    }

    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        name = try c.decode(String.self, forKey: .name)
        // `value` may be absent or explicit null.
        value = try c.decodeIfPresent(UiirValue.self, forKey: .value) ?? .null
    }

    enum CodingKeys: String, CodingKey { case name, value }
}

/// A node in the UIIR view tree (`docs/plan/swiftui-support.md` §3.1).
public struct UiirNode: Decodable, Equatable {
    public let id: String
    public let kind: String
    public let args: [String: UiirValue]
    public let modifiers: [UiirModifier]
    public var children: [UiirNode]

    public init(
        id: String,
        kind: String,
        args: [String: UiirValue] = [:],
        modifiers: [UiirModifier] = [],
        children: [UiirNode] = []
    ) {
        self.id = id
        self.kind = kind
        self.args = args
        self.modifiers = modifiers
        self.children = children
    }

    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        id = try c.decode(String.self, forKey: .id)
        kind = try c.decode(String.self, forKey: .kind)
        args = try c.decodeIfPresent([String: UiirValue].self, forKey: .args) ?? [:]
        modifiers = try c.decodeIfPresent([UiirModifier].self, forKey: .modifiers) ?? []
        children = try c.decodeIfPresent([UiirNode].self, forKey: .children) ?? []
    }

    enum CodingKeys: String, CodingKey { case id, kind, args, modifiers, children }
}
