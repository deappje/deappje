import AuthenticationServices
import Foundation
import UIKit

/// Where the sign-in happens: the system browser, which comes back to the
/// app's URL with the code. A test gives a browser of its own.
public protocol LoginBrowser: Sendable {
    /// Opens `url` and returns the callback URL the browser was sent to.
    func authenticate(url: URL, callbackScheme: String) async throws -> URL
}

/// Why a sign-in did not complete in the browser.
public enum LoginError: Error, Equatable {
    /// The person closed the browser.
    case cancelled
    /// The browser came back without a `code`.
    case noCode(URL)
    /// Kratos gave a browser URL that is not one.
    case badUrl(String)
}

/// `ASWebAuthenticationSession`: the system's sign-in sheet, sharing the
/// browser's cookies so a person signed in to Basable there is through at
/// once, and handing the `deappje://login?code=...` return to the app
/// without the URL ever reaching another app.
@MainActor
public final class WebAuthenticationBrowser: NSObject, LoginBrowser, ASWebAuthenticationPresentationContextProviding {
    private var session: ASWebAuthenticationSession?

    public override init() {}

    public func authenticate(url: URL, callbackScheme: String) async throws -> URL {
        try await withCheckedThrowingContinuation { continuation in
            let session = ASWebAuthenticationSession(url: url, callbackURLScheme: callbackScheme) { callback, error in
                if let callback {
                    continuation.resume(returning: callback)
                } else if let error = error as? ASWebAuthenticationSessionError, error.code == .canceledLogin {
                    continuation.resume(throwing: LoginError.cancelled)
                } else {
                    continuation.resume(throwing: error ?? LoginError.cancelled)
                }
            }
            session.presentationContextProvider = self
            session.prefersEphemeralWebBrowserSession = false
            self.session = session
            session.start()
        }
    }

    public func presentationAnchor(for session: ASWebAuthenticationSession) -> ASPresentationAnchor {
        let windows = UIApplication.shared.connectedScenes.compactMap { $0 as? UIWindowScene }.flatMap(\.windows)
        return windows.first(where: \.isKeyWindow) ?? windows.first ?? ASPresentationAnchor()
    }
}
