import DeappjeShell
import Foundation
import XCTest

final class RandomTests: XCTestCase {
    func testEveryDrawIs32FreshBytes() {
        let random = SecureRandom()
        let first = random.randomBytes()
        let second = random.randomBytes()
        XCTAssertEqual(first.count, 32)
        XCTAssertEqual(second.count, 32)
        XCTAssertNotEqual(first, second)
        XCTAssertNotEqual(first, Data(count: 32))
    }
}
