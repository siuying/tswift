import Foundation
import SwiftUI

// Value-passthrough render/effect & dialog/file-picker metadata modifiers.
// Each records a scalar / Angle+axis tuple / KeyEquivalent / nested shape or
// Image / String / Bool / URL straight onto the view node (no leading-dot
// token), exercised here so the host reads the recorded value back.
struct V: View {
    var body: some View {
        VStack(spacing: 4) {
            Text("effect")
                .luminanceToAlpha()
                .rotation3DEffect(.degrees(45), axis: (x: 0, y: 1, z: 0))
                .keyboardShortcut("s")
            Text("shape")
                .containerShape(Circle())
                .dialogIcon(Image(systemName: "trash"))
            Text("dialog")
                .fileDialogConfirmationLabel("Choose")
                .fileDialogCustomizationID("import-panel")
                .fileDialogMessage("Pick a file to import")
                .fileDialogImportsUnresolvedAliases(true)
                .fileDialogDefaultDirectory(URL(string: "file:///tmp")!)
                .toolbarItemHidden(true)
        }
    }
}
