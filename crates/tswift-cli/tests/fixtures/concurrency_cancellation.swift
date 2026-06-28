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

func detachedNotInherited() async {
    // A detached task is *not* a structured child: it never inherits the
    // spawning task's cancellation.
    let outer = Task { () -> Bool in
        let d = Task.detached { Task.isCancelled }
        return await d.value
    }
    outer.cancel()
    print("detached inherited cancel:", await outer.value)
}

func groupCancellation() async {
    let sum = await withTaskGroup(of: Int.self) { group -> Int in
        group.addTask { 1 }
        group.cancelAll()
        // Refused once the group is cancelled.
        let added = group.addTaskUnlessCancelled { 2 }
        print("addTaskUnlessCancelled after cancel:", added)
        // A child added after cancelAll() starts cancelled.
        group.addTask { Task.isCancelled ? 100 : 0 }
        var total = 0
        for await v in group { total += v }
        return total
    }
    print("group sum:", sum)
}

Task {
    await cooperative()
    await checking()
    await propagation()
    await detachedNotInherited()
    await groupCancellation()
}

// Outside any task body, `Task.isCancelled` is false.
print("top-level isCancelled:", Task.isCancelled)
