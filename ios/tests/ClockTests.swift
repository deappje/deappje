import DeappjeShell
import Foundation
import XCTest

final class ClockTests: XCTestCase {
    func testNowIsTheWallClockInMilliseconds() {
        let before = Int64((Date().timeIntervalSince1970 * 1000).rounded(.down))
        let now = SystemClock().nowMs()
        let after = Int64((Date().timeIntervalSince1970 * 1000).rounded(.up))
        XCTAssertGreaterThanOrEqual(now, before)
        XCTAssertLessThanOrEqual(now, after)
        // Milliseconds, not seconds and not nanoseconds: 2023 onwards, well
        // before the year 3000.
        XCTAssertGreaterThan(now, 1_680_000_000_000)
        XCTAssertLessThan(now, 32_500_000_000_000)
    }

    func testTwoReadsDoNotRunBackwards() {
        let clock = SystemClock()
        let first = clock.nowMs()
        let second = clock.nowMs()
        XCTAssertLessThanOrEqual(first, second)
    }
}
