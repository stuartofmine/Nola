// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Ensures capability rows expose controls only when the control performs a real action.

import XCTest
@testable import Nola

final class MacCapabilityPresentationTests: XCTestCase {
    func testRuntimeManagedCapabilitiesDoNotExposeDeadSettingsButtons() {
        for kind in [
            MacCapabilitySnapshot.Kind.files,
            .shell,
            .git,
            .network,
            .browser,
        ] {
            let capability = MacCapabilitySnapshot(
                kind: kind,
                title: "Runtime capability",
                detail: "Resolved per run",
                symbol: "circle",
                state: .optional
            )
            XCTAssertFalse(capability.hasUserAction, "\(kind) exposed a button without an action")
        }
    }

    func testSystemPermissionCapabilitiesExposeRealActions() {
        let accessibility = MacCapabilitySnapshot(
            kind: .accessibility,
            title: "Accessibility",
            detail: "Required",
            symbol: "accessibility",
            state: .permissionRequired
        )
        let automation = MacCapabilitySnapshot(
            kind: .automation,
            title: "Automation",
            detail: "Optional",
            symbol: "app.badge.checkmark",
            state: .optional
        )
        XCTAssertTrue(accessibility.hasUserAction)
        XCTAssertTrue(automation.hasUserAction)
    }
}
