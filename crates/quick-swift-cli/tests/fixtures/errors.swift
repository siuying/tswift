enum FileError: Error {
    case notFound(String)
    case permissionDenied
}
func read(_ name: String) throws -> String {
    guard name == "a.txt" else { throw FileError.notFound(name) }
    return "data"
}
do {
    defer { print("cleanup") }
    print(try read("x.txt"))
} catch FileError.notFound(let n) {
    print("missing \(n)")
} catch {
    print("other error")
}
print(try read("a.txt"))
