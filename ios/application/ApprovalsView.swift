import DeappjeCore
import SwiftUI

/// The key requests this device may approve (docs/01-deappje/02-device.md):
/// each with the requesting device's fingerprint, and whether the invite it
/// came through vouches for it. Without an invite the person compares the
/// code with what the other screen shows.
struct ApprovalsView: View {
    @Bindable var model: AppModel

    var body: some View {
        List {
            Section {
                LabeledContent("This device's code") {
                    Text(model.fingerprint)
                        .font(.body.monospaced())
                }
            } footer: {
                Text("Another member reads this code off your screen to approve this device.")
            }
            Section("Waiting for your approval") {
                if model.pending.isEmpty {
                    Text("Nothing to approve")
                        .foregroundStyle(.secondary)
                }
                ForEach(model.pending, id: \.requestId) { request in
                    VStack(alignment: .leading, spacing: 6) {
                        Text(model.groupNames[request.groupId] ?? "Group")
                            .font(.headline)
                        Text("\(request.platform) device of \(String(request.userId.prefix(8)))")
                            .font(.subheadline)
                        Text(request.fingerprint)
                            .font(.body.monospaced())
                        switch request.inviteVerified {
                        case true?:
                            Label("Came through your invite link", systemImage: "checkmark.seal")
                                .font(.caption)
                                .foregroundStyle(.green)
                        case false?:
                            Label("The invite tag does not match: do not approve", systemImage: "xmark.octagon")
                                .font(.caption)
                                .foregroundStyle(.red)
                        case nil:
                            Label("Compare the code with the other screen", systemImage: "eye")
                                .font(.caption)
                                .foregroundStyle(.secondary)
                        }
                        Button("Approve") {
                            Task { await model.approve(request) }
                        }
                        .buttonStyle(.borderedProminent)
                        .disabled(request.inviteVerified == false)
                    }
                    .padding(.vertical, 4)
                }
            }
        }
        .navigationTitle("Approvals")
        .refreshable { await model.refreshPending() }
        .task { await model.refreshPending() }
        .safeAreaInset(edge: .bottom) {
            ErrorBanner(message: model.lastError)
        }
    }
}
