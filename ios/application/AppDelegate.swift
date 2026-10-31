import Foundation
import os
import UIKit

/// What only a UIApplicationDelegate receives: the push token and the
/// silent pushes. It hands both to the model the app gave it.
@MainActor
public final class AppDelegate: NSObject, UIApplicationDelegate {
    /// Set by the app before the first push can arrive.
    public var model: AppModel?
    private let log = Logger(subsystem: "nl.deappje.app", category: "push")

    public func application(_ application: UIApplication, didRegisterForRemoteNotificationsWithDeviceToken deviceToken: Data) {
        Task { await model?.pushToken(deviceToken) }
    }

    public func application(_ application: UIApplication, didFailToRegisterForRemoteNotificationsWithError error: Error) {
        // The simulator has no APNs, and a phone without a network has none
        // yet: the foreground sync and the timer carry the app meanwhile.
        log.info("no push token: \(error.localizedDescription)")
    }

    public func application(_ application: UIApplication, didReceiveRemoteNotification userInfo: [AnyHashable: Any]) async -> UIBackgroundFetchResult {
        guard let wake = WakeUp(userInfo: userInfo), let model else { return .noData }
        await model.handle(wake)
        return .newData
    }
}
