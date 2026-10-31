import DeappjeCore
import Foundation

/// The wall clock, milliseconds since the Unix epoch, UTC: what authored
/// and edited times are stamped from (core/src/shell.rs, `Clock`).
public final class SystemClock: Clock, @unchecked Sendable {
    public init() {}

    public func nowMs() -> Int64 {
        Int64((Date().timeIntervalSince1970 * 1000).rounded(.down))
    }
}
