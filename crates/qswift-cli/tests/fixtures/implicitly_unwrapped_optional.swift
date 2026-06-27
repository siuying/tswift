// Implicitly unwrapped optionals (`T!`) carry an optional value but are used
// directly, auto-unwrapping at each use.
var name: String! = "Ada"
print(name)
print(name.uppercased())

let x: Int! = 10
let y = x + 5
print(y)

// As a parameter and a return type.
func shout(_ s: String!) -> String! {
    return s + "!"
}
print(shout("hi"))

// Reassignment through nil and back.
var maybe: Int! = nil
maybe = 42
print(maybe + 1)
