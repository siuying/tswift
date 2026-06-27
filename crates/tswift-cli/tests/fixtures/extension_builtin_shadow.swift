// User extensions on builtin types participate before stdlib algorithms.
extension Array {
    func sorted() -> String { "shadowed" }
}

print([2, 1].sorted())
