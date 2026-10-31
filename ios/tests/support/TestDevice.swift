import DeappjeCore
import DeappjeShell
import Foundation

/// A phone in a test: a core over the in-memory server, its seams in
/// memory, signed in as `userId` through a session token the server
/// accepts (`ModelServer.signIn`, then `Core.setSessionToken` and
/// `restoreSession` in place of the browser), its device registered.
public struct TestDevice {
    public let core: Core
    public let storage: MemoryStorage
    public let secrets: MemorySecrets
    public let clock: FixedClock
    public let deviceKey: SoftwareDeviceKey

    /// The origin every test server answers at.
    public static let baseUrl = "https://app.example"

    /// A user id a test names by a small number.
    public static func user(_ n: Int) -> Uuid {
        String(format: "00000000-0000-0000-0000-%012d", n)
    }

    /// Opens the core; nobody is signed in yet.
    public init(server: ModelServer, seed: UInt64, nowMs: Int64 = 1_700_000_000_000) {
        storage = MemoryStorage()
        secrets = MemorySecrets()
        clock = FixedClock(nowMs: nowMs)
        deviceKey = SoftwareDeviceKey()
        core = Core(transport: server, storage: storage, secrets: secrets, deviceKey: deviceKey, random: SeededRandom(seed: seed), clock: clock, baseUrl: Self.baseUrl)
    }

    /// Signs in as the user and registers the device (`ios`).
    public static func signedIn(server: ModelServer, seed: UInt64, userId: Uuid, platform: String = "ios") async throws -> TestDevice {
        let device = TestDevice(server: server, seed: seed)
        let token = "token-\(seed)"
        server.signIn(token: token, userId: userId)
        await device.core.setSessionToken(token: token)
        let identity = try await device.core.restoreSession()
        precondition(identity?.id == userId, "the kept token signs in as the user")
        try await device.core.registerDevice(platform: platform)
        return device
    }
}
