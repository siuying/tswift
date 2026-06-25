// expected-no-diagnostics
// Tier 3 — designated/convenience, two-phase init, required, failable.

class Vehicle {
    var wheels: Int
    init(wheels: Int) { self.wheels = wheels }
    convenience init() { self.init(wheels: 4) }
}

class Car: Vehicle {
    var brand: String
    init(brand: String) {
        self.brand = brand        // phase 1: initialise own stored properties
        super.init(wheels: 4)     // phase 2: delegate up
    }
}

class Registered {
    let id: Int
    required init(id: Int) { self.id = id }
}

class Account: Registered {
    required init(id: Int) { super.init(id: id) }
}

struct Percent {
    let value: Int
    init?(_ raw: Int) {
        guard (0 ... 100).contains(raw) else { return nil }
        self.value = raw
    }
}

let car = Car(brand: "Acme")
let anyVehicle = Vehicle()
let account = Account(id: 7)
let valid = Percent(50)
let invalid = Percent(150)

let _ = (car.brand, car.wheels, anyVehicle.wheels, account.id, valid?.value, invalid?.value)
