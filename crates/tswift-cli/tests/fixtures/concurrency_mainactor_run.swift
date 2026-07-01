// `MainActor.run { }` hops to the main actor and returns the closure's result;
// on the cooperative single-threaded executor it runs inline.
@MainActor
final class Model {
    var value = 0
    func bump() { value += 1 }
}

func run() async {
    let sum = await MainActor.run { 20 + 22 }
    print("sum \(sum)")

    let model = await MainActor.run { Model() }
    await MainActor.run { model.bump() }
    await MainActor.run { model.bump() }
    let v = await MainActor.run { model.value }
    print("value \(v)")
}
run()
