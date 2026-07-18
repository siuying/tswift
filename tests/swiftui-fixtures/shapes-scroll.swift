// Shape and ScrollView members: RoundedRectangle carries an asymmetric
// `cornerSize:` and a `style:` (RoundedCornerStyle), and ScrollView records a
// non-default `showsIndicators: false` alongside its horizontal `axes`.
import SwiftUI

struct RootView: View {
    var body: some View {
        ScrollView(.horizontal, showsIndicators: false) {
            HStack {
                RoundedRectangle(cornerRadius: 12, style: .continuous)
                RoundedRectangle(cornerSize: CGSize(width: 8, height: 4))
                Circle()
            }
        }
    }
}
