import DeappjeShell
import Foundation
import XCTest

final class StorageTests: XCTestCase {
    private let directory = FileManager.default.temporaryDirectory.appendingPathComponent("deappje-storage-\(UUID().uuidString)", isDirectory: true)

    override func tearDown() {
        try? FileManager.default.removeItem(at: directory)
    }

    func testWritesReadsListsAndDeletesByKey() throws {
        let storage = try FileStorage(directory: directory)
        XCTAssertNil(storage.get(key: "g/1/s"))
        XCTAssertEqual(storage.keys(prefix: ""), [])

        storage.put(key: "g/1/s", value: Data([1]))
        storage.put(key: "g/1/o/b", value: Data([4]))
        storage.put(key: "k/1/1", value: Data([3]))
        storage.put(key: "g/1/o/a", value: Data([2]))
        XCTAssertEqual(storage.get(key: "g/1/s"), Data([1]))
        storage.put(key: "g/1/s", value: Data([9]))
        XCTAssertEqual(storage.get(key: "g/1/s"), Data([9]), "a put replaces")

        XCTAssertEqual(storage.keys(prefix: "g/1/o/"), ["g/1/o/a", "g/1/o/b"])
        XCTAssertEqual(storage.keys(prefix: "g/"), ["g/1/o/a", "g/1/o/b", "g/1/s"], "byte order, as the core's BTreeMap")
        XCTAssertEqual(storage.keys(prefix: ""), ["g/1/o/a", "g/1/o/b", "g/1/s", "k/1/1"])
        XCTAssertEqual(storage.keys(prefix: "p/"), [])

        storage.delete(key: "k/1/1")
        XCTAssertNil(storage.get(key: "k/1/1"))
        storage.delete(key: "k/1/1")
        XCTAssertEqual(storage.keys(prefix: "k/"), [])
    }

    func testAReopenedStoreReadsWhatWasWritten() throws {
        let value = Data((0..<1000).map { UInt8($0 % 251) })
        try FileStorage(directory: directory).put(key: "g/7f3a-9b/o/0001", value: value)
        let again = try FileStorage(directory: directory)
        XCTAssertEqual(again.get(key: "g/7f3a-9b/o/0001"), value)
        XCTAssertEqual(again.keys(prefix: "g/7f3a-9b/"), ["g/7f3a-9b/o/0001"])
    }

    func testAStrayFileIsNotAKey() throws {
        let storage = try FileStorage(directory: directory)
        storage.put(key: "g/1/s", value: Data([1]))
        try Data().write(to: directory.appendingPathComponent(".DS_Store"))
        try Data().write(to: directory.appendingPathComponent("not ours.txt"))
        XCTAssertEqual(storage.keys(prefix: ""), ["g/1/s"])
    }

    func testFileNamesEncodeEveryKeyReversibly() throws {
        let storage = try FileStorage(directory: directory)
        let odd = "x/with space/%percent/ü/.dot"
        storage.put(key: odd, value: Data([1]))
        XCTAssertEqual(storage.keys(prefix: "x/"), [odd])
        XCTAssertEqual(storage.get(key: odd), Data([1]))
        let names = try FileManager.default.contentsOfDirectory(atPath: directory.path)
        XCTAssertEqual(names.count, 1)
        XCTAssertFalse(names[0].contains("/"))
        XCTAssertFalse(names[0].hasPrefix("."))
    }
}
