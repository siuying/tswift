// expected-no-diagnostics
// Tier 2c — optionals: T?, binding, force unwrap, chaining, coalescing, IUO,
// and the `case let x?` pattern.

struct User {
    var name: String
    var nickname: String?
}

func displayName(_ user: User?) -> String {
    guard let user = user else { return "no one" }
    if let nick = user.nickname {
        return nick
    }
    return user.nickname ?? user.name
}

let maybe: Int? = 5
let forced = maybe!
let chainLength = maybe?.description.count

let iuo: Int! = 10
let viaIUO = iuo + 1

var optionals: [Int?] = [1, nil, 3]
if case let value? = optionals[0] {
    optionals.append(value)
}

let _ = (displayName(User(name: "ada", nickname: nil)), forced, chainLength, viaIUO, optionals.count)