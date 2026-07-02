// Synthesized Codable across nested structs, arrays, optionals, and
// RawRepresentable enums.
import Foundation

struct User: Codable, Equatable {
    let id: Int
    let name: String
    let active: Bool
}

let u = User(id: 1, name: "Ann", active: true)
let data = try! JSONEncoder().encode(u)
print(String(data: data, encoding: .utf8)!)
print(try! JSONDecoder().decode(User.self, from: data) == u)

// Nested struct + array + optional fields round-trip with element types.
struct Team: Codable {
    let name: String
    let members: [User]
    let lead: User?
}
let t = Team(name: "X", members: [u], lead: u)
let td = try! JSONEncoder().encode(t)
print(String(data: td, encoding: .utf8)!)
let tdec = try! JSONDecoder().decode(Team.self, from: td)
print(tdec.members.count, tdec.members[0] == u, tdec.lead?.name ?? "nil")

// A RawRepresentable enum encodes/decodes as its raw value.
enum Role: String, Codable { case admin, guest }
struct Account: Codable, Equatable { let role: Role; let backup: Role? }
let acc = Account(role: .admin, backup: nil)
let ad = try! JSONEncoder().encode(acc)
print(String(data: ad, encoding: .utf8)!)
let adec = try! JSONDecoder().decode(Account.self, from: "{\"role\":\"guest\",\"backup\":\"admin\"}")
print(adec.role == .guest, adec.backup == .admin)

// Custom raw value: decode by raw value OR by case name; encode uses raw value.
enum Status: String, Codable { case active = "ACTIVE", inactive }
struct Record: Codable { let status: Status }
print(String(data: try! JSONEncoder().encode(Record(status: .active)), encoding: .utf8)!)
print(try! JSONDecoder().decode(Record.self, from: "{\"status\":\"ACTIVE\"}").status == .active)
print(try! JSONDecoder().decode(Record.self, from: "{\"status\":\"active\"}").status == .active)

// Int raw value enum.
enum Level: Int, Codable { case low = 1, high = 9 }
struct Setting: Codable { let level: Level }
print(String(data: try! JSONEncoder().encode(Setting(level: .high)), encoding: .utf8)!)
print(try! JSONDecoder().decode(Setting.self, from: "{\"level\":9}").level == .high)

// Payload-free enum without raw values encodes/decodes by case name.
enum Dir: Codable { case north, south }
struct Move: Codable { let dir: Dir }
print(String(data: try! JSONEncoder().encode(Move(dir: .north)), encoding: .utf8)!)
print(try! JSONDecoder().decode(Move.self, from: "{\"dir\":\"south\"}").dir == .south)
