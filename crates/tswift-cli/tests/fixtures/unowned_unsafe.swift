// `unowned(unsafe)` references: a non-owning reference that does not retain its
// referent (like `unowned`, without the runtime safety check).
class Owner {
    var name: String
    init(_ n: String) { name = n }
}
class Ref {
    unowned(unsafe) var owner: Owner
    init(_ o: Owner) { owner = o }
}
let o = Owner("root")
let r = Ref(o)
print(r.owner.name)
print(r.owner === o)
