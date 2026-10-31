import DeappjeShell
import Foundation
import XCTest

final class SecretsTests: XCTestCase {
    private let keychain = Keychain(service: "nl.deappje.tests.secrets.\(UUID().uuidString)")

    override func tearDown() {
        let secrets = KeychainSecrets(keychain: keychain)
        for name in ["device_id", "session_token", "device-key"] {
            secrets.deleteSecret(name: name)
        }
        keychain.delete(KeychainDeviceKey.account)
    }

    func testSetsReadsReplacesAndDeletes() {
        let secrets = KeychainSecrets(keychain: keychain)
        XCTAssertNil(secrets.getSecret(name: "device_id"))
        secrets.setSecret(name: "device_id", value: "one")
        XCTAssertEqual(secrets.getSecret(name: "device_id"), "one")
        secrets.setSecret(name: "device_id", value: "two")
        XCTAssertEqual(secrets.getSecret(name: "device_id"), "two")
        secrets.setSecret(name: "session_token", value: "ory_st_abc")
        XCTAssertEqual(secrets.getSecret(name: "session_token"), "ory_st_abc")
        XCTAssertEqual(secrets.getSecret(name: "device_id"), "two", "names are separate items")
        secrets.deleteSecret(name: "device_id")
        XCTAssertNil(secrets.getSecret(name: "device_id"))
        secrets.deleteSecret(name: "device_id")
        XCTAssertEqual(secrets.getSecret(name: "session_token"), "ory_st_abc")
    }

    func testTheKeychainOutlivesTheInstance() {
        KeychainSecrets(keychain: keychain).setSecret(name: "device_id", value: "kept")
        XCTAssertEqual(KeychainSecrets(keychain: keychain).getSecret(name: "device_id"), "kept")
    }

    func testASecretNeverLandsOnTheDeviceKeysItem() {
        KeychainSecrets(keychain: keychain).setSecret(name: "device-key", value: "not a key")
        XCTAssertNil(keychain.data(KeychainDeviceKey.account))
    }
}
