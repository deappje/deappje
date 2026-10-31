import DeappjeCore
import Foundation

/// The core's small named strings (the device id, the session token) in the
/// Keychain (core/src/shell.rs, `Secrets`). Each name is one item, under
/// its own account prefix so it can never collide with the device key.
public final class KeychainSecrets: Secrets, @unchecked Sendable {
    private let keychain: Keychain

    public init(keychain: Keychain) {
        self.keychain = keychain
    }

    public func getSecret(name: String) -> String? {
        keychain.data(Self.account(name)).map { String(decoding: $0, as: UTF8.self) }
    }

    public func setSecret(name: String, value: String) {
        keychain.set(Self.account(name), Data(value.utf8))
    }

    public func deleteSecret(name: String) {
        keychain.delete(Self.account(name))
    }

    static func account(_ name: String) -> String {
        "secret." + name
    }
}
