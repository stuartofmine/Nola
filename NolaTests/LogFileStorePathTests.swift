// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Verifies that runtime diagnostics remain in Nola's application support namespace.

import XCTest
@testable import Nola

final class LogFileStorePathTests: XCTestCase {
    func testLogFileUsesProductOwnedApplicationSupportDirectory() {
        let url = LogFileStore.shared.logFileURL()

        XCTAssertEqual(url.lastPathComponent, "nola.log")
        XCTAssertEqual(url.deletingLastPathComponent().lastPathComponent, "Logs")
        XCTAssertTrue(
            url.path.contains("Nola-Test-Runtime-")
                || url.deletingLastPathComponent().deletingLastPathComponent().lastPathComponent == "Nola",
            "Tests must use an isolated runtime; production must use Nola's support directory."
        )
    }
}
