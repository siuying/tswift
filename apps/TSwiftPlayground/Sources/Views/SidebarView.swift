import SwiftUI
import TSwiftCore

/// The Studio sidebar: a **Files** section (select/create/rename/delete) and a
/// **Symbols** outline (tap to jump the editor to the declaration).
struct SidebarView: View {
    @ObservedObject var model: StudioModel
    @State private var showingNewFile = false
    @State private var newFileName = ""
    @State private var renaming: String?
    @State private var renameText = ""
    @State private var deleting: String?

    var body: some View {
        List {
            filesSection
            symbolsSection
        }
        .listStyle(.sidebar)
        .toolbar {
            ToolbarItem(placement: .topBarTrailing) {
                Button {
                    newFileName = ""
                    showingNewFile = true
                } label: { Image(systemName: "doc.badge.plus") }
            }
        }
        .alert("New File", isPresented: $showingNewFile) {
            TextField("name.swift", text: $newFileName)
            Button("Create") { model.createFile(newFileName) }
            Button("Cancel", role: .cancel) {}
        }
        .alert("Rename File", isPresented: Binding(
            get: { renaming != nil }, set: { if !$0 { renaming = nil } }
        )) {
            TextField("name.swift", text: $renameText)
            Button("Rename") {
                if let old = renaming { model.renameFile(old, to: renameText) }
                renaming = nil
            }
            Button("Cancel", role: .cancel) { renaming = nil }
        }
        .confirmationDialog(
            "Delete \u{201C}\(deleting ?? "")\u{201D}?",
            isPresented: Binding(
                get: { deleting != nil },
                set: { if !$0 { deleting = nil } }
            ),
            titleVisibility: .visible
        ) {
            Button("Delete", role: .destructive) {
                if let name = deleting { model.deleteFile(name) }
                deleting = nil
            }
            Button("Cancel", role: .cancel) { deleting = nil }
        } message: {
            Text("This cannot be undone.")
        }
    }

    // MARK: Files

    private var filesSection: some View {
        Section("Files") {
            ForEach(model.files) { file in
                Button {
                    model.select(file.name)
                } label: {
                    Label(file.name, systemImage: fileIcon(file.name))
                        .foregroundStyle(file.name == model.selectedFileName ? Color.accentColor : .primary)
                }
                .swipeActions(edge: .trailing) {
                    Button(role: .destructive) {
                        deleting = file.name
                    } label: { Label("Delete", systemImage: "trash") }
                    Button {
                        renaming = file.name
                        renameText = file.name
                    } label: { Label("Rename", systemImage: "pencil") }
                    .tint(.blue)
                }
            }
        }
    }

    private func fileIcon(_ name: String) -> String {
        name == Project.entryFileName ? "play.square" : "doc.text"
    }

    // MARK: Symbols

    @ViewBuilder
    private var symbolsSection: some View {
        Section("Symbols") {
            if model.symbols.isEmpty {
                Text("No symbols")
                    .foregroundStyle(.secondary)
                    .font(.caption)
            } else {
                ForEach(model.symbols) { symbol in
                    Button {
                        model.jump(to: symbol)
                    } label: {
                        SymbolRow(symbol: symbol)
                    }
                }
            }
        }
    }
}

/// One row in the symbol outline: an icon by kind, the name (indented if
/// nested), and its file:line.
struct SymbolRow: View {
    let symbol: TSwiftSymbol

    var body: some View {
        HStack(spacing: 6) {
            if symbol.container != nil {
                Spacer().frame(width: 14)
            }
            Image(systemName: icon)
                .foregroundStyle(color)
                .font(.caption)
            VStack(alignment: .leading, spacing: 1) {
                Text(symbol.name)
                    .font(.callout)
                    .foregroundStyle(.primary)
                Text("\(symbol.file):\(symbol.line)")
                    .font(.caption2.monospaced())
                    .foregroundStyle(.secondary)
            }
        }
    }

    private var icon: String {
        switch symbol.kind {
        case "struct": return "square.stack.3d.up"
        case "class": return "cube"
        case "enum": return "list.bullet.rectangle"
        case "protocol": return "p.circle"
        case "func": return "function"
        case "var", "let": return "v.square"
        case "case": return "c.circle"
        case "init": return "wrench"
        case "subscript": return "number.square"
        default: return "curlybraces"
        }
    }

    private var color: Color {
        switch symbol.kind {
        case "struct", "class", "enum", "protocol": return .teal
        case "func", "init", "subscript": return .indigo
        case "var", "let": return .blue
        case "case": return .orange
        default: return .secondary
        }
    }
}
