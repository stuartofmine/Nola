// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Prevents permission-bound keyboard operations from regressing to an external process identity.

import CoreGraphics
import XCTest
@testable import Nola

final class MacComputerUseBridgeTests: XCTestCase {
    func testKeyboardMapperSupportsCommonTypingAndNavigationKeys() {
        XCTAssertEqual(MacKeyboardEventMapper.keyCode(for: "A"), 0)
        XCTAssertEqual(MacKeyboardEventMapper.keyCode(for: "enter"), 36)
        XCTAssertEqual(MacKeyboardEventMapper.keyCode(for: "left_arrow"), 123)
        XCTAssertEqual(MacKeyboardEventMapper.keyCode(for: "escape"), 53)
    }

    func testKeyboardMapperNormalizesShortcutModifiers() {
        let flags = MacKeyboardEventMapper.flags(for: ["CMD", "Shift", "alt", "ctrl"])
        XCTAssertTrue(flags.contains(.maskCommand))
        XCTAssertTrue(flags.contains(.maskShift))
        XCTAssertTrue(flags.contains(.maskAlternate))
        XCTAssertTrue(flags.contains(.maskControl))
    }
}
