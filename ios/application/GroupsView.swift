import DeappjeCore
import SwiftUI

/// The groups the user is in, with the way into each, a new group, a join by
/// link and the approvals.
struct GroupsView: View {
    @Bindable var model: AppModel
    @State private var newGroup = false
    @State private var joining = false

    var body: some View {
        List {
            if model.groups.isEmpty {
                ContentUnavailableView("No groups yet", systemImage: "person.3", description: Text("Make one, or join through a link someone sent you."))
            }
            ForEach(model.groups, id: \.id) { group in
                NavigationLink(value: group.id) {
                    VStack(alignment: .leading) {
                        Text(model.groupNames[group.id] ?? "Group")
                        if !group.holdsKeys {
                            Text("Waiting for a member to approve this device")
                                .font(.caption)
                                .foregroundStyle(.secondary)
                        }
                    }
                }
            }
        }
        .navigationTitle("Groups")
        .navigationDestination(for: Uuid.self) { groupId in
            TimelineView(model: model, groupId: groupId)
        }
        .toolbar {
            ToolbarItem(placement: .topBarLeading) {
                NavigationLink {
                    ApprovalsView(model: model)
                } label: {
                    Label("Approvals", systemImage: "checkmark.shield")
                }
                .badge(model.pending.count)
            }
            ToolbarItem(placement: .topBarTrailing) {
                Menu {
                    Button("New group", systemImage: "plus") { newGroup = true }
                    Button("Join by link", systemImage: "link") { joining = true }
                    Divider()
                    Button("Sign out", role: .destructive) { Task { await model.signOut() } }
                } label: {
                    Label("More", systemImage: "ellipsis.circle")
                }
            }
        }
        .refreshable {
            await model.sync()
            await model.refreshPending()
        }
        .safeAreaInset(edge: .bottom) {
            ErrorBanner(message: model.lastError)
        }
        .sheet(isPresented: $newGroup) {
            TextSheet(title: "New group", prompt: "Name", action: "Create") { name in
                _ = await model.createGroup(name: name)
            }
        }
        .sheet(isPresented: $joining) {
            TextSheet(title: "Join by link", prompt: "Paste the invite link", action: "Join") { link in
                _ = await model.join(link: link)
            }
        }
    }
}

/// One text field and one button, as a sheet.
struct TextSheet: View {
    let title: String
    let prompt: String
    let action: String
    let perform: (String) async -> Void
    @Environment(\.dismiss) private var dismiss
    @State private var text = ""
    @State private var busy = false

    var body: some View {
        NavigationStack {
            Form {
                TextField(prompt, text: $text, axis: .vertical)
                    .textInputAutocapitalization(.never)
                    .autocorrectionDisabled()
            }
            .navigationTitle(title)
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .cancellationAction) {
                    Button("Cancel") { dismiss() }
                }
                ToolbarItem(placement: .confirmationAction) {
                    Button(action) {
                        busy = true
                        Task {
                            await perform(text.trimmingCharacters(in: .whitespacesAndNewlines))
                            dismiss()
                        }
                    }
                    .disabled(busy || text.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty)
                }
            }
        }
    }
}
