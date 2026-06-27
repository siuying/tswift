// `unowned` references do not retain their referent and are read directly
// (no optional unwrap), used for non-optional back-references.
class Customer {
    let name: String
    var card: Card?
    init(name: String) {
        self.name = name
    }
}

class Card {
    let number: Int
    unowned let holder: Customer
    init(number: Int, holder: Customer) {
        self.number = number
        self.holder = holder
    }
    func describe() -> String {
        "card \(number) held by \(holder.name)"
    }
}

let ann = Customer(name: "Ann")
ann.card = Card(number: 1234, holder: ann)
print(ann.card!.describe())
print(ann.card!.holder.name)
