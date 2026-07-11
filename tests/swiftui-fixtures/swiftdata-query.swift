// SwiftData `@Query` + `.modelContainer(for:)` in a SwiftUI render session
// (ADR-0016 Slice 10b). A root view seeds an in-memory store once, then a
// child view lists `@Query` results. Dispatching the "add" button inserts +
// saves; because `body` re-evaluates every dispatch, the re-render reflects the
// new row with no change-notification hook.

import SwiftData
import SwiftUI

@Model
class Note {
    var title: String
    init(title: String) { self.title = title }
}

struct NoteList: View {
    @Query(sort: \.title) var tasks: [Note]

    var body: some View {
        VStack {
            Button("add") {
                if let ctx = try? __tswiftCurrentModelContext() {
                    ctx.insert(Note(title: "row-\(tasks.count + 1)"))
                    try? ctx.save()
                }
            }
            List {
                ForEach(tasks) { task in
                    Text(task.title)
                }
            }
        }
    }
}

struct RootView: View {
    var body: some View {
        NoteList()
            .modelContainer(for: Note.self, inMemory: true)
    }
}
