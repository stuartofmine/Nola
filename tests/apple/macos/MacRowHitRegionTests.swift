// SPDX-License-Identifier: Apache-2.0
import AppKit
import Combine
import SwiftUI
import XCTest
@testable import Nola

@MainActor
final class MacRowHitRegionTests: XCTestCase {
    func testDisclosureExposesOneFullWidthTargetIncludingTrailingWhitespace() async throws {
        let accessibility = try MacHostedAccessibilitySession()
        defer { accessibility.restore() }
        let state = RowState()
        let host = NSHostingView(rootView: RowProbe(state: state).environment(\.accessibilityEnabled, true).frame(width: 360, height: 140))
        host.frame = NSRect(x: 0, y: 0, width: 360, height: 140)
        let window = NSWindow(contentRect: host.bounds, styleMask: [.titled], backing: .buffered, defer: false)
        window.isReleasedWhenClosed = false
        window.contentView = host
        window.makeKeyAndOrderFront(nil)
        defer { window.orderOut(nil); window.contentView = nil; window.close() }
        let deadline = ContinuousClock.now + .seconds(2)
        while objects(host).filter { let role = $0.accessibilityRole?() ?? nil; return role == .button || role == .disclosureTriangle }.isEmpty && ContinuousClock.now < deadline { try await Task.sleep(for: .milliseconds(20)) }
        host.layoutSubtreeIfNeeded()
        let actions = objects(host).filter { let role = $0.accessibilityRole?() ?? nil; return role == .button || role == .disclosureTriangle }
        XCTAssertEqual(actions.count, 1)
        let action = try XCTUnwrap(actions.first)
        let frame = try XCTUnwrap(action.accessibilityFrame?())
        XCTAssertGreaterThanOrEqual(frame.width, 330, "The empty trailing area belongs to the row, not only its label or arrow")
        XCTAssertGreaterThanOrEqual(frame.height, 36)
        XCTAssertTrue(action.accessibilityPerformPress?() ?? false)
        XCTAssertTrue(state.expanded)
    }

    func testExtensionFooterOpensDetailsFromAFullWidthControl() async throws {
        let accessibility = try MacHostedAccessibilitySession()
        defer { accessibility.restore() }
        let item = MacExtensionItem(id: "hit-region-fixture", kind: .skill, source: .global,
            title: "Fixture skill", detail: "Reads a selected document", path: nil, isEnabled: true)
        let navigation = MacExtensionsNavigationState()
        let catalog = MacExtensionsCatalogLoader(local: { _ in [item] }, host: { [] }, removedSkills: { false })
        let host = NSHostingView(rootView: MacExtensionsCenterView(remoteHost: PersistenceTestRemoteHost(),
            kinds: [.skill], navigation: navigation, catalog: catalog)
            .environment(\.accessibilityEnabled, true).frame(width: 520, height: 720))
        let window = NSWindow(contentRect: NSRect(x: 0, y: 0, width: 520, height: 720),
            styleMask: [.borderless], backing: .buffered, defer: false)
        window.isReleasedWhenClosed = false
        window.contentView = host
        window.orderFrontRegardless()
        defer { window.orderOut(nil); window.contentView = nil; window.close() }
        let identifier = "mac.extensions.details." + item.id
        let deadline = ContinuousClock.now + .seconds(4)
        while !objects(host).contains(where: { ($0.accessibilityIdentifier?() ?? nil) == identifier }),
              ContinuousClock.now < deadline { try await Task.sleep(for: .milliseconds(20)) }
        let control = try XCTUnwrap(objects(host).first { ($0.accessibilityIdentifier?() ?? nil) == identifier })
        let frame = try XCTUnwrap(control.accessibilityFrame?())
        XCTAssertGreaterThan(frame.width, 240, "The source and trailing whitespace must open details, replacing the tiny arrow target")
        XCTAssertGreaterThanOrEqual(frame.height, 36)
        XCTAssertTrue(control.accessibilityPerformPress?() ?? false)
        XCTAssertEqual(navigation.selectedID, item.id)
    }

    private func objects(_ root: AnyObject) -> [AnyObject] {
        var queue = [root], visited = Set<ObjectIdentifier>(), result: [AnyObject] = []
        var index = 0
        while index < queue.count, result.count < 4000 {
            let object = queue[index]
            index += 1
            guard visited.insert(ObjectIdentifier(object)).inserted else { continue }
            result.append(object)
            let children = object.accessibilityChildren?() ?? []
            queue.append(contentsOf: (children + NSAccessibility.unignoredChildren(from: children)).map { $0 as AnyObject })
            if let view = object as? NSView { queue.append(contentsOf: view.subviews) }
        }
        return result
    }

}

@MainActor
private final class RowState: ObservableObject { @Published var expanded = false }
private struct RowProbe: View {
    @ObservedObject var state: RowState
    var body: some View {
        VStack {
            DisclosureGroup("Tools", isExpanded: $state.expanded) { Text("Visible content") }
                .disclosureGroupStyle(MacRowDisclosureStyle())
            Spacer()
        }.padding(12)
    }
}
