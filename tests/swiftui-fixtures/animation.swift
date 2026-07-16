// UI9 — Animation curve factories. Exercises the `Animation` value type end to
// end: the standard `.default` curve (reachable via backtick-escaped keyword),
// the timing/ease families, the spring families, and the new custom curves
// (`timingCurve`, `interpolatingSpring`, `interactiveSpring`) plus the chained
// `delay`/`speed`/`repeatCount`/`repeatForever` modifiers.
struct V: View {
    var body: some View {
        VStack(spacing: 8) {
            Text("default").animation(.default, value: 0)
            Text("linear").animation(.linear(duration: 0.25), value: 0)
            Text("easeInOut").animation(.easeInOut(duration: 0.3), value: 0)
            Text("spring").animation(.spring(response: 0.5, dampingFraction: 0.8), value: 0)
            Text("bouncy").animation(.bouncy(duration: 0.4, extraBounce: 0.1), value: 0)
            Text("smooth").animation(.smooth, value: 0)
            Text("snappy").animation(.snappy, value: 0)
            Text("timingCurve").animation(.timingCurve(0.2, 0.0, 0.8, 1.0, duration: 0.5), value: 0)
            Text("interpolating").animation(.interpolatingSpring(mass: 1.0, stiffness: 100.0, damping: 10.0, initialVelocity: 0.0), value: 0)
            Text("interactive").animation(.interactiveSpring(response: 0.15, dampingFraction: 0.86, blendDuration: 0.25), value: 0)
            Text("delayed").animation(.easeIn(duration: 0.2).delay(0.1).speed(2.0), value: 0)
            Text("repeatCount").animation(.linear.repeatCount(3, autoreverses: true), value: 0)
            Text("repeatForever").animation(.easeInOut.repeatForever(autoreverses: false), value: 0)
        }
    }
}
