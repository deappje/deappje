import SwiftUI

/// One button: Sign in with Basable, in the system browser.
struct SignInView: View {
    @Bindable var model: AppModel
    @State private var signingIn = false

    var body: some View {
        VStack(spacing: 24) {
            Spacer()
            Text("deappje")
                .font(.largeTitle.bold())
            Text("Your groups, end to end encrypted.")
                .foregroundStyle(.secondary)
            Spacer()
            Button {
                signingIn = true
                Task {
                    await model.signIn()
                    signingIn = false
                }
            } label: {
                Text("Sign in with Basable")
                    .frame(maxWidth: .infinity)
            }
            .buttonStyle(.borderedProminent)
            .controlSize(.large)
            .disabled(signingIn)
            .padding(.horizontal)
            ErrorBanner(message: model.lastError)
            Text(Deployment.environment)
                .font(.caption2)
                .foregroundStyle(.tertiary)
        }
        .padding(.bottom)
    }
}
