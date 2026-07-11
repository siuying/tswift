// Two sibling `.modelContainer(for: Note.self, inMemory: true)` on distinct
// views. Each modifier instance owns its own in-memory database, so a row
// seeded through one pane's scoped context must not appear in the other pane's
// `@Query`. Each pane seeds one row the first time its body renders (guarded by
// `isEmpty`, so re-renders stay idempotent) — the insert flows through the
// context published for *that* pane's subtree, proving no context leaks across
// siblings and that in-memory containers get per-site identity. The render
// golden shows PaneA holding only its `A-*` row and PaneB only its `B-*` row.

import SwiftData
import SwiftUI

@Model
class Note {
    var title: String
    init(title: String) { self.title = title }
}

struct PaneA: View {
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
            ctx.insert(Note(title: "A-1"))
            try? ctx.save()
        }
    }
}

struct PaneB: View {
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
            ctx.insert(Note(title: "B-1"))
            try? ctx.save()
        }
    }
}

struct RootView: View {
    var body: some View {
        VStack {
            PaneA().modelContainer(for: Note.self, inMemory: true)
            PaneB().modelContainer(for: Note.self, inMemory: true)
        }
    }
}
