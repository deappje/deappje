import DeappjeCore
import Foundation

/// One HTTP request, bytes in and bytes out, over URLSession
/// (core/src/transport.rs). The core adds every header it needs (the
/// session token, the content type) and reads the status itself; TLS is
/// the system's. Cookies are refused: the session travels as a token, and
/// a cookie Kratos set on one flow must not colour the next.
public final class URLSessionTransport: Transport, @unchecked Sendable {
    private let session: URLSession

    /// Over the configuration, which a test may give protocol classes.
    public init(configuration: URLSessionConfiguration = URLSessionTransport.configuration()) {
        session = URLSession(configuration: configuration)
    }

    /// The app's configuration: ephemeral, no cookies, no cache.
    public static func configuration() -> URLSessionConfiguration {
        let configuration = URLSessionConfiguration.ephemeral
        configuration.httpShouldSetCookies = false
        configuration.httpCookieAcceptPolicy = .never
        configuration.urlCache = nil
        configuration.requestCachePolicy = .reloadIgnoringLocalCacheData
        return configuration
    }

    public func send(request: Request) async throws -> Response {
        guard let url = URL(string: request.url) else {
            throw TransportError.Network(message: "not a URL: \(request.url)")
        }
        var urlRequest = URLRequest(url: url)
        urlRequest.httpMethod = request.method
        for header in request.headers {
            urlRequest.setValue(header.value, forHTTPHeaderField: header.name)
        }
        if !request.body.isEmpty {
            urlRequest.httpBody = request.body
        }
        let (body, response): (Data, URLResponse)
        do {
            (body, response) = try await session.data(for: urlRequest)
        } catch {
            throw TransportError.Network(message: error.localizedDescription)
        }
        guard let http = response as? HTTPURLResponse else {
            throw TransportError.Network(message: "no HTTP response from \(request.url)")
        }
        let headers = http.allHeaderFields
            .compactMap { name, value -> Header? in
                guard let name = name as? String else { return nil }
                return Header(name: name.lowercased(), value: String(describing: value))
            }
            .sorted { $0.name < $1.name }
        return Response(status: UInt16(http.statusCode), headers: headers, body: body)
    }
}
