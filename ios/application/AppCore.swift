import DeappjeCore
import DeappjeShell
import Foundation

/// The one `Core` of the app, over the production seams and the deployment
/// of this build.
public enum AppCore {
    /// The Keychain service every item of the app is under.
    public static let keychainService = "nl.deappje.app"
    /// Where the native sign-in returns to (Info.plist registers the scheme;
    /// core/src/kratos.rs puts the code in its `code` query item).
    public static let loginReturnUrl = "deappje://login"
    /// The scheme of that URL.
    public static let loginCallbackScheme = "deappje"

    /// Opens the core: the device id and the session token from the
    /// Keychain, the store from Application Support, the key from the
    /// Keychain, entropy and the clock from the system, HTTP from
    /// URLSession, against `Deployment.baseUrl`.
    public static func open() -> Core {
        let keychain = Keychain(service: keychainService)
        let storage: FileStorage
        do {
            storage = try FileStorage.applicationSupport()
        } catch {
            preconditionFailure("the store's directory cannot be made: \(error)")
        }
        return Core(
            transport: URLSessionTransport(),
            storage: storage,
            secrets: KeychainSecrets(keychain: keychain),
            deviceKey: KeychainDeviceKey(keychain: keychain),
            random: SecureRandom(),
            clock: SystemClock(),
            baseUrl: Deployment.baseUrl
        )
    }
}
