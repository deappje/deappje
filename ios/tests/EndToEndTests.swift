import DeappjeCore
import DeappjeShell
import Foundation
import TestSupport
import XCTest

/// Two devices of two users against the core's in-memory server
/// (docs/01-deappje/03-testing.md, "App shells"): the whole join through the
/// bindings, no network and no Kratos.
final class EndToEndTests: XCTestCase {
    func testAJoinerReadsAPostOnceAHolderApproves() async throws {
        let server = ModelServer.open(baseUrl: TestDevice.baseUrl, seed: Data(repeating: 1, count: 32))
        let (userA, userB) = (TestDevice.user(1), TestDevice.user(2))

        // A signs in, registers, makes a group with its profile, posts.
        let a = try await TestDevice.signedIn(server: server, seed: 11, userId: userA)
        let userId = await a.core.userId()
        XCTAssertEqual(userId, userA)
        let group = try await a.core.createGroup(parentGroupId: nil)
        let profile = GroupProfile(name: "Us", description: "", avatar: Data(), messageRetentionMonths: nil, kind: .group)
        let profileId = try await a.core.createObject(groupId: group, parentId: nil, body: .groupProfile(profile))
        XCTAssertEqual(profileId, a.core.groupProfileId(groupId: group))
        let post = try await a.core.createObject(groupId: group, parentId: nil, body: .post(Post(text: "hello from A")))
        let pendingPost = await a.core.readObject(groupId: group, id: post)
        XCTAssertEqual(pendingPost?.pending, true)
        var report = try await a.core.sync()
        XCTAssertTrue(report.failed.isEmpty, "\(report)")
        XCTAssertTrue(report.changed.contains(group), "\(report)")
        let acceptedPost = await a.core.readObject(groupId: group, id: post)
        XCTAssertEqual(acceptedPost?.pending, false)
        XCTAssertGreaterThan(acceptedPost?.seq ?? 0, 0)

        // A's invite link; the secret travels in the group as an object.
        let link = try await a.core.createInvite(groupId: group, role: "member", expiresAtMs: 0, maxUses: 0)
        XCTAssertTrue(link.hasPrefix("/join/"), link)
        XCTAssertTrue(link.contains("#"), "the secret is the fragment: \(link)")
        _ = try await a.core.sync()

        // B signs in, registers, joins through the link: a member without keys.
        let b = try await TestDevice.signedIn(server: server, seed: 22, userId: userB)
        let joined = try await b.core.join(link: link)
        XCTAssertEqual(joined.groupId, group)
        var groupsOfB = try await b.core.listGroups()
        XCTAssertEqual(groupsOfB.map(\.id), [group])
        XCTAssertFalse(groupsOfB[0].holdsKeys)
        let unreadable = await b.core.readObject(groupId: group, id: post)
        XCTAssertNil(unreadable, "no key, no post")

        // A sees the request, vouched for by the invite, with B's fingerprint.
        let pending = try await a.core.pendingKeyRequests()
        XCTAssertEqual(pending.count, 1, "\(pending)")
        XCTAssertEqual(pending[0].groupId, group)
        XCTAssertEqual(pending[0].userId, userB)
        XCTAssertEqual(pending[0].platform, "ios")
        XCTAssertEqual(pending[0].inviteVerified, true)
        let fingerprintOfB = await b.core.fingerprint()
        XCTAssertEqual(pending[0].fingerprint, fingerprintOfB)
        try await a.core.approveKeyRequest(requestId: pending[0].requestId)
        let afterApproval = try await a.core.pendingKeyRequests()
        XCTAssertTrue(afterApproval.isEmpty)

        // B syncs: the key arrives, the group is read, the post is there.
        report = try await b.core.sync()
        XCTAssertEqual(report.keysReceived, [group], "\(report)")
        XCTAssertTrue(report.changed.contains(group), "\(report)")
        XCTAssertTrue(report.failed.isEmpty, "\(report)")
        groupsOfB = try await b.core.listGroups()
        XCTAssertTrue(groupsOfB[0].holdsKeys)
        XCTAssertEqual(groupsOfB[0].phase, "holder")
        let seen = await b.core.readObject(groupId: group, id: post)
        XCTAssertEqual(seen?.body, .post(Post(text: "hello from A")))
        XCTAssertEqual(seen?.author, userA)
        XCTAssertEqual(seen?.pending, false)
        let name = await b.core.readObject(groupId: group, id: b.core.groupProfileId(groupId: group))
        XCTAssertEqual(name?.body, .groupProfile(profile))

        // The store was written through: a reopened core over B's seams holds it.
        let again = Core(transport: server, storage: b.storage, secrets: b.secrets, deviceKey: b.deviceKey, random: SeededRandom(seed: 99), clock: b.clock, baseUrl: TestDevice.baseUrl)
        let deviceId = await b.core.deviceId()
        let deviceIdAgain = await again.deviceId()
        XCTAssertEqual(deviceIdAgain, deviceId)
        let objects = await again.listObjects(groupId: group)
        XCTAssertTrue(objects.contains { $0.id == post }, "\(objects)")
    }
}
