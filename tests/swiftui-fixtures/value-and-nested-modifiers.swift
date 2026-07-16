// Value-passthrough (scalar/color/string/edge) and nested-view modifiers that
// carry no leading-dot token: they never contend in the implicit-member
// namespace, so the host interprets the recorded name + args directly.
struct V: View {
    var body: some View {
        VStack(spacing: 8) {
            Text("pos")
                .position(x: 40, y: 20)
                .accentColor(.red)
            Text("mask")
                .mask {
                    Rectangle()
                }
            Text("row")
                .listRowInsets(EdgeInsets(top: 2, leading: 4, bottom: 2, trailing: 4))
                .listRowBackground(Color.blue)
            Text("safe")
                .safeAreaPadding(12)
                .lineHeight(1.5)
            Text("menu")
                .contextMenu {
                    Button("copy") {}
                }
                .navigationBarTitle("Title")
        }
    }
}
