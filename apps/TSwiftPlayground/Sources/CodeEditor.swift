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
        textView.theme = MonospaceTheme()
        textView.backgroundColor = .secondarySystemBackground
        textView.contentInset = UIEdgeInsets(top: 8, left: 4, bottom: 8, right: 4)
        textView.text = text
        return textView
    }

    func updateUIView(_ textView: TextView, context: Context) {
        // Only push external changes (e.g. loading a sample) — never echo the
        // user's own keystrokes back, which would reset the caret.
        if textView.text != text {
            textView.text = text
        }
        textView.highlightedRanges = Self.highlightedRanges(for: diagnostics, in: textView.text)
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

        init(text: Binding<String>) {
            self.text = text
        }

        func textViewDidChange(_ textView: TextView) {
            text.wrappedValue = textView.text
        }
    }
}

/// A minimal monospaced theme. Runestone requires a `Theme`; this gives system
/// monospaced text on a neutral background with no language-specific colors
/// (highlighting is a deferred follow-up).
private final class MonospaceTheme: Runestone.Theme {
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

    func textColor(for highlightName: String) -> UIColor? { nil }
}
