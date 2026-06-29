import Runestone
import SwiftUI
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
