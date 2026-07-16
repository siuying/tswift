// C2 typed-token surface (issue #203): leading-dot members that resolve by the
// modifier's declared parameter type — frame `alignment:`, `.frame(maxWidth:
// .infinity)`, and directional `.padding(.horizontal, _)`. These collide across
// the alignment/edge/axis namespaces and were unresolvable before the typed
// prelude shims.
import SwiftUI

struct V: View {
    var body: some View {
        VStack {
            Text("Banner")
                .frame(maxWidth: .infinity, alignment: .leading)
                .padding(.horizontal, 12)
            Text("Centered")
                .frame(width: 200, height: 44, alignment: .center)
            Text("Tagline")
                .multilineTextAlignment(.center)
                .padding(.vertical)
        }
    }
}
