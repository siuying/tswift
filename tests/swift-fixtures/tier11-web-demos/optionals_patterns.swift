// expected-no-diagnostics
// Tier 11 / Web demo — Optionals: guard let, chaining, coalescing, compactMap.

struct Contact {
    var name: String
    var phone: String?
    var email: String?
}

func reachableAt(_ c: Contact?) -> String {
    guard let c = c else { return "nobody" }
    return c.phone ?? c.email ?? "\(c.name) (unreachable)"
}

let alice = Contact(name: "Alice", phone: "+1-555-0100", email: nil)
let bob   = Contact(name: "Bob",   phone: nil,          email: "bob@example.com")
let ghost = Contact(name: "Ghost", phone: nil,          email: nil)

print(reachableAt(alice))
print(reachableAt(bob))
print(reachableAt(ghost))
print(reachableAt(nil))

// Optional chaining
let length: Int? = alice.phone?.count
print("Phone length: \(length ?? 0)")

// nil-coalescing chain
let tag = ghost.email ?? ghost.phone ?? "no tag"
print("tag: \(tag)")

// compactMap filters out nils
let items: [Int?] = [1, nil, 3, nil, 5]
let nonNil = items.compactMap { $0 }
print("non-nil: \(nonNil)")
