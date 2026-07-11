import Runestone
import SwiftUI
import TSwiftUI
import UIKit

/// A SwiftUI wrapper over Runestone's `TextView`: a monospaced, line-numbered
/// code editor with autocorrect/autocapitalize off. Text edits flow back through
/// the `text` binding; the playground debounces recompiles on top of that.
///
/// Syntax highlighting (a Tree-sitter Swift grammar) is intentionally deferred
/// per the roadmap — Runestone renders perfectly readable plain monospaced text
/// without a language set, which keeps the first cut dependency-light.
struct CodeEditor: UIViewRepresentable {
    @Binding var text: String
    /// Frontend diagnostics to mark inline (error/warning underlines on the
    /// affected source range). Empty leaves the text unmarked.
    var diagnostics: [PreviewSession.Diagnostic] = []
    /// A pending jump request (from tapping a symbol in the outline). When it
    /// changes, the editor moves the caret to that 1-based line.
    var jumpTarget: JumpTarget?

    func makeUIView(context: Context) -> TextView {
        let textView = TextView()
        textView.editorDelegate = context.coordinator
        textView.showLineNumbers = true
        textView.lineSelectionDisplayType = .line
        textView.autocorrectionType = .no
        textView.autocapitalizationType = .none
        textView.smartQuotesType = .no
        textView.smartDashesType = .no
        textView.smartInsertDeleteType = .no
        textView.spellCheckingType = .no
        textView.isLineWrappingEnabled = false
        textView.keyboardType = .asciiCapable
        textView.theme = SwiftEditorTheme()
        textView.backgroundColor = .secondarySystemBackground
        textView.contentInset = UIEdgeInsets(top: 8, left: 4, bottom: 8, right: 4)
        // Drive syntax highlighting with the tree-sitter Swift grammar.
        textView.setState(TextViewState(text: text, theme: SwiftEditorTheme(), language: .swift))
        return textView
    }

    func updateUIView(_ textView: TextView, context: Context) {
        // Only push external changes (e.g. loading a sample / switching files) —
        // never echo the user's own keystrokes back, which would reset the caret.
        if textView.text != text {
            textView.text = text
        }
        textView.highlightedRanges = Self.highlightedRanges(for: diagnostics, in: textView.text)
        // Apply a jump request once per distinct token.
        if let target = jumpTarget, target != context.coordinator.lastJump {
            context.coordinator.lastJump = target
            Self.jump(textView, toLine: target.line)
        }
    }

    /// Move the caret to the start of a 1-based `line` and scroll it into view.
    /// Out-of-range lines are ignored.
    static func jump(_ textView: TextView, toLine line: Int) {
        let ns = textView.text as NSString
        var lineStarts: [Int] = [0]
        ns.enumerateSubstrings(in: NSRange(location: 0, length: ns.length),
                               options: [.byLines, .substringNotRequired]) { _, _, enclosing, _ in
            lineStarts.append(enclosing.location + enclosing.length)
        }
        guard line >= 1, line <= lineStarts.count else { return }
        let offset = min(lineStarts[line - 1], ns.length)
        textView.selectedRange = NSRange(location: offset, length: 0)
    }

    /// Map diagnostics (1-based line/col) to Runestone highlight ranges: a tinted
    /// underline from the error column to the end of that line. Out-of-range
    /// lines are skipped so a stale diagnostic never crashes the mapping.
    static func highlightedRanges(
        for diagnostics: [PreviewSession.Diagnostic],
        in text: String
    ) -> [HighlightedRange] {
        guard !diagnostics.isEmpty else { return [] }
        let ns = text as NSString
        // Byte/line scan once: the UTF-16 start offset of each 1-based line.
        var lineStarts: [Int] = [0]
        ns.enumerateSubstrings(in: NSRange(location: 0, length: ns.length),
                               options: [.byLines, .substringNotRequired]) { _, _, enclosing, _ in
            lineStarts.append(enclosing.location + enclosing.length)
        }
        return diagnostics.compactMap { d in
            guard d.line >= 1, d.line <= lineStarts.count else { return nil }
            let lineStart = lineStarts[d.line - 1]
            let lineEnd = d.line < lineStarts.count ? lineStarts[d.line] : ns.length
            let from = min(lineStart + max(d.col - 1, 0), lineEnd)
            let length = max(lineEnd - from, 1)
            guard from + length <= ns.length || from < ns.length else { return nil }
            let range = NSRange(location: from, length: min(length, ns.length - from))
            let color: UIColor = d.isError
                ? UIColor.systemRed.withAlphaComponent(0.28)
                : UIColor.systemOrange.withAlphaComponent(0.25)
            return HighlightedRange(range: range, color: color)
        }
    }

    func makeCoordinator() -> Coordinator {
        Coordinator(text: $text)
    }

    final class Coordinator: NSObject, TextViewDelegate {
        private let text: Binding<String>
        /// The last jump request applied, so the same request isn't re-run on
        /// every unrelated `updateUIView`.
        var lastJump: JumpTarget?

        init(text: Binding<String>) {
            self.text = text
        }

        func textViewDidChange(_ textView: TextView) {
            text.wrappedValue = textView.text
        }
    }
}

/// A monospaced theme that colours the tree-sitter Swift capture names
/// (`keyword`, `string`, `comment`, `type`, …) emitted by `swift-highlights.scm`.
private final class SwiftEditorTheme: Runestone.Theme {
    let font: UIFont = .monospacedSystemFont(ofSize: 14, weight: .regular)
    let textColor: UIColor = .label

    let gutterBackgroundColor: UIColor = .secondarySystemBackground
    let gutterHairlineColor: UIColor = .separator

    let lineNumberColor: UIColor = .tertiaryLabel
    let lineNumberFont: UIFont = .monospacedSystemFont(ofSize: 12, weight: .regular)

    let selectedLineBackgroundColor: UIColor = .tertiarySystemBackground
    let selectedLinesLineNumberColor: UIColor = .secondaryLabel
    let selectedLinesGutterBackgroundColor: UIColor = .secondarySystemBackground

    let invisibleCharactersColor: UIColor = .quaternaryLabel

    let pageGuideHairlineColor: UIColor = .separator
    let pageGuideBackgroundColor: UIColor = .secondarySystemBackground

    let markedTextBackgroundColor: UIColor = .systemFill

    /// Map a tree-sitter capture name to a colour. Names are dotted
    /// (`keyword.return`, `punctuation.bracket`); match on the leading segment so
    /// every `keyword.*` shares the keyword colour, etc. Unknown names fall back
    /// to the default text colour (return nil).
    func textColor(for highlightName: String) -> UIColor? {
        let root = highlightName.split(separator: ".").first.map(String.init) ?? highlightName
        switch root {
        case "keyword": return UIColor(red: 0.78, green: 0.33, blue: 0.55, alpha: 1) // magenta-pink
        case "string", "character": return UIColor(red: 0.78, green: 0.30, blue: 0.27, alpha: 1) // red
        case "comment": return UIColor.systemGray
        case "type", "constructor": return UIColor(red: 0.20, green: 0.49, blue: 0.62, alpha: 1) // teal
        case "number", "boolean", "constant": return UIColor(red: 0.10, green: 0.40, blue: 0.80, alpha: 1) // blue
        case "function": return UIColor(red: 0.36, green: 0.36, blue: 0.84, alpha: 1) // indigo
        case "attribute": return UIColor(red: 0.55, green: 0.36, blue: 0.07, alpha: 1) // brown
        case "operator", "punctuation": return UIColor.secondaryLabel
        case "variable":
            // Only tint the special variable kinds; plain identifiers stay default.
            if highlightName.hasPrefix("variable.builtin") {
                return UIColor(red: 0.78, green: 0.33, blue: 0.55, alpha: 1)
            }
            return nil
        case "label": return UIColor.systemTeal
        default: return nil
        }
    }
}
