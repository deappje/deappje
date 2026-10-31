import Foundation
import Security

/// The one place the Keychain is touched: generic-password items under one
/// service, one item per account name, readable after the first unlock and
/// never leaving this device (`kSecAttrAccessibleAfterFirstUnlockThisDeviceOnly`:
/// a background sync after a reboot can still read the session token, a
/// backup or a new phone gets nothing, and a new device is a new device key,
/// as `docs/01-deappje/02-device.md` wants). `KeychainSecrets` keeps the
/// core's small strings in it and `KeychainDeviceKey` the X25519 private
/// half; the two never share an account name.
///
/// A Keychain failure other than "not there" is a broken device, not a
/// condition to carry on from: it stops the app.
public struct Keychain: Sendable {
    /// The `kSecAttrService` every item carries.
    public let service: String

    public init(service: String) {
        self.service = service
    }

    /// The item's bytes, or nil when there is none.
    public func data(_ account: String) -> Data? {
        var query = base(account)
        query[kSecReturnData as String] = true
        query[kSecMatchLimit as String] = kSecMatchLimitOne
        var out: CFTypeRef?
        let status = SecItemCopyMatching(query as CFDictionary, &out)
        switch status {
        case errSecSuccess:
            return out as? Data
        case errSecItemNotFound:
            return nil
        default:
            preconditionFailure("keychain: reading \(account) failed with \(status)")
        }
    }

    /// Writes or replaces the item.
    public func set(_ account: String, _ value: Data) {
        let update: [String: Any] = [kSecValueData as String: value]
        var status = SecItemUpdate(base(account) as CFDictionary, update as CFDictionary)
        if status == errSecItemNotFound {
            var item = base(account)
            item[kSecValueData as String] = value
            item[kSecAttrAccessible as String] = kSecAttrAccessibleAfterFirstUnlockThisDeviceOnly
            status = SecItemAdd(item as CFDictionary, nil)
        }
        precondition(status == errSecSuccess, "keychain: writing \(account) failed with \(status)")
    }

    /// Removes the item, if there is one.
    public func delete(_ account: String) {
        let status = SecItemDelete(base(account) as CFDictionary)
        precondition(status == errSecSuccess || status == errSecItemNotFound, "keychain: deleting \(account) failed with \(status)")
    }

    private func base(_ account: String) -> [String: Any] {
        [
            kSecClass as String: kSecClassGenericPassword,
            kSecAttrService as String: service,
            kSecAttrAccount as String: account,
        ]
    }
}
