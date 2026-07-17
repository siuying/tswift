import SwiftUI

struct FileDocumentModifiers: View {
    @State private var shown = false
    @State private var crown = 0.0

    var body: some View {
        VStack {
            Text("Import")
                .fileImporter(isPresented: $shown, allowedContentTypes: []) { _ in }
            Text("Export")
                .fileExporter(isPresented: $shown, document: nil, contentType: nil) { _ in }
            Text("Move")
                .fileMover(isPresented: $shown, file: nil) { _ in }
            Text("Dialog")
                .dismissalConfirmationDialog("Discard?", isPresented: $shown) { }
            Text("Crown")
                .digitalCrownRotation($crown)
        }
    }
}
