// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Verifies persistent background services never bind to ephemeral Xcode products.

import XCTest
@testable import Nola

final class MacProcessRuntimePolicyTests: XCTestCase {
    func testPersistentSchedulerOnlyAcceptsStableApplicationBundles() {
        XCTAssertTrue(
            MacProcessRuntimePolicy.isStableInstalledApplication(
                URL(fileURLWithPath: "/Applications/Nola.app")
            )
        )
        XCTAssertTrue(
            MacProcessRuntimePolicy.isStableInstalledApplication(
                URL(fileURLWithPath: "/Users/example/Applications/Nola.app")
            )
        )
        XCTAssertFalse(
            MacProcessRuntimePolicy.isStableInstalledApplication(
                URL(fileURLWithPath: "/Users/example/Library/Developer/Xcode/DerivedData/Nola/Build/Products/Debug/Nola.app")
            )
        )
        XCTAssertFalse(
            MacProcessRuntimePolicy.isStableInstalledApplication(
                URL(fileURLWithPath: "/Users/example/Nola/Build/Products/Debug/Nola.app")
            )
        )
        XCTAssertFalse(
            MacProcessRuntimePolicy.isStableInstalledApplication(
                URL(fileURLWithPath: "/usr/local/bin/nola")
            )
        )
    }
}
