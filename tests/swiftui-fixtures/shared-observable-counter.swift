import SwiftUI

class CounterModel: ObservableObject {
    @Published var count = 0
    func increment() { count += 1 }
}

struct CounterButton: View {
    @ObservedObject var model: CounterModel

    var body: some View {
        Button("Increment \(model.count)") { model.increment() }
    }
}

struct SharedObservableCounter: View {
    @StateObject var model = CounterModel()

    var body: some View {
        VStack {
            Text("Total: \(model.count)")
            CounterButton(model: model)
            Text("Mirror: \(model.count)")
        }
    }
}
