// C3 — horizontal ScrollView axis (issue #190).
import SwiftUI

struct V: View {
    var body: some View {
        ScrollView(.horizontal) {
            HStack(spacing: 8) {
                Text("one")
                Text("two")
                Text("three")
                Text("four")
            }
        }
    }
}
