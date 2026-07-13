// Grouped List with titled Sections — exercises List, Section(header), and a
// keyed ForEach of rows nested inside a section.
import SwiftUI

struct SectionsView: View {
    var body: some View {
        List {
            Section("Favorites") {
                ForEach(["Apple", "Cherry"], id: \.self) { fruit in
                    Text(fruit)
                }
            }
            Section("Other") {
                Text("Banana")
            }
        }
    }
}
