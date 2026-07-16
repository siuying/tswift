// Form tab — a TextField bound to @State, echoed live in a greeting, plus a
// masked SecureField. Typing emits `set` events that write through the binding.
import SwiftUI

struct FormView: View {
    @State private var name = "World"
    @State private var secret = ""

    var body: some View {
        VStack {
            TextField("Name", text: $name)
            SecureField("Password", text: $secret)
            Text("Hello, \(name)!")
        }
    }
}
