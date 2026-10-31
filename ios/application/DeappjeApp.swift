import SwiftUI

/// The app: one model over one core, the push delegate, and the schedule
/// that follows the scene (a sync on every activation, every 30 s while
/// active, nothing in the background but what a push wakes).
@main
struct DeappjeApp: App {
    @UIApplicationDelegateAdaptor(AppDelegate.self) private var delegate
    @Environment(\.scenePhase) private var scenePhase
    @State private var model: AppModel

    init() {
        _model = State(initialValue: AppModel(core: AppCore.open(), browser: WebAuthenticationBrowser()))
    }

    var body: some Scene {
        WindowGroup {
            RootView(model: model)
                .task {
                    delegate.model = model
                    model.registerForPush = { UIApplication.shared.registerForRemoteNotifications() }
                    await model.start()
                }
                .onChange(of: scenePhase) { _, phase in
                    switch phase {
                    case .active:
                        Task { await model.activated() }
                    case .background:
                        model.deactivated()
                    case .inactive:
                        break
                    @unknown default:
                        break
                    }
                }
        }
    }
}
