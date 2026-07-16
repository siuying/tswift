import SwiftData

// ModelContext change tracking, fetchCount, rollback, and transaction against
// real SQLite (the CLI backs `tswift.db.*` with the system libsqlite3).

struct HostError: Error { let message: String }

@Model
class Book {
    var title: String
    var pages: Int
    init(title: String, pages: Int) {
        self.title = title
        self.pages = pages
    }
}

do {
    let config = ModelConfiguration(isStoredInMemoryOnly: true)
    let container = try ModelContainer(for: Book.self, configurations: config)
    let ctx = container.mainContext

    // Fresh context has no changes.
    print("hasChanges (empty): \(ctx.hasChanges)")

    // Pending inserts are tracked before save.
    let a = Book(title: "Dune", pages: 412)
    let b = Book(title: "Hyperion", pages: 482)
    ctx.insert(a)
    ctx.insert(b)
    print("hasChanges (inserted): \(ctx.hasChanges)")
    print("insertedCount: \(ctx.insertedModelsArray.count)")

    try ctx.save()
    print("hasChanges (saved): \(ctx.hasChanges)")

    // fetchCount reflects the store.
    print("fetchCount: \(try ctx.fetchCount(FetchDescriptor<Book>()))")

    // Mutating a persisted object marks it changed.
    a.pages = 500
    print("changedCount: \(ctx.changedModelsArray.count)")
    print("hasChanges (dirty): \(ctx.hasChanges)")

    // rollback reverts the dirty edit.
    ctx.rollback()
    print("pages after rollback: \(a.pages)")
    print("hasChanges (rolled back): \(ctx.hasChanges)")

    // Deleting marks the object for deletion.
    ctx.delete(b)
    print("deletedCount: \(ctx.deletedModelsArray.count)")
    ctx.rollback()
    print("deletedCount after rollback: \(ctx.deletedModelsArray.count)")
    print("fetchCount after rollback: \(try ctx.fetchCount(FetchDescriptor<Book>()))")

    // transaction commits the whole body atomically.
    try ctx.transaction {
        ctx.insert(Book(title: "Neuromancer", pages: 271))
        ctx.insert(Book(title: "Snow Crash", pages: 480))
    }
    print("fetchCount after transaction: \(try ctx.fetchCount(FetchDescriptor<Book>()))")
    print("hasChanges after transaction: \(ctx.hasChanges)")

    // ModelConfiguration value-type properties reflect init state.
    print("config inMemory: \(config.isStoredInMemoryOnly)")
    let named = ModelConfiguration("Library", isStoredInMemoryOnly: true)
    print("config name: \(named.name ?? "nil")")

    // FetchDescriptor properties reflect its configured state, and fetchOffset
    // paginates the SELECT.
    var descriptor = FetchDescriptor<Book>(sortBy: [SortDescriptor(\.pages)])
    descriptor.fetchLimit = 2
    descriptor.fetchOffset = 1
    print("descriptor limit: \(descriptor.fetchLimit ?? -1)")
    print("descriptor offset: \(descriptor.fetchOffset)")
    print("descriptor sortBy count: \(descriptor.sortBy.count)")
    print("descriptor has predicate: \(descriptor.predicate != nil)")
    let page = try ctx.fetch(descriptor)
    print("paged titles: \(page.map { $0.title })")
} catch {
    print("unexpected: \(error)")
}
