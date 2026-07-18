import SwiftData
import SwiftUI

@Model
class Todo {
    var title: String

    init(title: String) {
        self.title = title
    }
}

struct TodoList: View {
    @Query(sort: \.title) var todos: [Todo]

    var body: some View {
        List {
            let _ = seedIfEmpty()
            ForEach(todos) { todo in
                Text(todo.title)
            }
        }
    }

    func seedIfEmpty() {
        if todos.isEmpty, let context = try? __tswiftCurrentModelContext() {
            context.insert(Todo(title: "Buy milk"))
            try? context.save()
        }
    }
}

@main
struct TodoApp: App {
    var body: some Scene {
        WindowGroup {
            TodoList()
        }
        .modelContainer(for: Todo.self, inMemory: true)
    }
}
