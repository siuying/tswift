// NavigationSplitView renders every column eagerly as an ordinary child
// (sidebar first, detail last), mirroring how NavigationStack renders each
// screen (ADR-0013 §1). The `columns` arg records the count; the host lays the
// children out as columns and owns the split/collapse behaviour. Selection-
// driven detail is host-driven (honest scope) — the runtime does not collapse
// the detail to a sidebar selection.
import SwiftUI

struct RootView: View {
    let items = ["Inbox", "Sent", "Drafts"]

    var body: some View {
        NavigationSplitView {
            List(items, id: \.self) { item in
                Text(item)
            }
            .navigationTitle("Mailboxes")
        } detail: {
            Text("Select a mailbox")
                .navigationTitle("Detail")
        }
    }
}
