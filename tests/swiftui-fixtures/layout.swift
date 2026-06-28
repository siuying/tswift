// C2 — layout modifiers & container args (issue #189).
// Collision-free subset: stack spacing, numeric frame min/max, Spacer
// minLength, offset. (alignment + .infinity + edge padding deferred.)
struct V: View {
    var body: some View {
        VStack(spacing: 24) {
            Text("Top")
                .frame(maxWidth: 320, minHeight: 44)
            HStack(spacing: 8) {
                Text("Left")
                Spacer(minLength: 12)
                Text("Right").offset(x: 4, y: -2)
            }
            .frame(maxWidth: 320)
        }
    }
}
