@propertyWrapper
struct Capitalized {
    private var value: String = ""
    var wrappedValue: String {
        get { value }
        set { value = newValue.count > 0 ? newValue : "?" }
    }
    init(wrappedValue: String) { self.wrappedValue = wrappedValue }
}
struct User {
    @Capitalized var name: String = "sam"
}
var u = User()
print(u.name)
u.name = "alice"
print(u.name)
