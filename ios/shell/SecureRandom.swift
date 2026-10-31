import DeappjeCore
import Foundation
import Security

/// Entropy from the system: `SecRandomCopyBytes`, the kernel's CSPRNG. The
/// core seeds its own generator from it and asks again only when it
/// reseeds (core/src/shell.rs, `Random`).
public final class SecureRandom: Random, @unchecked Sendable {
    public init() {}

    public func randomBytes() -> Data {
        var bytes = [UInt8](repeating: 0, count: 32)
        let status = bytes.withUnsafeMutableBytes { buffer in
            SecRandomCopyBytes(kSecRandomDefault, buffer.count, buffer.baseAddress!)
        }
        precondition(status == errSecSuccess, "SecRandomCopyBytes failed with \(status)")
        return Data(bytes)
    }
}
