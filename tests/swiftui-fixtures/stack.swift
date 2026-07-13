// Stack tab — ZStack depth composition over shapes, each filled and framed.
import SwiftUI

struct StackView: View {
    var body: some View {
        ZStack {
            RoundedRectangle(cornerRadius: 24)
                .fill(Color.indigo)
                .frame(width: 200, height: 120)
            Circle()
                .fill(.white)
                .frame(width: 64, height: 64)
            Text("Stack")
                .font(.headline)
                .foregroundColor(.white)
        }
    }
}
