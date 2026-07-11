// Starter projects for the Web Studio. Each is a plain `{ name, files }` seed
// fed to `createProject`. Every sample here is verified to run/render in the
// shipped playground wasm (see website/test/studio.mjs's sample checks and the
// wasm smoke coverage) — samples are trimmed to what the runtime actually
// supports today rather than aspirational Swift.
//
// SwiftData + SwiftUI: `@Query` / `.modelContainer(for:inMemory:)` render live
// through the wasm `swiftUICompile*` path once the `tswift.db` host service is
// installed (the render session now installs the host-call handler before
// SwiftData, so the `tswift.db.*` wire registers — see
// crates/tswift-wasm/src/swiftui.rs). Both a console SwiftData sample and a
// live SwiftData-backed SwiftUI list ship below.

export const SAMPLES = [
  {
    id: 'console',
    name: 'Hello (SwiftPM)',
    description: 'A multi-file console package with an optional Package.swift.',
    files: [
      {
        path: 'Package.swift',
        source: `// swift-tools-version:5.9
// Optional in Studio — the runtime compiles the .swift files directly and this
// manifest is excluded from the compilation unit. It documents the package for
// a real SwiftPM build. Studio understands the Sources/ layout enough to run.
import PackageDescription

let package = Package(
    name: "Hello",
    targets: [.executableTarget(name: "Hello", path: "Sources")]
)
`,
      },
      {
        path: 'Sources/main.swift',
        source: `// Top-level code — the entry point. Compiled together with the other files.
let greeter = Greeter(name: "Studio")
print(greeter.greeting())

for planet in Planet.allCases {
    print("• \\(planet.label) is \\(planet.distanceAU) AU from the Sun")
}
`,
      },
      {
        path: 'Sources/Greeter.swift',
        source: `struct Greeter {
    let name: String
    func greeting() -> String {
        "Hello from \\(name)! 👋"
    }
}
`,
      },
      {
        path: 'Sources/Planet.swift',
        source: `enum Planet: CaseIterable {
    case mercury, venus, earth, mars

    var label: String {
        switch self {
        case .mercury: return "Mercury"
        case .venus:   return "Venus"
        case .earth:   return "Earth"
        case .mars:    return "Mars"
        }
    }

    var distanceAU: Double {
        switch self {
        case .mercury: return 0.39
        case .venus:   return 0.72
        case .earth:   return 1.00
        case .mars:    return 1.52
        }
    }
}
`,
      },
    ],
  },

  {
    id: 'swiftui-counter',
    name: 'SwiftUI Counter',
    description: 'A live SwiftUI view driven by @State.',
    files: [
      {
        path: 'CounterView.swift',
        source: `import SwiftUI

struct CounterView: View {
    @State private var count = 0

    var body: some View {
        VStack(spacing: 16) {
            Text("\\(count)")
                .font(.largeTitle)
                .fontWeight(.bold)
            HStack(spacing: 12) {
                Button("−") { count -= 1 }
                    .foregroundColor(.white)
                    .padding()
                    .background(Color.red)
                    .cornerRadius(8)
                Button("+") { count += 1 }
                    .foregroundColor(.white)
                    .padding()
                    .background(Color.blue)
                    .cornerRadius(8)
            }
        }
        .padding()
    }
}
`,
      },
    ],
  },

  {
    id: 'swiftui-todo',
    name: 'SwiftUI Todo',
    description: 'A live SwiftUI todo list driven by @State.',
    files: [
      {
        path: 'TodoView.swift',
        source: `import SwiftUI

struct TodoView: View {
    @State private var tasks = ["Buy milk", "Ship Studio", "Star the repo"]
    @State private var added = 0

    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            Text("Todos")
                .font(.largeTitle)
                .fontWeight(.bold)

            ForEach(tasks, id: \\.self) { task in
                HStack {
                    Text("•")
                    Text(task)
                    Spacer()
                }
            }

            Text("\\(tasks.count) items")
                .foregroundColor(.secondary)

            Button("Add task") {
                added += 1
                tasks.append("New task \\(added)")
            }
            .foregroundColor(.white)
            .padding()
            .background(Color.blue)
            .cornerRadius(8)
        }
        .padding()
    }
}
`,
      },
    ],
  },

  {
    id: 'swiftdata-swiftui',
    name: 'SwiftData + SwiftUI',
    description: 'A live SwiftUI list backed by SwiftData @Query + .modelContainer.',
    files: [
      {
        path: 'Note.swift',
        source: `import SwiftData

@Model
class Note {
    var title: String
    init(title: String) { self.title = title }
}
`,
      },
      {
        path: 'NoteListView.swift',
        source: `import SwiftData
import SwiftUI

// A SwiftData-backed list: @Query reads the environment's model context, and
// the "Add note" button inserts + saves. Because the SwiftUI session
// re-evaluates \`body\` on every dispatch, the new row appears with no explicit
// change-notification hook.
struct NoteListView: View {
    @Query(sort: \\.title) var notes: [Note]

    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            Text("Notes")
                .font(.largeTitle)
                .fontWeight(.bold)

            ForEach(notes) { note in
                HStack {
                    Text("•")
                    Text(note.title)
                    Spacer()
                }
            }

            Text("\\(notes.count) notes")
                .foregroundColor(.secondary)

            Button("Add note") {
                if let ctx = try? __tswiftCurrentModelContext() {
                    ctx.insert(Note(title: "Note \\(notes.count + 1)"))
                    try? ctx.save()
                }
            }
            .foregroundColor(.white)
            .padding()
            .background(Color.blue)
            .cornerRadius(8)
        }
        .padding()
    }
}

struct RootView: View {
    var body: some View {
        NoteListView()
            .modelContainer(for: Note.self, inMemory: true)
    }
}
`,
      },
    ],
  },

  {
    id: 'swiftdata',
    name: 'SwiftData (console)',
    description: 'SwiftData @Model insert / save / fetch against an in-memory store.',
    files: [
      {
        path: 'TodoItem.swift',
        source: `import SwiftData

@Model
class TodoItem {
    var title: String
    var done: Bool
    init(title: String, done: Bool = false) {
        self.title = title
        self.done = done
    }
}
`,
      },
      {
        path: 'main.swift',
        source: `import SwiftData

// An in-memory SwiftData store: insert two rows, save, then fetch them back.
do {
    let container = try ModelContainer(
        for: TodoItem.self,
        configurations: ModelConfiguration(isStoredInMemoryOnly: true))
    let context = container.mainContext

    context.insert(TodoItem(title: "Buy milk"))
    context.insert(TodoItem(title: "Ship Studio", done: true))
    try context.save()
    print("inserted 2 todos")

    let items = try context.fetch(FetchDescriptor<TodoItem>())
    print("fetched \\(items.count):")
    for item in items {
        print("  \\(item.done ? "[x]" : "[ ]") \\(item.title)")
    }
} catch {
    print("error: \\(error)")
}
`,
      },
    ],
  },
];

/** Look up a sample by id (defaults to the first). */
export function sampleById(id) {
  return SAMPLES.find((s) => s.id === id) || SAMPLES[0];
}
