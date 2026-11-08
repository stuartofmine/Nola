// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Verifies macOS context usage is text-token based and multilingual.

import XCTest
@testable import Nola

@MainActor
final class MacContextTokenEstimatorTests: XCTestCase {
    func testCJKTextUsesDenseTokenEstimate() {
        XCTAssertEqual(
            MacContextTokenEstimator.estimateText(String(repeating: "你", count: 100)),
            100
        )
    }

    func testASCIITextUsesUTF8Ratio() {
        XCTAssertEqual(
            MacContextTokenEstimator.estimateText(String(repeating: "a", count: 400)),
            100
        )
    }

    func testContextIncludesMessagesDraftPromptAndPreferences() {
        let messageOnly = MacContextTokenEstimator.estimateComponents(
            messageTexts: [String(repeating: "你", count: 80)]
        )
        let complete = MacContextTokenEstimator.estimateComponents(
            messageTexts: [String(repeating: "你", count: 80)],
            draft: String(repeating: "a", count: 40),
            promptTemplate: String(repeating: "b", count: 40),
            userPreferences: String(repeating: "我", count: 10)
        )

        XCTAssertEqual(messageOnly, 87)
        XCTAssertEqual(complete, 121)
        XCTAssertGreaterThan(complete, messageOnly)
    }
}
