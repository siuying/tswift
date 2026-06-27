import Foundation

/// A patch op from `tswift swiftui dispatch`, mirroring the union in
/// `web/swiftui-canvas/src/apply-patch.ts` (`docs/plan/swiftui-support.md` §3.2).
public enum Patch: Decodable, Equatable {
    case mount(node: UiirNode)
    case insert(parentId: String, index: Int, node: UiirNode)
    case remove(id: String)
    case replace(id: String, node: UiirNode)
    case setText(id: String, text: String)
    case setModifiers(id: String, modifiers: [UiirModifier])
    case setArgs(id: String, args: [String: UiirValue])
    case move(parentId: String, id: String, index: Int)

    enum CodingKeys: String, CodingKey {
        case op, node, parentId, index, id, text, modifiers, args
    }

    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        let op = try c.decode(String.self, forKey: .op)
        switch op {
        case "mount":
            self = .mount(node: try c.decode(UiirNode.self, forKey: .node))
        case "insert":
            self = .insert(
                parentId: try c.decode(String.self, forKey: .parentId),
                index: try c.decode(Int.self, forKey: .index),
                node: try c.decode(UiirNode.self, forKey: .node)
            )
        case "remove":
            self = .remove(id: try c.decode(String.self, forKey: .id))
        case "replace":
            self = .replace(
                id: try c.decode(String.self, forKey: .id),
                node: try c.decode(UiirNode.self, forKey: .node)
            )
        case "setText":
            self = .setText(
                id: try c.decode(String.self, forKey: .id),
                text: try c.decode(String.self, forKey: .text)
            )
        case "setModifiers":
            self = .setModifiers(
                id: try c.decode(String.self, forKey: .id),
                modifiers: try c.decode([UiirModifier].self, forKey: .modifiers)
            )
        case "setArgs":
            self = .setArgs(
                id: try c.decode(String.self, forKey: .id),
                args: try c.decode([String: UiirValue].self, forKey: .args)
            )
        case "move":
            self = .move(
                parentId: try c.decode(String.self, forKey: .parentId),
                id: try c.decode(String.self, forKey: .id),
                index: try c.decode(Int.self, forKey: .index)
            )
        default:
            throw DecodingError.dataCorruptedError(
                forKey: .op, in: c,
                debugDescription: "Unknown patch op \(op)"
            )
        }
    }
}
