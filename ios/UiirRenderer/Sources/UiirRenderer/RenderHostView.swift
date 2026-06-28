import SwiftUI

/// Hosts a `RenderModel`'s current tree, filling the device and supplying the
/// adaptive system background. Snapshots are taken of this view across a
/// device × appearance matrix (see `SnapshotTests`); the color scheme is driven
/// by the snapshot's trait collection, so `.primary`/`.secondary` and the
/// background adapt to light/dark exactly as on a real device.
public struct RenderHostView: View {
    @ObservedObject var model: RenderModel
    @Environment(\.uiirEventSink) private var eventSink

    public init(model: RenderModel) {
        self.model = model
    }

    public var body: some View {
        ViewFactory.render(model.root, eventSink: eventSink)
            .frame(maxWidth: .infinity, maxHeight: .infinity)
            .background(Color(.systemBackground))
    }
}
