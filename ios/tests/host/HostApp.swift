import SwiftUI

/// The app the tests run inside: the Keychain answers a process with a
/// bundle identity only (a bare xctest gets errSecMissingEntitlement on the
/// simulator), and this one does nothing of its own, so no network call or
/// kept session colours a test.
@main
struct HostApp: App {
    var body: some Scene {
        WindowGroup {
            Text("deappje tests")
        }
    }
}
