import SwiftUI

struct FruitList: View {
    let fruits = ["Apple", "Banana", "Cherry"]

    var body: some View {
        NavigationStack {
            List {
                ForEach(fruits, id: \.self) { fruit in
                    NavigationLink(fruit) {
                        Text("Selected \\(fruit)")
                    }
                }
            }
        }
    }
}

struct ListApp: App {
    var body: some Scene {
        WindowGroup {
            FruitList()
        }
    }
}

ListApp.main()
