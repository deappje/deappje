import SwiftUI

/// The screen for the session's state.
struct RootView: View {
    @Bindable var model: AppModel

    var body: some View {
        switch model.state {
        case .starting:
            ProgressView("deappje")
        case .signedOut:
            SignInView(model: model)
        case .signedIn:
            NavigationStack {
                GroupsView(model: model)
            }
        }
    }
}

/// The last error, under the content, until the next thing goes right.
struct ErrorBanner: View {
    let message: String?

    var body: some View {
        if let message {
            Text(message)
                .font(.footnote)
                .foregroundStyle(.white)
                .padding(10)
                .frame(maxWidth: .infinity)
                .background(.red, in: RoundedRectangle(cornerRadius: 8))
                .padding(.horizontal)
        }
    }
}
