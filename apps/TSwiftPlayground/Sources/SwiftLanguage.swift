import Foundation
import Runestone
import TreeSitterSwift

/// The Runestone Tree-sitter language for Swift: the `tree-sitter-swift` grammar
/// (`tree_sitter_swift()`) paired with its highlights query (bundled as
/// `swift-highlights.scm`). Capture names from the query are coloured by
/// `SwiftEditorTheme.textColor(for:)`.
extension TreeSitterLanguage {
    static var swift: TreeSitterLanguage {
        let highlights = Bundle.main.url(forResource: "swift-highlights", withExtension: "scm")
            .flatMap { try? String(contentsOf: $0, encoding: .utf8) }
            .map { TreeSitterLanguage.Query(string: $0) }
        return TreeSitterLanguage(tree_sitter_swift(), highlightsQuery: highlights)
    }
}
