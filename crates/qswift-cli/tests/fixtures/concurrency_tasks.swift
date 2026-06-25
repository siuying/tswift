// `Task { }`, `Task.detached { }`, awaiting `.value`, and cooperative
// cancellation via `cancel()` / `isCancelled`.
func run() async {
    let t = Task { 20 + 1 }
    let v = await t.value
    print("task value \(v)")

    let d = Task.detached { 7 * 6 }
    let dv = await d.value
    print("detached value \(dv)")

    let c = Task { 5 }
    c.cancel()
    print("isCancelled \(c.isCancelled)")
    let cv = await c.value
    print("still completes \(cv)")
}
run()
