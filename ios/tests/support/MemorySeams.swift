import DeappjeCore
import DeappjeShell
import Foundation

/// Secrets in memory: a test's device.
public final class MemorySecrets: Secrets, @unchecked Sendable {
    private let lock = NSLock()
    private var values: [String: String] = [:]

    public init() {}

    public func getSecret(name: String) -> String? {
        lock.withLock { values[name] }
    }

    public func setSecret(name: String, value: String) {
        lock.withLock { values[name] = value }
    }

    public func deleteSecret(name: String) {
        lock.withLock { _ = values.removeValue(forKey: name) }
    }
}

/// Storage in memory: a test's device.
public final class MemoryStorage: Storage, @unchecked Sendable {
    private let lock = NSLock()
    private var blobs: [String: Data] = [:]

    public init() {}

    public var count: Int {
        lock.withLock { blobs.count }
    }

    public func get(key: String) -> Data? {
        lock.withLock { blobs[key] }
    }

    public func put(key: String, value: Data) {
        lock.withLock { blobs[key] = value }
    }

    public func delete(key: String) {
        lock.withLock { _ = blobs.removeValue(forKey: key) }
    }

    public func keys(prefix: String) -> [String] {
        lock.withLock { blobs.keys.filter { $0.hasPrefix(prefix) }.sorted { $0.utf8.lexicographicallyPrecedes($1.utf8) } }
    }
}

/// A clock a test moves.
public final class FixedClock: Clock, @unchecked Sendable {
    private let lock = NSLock()
    private var now: Int64

    public init(nowMs: Int64) {
        now = nowMs
    }

    public func nowMs() -> Int64 {
        lock.withLock { now }
    }

    public func advance(ms: Int64) {
        lock.withLock { now += ms }
    }
}

/// Entropy from a seed: SplitMix64, so every run of a test draws the same
/// bytes (the core seeds its own generator from the first draw).
public final class SeededRandom: Random, @unchecked Sendable {
    private let lock = NSLock()
    private var state: UInt64

    public init(seed: UInt64) {
        state = seed
    }

    public func randomBytes() -> Data {
        lock.withLock {
            var out = Data(capacity: 32)
            for _ in 0..<4 {
                state &+= 0x9E37_79B9_7F4A_7C15
                var z = state
                z = (z ^ (z >> 30)) &* 0xBF58_476D_1CE4_E5B9
                z = (z ^ (z >> 27)) &* 0x94D0_49BB_1331_11EB
                z ^= z >> 31
                withUnsafeBytes(of: z.littleEndian) { out.append(contentsOf: $0) }
            }
            return out
        }
    }
}
