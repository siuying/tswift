// GeometryReader hands its content a deterministic GeometryProxy (headless
// tier): `size` is the runtime's default proposed size, `frame(in:)` sits at
// the origin, and `safeAreaInsets` is zero. The content reads them, so the
// rendered text is geometry-dependent and proves the proxy is real (no fake
// device-pixel parity is claimed; see docs/plan/swiftui-support.md).
import SwiftUI

struct RootView: View {
    var body: some View {
        GeometryReader { geo in
            VStack {
                Text("W=\(Int(geo.size.width)) H=\(Int(geo.size.height))")
                Text("half=\(Int(geo.frame(in: .local).midX))")
                Text("safe=\(Int(geo.safeAreaInsets.top))")
            }
            .frame(width: geo.size.width, height: geo.size.height)
        }
    }
}
