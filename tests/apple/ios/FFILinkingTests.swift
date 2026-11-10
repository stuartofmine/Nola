// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Implements FFILinkingTests logic in the project module.
// - Interfaces:
//   - FFILinkingTests
// - APIs:
//   - none
//
import XCTest
@testable import Nola

final class FFILinkingTests: XCTestCase {
    func testRustInteropGeneratesDedupeKey() {
        let key = RustInterop.generateDedupeKey(
            source: "calendar",
            title: "Test",
            dateISO: nil,
            sourceId: nil
        )

        XCTAssertNotNil(key)
        XCTAssertFalse(key?.isEmpty ?? true)
    }
}
