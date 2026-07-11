import SwiftData

// End-to-end SwiftData fetch path against real SQLite (the CLI backs
// `tswift.db.*` with the system libsqlite3). Exercises `#Predicate`,
// `SortDescriptor`, `FetchDescriptor`, `fetchLimit`, the identity map, and
// mutate-a-fetched-object-then-save.

struct HostError: Error { let message: String }

@Model
class Movie {
    var title: String
    var year: Int
    init(title: String, year: Int) {
        self.title = title
        self.year = year
    }
}

do {
    let container = try ModelContainer(
        for: Movie.self,
        configurations: ModelConfiguration(isStoredInMemoryOnly: true))
    let ctx = container.mainContext

    for (title, year) in [("Arrival", 2016), ("Dune", 2021), ("Sicario", 2015), ("Blade Runner 2049", 2017)] {
        ctx.insert(Movie(title: title, year: year))
    }
    try ctx.save()

    // Filter (year > 2015) + sort (year descending).
    let recent = FetchDescriptor<Movie>(
        predicate: #Predicate<Movie> { movie in
            movie.year > 2015
        },
        sortBy: [SortDescriptor(\.year, order: .reverse)])
    let results = try ctx.fetch(recent)
    print("recent count: \(results.count)")
    for movie in results {
        print("  \(movie.year) \(movie.title)")
    }

    // fetchLimit caps the row count.
    var limited = FetchDescriptor<Movie>(sortBy: [SortDescriptor(\.year)])
    limited.fetchLimit = 2
    let firstTwo = try ctx.fetch(limited)
    print("limited: \(firstTwo.map { $0.title }.joined(separator: ", "))")

    // String predicate.
    let dunes = try ctx.fetch(FetchDescriptor<Movie>(
        predicate: #Predicate<Movie> { movie in
            movie.title.hasPrefix("D")
        }))
    print("hasPrefix D: \(dunes.map { $0.title }.joined(separator: ", "))")

    // Mutate a fetched object and save -> UPDATE by rowid; re-fetch sees it.
    if let first = results.first {
        first.year = 1999
        try ctx.save()
    }
    let reordered = try ctx.fetch(FetchDescriptor<Movie>(sortBy: [SortDescriptor(\.year)]))
    print("after edit oldest: \(reordered.first!.year) \(reordered.first!.title)")
} catch {
    print("unexpected: \(error)")
}
