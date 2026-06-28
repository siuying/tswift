import SwiftUI

/// A mutable UIIR tree that applies patches in place, mirroring
/// `web/swiftui-canvas/src/apply-patch.ts`. The snapshot host view observes it.
@MainActor
public final class RenderModel: ObservableObject {
    @Published public private(set) var root: UiirNode

    public init(root: UiirNode) {
        self.root = root
    }

    /// Apply an ordered batch of patches (one event step).
    public func apply(_ patches: [Patch]) {
        for patch in patches { applyOne(patch) }
    }

    private func applyOne(_ patch: Patch) {
        switch patch {
        case let .mount(node):
            root = node
        case let .setText(id, text):
            mutate(id) { node in
                node = UiirNode(
                    id: node.id, kind: node.kind,
                    args: merged(node.args, ["verbatim": .string(text)]),
                    modifiers: node.modifiers, children: node.children
                )
            }
        case let .setModifiers(id, modifiers):
            mutate(id) { node in
                node = UiirNode(
                    id: node.id, kind: node.kind, args: node.args,
                    modifiers: modifiers, children: node.children
                )
            }
        case let .setArgs(id, args):
            // A whole-args replacement (matches the runtime's `args_json`, which
            // emits every visible arg, and the web applier). Merging would leak
            // a stale arg when one disappears (e.g. `ScrollView(.horizontal)` ->
            // `ScrollView {}` must drop `axes`).
            mutate(id) { node in
                node = UiirNode(
                    id: node.id, kind: node.kind,
                    args: args,
                    modifiers: node.modifiers, children: node.children
                )
            }
        case let .replace(id, node):
            mutate(id) { $0 = node }
        case let .insert(parentId, index, node):
            mutateChildren(parentId) { children in
                let i = min(max(index, 0), children.count)
                children.insert(node, at: i)
            }
        case let .remove(id):
            guard let parentId = parentId(of: id) else { return }
            mutateChildren(parentId) { children in
                children.removeAll { $0.id == id }
            }
        case let .move(parentId, id, index):
            mutateChildren(parentId) { children in
                guard let from = children.firstIndex(where: { $0.id == id }) else { return }
                let node = children.remove(at: from)
                let i = min(max(index, 0), children.count)
                children.insert(node, at: i)
            }
        }
    }

    // MARK: - Tree helpers

    private func merged(
        _ base: [String: UiirValue], _ delta: [String: UiirValue]
    ) -> [String: UiirValue] {
        var out = base
        for (k, v) in delta { out[k] = v }
        return out
    }

    /// Find the node with `id` and rewrite it in place.
    private func mutate(_ id: String, _ body: (inout UiirNode) -> Void) {
        var copy = root
        if Self.rewrite(&copy, id: id, body) { root = copy }
    }

    /// Find the node with `id` and rewrite its `children` in place.
    private func mutateChildren(_ id: String, _ body: (inout [UiirNode]) -> Void) {
        mutate(id) { node in
            var kids = node.children
            body(&kids)
            node = UiirNode(
                id: node.id, kind: node.kind, args: node.args,
                modifiers: node.modifiers, children: kids
            )
        }
    }

    private func parentId(of id: String) -> String? {
        // Structural-path ids: "0.1.2" -> parent "0.1".
        guard let dot = id.lastIndex(of: ".") else { return nil }
        return String(id[..<dot])
    }

    @discardableResult
    private static func rewrite(
        _ node: inout UiirNode, id: String, _ body: (inout UiirNode) -> Void
    ) -> Bool {
        if node.id == id {
            body(&node)
            return true
        }
        for i in node.children.indices {
            if rewrite(&node.children[i], id: id, body) { return true }
        }
        return false
    }
}
