import DeappjeCore
import SwiftUI

/// A group's posts, newest first, with a composer; own posts can be edited
/// and deleted. A device without the group's keys sees its code instead.
struct TimelineView: View {
    @Bindable var model: AppModel
    let groupId: Uuid
    @State private var timeline: TimelineModel
    @State private var draft = ""
    @State private var editing: ObjectView?
    @State private var inviteLink: String?

    init(model: AppModel, groupId: Uuid) {
        self.model = model
        self.groupId = groupId
        _timeline = State(initialValue: TimelineModel(core: model.core, groupId: groupId))
    }

    private var holdsKeys: Bool {
        model.groups.first { $0.id == groupId }?.holdsKeys ?? false
    }

    private var userId: Uuid? {
        if case let .signedIn(userId) = model.state { return userId }
        return nil
    }

    var body: some View {
        Group {
            if holdsKeys {
                posts
            } else {
                ContentUnavailableView {
                    Label("Waiting for approval", systemImage: "key")
                } description: {
                    Text("A member of this group approves this device by comparing its code:\n\(model.fingerprint)")
                        .font(.body.monospaced())
                }
            }
        }
        .navigationTitle(model.groupNames[groupId] ?? "Group")
        .navigationBarTitleDisplayMode(.inline)
        .toolbar {
            if holdsKeys {
                ToolbarItem(placement: .topBarTrailing) {
                    Button("Invite", systemImage: "person.badge.plus") {
                        Task {
                            if let link = await model.createInvite(groupId: groupId) {
                                UIPasteboard.general.string = link
                                inviteLink = link
                            }
                        }
                    }
                }
            }
        }
        .task(id: model.groups) { await timeline.load() }
        .refreshable {
            await model.sync()
            await timeline.load()
        }
        .alert("Invite link copied", isPresented: Binding(get: { inviteLink != nil }, set: { if !$0 { inviteLink = nil } })) {
            Button("OK") { inviteLink = nil }
        } message: {
            Text(inviteLink ?? "")
        }
        .sheet(item: $editing) { post in
            EditPostSheet(text: TimelineModel.text(of: post)) { text in
                try? await timeline.edit(post.id, text: text)
            }
        }
    }

    private var posts: some View {
        List {
            ForEach(timeline.posts, id: \.id) { post in
                VStack(alignment: .leading, spacing: 4) {
                    Text(TimelineModel.text(of: post))
                    HStack {
                        Text(post.author == userId ? "You" : String(post.author.prefix(8)))
                        Text(Date(timeIntervalSince1970: Double(post.authoredAtMs) / 1000), style: .relative)
                        if post.pending {
                            Image(systemName: "clock")
                        }
                        if post.editedAtMs > post.authoredAtMs {
                            Text("edited")
                        }
                    }
                    .font(.caption)
                    .foregroundStyle(.secondary)
                }
                .swipeActions {
                    if post.author == userId {
                        Button("Delete", systemImage: "trash", role: .destructive) {
                            Task { try? await timeline.delete(post.id) }
                        }
                        Button("Edit", systemImage: "pencil") { editing = post }
                    }
                }
            }
        }
        .listStyle(.plain)
        .safeAreaInset(edge: .bottom) {
            VStack(spacing: 0) {
                ErrorBanner(message: model.lastError)
                HStack {
                    TextField("Write a post", text: $draft, axis: .vertical)
                        .textFieldStyle(.roundedBorder)
                    Button("Send", systemImage: "arrow.up.circle.fill") {
                        let text = draft.trimmingCharacters(in: .whitespacesAndNewlines)
                        draft = ""
                        Task { _ = try? await timeline.post(text) }
                    }
                    .labelStyle(.iconOnly)
                    .disabled(draft.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty)
                }
                .padding()
                .background(.bar)
            }
        }
    }
}

extension ObjectView: @retroactive Identifiable {}

/// The whole text of an own post, to replace.
struct EditPostSheet: View {
    @State var text: String
    let save: (String) async -> Void
    @Environment(\.dismiss) private var dismiss

    var body: some View {
        NavigationStack {
            Form {
                TextField("Post", text: $text, axis: .vertical)
            }
            .navigationTitle("Edit post")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .cancellationAction) {
                    Button("Cancel") { dismiss() }
                }
                ToolbarItem(placement: .confirmationAction) {
                    Button("Save") {
                        Task {
                            await save(text.trimmingCharacters(in: .whitespacesAndNewlines))
                            dismiss()
                        }
                    }
                    .disabled(text.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty)
                }
            }
        }
    }
}
