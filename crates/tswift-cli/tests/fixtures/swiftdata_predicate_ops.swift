import SwiftData

// SwiftData #Predicate: collection membership (`IN`) and String `.isEmpty`,
// compiled to SQL and run against real SQLite.

@Model
class Movie {
    var title: String
    var genre: String
    var note: String
    init(title: String, genre: String, note: String = "") {
        self.title = title
        self.genre = genre
        self.note = note
    }
}

do {
    let container = try ModelContainer(
        for: Movie.self,
        configurations: ModelConfiguration(isStoredInMemoryOnly: true))
    let ctx = container.mainContext

    ctx.insert(Movie(title: "Arrival", genre: "SciFi", note: "great"))
    ctx.insert(Movie(title: "Dune", genre: "SciFi"))
    ctx.insert(Movie(title: "Sicario", genre: "Thriller", note: "tense"))
    ctx.insert(Movie(title: "Heat", genre: "Crime"))
    try ctx.save()

    // Collection membership -> `genre IN (?, ?)`.
    let wanted = ["SciFi", "Crime"]
    let inList = try ctx.fetch(FetchDescriptor<Movie>(
        predicate: #Predicate<Movie> { movie in
            wanted.contains(movie.genre)
        },
        sortBy: [SortDescriptor(\.title)]))
    print("in-list: \(inList.map { $0.title }.joined(separator: ", "))")

    // String isEmpty -> `(note IS NULL OR note = '')`.
    let noNote = try ctx.fetch(FetchDescriptor<Movie>(
        predicate: #Predicate<Movie> { movie in
            movie.note.isEmpty
        },
        sortBy: [SortDescriptor(\.title)]))
    print("empty-note: \(noNote.map { $0.title }.joined(separator: ", "))")

    // Negated isEmpty -> `NOT (...)`.
    let hasNote = try ctx.fetch(FetchDescriptor<Movie>(
        predicate: #Predicate<Movie> { movie in
            !movie.note.isEmpty
        },
        sortBy: [SortDescriptor(\.title)]))
    print("has-note: \(hasNote.map { $0.title }.joined(separator: ", "))")

    // Empty collection -> constant-false (no rows).
    let none: [String] = []
    let empty = try ctx.fetch(FetchDescriptor<Movie>(
        predicate: #Predicate<Movie> { movie in
            none.contains(movie.genre)
        }))
    print("empty-in: \(empty.count)")
} catch {
    print("unexpected: \(error)")
}
