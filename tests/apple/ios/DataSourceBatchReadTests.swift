// SPDX-License-Identifier: Apache-2.0
import XCTest
@testable import Nola

final class DataSourceBatchReadTests: XCTestCase {
    func testSuccessfulSourcesKeepAllEvents() async throws {
        let values = try await DataSourceBatchRead.collect(reads: [{ @Sendable in [1, 2] }, { @Sendable in [3] }])
        XCTAssertEqual(values.sorted(), [1, 2, 3])
    }

    func testFailedSourceCannotBecomeAnEmptySuccessfulCount() async {
        enum Failure: Error { case offline }
        do {
            let _: [Int] = try await DataSourceBatchRead.collect(reads: [{ @Sendable in [1] }, { @Sendable in throw Failure.offline }])
            XCTFail("A partial result cannot be displayed as a complete event count")
        } catch { XCTAssertTrue(error is Failure) }
    }

    func testCancelledReadIsNotAZeroCount() async {
        do {
            let _: [Int] = try await DataSourceBatchRead.collect(reads: [{ @Sendable in throw CancellationError() }])
            XCTFail("Cancellation must reach the view without changing its last successful count")
        } catch { XCTAssertTrue(error is CancellationError) }
    }
}
