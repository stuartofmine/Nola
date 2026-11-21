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

    func element(_ identifier: String, in root: NSView) -> AnyObject? {
        var queue = [root as AnyObject]
        var visited = Set<ObjectIdentifier>()
        while let object = queue.popLast() {
            guard visited.insert(ObjectIdentifier(object)).inserted else { continue }
            if object.accessibilityIdentifier?() == identifier { return object }
            if let view = object as? NSView { queue.append(contentsOf: view.subviews) }
            let children = object.accessibilityChildren?() ?? []
            queue.append(contentsOf: (children + NSAccessibility.unignoredChildren(from: children)).map { $0 as AnyObject })
        }
        return nil
    }

    struct Control: Hashable, Encodable {
        let role: String
        let identifier: String
        let label: String
        let frame: String
        let enabled: Bool
    }

    func visibleControls(in root: NSView) -> [Control] {
        guard let window = root.window else { return [] }
        let viewport = window.convertToScreen(root.convert(root.bounds, to: nil))
        let roles = Set(["AXButton", "AXPopUpButton", "AXMenuButton", "AXCheckBox", "AXRadioButton",
                         "AXLink", "AXDisclosureTriangle", "AXTextField", "AXTextArea", "AXComboBox",
                         "AXSlider", "AXIncrementor", "AXTab", "AXSwitch", "AXMenuItem"])
        var queue = [root as AnyObject]
        var visited = Set<ObjectIdentifier>()
        var controls = Set<Control>()
        while let object = queue.popLast() {
            guard visited.insert(ObjectIdentifier(object)).inserted else { continue }
            if let role = (object.accessibilityRole?() ?? nil)?.rawValue, roles.contains(role),
               let frame = object.accessibilityFrame?(), !frame.isEmpty, viewport.intersects(frame) {
                controls.insert(Control(role: role, identifier: (object.accessibilityIdentifier?() ?? nil) ?? "",
                    label: (object.accessibilityLabel?() ?? nil) ?? "", frame: NSStringFromRect(frame),
                    enabled: object.isAccessibilityEnabled?() ?? true))
            }
            if let view = object as? NSView { queue.append(contentsOf: view.subviews) }
            let children = object.accessibilityChildren?() ?? []
            queue.append(contentsOf: (children + NSAccessibility.unignoredChildren(from: children)).map { $0 as AnyObject })
        }
        return controls.sorted { ($0.frame, $0.role, $0.identifier, $0.label) < ($1.frame, $1.role, $1.identifier, $1.label) }
    }

    func restore() {
        guard !restored else { return }
        application.perform(setter, with: previous, with: attribute)
        restored = true
    }
}
