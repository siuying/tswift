import SwiftUI

/// A fixed-size host that renders a `RenderModel`'s current tree. Snapshots are
/// taken of this view; mutating the model (via patches) re-renders it.
public struct RenderHostView: View {
    @ObservedObject var model: RenderModel
    let size: CGSize

    public init(model: RenderModel, size: CGSize = CGSize(width: 320, height: 480)) {
        self.model = model
        self.size = size
    }

    public var body: some View {
        ViewFactory.render(model.root)
            .frame(width: size.width, height: size.height)
            .background(Color(white: 0.12))
    }
}
