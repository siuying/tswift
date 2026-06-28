// Synthesized Codable across nested structs, arrays, optionals, and
// RawRepresentable enums.
struct User: Codable, Equatable {
    let id: Int
    let name: String
    let active: Bool
}

let u = User(id: 1, name: "Ann", active: true)
let data = try! JSONEncoder().encode(u)
print(data)
print(try! JSONDecoder().decode(User.self, from: data) == u)

// Nested struct + array + optional fields round-trip with element types.
struct Team: Codable {
    let name: String
    let members: [User]
    let lead: User?
}
let t = Team(name: "X", members: [u], lead: u)
let td = try! JSONEncoder().encode(t)
print(td)
let tdec = try! JSONDecoder().decode(Team.self, from: td)
print(tdec.members.count, tdec.members[0] == u, tdec.lead?.name ?? "nil")

// A RawRepresentable enum encodes/decodes as its raw value.
enum Role: String, Codable { case admin, guest }
struct Account: Codable, Equatable { let role: Role; let backup: Role? }
let acc = Account(role: .admin, backup: nil)
let ad = try! JSONEncoder().encode(acc)
print(ad)
let adec = try! JSONDecoder().decode(Account.self, from: "{\"role\":\"guest\",\"backup\":\"admin\"}")
print(adec.role == .guest, adec.backup == .admin)
