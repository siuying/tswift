// Observation tab — MVVM with an ObservableObject view model owned by a
// @StateObject. Button actions mutate @Published state; the view re-renders.
import SwiftUI

class CounterModel: ObservableObject {
    @Published var count = 0
    func increment() { count += 1 }
    func decrement() { count -= 1 }
}

struct ObservableView: View {
    @StateObject var model = CounterModel()

    var body: some View {
        VStack {
            Text("Count: \(model.count)")
                .font(.largeTitle)
            HStack {
                Button("−") { model.decrement() }
                Button("+") { model.increment() }
            }
        }
    }
}
