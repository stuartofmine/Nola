// SPDX-License-Identifier: Apache-2.0
import AppKit
import XCTest

/// Enables lazy SwiftUI AX subtrees only in the current test-host process.
/// The public AXUIElement client API returns apiDisabled for this unsigned
/// self-process fixture. AppKit's documented in-process informal AX protocol
/// supports the attribute without TCC access or system preference changes.
/// It has no equivalent property in the modern NSAccessibility protocol, so
/// this test adapter dispatches those documented Objective-C selectors directly.
@MainActor
final class MacHostedAccessibilitySession {
    private let application = NSApplication.shared
    private let attribute = "AXEnhancedUserInterface" as NSString
    private let getter = NSSelectorFromString("accessibilityAttributeValue:")
    private let setter = NSSelectorFromString("accessibilitySetValue:forAttribute:")
    private let previous: NSNumber
    private var restored = false

    init() throws {
        previous = try XCTUnwrap(application.perform(getter, with: attribute)?.takeUnretainedValue() as? NSNumber,
                                "The hosted AX fixture must capture its existing process-local activation state.")
        application.perform(setter, with: NSNumber(value: true), with: attribute)
    }

    func restore() {
        guard !restored else { return }
        application.perform(setter, with: previous, with: attribute)
        restored = true
    }
}
