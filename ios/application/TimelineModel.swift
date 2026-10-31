import DeappjeCore
import Foundation
import Observation

/// One group's timeline: its posts, newest first, and the writes a person
/// makes to them. Every write is local to the core's outbox until the sync
/// that follows it pushes it.
@MainActor
@Observable
public final class TimelineModel {
    public let groupId: Uuid
    private let core: Core
    /// The posts this device can read, newest first.
    public private(set) var posts: [ObjectView] = []

    public init(core: Core, groupId: Uuid) {
        self.core = core
        self.groupId = groupId
    }

    /// Reads the posts out of the core's store.
    public func load() async {
        posts = await core.listObjects(groupId: groupId)
            .filter { if case .post = $0.body { return true } else { return false } }
            .sorted { $0.authoredAtMs > $1.authoredAtMs }
    }

    /// The post's text.
    public static func text(of post: ObjectView) -> String {
        if case let .post(body) = post.body { return body.text }
        return ""
    }

    /// A new post, then a sync.
    public func post(_ text: String) async throws -> Uuid {
        let id = try await core.createObject(groupId: groupId, parentId: nil, body: .post(Post(text: text)))
        await settle()
        return id
    }

    /// New text for an own post, then a sync.
    public func edit(_ id: Uuid, text: String) async throws {
        try await core.editObject(groupId: groupId, id: id, body: .post(Post(text: text)))
        await settle()
    }

    /// Deletes a post and what hangs under it, then a sync.
    public func delete(_ id: Uuid) async throws {
        try await core.deleteObject(groupId: groupId, id: id)
        await settle()
    }

    /// Pulls and pushes, then reads again; a sync that fails leaves the
    /// write in the outbox for the next one.
    private func settle() async {
        _ = try? await core.sync()
        await load()
    }
}
