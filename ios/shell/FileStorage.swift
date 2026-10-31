import DeappjeCore
import Foundation

/// The core's local store on disk (core/src/shell.rs, `Storage`): one file
/// per key in one directory under Application Support, written atomically
/// and protected until the first unlock after a boot (the same class as the
/// Keychain items, so a background sync can open the store whenever it can
/// read the session token). The device's private key is never in it.
///
/// A key (`g/<group>/o/<id>`, `k/<group>/<n>`, ...) becomes a file name by
/// percent-encoding everything but letters, digits, `-` and `_`, so the
/// listing can turn names back into keys exactly. A write that fails stops
/// the app: the core writes through on every change and relies on reading
/// the same back at the next open.
public final class FileStorage: Storage, @unchecked Sendable {
    /// Where the files are.
    public let directory: URL

    private static let safe = CharacterSet(charactersIn: "abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789-_")

    /// Over a directory, created if needed.
    public init(directory: URL) throws {
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true, attributes: [.protectionKey: FileProtectionType.completeUntilFirstUserAuthentication])
        self.directory = directory
    }

    /// The app's store: `Application Support/deappje/<name>`.
    public static func applicationSupport(name: String = "store") throws -> FileStorage {
        let support = try FileManager.default.url(for: .applicationSupportDirectory, in: .userDomainMask, appropriateFor: nil, create: true)
        return try FileStorage(directory: support.appendingPathComponent("deappje", isDirectory: true).appendingPathComponent(name, isDirectory: true))
    }

    public func get(key: String) -> Data? {
        do {
            return try Data(contentsOf: url(for: key))
        } catch let error as CocoaError where error.code == .fileReadNoSuchFile {
            return nil
        } catch {
            preconditionFailure("storage: reading \(key) failed: \(error)")
        }
    }

    public func put(key: String, value: Data) {
        do {
            try value.write(to: url(for: key), options: [.atomic, .completeFileProtectionUntilFirstUserAuthentication])
        } catch {
            preconditionFailure("storage: writing \(key) failed: \(error)")
        }
    }

    public func delete(key: String) {
        do {
            try FileManager.default.removeItem(at: url(for: key))
        } catch let error as CocoaError where error.code == .fileNoSuchFile {
            return
        } catch {
            preconditionFailure("storage: deleting \(key) failed: \(error)")
        }
    }

    public func keys(prefix: String) -> [String] {
        let names: [String]
        do {
            names = try FileManager.default.contentsOfDirectory(atPath: directory.path)
        } catch {
            preconditionFailure("storage: listing failed: \(error)")
        }
        return names
            .compactMap(Self.key(fileName:))
            .filter { $0.hasPrefix(prefix) }
            .sorted { $0.utf8.lexicographicallyPrecedes($1.utf8) }
    }

    private func url(for key: String) -> URL {
        directory.appendingPathComponent(Self.fileName(key: key), isDirectory: false)
    }

    static func fileName(key: String) -> String {
        key.addingPercentEncoding(withAllowedCharacters: safe)!
    }

    /// The key a file name encodes; nil for a file that is not one of ours.
    static func key(fileName: String) -> String? {
        guard let key = fileName.removingPercentEncoding, Self.fileName(key: key) == fileName else { return nil }
        return key
    }
}
