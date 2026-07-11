// Nested `.modelContainer(for:)`: an outer container wraps the whole tree; an
// inner container wraps a nested pane. The nested pane's `@Query` must read the
// *nearest* ancestor (the inner container), while the outer pane reads the
// outer one — nearest-ancestor wins, and the inner scope is restored to the
// outer on the way back up (no leak). Each pane seeds one row through its own
// scoped context the first time it renders. The golden shows the outer pane
// with `outer-1` and the inner pane with `inner-1`, in separate databases.

import SwiftData
import SwiftUI

@Model
class Note {
    var title: String
    init(title: String) { self.title = title }
}

struct SeededList: View {
    let seed: String
    @Query(sort: \.title) var notes: [Note]

    var body: some View {
        List {
            let _ = seedIfEmpty()
            ForEach(notes) { note in
                Text(note.title)
            }
        }
    }

    func seedIfEmpty() {
        if notes.isEmpty, let ctx = try? __tswiftCurrentModelContext() {
            ctx.insert(Note(title: seed))
            try? ctx.save()
        }
    }
}

struct InnerPane: View {
    var body: some View {
        SeededList(seed: "inner-1")
            .modelContainer(for: Note.self, inMemory: true)
    }
}

struct OuterPane: View {
    var body: some View {
        VStack {
            SeededList(seed: "outer-1")
            InnerPane()
        }
    }
}

struct RootView: View {
    var body: some View {
        OuterPane()
            .modelContainer(for: Note.self, inMemory: true)
    }
}
