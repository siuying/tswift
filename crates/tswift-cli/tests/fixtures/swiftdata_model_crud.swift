import SwiftData

// End-to-end SwiftData core surface against real SQLite (the CLI backs
// `tswift.db.*` with the system libsqlite3 — see `tswift-cli/src/db.rs`).
// `fetch` is a later slice, so state is observed through the model objects and
// the fact that valid SQL executes without throwing; a deliberately-invalid
// operation exercises the throwing/rollback path.

struct HostError: Error { let message: String }

@Model
class Movie {
    var title: String
    var year: Int
    var rating: Double?
    init(title: String, year: Int, rating: Double? = nil) {
        self.title = title
        self.year = year
        self.rating = rating
    }
}

@Model
class Actor {
    var name: String
    init(name: String) { self.name = name }
}

do {
    // In-memory store, two model types.
    let config = ModelConfiguration(isStoredInMemoryOnly: true)
    let container = try ModelContainer(for: Movie.self, Actor.self, configurations: config)
    let ctx = container.mainContext

    // Insert + save.
    let m = Movie(title: "Arrival", year: 2016)
    ctx.insert(m)
    ctx.insert(Actor(name: "Amy Adams"))
    try ctx.save()
    print("saved insert")

    // Re-inserting the same object is idempotent (no duplicate row).
    ctx.insert(m)
    try ctx.save()
    print("idempotent")

    // Mutate + save -> UPDATE.
    m.year = 2017
    m.rating = 4.5
    try ctx.save()
    print("saved update")

    // Delete + save.
    ctx.delete(m)
    try ctx.save()
    print("saved delete")
} catch {
    print("unexpected: \(error)")
}

// A second, independent context over its own container.
do {
    let container = try ModelContainer(for: Movie.self, configurations: ModelConfiguration(isStoredInMemoryOnly: true))
    let ctx = ModelContext(container)
    ctx.insert(Movie(title: "Sicario", year: 2015))
    try ctx.save()
    print("second context ok")
} catch {
    print("unexpected: \(error)")
}
