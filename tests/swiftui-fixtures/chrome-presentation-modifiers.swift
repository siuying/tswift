import SwiftUI

struct ChromePresentationModifiers: View {
    @State private var findShown = false

    var body: some View {
        VStack {
            Text("Bar")
                .toolbar {
                    Button("Save") {}
                }
            Text("Table")
                .tableColumnHeaders(.hidden)
            Text("Presentation")
                .presentationPreventsAppTermination(true)
            Text("TouchBar")
                .touchBarCustomizationLabel(Text("Label"))
            Text("Find")
                .findNavigator(isPresented: $findShown)
        }
    }
}
