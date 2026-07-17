import SwiftUI

struct ItemProviderModifiers: View {
    var body: some View {
        VStack {
            Text("Transfer")
                .copyable(["item"])
                .cuttable { return ["item"] }
                .pasteDestination(for: String.self) { _ in }
            Text("Providers")
                .itemProvider { nil }
                .userActivity("com.example.activity") { _ in }
                .exportsItemProviders(["public.text"]) { return [] }
                .importsItemProviders(["public.text"]) { _ in }
        }
    }
}
