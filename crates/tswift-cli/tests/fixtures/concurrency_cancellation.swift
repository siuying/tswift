// Task cancellation: the running task's `Task.isCancelled`,
// `Task.checkCancellation()` throwing `CancellationError`, and structured
// propagation of cancellation to a child task.

func cooperative() async {
    let t = Task { () -> Int in
        // The body observes its own cancellation through the static accessor.
        if Task.isCancelled { return -1 }
        return 100
    }
    t.cancel()
    print("isCancelled:", t.isCancelled)
    print("value:", await t.value)
}

func checking() async {
    let t = Task { () -> String in
        do {
            try Task.checkCancellation()
            return "ran to completion"
        } catch is CancellationError {
            return "threw CancellationError"
        }
    }
    t.cancel()
    print(await t.value)
}

func propagation() async {
    let outer = Task { () -> Bool in
        // A child spawned inside a cancelled task inherits cancellation.
        let inner = Task { Task.isCancelled }
        return await inner.value
    }
    outer.cancel()
    print("child inherited cancel:", await outer.value)
}

func topLevel() async {
    // Outside any task body, `Task.isCancelled` is false.
    print("top-level isCancelled:", Task.isCancelled)
}

Task {
    await cooperative()
    await checking()
    await propagation()
    await topLevel()
}
