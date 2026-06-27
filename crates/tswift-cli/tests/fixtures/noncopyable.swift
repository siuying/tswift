// Suppressed constraints `~Copyable` / `~Escapable` on types, generic
// parameters, and protocols. They are no-ops in the tree-walker.
struct FileHandle: ~Copyable {
    var fd: Int
    consuming func close() {
        print("closing \(fd)")
        discard self
    }
}

let h = FileHandle(fd: 3)
h.close()

func identity<T: ~Copyable>(_ value: consuming T) -> T { value }
print(identity(42))

struct Pair<T: ~Copyable & ~Escapable> {
    var first: T
    var second: T
}
let p = Pair(first: 1, second: 2)
print(p.first + p.second)

protocol Resource: ~Copyable {
    var name: String { get }
}
struct Socket: Resource, ~Copyable {
    let name: String
}
print(Socket(name: "tcp").name)
