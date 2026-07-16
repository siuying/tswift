// AsyncImage fixtures (ADR-0013 §4).
//
// v1 bare: host loads natively (web <img>, iOS AsyncImage). No phase, no closures.
// v1.5 content+placeholder: labeled-closure form; phase state drives children.
// Note: multi-trailing-closure syntax (`} placeholder: {`) is not yet supported
// by the frontend; use labeled `content:` / `placeholder:` args instead.
import Foundation
import SwiftUI

struct AsyncImageView: View {
    var body: some View {
        VStack {
            // v1 bare — host loads natively; no phase events needed.
            AsyncImage(url: URL(string: "https://example.com/photo.jpg"))

            // v1.5 content+placeholder — placeholder shown until imagePhase success.
            AsyncImage(
                url: URL(string: "https://example.com/avatar.jpg"),
                content: { image in
                    image.resizable().scaledToFit()
                },
                placeholder: { ProgressView() }
            )
        }
    }
}
