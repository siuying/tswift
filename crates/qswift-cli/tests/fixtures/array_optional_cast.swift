// Covariant array cast `[T]` as `[T?]`, then appending nil.
struct Contact { let name: String }
let contacts = [Contact(name: "a"), Contact(name: "b")] as [Contact?] + [nil]
print(contacts.count)
for c in contacts { print(c?.name ?? "nil") }

let ints = [1, 2, 3] as [Int?]
print(ints.compactMap { $0 }.reduce(0, +))
