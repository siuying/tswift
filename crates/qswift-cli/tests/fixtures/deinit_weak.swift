class Resource {
    let id: Int
    init(_ i: Int) { id = i }
    deinit { print("releasing \(id)") }
}
func use() {
    let r = Resource(1)
    print("using \(r.id)")
}
use()
print("done")
