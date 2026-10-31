import DeappjeCore
import Foundation
import Observation
import os

/// The app's state over the one `Core`: who is signed in, the groups, the
/// approver's list, the sync schedule and the wake-ups. Every screen reads
/// it; nothing here parses an object value or touches a key, the core does
/// (docs/01-deappje/01-architecture.md, "The shells").
@MainActor
@Observable
public final class AppModel {
    /// Where the session stands.
    public enum State: Equatable, Sendable {
        /// Asking the kept session.
        case starting
        /// No session: the sign-in screen.
        case signedOut
        /// Signed in as the user.
        case signedIn(userId: Uuid)
    }

    /// How often a sync runs while the app is active.
    public static let syncInterval: Duration = .seconds(30)

    public let core: Core
    public private(set) var state: State = .starting
    /// The groups, as the last `listGroups` had them.
    public private(set) var groups: [GroupView] = []
    /// Each group's name from its profile, where this device can read it.
    public private(set) var groupNames: [Uuid: String] = [:]
    /// The key requests this device may approve.
    public private(set) var pending: [PendingRequestView] = []
    /// This device's fingerprint, what an approver compares.
    public private(set) var fingerprint: String = ""
    /// The last thing that went wrong, for the screen; nil once something
    /// went right again.
    public var lastError: String?
    /// Set by the app: asks the system for a push token (a silent push
    /// needs no permission). Nil in tests.
    public var registerForPush: (@MainActor () -> Void)?

    private let browser: LoginBrowser
    private var syncLoop: Task<Void, Never>?
    private let log = Logger(subsystem: "nl.deappje.app", category: "model")

    public init(core: Core, browser: LoginBrowser) {
        self.core = core
        self.browser = browser
    }

    // MARK: Session

    /// At launch: whether the kept session still signs in.
    public func start() async {
        fingerprint = await core.fingerprint()
        do {
            if let identity = try await core.restoreSession() {
                await signedIn(identity)
            } else {
                state = .signedOut
            }
        } catch {
            state = .signedOut
            report(error)
        }
    }

    /// Sign in with Basable: the browser, then the code to the core.
    public func signIn() async {
        do {
            let start = try await core.startLogin(returnTo: AppCore.loginReturnUrl)
            guard let url = URL(string: start.browserUrl) else { throw LoginError.badUrl(start.browserUrl) }
            let callback = try await browser.authenticate(url: url, callbackScheme: AppCore.loginCallbackScheme)
            guard let code = URLComponents(url: callback, resolvingAgainstBaseURL: false)?.queryItems?.first(where: { $0.name == "code" })?.value else {
                throw LoginError.noCode(callback)
            }
            let identity = try await core.finishLogin(start: start, code: code)
            await signedIn(identity)
        } catch LoginError.cancelled {
            return
        } catch {
            report(error)
        }
    }

    private func signedIn(_ identity: Identity) async {
        state = .signedIn(userId: identity.id)
        lastError = nil
        do {
            try await core.registerDevice(platform: "ios")
        } catch {
            report(error)
        }
        registerForPush?()
        await activated()
    }

    /// Ends the session and forgets what the screens showed.
    public func signOut() async {
        deactivated()
        do {
            try await core.logout()
        } catch {
            report(error)
        }
        state = .signedOut
        groups = []
        groupNames = [:]
        pending = []
    }

    // MARK: The schedule

    /// On becoming active: a sync and the approver's list at once, then a
    /// sync every `syncInterval` until `deactivated`.
    public func activated() async {
        guard case .signedIn = state else { return }
        syncLoop?.cancel()
        syncLoop = Task { [weak self] in
            while !Task.isCancelled {
                guard (try? await Task.sleep(for: AppModel.syncInterval)) != nil else { return }
                await self?.sync()
            }
        }
        await sync()
        await refreshPending()
    }

    /// On leaving the foreground: the timer stops.
    public func deactivated() {
        syncLoop?.cancel()
        syncLoop = nil
    }

    /// Whether the timer runs.
    public var isSyncing: Bool {
        syncLoop != nil
    }

    // MARK: Sync and wake-ups

    /// One sync of everything, then the groups read again.
    public func sync() async {
        guard case .signedIn = state else { return }
        do {
            let report = try await core.sync()
            if !report.failed.isEmpty {
                lastError = report.failed.joined(separator: "\n")
            }
            for alert in report.pinAlerts {
                log.error("device \(alert.deviceId) offers key \(alert.offered), pinned \(alert.pinned)")
            }
            await loadGroups()
        } catch {
            report(error)
        }
    }

    /// A silent push: the call its reason names (core/AGENTS.md, "Wake-ups").
    public func handle(_ wake: WakeUp) async {
        switch wake {
        case .sync:
            await sync()
        case .keyRequest:
            await refreshPending()
        case let .historyReady(requestId, throughSeq, complete):
            await core.historyReady(requestId: requestId, throughSeq: throughSeq, complete: complete)
            await sync()
        }
    }

    /// The system gave a push token: the server gets it as `apns`.
    public func pushToken(_ deviceToken: Data) async {
        let hex = deviceToken.map { String(format: "%02x", $0) }.joined()
        do {
            try await core.setPushToken(provider: "apns", token: hex)
        } catch {
            report(error)
        }
    }

    // MARK: Groups

    /// The groups and their names.
    public func loadGroups() async {
        do {
            let groups = try await core.listGroups()
            var names: [Uuid: String] = [:]
            for group in groups where group.holdsKeys {
                if case let .groupProfile(profile)? = await core.readObject(groupId: group.id, id: core.groupProfileId(groupId: group.id))?.body {
                    names[group.id] = profile.name
                }
            }
            self.groups = groups
            groupNames = names
        } catch {
            report(error)
        }
    }

    /// A new group with its profile, pushed at once.
    public func createGroup(name: String) async -> Uuid? {
        do {
            let id = try await core.createGroup(parentGroupId: nil)
            let profile = GroupProfile(name: name, description: "", avatar: Data(), messageRetentionMonths: nil, kind: .group)
            _ = try await core.createObject(groupId: id, parentId: nil, body: .groupProfile(profile))
            await sync()
            return id
        } catch {
            report(error)
            return nil
        }
    }

    /// Joins through a pasted link; the keys come once a holder approves.
    public func join(link: String) async -> JoinResult? {
        do {
            let result = try await core.join(link: link)
            await loadGroups()
            return result
        } catch {
            report(error)
            return nil
        }
    }

    /// An invite link for the group, whole, ready to share.
    public func createInvite(groupId: Uuid) async -> String? {
        do {
            let path = try await core.createInvite(groupId: groupId, role: "member", expiresAtMs: 0, maxUses: 0)
            await sync()
            return Deployment.baseUrl + path
        } catch {
            report(error)
            return nil
        }
    }

    // MARK: Approvals

    /// The approver's list, which no sync lists: asked on every foreground
    /// and on a `key_request` wake-up.
    public func refreshPending() async {
        guard case .signedIn = state else { return }
        do {
            pending = try await core.pendingKeyRequests()
        } catch {
            report(error)
        }
    }

    /// The person compared the fingerprint, or the invite vouched for it.
    public func approve(_ request: PendingRequestView) async {
        do {
            try await core.approveKeyRequest(requestId: request.requestId)
            pending.removeAll { $0.requestId == request.requestId }
        } catch {
            report(error)
        }
    }

    private func report(_ error: Error) {
        log.error("\(String(describing: error))")
        lastError = String(describing: error)
    }
}
