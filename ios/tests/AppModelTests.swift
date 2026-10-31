import DeappjeApp
import DeappjeCore
import DeappjeShell
import Foundation
import TestSupport
import XCTest

/// A browser no test opens.
struct NoBrowser: LoginBrowser {
    func authenticate(url: URL, callbackScheme: String) async throws -> URL {
        throw LoginError.cancelled
    }
}

@MainActor
final class AppModelTests: XCTestCase {
    private func server() -> ModelServer {
        ModelServer.open(baseUrl: TestDevice.baseUrl, seed: Data(repeating: 2, count: 32))
    }

    /// A model over a device whose kept token signs in as the user.
    private func signedInModel(server: ModelServer, seed: UInt64, userId: Uuid) async -> (AppModel, TestDevice) {
        let device = TestDevice(server: server, seed: seed)
        let token = "token-\(seed)"
        server.signIn(token: token, userId: userId)
        await device.core.setSessionToken(token: token)
        let model = AppModel(core: device.core, browser: NoBrowser())
        await model.start()
        model.deactivated()
        return (model, device)
    }

    func testStartsSignedOutWithoutASession() async {
        let device = TestDevice(server: server(), seed: 1)
        let model = AppModel(core: device.core, browser: NoBrowser())
        XCTAssertEqual(model.state, .starting)
        await model.start()
        XCTAssertEqual(model.state, .signedOut)
        XCTAssertEqual(model.fingerprint.count, 14, "xxxx-xxxx-xxxx: \(model.fingerprint)")
        XCTAssertFalse(model.isSyncing)
        // The model server speaks whoami only (core/src/harness/server.rs), so
        // the login flow fails before the browser opens: a failure the screen
        // is told about, not one swallowed.
        await model.signIn()
        XCTAssertEqual(model.state, .signedOut)
        XCTAssertNotNil(model.lastError)
    }

    func testRestoresTheKeptSessionRegistersAsksForPushAndSyncsWhileActive() async throws {
        let server = server()
        let device = TestDevice(server: server, seed: 2)
        server.signIn(token: "kept", userId: TestDevice.user(1))
        await device.core.setSessionToken(token: "kept")
        let model = AppModel(core: device.core, browser: NoBrowser())
        final class Flag: @unchecked Sendable { var asked = false }
        let push = Flag()
        model.registerForPush = { push.asked = true }

        await model.start()
        XCTAssertEqual(model.state, .signedIn(userId: TestDevice.user(1)))
        XCTAssertTrue(push.asked, "a silent push needs no permission: asked at once")
        XCTAssertTrue(model.isSyncing, "the 30 s timer runs while active")
        XCTAssertNil(model.lastError)
        model.deactivated()
        XCTAssertFalse(model.isSyncing)
        await model.activated()
        XCTAssertTrue(model.isSyncing)
        model.deactivated()

        // The device is registered: a second core of the same user asking
        // for keys would be listed; here, the push token lands.
        await model.pushToken(Data([0xde, 0xad, 0xbe, 0xef]))
        XCTAssertNil(model.lastError)

        await model.signOut()
        XCTAssertEqual(model.state, .signedOut)
        let restored = try await device.core.restoreSession()
        XCTAssertNil(restored, "the token is gone")
    }

    func testTwoPhonesShareAGroupThroughTheModels() async throws {
        let server = server()
        let (a, deviceA) = await signedInModel(server: server, seed: 10, userId: TestDevice.user(1))
        let (b, deviceB) = await signedInModel(server: server, seed: 20, userId: TestDevice.user(2))

        // A makes a group: listed by name at once.
        let group = await a.createGroup(name: "Us")
        XCTAssertNotNil(group, a.lastError ?? "")
        XCTAssertEqual(a.groups.map(\.id), [group!])
        XCTAssertEqual(a.groupNames[group!], "Us")
        XCTAssertTrue(a.groups[0].holdsKeys)

        // A invites; B joins by the whole link; B is in without keys.
        let link = await a.createInvite(groupId: group!)
        XCTAssertEqual(link?.hasPrefix(Deployment.baseUrl + "/join/"), true, link ?? "")
        let joined = await b.join(link: link!)
        XCTAssertEqual(joined?.groupId, group)
        XCTAssertEqual(b.groups.map(\.id), [group!])
        XCTAssertFalse(b.groups[0].holdsKeys)
        XCTAssertNil(b.groupNames[group!], "the name is in an object B cannot open yet")

        // A's approvals list, as the foreground and a key_request wake-up ask for it.
        await a.handle(.keyRequest)
        XCTAssertEqual(a.pending.count, 1)
        XCTAssertEqual(a.pending[0].fingerprint, b.fingerprint)
        XCTAssertEqual(a.pending[0].inviteVerified, true)
        await a.approve(a.pending[0])
        XCTAssertTrue(a.pending.isEmpty)

        // B syncs (a head wake-up): keys, the name, the group readable.
        await b.handle(.sync)
        XCTAssertNil(b.lastError)
        XCTAssertTrue(b.groups[0].holdsKeys)
        XCTAssertEqual(b.groupNames[group!], "Us")

        // B posts; A's timeline shows it after a sync; B edits; B deletes.
        let timelineB = TimelineModel(core: deviceB.core, groupId: group!)
        let post = try await timelineB.post("hello")
        XCTAssertEqual(timelineB.posts.map(\.id), [post])
        XCTAssertEqual(timelineB.posts[0].pending, false, "pushed by the sync that follows a write")
        let timelineA = TimelineModel(core: deviceA.core, groupId: group!)
        await a.sync()
        await timelineA.load()
        XCTAssertEqual(timelineA.posts.map(TimelineModel.text(of:)), ["hello"])
        XCTAssertEqual(timelineA.posts[0].author, TestDevice.user(2))

        deviceB.clock.advance(ms: 1_000)
        try await timelineB.edit(post, text: "hello!")
        await a.sync()
        await timelineA.load()
        XCTAssertEqual(timelineA.posts.map(TimelineModel.text(of:)), ["hello!"])
        XCTAssertGreaterThan(timelineA.posts[0].editedAtMs, timelineA.posts[0].authoredAtMs)

        deviceB.clock.advance(ms: 1_000)
        let second = try await timelineB.post("second")
        XCTAssertEqual(timelineB.posts.map(\.id), [second, post], "newest first")

        try await timelineB.delete(post)
        await a.sync()
        await timelineA.load()
        XCTAssertEqual(timelineA.posts.map(\.id), [second])

        // A history_ready wake-up for a request this device never made is
        // nothing to the core, and the sync after it still runs.
        await a.handle(.historyReady(requestId: TestDevice.user(9), throughSeq: 3, complete: true))
        XCTAssertNil(a.lastError)
    }

    func testWakeUpsMapToTheCallsTheCoreDocuments() {
        for reason in ["head", "keys", "rotate", "history"] {
            XCTAssertEqual(WakeUp(payload: ["reason": reason, "group_id": "g"]), .sync, reason)
        }
        XCTAssertEqual(WakeUp(payload: ["reason": "key_request", "group_id": "g", "request_id": "r"]), .keyRequest)
        XCTAssertEqual(
            WakeUp(payload: ["reason": "history_ready", "request_id": "r", "through_seq": "5", "complete": "true"]),
            .historyReady(requestId: "r", throughSeq: 5, complete: true)
        )
        XCTAssertEqual(
            WakeUp(payload: ["reason": "history_ready", "request_id": "r", "through_seq": "0", "complete": "false"]),
            .historyReady(requestId: "r", throughSeq: 0, complete: false)
        )
        XCTAssertNil(WakeUp(payload: ["reason": "history_ready", "request_id": "r"]), "a half payload is no wake-up")
        XCTAssertNil(WakeUp(payload: ["reason": "chat"]), "a reason this app does not know")
        XCTAssertNil(WakeUp(payload: [:]))

        // As APNs delivers it: {"aps": {"content-available": 1}, "wake": {...}}.
        let userInfo: [AnyHashable: Any] = [
            "aps": ["content-available": 1],
            "wake": ["reason": "history_ready", "request_id": "r", "through_seq": "7", "complete": "true"],
        ]
        XCTAssertEqual(WakeUp(userInfo: userInfo), .historyReady(requestId: "r", throughSeq: 7, complete: true))
        XCTAssertEqual(WakeUp(userInfo: ["aps": ["content-available": 1], "wake": ["reason": "head"]]), .sync)
        XCTAssertNil(WakeUp(userInfo: ["aps": ["alert": "hi"]]), "a visible notification is not a wake-up")
    }
}
