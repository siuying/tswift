// expected-no-diagnostics
// Tier 3 — weak/unowned references, deinit, identity operators.

class ListNode {
    var value: Int
    var next: ListNode?
    weak var prev: ListNode?
    init(_ value: Int) { self.value = value }
    deinit { print("releasing \(value)") }
}

class Resource {
    let id: Int
    init(id: Int) { self.id = id }
}

class Consumer {
    unowned let resource: Resource
    init(_ resource: Resource) { self.resource = resource }
}

let first = ListNode(1)
let second = ListNode(2)
first.next = second
second.prev = first

let sameNode = (first === first)
let differentNode = (first !== second)

let shared = Resource(id: 99)
let consumer = Consumer(shared)

let _ = (first.value, second.prev?.value, sameNode, differentNode, consumer.resource.id)
