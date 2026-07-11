// SwiftData `@Query` rendered with NO `.modelContainer(for:)` ancestor. The
// environment has no model context, so `__tswiftCurrentModelContext()` throws a
// clear diagnostic that `@Query`'s `try?` catches, degrading to an empty array
// — a stale/leaked context from an unrelated subtree must never surface here.

import SwiftData
import SwiftUI

@Model
class Note {
    var title: String
    init(title: String) { self.title = title }
}

struct NoteList: View {
    @Query(sort: \.title) var notes: [Note]

    var body: some View {
        List {
            ForEach(notes) { note in
                Text(note.title)
            }
        }
    }
}
