// SPDX-License-Identifier: Apache-2.0
import AppKit
import SwiftUI
import XCTest
@testable import Nola

@MainActor
final class MacPermissionGuidanceTests: XCTestCase {
    func testPermissionFailureSettlesBusyStateAndExplicitRetryClearsError() async throws {
        let session = try MacHostedAccessibilitySession()
        defer { session.restore() }
        let suite = "permission-guidance-\(UUID())"
        let preferences = try XCTUnwrap(UserDefaults(suiteName: suite))
        defer { preferences.removePersistentDomain(forName: suite) }
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(suite)
        defer { try? FileManager.default.removeItem(at: root) }
        let registry = MacDataSourceRegistry(storeURL: root.appendingPathComponent("registry.json"))
        var calls = 0
        var reply: CheckedContinuation<Void, Error>?
        let service = MacPlatformDataService(registry: registry, startsPeriodicRefresh: false,
            preferences: preferences, permissionRequest: { _ in
                calls += 1
                if calls == 1 { try await withCheckedThrowingContinuation { reply = $0 } }
            })
        let first = Task { await service.requestAccess(for: .contacts) }
        while reply == nil { await Task.yield() }
        XCTAssertTrue(service.requestingAccess.contains(.contacts))
        await service.requestAccess(for: .contacts)
        XCTAssertEqual(calls, 1, "Repeated clicks cannot launch a second system request")
        reply?.resume(throwing: NSError(domain: "PermissionFixture", code: 1,
            userInfo: [NSLocalizedDescriptionKey: "Fixture system request unavailable"]))
        await first.value
        XCTAssertTrue(service.requestingAccess.isEmpty)
        XCTAssertEqual(service.permissionRequestErrors[.contacts], "Fixture system request unavailable")
        XCTAssertNil(service.permissionRequestErrors[.calendar])
        let web = MacWebSourceStore(storeURL: root.appendingPathComponent("web.json"), registry: registry)
        let host = NSHostingView(rootView: MacDataCenterView(service: service, registry: registry,
            webSources: web, initialSection: "sources").environment(\.accessibilityEnabled, true)
            .frame(width: 680, height: 900))
        let window = NSWindow(contentRect: NSRect(x: 0, y: 0, width: 680, height: 900),
            styleMask: [.borderless], backing: .buffered, defer: false)
        window.isReleasedWhenClosed = false; window.contentView = host; window.orderFrontRegardless()
        defer { window.close() }
        try await Task.sleep(for: .milliseconds(200)); host.layoutSubtreeIfNeeded()
        XCTAssertNotNil(accessibilityObjects(host).first {
            ($0.accessibilityIdentifier?() ?? nil) == "data-source-permission-error-contacts"
        }, "A failed system request must have visible, accessible recovery feedback")
        await service.requestAccess(for: .contacts)
        XCTAssertEqual(calls, 2)
        XCTAssertNil(service.permissionRequestErrors[.contacts])
        XCTAssertTrue(service.requestingAccess.isEmpty)
    }

    func testDataPermissionSettingsOpenTheRelevantSystemPane() {
        XCTAssertTrue(MacDataSourceSnapshot.Kind.contacts.permissionSettingsURL.hasSuffix("Privacy_Contacts"))
        XCTAssertTrue(MacDataSourceSnapshot.Kind.calendar.permissionSettingsURL.hasSuffix("Privacy_Calendars"))
        XCTAssertTrue(MacDataSourceSnapshot.Kind.reminders.permissionSettingsURL.hasSuffix("Privacy_Reminders"))
        XCTAssertTrue(MacDataSourceSnapshot.Kind.notifications.permissionSettingsURL.contains("Notifications-Settings"))
    }

    func testFirstRunGuideNavigatesWithoutPretendSelectionsOrPreferenceWrites() async throws {
        let session = try MacHostedAccessibilitySession()
        defer { session.restore() }
        let suite = "onboarding-guidance-\(UUID())"
        let preferences = try XCTUnwrap(UserDefaults(suiteName: suite))
        defer { preferences.removePersistentDomain(forName: suite) }
        preferences.set("existing-host", forKey: "nola.mac.runtime.preference")
        preferences.set("existing-sources", forKey: "nola.mac.planned.sources")
        for width: CGFloat in [520, 1040] {
            for dark in [false, true] {
                var completions = 0
                let host = NSHostingView(rootView: MacOnboardingView { completions += 1 }
                    .defaultAppStorage(preferences)
                    .environment(\.colorScheme, dark ? .dark : .light)
                    .environment(\.accessibilityEnabled, true)
                    .transaction { $0.animation = nil; $0.disablesAnimations = true }
                    .frame(width: width, height: 820))
                let window = NSWindow(contentRect: NSRect(x: 0, y: 0, width: width, height: 820),
                    styleMask: [.borderless], backing: .buffered, defer: false)
                window.isReleasedWhenClosed = false; window.contentView = host; window.orderFrontRegardless()
                defer { window.close() }
                for step in 0..<4 {
                    try await Task.sleep(for: .milliseconds(80))
                    host.layoutSubtreeIfNeeded()
                    let objects = accessibilityObjects(host)
                    XCTAssertFalse(objects.contains { object in
                        let role = object.accessibilityRole?() ?? nil
                        return role == .radioButton || role == .checkBox
                    }, "The guide must not offer choices that do not configure execution")
                    if let bitmap = host.bitmapImageRepForCachingDisplay(in: host.bounds) {
                        host.cacheDisplay(in: host.bounds, to: bitmap)
                        let attachment = XCTAttachment(data: try XCTUnwrap(bitmap.representation(using: .png, properties: [:])), uniformTypeIdentifier: "public.png")
                        attachment.name = "onboarding-\(step)-\(Int(width))-\(dark ? "dark" : "light")"
                        attachment.lifetime = .keepAlways; add(attachment)
                    }
                    let candidate: AnyObject? = objects.first { ($0.accessibilityIdentifier?() ?? nil) == "onboarding.next" }
                    let next: AnyObject = try XCTUnwrap(candidate)
                    XCTAssertTrue(next.accessibilityPerformPress?() ?? false)
                }
                try await Task.sleep(for: .milliseconds(60))
                XCTAssertEqual(completions, 1)
                XCTAssertEqual(preferences.string(forKey: "nola.mac.runtime.preference"), "existing-host")
                XCTAssertEqual(preferences.string(forKey: "nola.mac.planned.sources"), "existing-sources")
                window.close()
            }
        }
    }

    private func accessibilityObjects(_ root: AnyObject) -> [AnyObject] {
        var queue = [root], visited = Set<ObjectIdentifier>(), result: [AnyObject] = []
        var index = 0
        while index < queue.count, result.count < 4000 {
            let object = queue[index]; index += 1
            guard visited.insert(ObjectIdentifier(object)).inserted else { continue }
            result.append(object)
            let children = object.accessibilityChildren?() ?? []
            queue.append(contentsOf: (children + NSAccessibility.unignoredChildren(from: children)).map { $0 as AnyObject })
            if let view = object as? NSView { queue.append(contentsOf: view.subviews) }
        }
        return result
    }
}
