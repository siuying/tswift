struct User: Codable {
    let name: String
    let age: Int
}
struct Team: Codable {
    let title: String
    let size: Int
}
@main
struct App {
    static func main() throws {
        let u = User(name: "Sam", age: 30)
        let data = try JSONEncoder().encode(u)
        print(data)
        let back = try JSONDecoder().decode(User.self, from: data)
        print(back.name, back.age)
        let t = try JSONDecoder().decode(Team.self, from: "{\"title\":\"Eng\",\"size\":12}")
        print(t.title, t.size)
    }
}
