// SPDX-License-Identifier: Apache-2.0
import AppKit
import SwiftUI
import XCTest
@testable import Nola

@MainActor
final class MacSecondaryHeaderNavigationTests: XCTestCase {
    func testDataAndAutomationHeadersRestoreNavigationAtCompactAndRegularWidths() async throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent("page-navigation-\(UUID().uuidString)")
        defer { try? FileManager.default.removeItem(at: root) }
        let preferencesName = "page-navigation-\(UUID().uuidString)"
        let preferences = try XCTUnwrap(UserDefaults(suiteName: preferencesName))
        defer { preferences.removePersistentDomain(forName: preferencesName) }
        let registry = MacDataSourceRegistry(storeURL: root.appendingPathComponent("registry.json"))
        let web = MacWebSourceStore(storeURL: root.appendingPathComponent("web.json"), registry: registry)
        let service = MacPlatformDataService(registry: registry, startsPeriodicRefresh: false, preferences: preferences)
        let activity = MacActivityStore(baseDirectory: root.appendingPathComponent("Activity"))
        let environment = MacAppEnvironment(applicationSupportRoot: root.appendingPathComponent("Support"), cachesRoot: root.appendingPathComponent("Caches"))
        let automation = try MacAutomationStore(baseDirectory: root.appendingPathComponent("Automations"),
            launchAgentDirectory: root.appendingPathComponent("LaunchAgents"), synchronizeSystemJobs: false,
            appEnvironment: environment, activityStore: activity,
            completionNotifier: { _, _ in }, dataSourceRegistry: registry, webSourceStore: web)
        let widths: [CGFloat] = [680, 1040]
        for width in widths {
            var dataNavigations = 0
            try await pressNavigation(in: MacDataCenterView(navigationAction: { dataNavigations += 1 },
                service: service, registry: registry, webSources: web), width: width)
            XCTAssertEqual(dataNavigations, 1)
            var automationNavigations = 0
            try await pressNavigation(in: MacAutomationCenterView(navigationAction: { automationNavigations += 1 },
                store: automation, activityStore: activity), width: width)
            XCTAssertEqual(automationNavigations, 1)
        }
    }

    private func pressNavigation<Content: View>(in content: Content, width: CGFloat) async throws {
        let host = NSHostingView(rootView: content.frame(width: width, height: 640)
            .environment(\.accessibilityEnabled, true))
        let window = NSWindow(contentRect: NSRect(x: 0, y: 0, width: width, height: 640),
            styleMask: [.borderless], backing: .buffered, defer: false)
        window.isReleasedWhenClosed = false
        window.contentView = host
        window.orderFrontRegardless()
        defer { window.close() }
        let label = String(localized: "Toggle Sidebar")
        let deadline = Date().addingTimeInterval(3)
        repeat {
            host.layoutSubtreeIfNeeded()
            if let button = accessibilityObjects(host).first(where: { ($0.accessibilityLabel?() ?? nil) == label }) {
                XCTAssertTrue(button.accessibilityPerformPress?() ?? false)
                await Task.yield()
                return
            }
            try await Task.sleep(for: .milliseconds(20))
        } while Date() < deadline
        XCTFail("Mounted page has no accessible navigation button at width \(width)")
    }

    private func accessibilityObjects(_ root: AnyObject) -> [AnyObject] {
        var queue: [AnyObject] = [root]
        var visited = Set<ObjectIdentifier>()
        var result: [AnyObject] = []
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
