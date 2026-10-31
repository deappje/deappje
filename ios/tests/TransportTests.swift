import DeappjeCore
import DeappjeShell
import Foundation
import XCTest

/// Answers every request of a URLSession from the test, and keeps the last.
final class StubProtocol: URLProtocol {
    final class Box: @unchecked Sendable {
        var handler: ((URLRequest, Data) throws -> (HTTPURLResponse, Data))?
    }

    nonisolated(unsafe) static let box = Box()

    override class func canInit(with request: URLRequest) -> Bool { true }
    override class func canonicalRequest(for request: URLRequest) -> URLRequest { request }

    override func startLoading() {
        do {
            let (response, data) = try Self.box.handler!(request, Self.body(of: request))
            client?.urlProtocol(self, didReceive: response, cacheStoragePolicy: .notAllowed)
            client?.urlProtocol(self, didLoad: data)
            client?.urlProtocolDidFinishLoading(self)
        } catch {
            client?.urlProtocol(self, didFailWithError: error)
        }
    }

    override func stopLoading() {}

    /// URLSession hands a protocol the body as a stream.
    private static func body(of request: URLRequest) -> Data {
        if let body = request.httpBody { return body }
        guard let stream = request.httpBodyStream else { return Data() }
        stream.open()
        defer { stream.close() }
        var out = Data()
        var buffer = [UInt8](repeating: 0, count: 4096)
        while stream.hasBytesAvailable {
            let read = stream.read(&buffer, maxLength: buffer.count)
            if read <= 0 { break }
            out.append(buffer, count: read)
        }
        return out
    }
}

final class TransportTests: XCTestCase {
    private func transport() -> URLSessionTransport {
        let configuration = URLSessionTransport.configuration()
        configuration.protocolClasses = [StubProtocol.self]
        return URLSessionTransport(configuration: configuration)
    }

    func testTheRequestGoesThroughWholeAndTheResponseComesBackLowercased() async throws {
        final class Seen: @unchecked Sendable {
            var request: URLRequest?
            var body = Data()
        }
        let seen = Seen()
        StubProtocol.box.handler = { request, body in
            seen.request = request
            seen.body = body
            let response = HTTPURLResponse(url: request.url!, statusCode: 412, httpVersion: "HTTP/1.1", headerFields: ["Content-Type": "application/json", "X-Deappje": "yes"])!
            return (response, Data(#"{"code":"failed_precondition"}"#.utf8))
        }
        let request = Request(
            method: "POST",
            url: "https://app.example/api/relay.v1.RelayService/Push",
            headers: [Header(name: "content-type", value: "application/proto"), Header(name: "x-session-token", value: "tok")],
            body: Data([1, 2, 3])
        )
        let response = try await transport().send(request: request)

        XCTAssertEqual(seen.request?.httpMethod, "POST")
        XCTAssertEqual(seen.request?.url?.absoluteString, request.url)
        XCTAssertEqual(seen.request?.value(forHTTPHeaderField: "content-type"), "application/proto")
        XCTAssertEqual(seen.request?.value(forHTTPHeaderField: "x-session-token"), "tok")
        XCTAssertEqual(seen.body, Data([1, 2, 3]))

        XCTAssertEqual(response.status, 412)
        XCTAssertTrue(response.headers.contains(Header(name: "content-type", value: "application/json")), "\(response.headers)")
        XCTAssertTrue(response.headers.contains(Header(name: "x-deappje", value: "yes")), "\(response.headers)")
        XCTAssertEqual(response.body, Data(#"{"code":"failed_precondition"}"#.utf8))
    }

    func testAGetCarriesNoBody() async throws {
        final class Seen: @unchecked Sendable {
            var hadBody = true
        }
        let seen = Seen()
        StubProtocol.box.handler = { request, body in
            seen.hadBody = request.httpBodyStream != nil || !body.isEmpty
            return (HTTPURLResponse(url: request.url!, statusCode: 200, httpVersion: nil, headerFields: nil)!, Data([7]))
        }
        let response = try await transport().send(request: Request(method: "GET", url: "https://app.example/bucket/x", headers: [], body: Data()))
        XCTAssertFalse(seen.hadBody)
        XCTAssertEqual(response.status, 200)
        XCTAssertEqual(response.body, Data([7]))
    }

    func testTheNetworkFailingIsATransportError() async {
        StubProtocol.box.handler = { _, _ in throw URLError(.notConnectedToInternet) }
        do {
            _ = try await transport().send(request: Request(method: "GET", url: "https://app.example/", headers: [], body: Data()))
            XCTFail("no response was possible")
        } catch TransportError.Network {
            // What the core maps to `Unavailable`.
        } catch {
            XCTFail("\(error)")
        }
    }

    func testANonUrlIsATransportError() async {
        do {
            _ = try await transport().send(request: Request(method: "GET", url: "", headers: [], body: Data()))
            XCTFail("no request was possible")
        } catch TransportError.Network {
        } catch {
            XCTFail("\(error)")
        }
    }
}
