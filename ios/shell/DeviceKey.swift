import CryptoKit
import DeappjeCore
import Foundation

/// The X25519 step as CryptoKit does it (core/src/shell.rs, `DeviceKey`).
enum X25519 {
    /// The raw 32-byte shared point of `key` and `theirPublic`. CryptoKit
    /// refuses a peer key whose product is the all-zero point (the low-order
    /// points) and a key of the wrong size; the seam cannot throw, so it
    /// answers 32 zero bytes, which is the result RFC 9180 forbids and the
    /// core refuses itself.
    static func sharedPoint(_ key: Curve25519.KeyAgreement.PrivateKey, with theirPublic: Data) -> Data {
        guard let peer = try? Curve25519.KeyAgreement.PublicKey(rawRepresentation: theirPublic),
            let secret = try? key.sharedSecretFromKeyAgreement(with: peer)
        else {
            return Data(count: 32)
        }
        return secret.withUnsafeBytes { Data($0) }
    }
}

/// A device key held in memory: a test's device, and the key
/// `KeychainDeviceKey` loads for the length of one Diffie-Hellman step.
public final class SoftwareDeviceKey: DeviceKey, @unchecked Sendable {
    private let privateKey: Curve25519.KeyAgreement.PrivateKey

    public init(privateKey: Curve25519.KeyAgreement.PrivateKey) {
        self.privateKey = privateKey
    }

    /// A fresh key from the system's entropy.
    public convenience init() {
        self.init(privateKey: Curve25519.KeyAgreement.PrivateKey())
    }

    public func publicKeyBytes() -> Data {
        privateKey.publicKey.rawRepresentation
    }

    public func dhBytes(theirPublic: Data) -> Data {
        X25519.sharedPoint(privateKey, with: theirPublic)
    }
}

/// The device's key as the app holds it (docs/01-deappje/02-device.md, "Key
/// exchange protocol"): one X25519 keypair, minted on first use.
///
/// The Secure Enclave cannot hold it: the enclave does P-256 (NIST) key
/// agreement only, and the wrap format is DHKEM(X25519) for every device on
/// every platform. The private half therefore lives in the Keychain as a
/// generic-password item, this device only and after first unlock, and is
/// loaded into memory for one Diffie-Hellman step at a time; only the public
/// half stays in this object. The core never sees the private half
/// (core/AGENTS.md, rule 1).
public final class KeychainDeviceKey: DeviceKey, @unchecked Sendable {
    /// The Keychain account the private half lives under.
    public static let account = "device-key"

    private let keychain: Keychain
    private let publicKey: Data

    /// Loads the device's key, minting one the first time. A key that is
    /// there but will not parse stops the app: a device whose key changed is
    /// a new device to every peer and the server, and that is a decision a
    /// person makes (`02-device.md`, "Implementation record"), never a
    /// fallback.
    public init(keychain: Keychain) {
        self.keychain = keychain
        if let raw = keychain.data(Self.account) {
            guard let key = try? Curve25519.KeyAgreement.PrivateKey(rawRepresentation: raw) else {
                preconditionFailure("the device key in the keychain is not an X25519 key")
            }
            publicKey = key.publicKey.rawRepresentation
        } else {
            let key = Curve25519.KeyAgreement.PrivateKey()
            keychain.set(Self.account, key.rawRepresentation)
            publicKey = key.publicKey.rawRepresentation
        }
    }

    public func publicKeyBytes() -> Data {
        publicKey
    }

    public func dhBytes(theirPublic: Data) -> Data {
        guard let raw = keychain.data(Self.account), let key = try? Curve25519.KeyAgreement.PrivateKey(rawRepresentation: raw) else {
            preconditionFailure("the device key left the keychain")
        }
        return X25519.sharedPoint(key, with: theirPublic)
    }
}
