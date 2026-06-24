struct Account {
    var balance: Int = 0 {
        willSet { print("will: \(balance) -> \(newValue)") }
        didSet { print("did: \(oldValue) -> \(balance)") }
    }
    var inDollars: Int { get { balance / 100 } set { balance = newValue * 100 } }
}
var acc = Account()
acc.balance = 500
print(acc.inDollars)
acc.inDollars = 7
print(acc.balance)
