// Class-only protocols (`: AnyObject`): only classes may conform, the
// existential holds a reference, and `weak` references to the protocol type
// participate in ARC.
protocol Observer: AnyObject {
    func notify(_ msg: String)
}
class Logger: Observer {
    var last = ""
    func notify(_ msg: String) { last = msg }
}
final class Subject {
    weak var observer: Observer?
    func fire(_ m: String) { observer?.notify(m) }
}
let l = Logger()
let s = Subject()
s.observer = l
s.fire("hello")
print(l.last)

// `AnyObject` existential + identity.
let a: AnyObject = l
let b: AnyObject = l
print(a === b)

// A class-only protocol with a property requirement.
protocol Named: AnyObject { var name: String { get } }
class Person: Named { let name = "Ada" }
let p: Named = Person()
print(p.name)
