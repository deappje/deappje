import CryptoKit
import DeappjeShell
import Foundation
import XCTest

/// RFC 7748, section 6.1: the X25519 test vector.
private enum Rfc7748 {
    static let alicePrivate = hex("77076d0a7318a57d3c16c17251b26645df4c2f87ebc0992ab177fba51db92c2a")
    static let alicePublic = hex("8520f0098930a754748b7ddcb43ef75a0dbf3a0d26381af4eba4a98eaa9b4e6a")
    static let bobPrivate = hex("5dab087e624a8a4b79e17f8b83800ee66f3bb1292618b6fd1c2f8b27ff88e0eb")
    static let bobPublic = hex("de9edb7d7b7dc1b4d35b61c2ece435373f8343c85b78674dadfc7e146f882b4f")
    static let shared = hex("4a5d9d5ba4ce2de1728e3bf480350f25e07e21c947d19e3376f09b3c1e161742")

    static func hex(_ s: String) -> Data {
        var out = Data(capacity: s.count / 2)
        var index = s.startIndex
        while index < s.endIndex {
            let next = s.index(index, offsetBy: 2)
            out.append(UInt8(s[index..<next], radix: 16)!)
            index = next
        }
        return out
    }
}

final class DeviceKeyTests: XCTestCase {
    private let keychain = Keychain(service: "nl.deappje.tests.devicekey.\(UUID().uuidString)")

    override func tearDown() {
        keychain.delete(KeychainDeviceKey.account)
    }

    func testTheSoftwareKeyMeetsTheRfc7748Vector() throws {
        let alice = SoftwareDeviceKey(privateKey: try Curve25519.KeyAgreement.PrivateKey(rawRepresentation: Rfc7748.alicePrivate))
        let bob = SoftwareDeviceKey(privateKey: try Curve25519.KeyAgreement.PrivateKey(rawRepresentation: Rfc7748.bobPrivate))
        XCTAssertEqual(alice.publicKeyBytes(), Rfc7748.alicePublic)
        XCTAssertEqual(bob.publicKeyBytes(), Rfc7748.bobPublic)
        XCTAssertEqual(alice.dhBytes(theirPublic: Rfc7748.bobPublic), Rfc7748.shared)
        XCTAssertEqual(bob.dhBytes(theirPublic: Rfc7748.alicePublic), Rfc7748.shared)
    }

    func testAFreshKeyIsItsOwnAndAgreesWithAPeer() {
        let a = SoftwareDeviceKey()
        let b = SoftwareDeviceKey()
        XCTAssertEqual(a.publicKeyBytes().count, 32)
        XCTAssertNotEqual(a.publicKeyBytes(), b.publicKeyBytes())
        XCTAssertEqual(a.dhBytes(theirPublic: b.publicKeyBytes()), b.dhBytes(theirPublic: a.publicKeyBytes()))
        XCTAssertNotEqual(a.dhBytes(theirPublic: b.publicKeyBytes()), Data(count: 32))
    }

    func testALowOrderPointAnswersTheAllZeroResultTheCoreRefuses() {
        let key = SoftwareDeviceKey()
        XCTAssertEqual(key.dhBytes(theirPublic: Data(count: 32)), Data(count: 32), "the identity point")
        XCTAssertEqual(key.dhBytes(theirPublic: Data(count: 31)), Data(count: 32), "not a key at all")
    }

    func testTheKeychainKeyIsMintedOnceAndLoadedForEachStep() {
        let first = KeychainDeviceKey(keychain: keychain)
        let second = KeychainDeviceKey(keychain: keychain)
        XCTAssertEqual(first.publicKeyBytes().count, 32)
        XCTAssertEqual(first.publicKeyBytes(), second.publicKeyBytes(), "one key per device")
        XCTAssertNotNil(keychain.data(KeychainDeviceKey.account), "the private half is in the keychain")
        let peer = SoftwareDeviceKey()
        XCTAssertEqual(first.dhBytes(theirPublic: peer.publicKeyBytes()), peer.dhBytes(theirPublic: first.publicKeyBytes()))
    }

    func testTheKeychainKeyIsTheRfc7748KeyWhenAlicesIsInTheKeychain() {
        keychain.set(KeychainDeviceKey.account, Rfc7748.alicePrivate)
        let alice = KeychainDeviceKey(keychain: keychain)
        XCTAssertEqual(alice.publicKeyBytes(), Rfc7748.alicePublic)
        XCTAssertEqual(alice.dhBytes(theirPublic: Rfc7748.bobPublic), Rfc7748.shared)
    }
}
