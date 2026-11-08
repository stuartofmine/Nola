// SPDX-License-Identifier: Apache-2.0
import AppKit
import SwiftUI
import XCTest
@testable import Nola

@MainActor
final class MacSettingsDefaultModelTests: XCTestCase {
    func testLocalDefaultSummaryDoesNotAttemptToLoadAnAPIConfiguration() throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: root) }
        let application = try MacApplicationPort(databaseURL: root.appendingPathComponent("local-default.db"))
        let profile = try XCTUnwrap(MacLocalModelProfile.available.first)
        let store = MacDefaultChatModelStore(application: application)
        try store.set(.profile(profile.id), expectedRevision: nil,
                      candidates: [.init(profileID: profile.id)])
        let preference = MacDefaultChatModelPresentation(store: store)
        XCTAssertNil(preference.error)
        XCTAssertEqual(preference.snapshot?.selection, .profile(profile.id))
        switch preference.resolution {
        case let .resolved(id, origin):
            XCTAssertEqual(id, profile.id)
            XCTAssertEqual(origin, .default)
        case let .unavailable(id, origin, reason):
            XCTAssertEqual(id, profile.id)
            XCTAssertEqual(origin, .default)
            XCTAssertEqual(reason, .notInstalled)
        default:
            XCTFail("A local model must resolve or show its installation state, not an API configuration error.")
        }
    }

    func testHostedSummaryTracksCanonicalDefaultAndOpensItsExactProfile() async throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: root) }
        let application = try MacApplicationPort(databaseURL: root.appendingPathComponent("settings.db"))
        let configuration = MacAppleChatConfigurationStore(applicationPort: application)
        try configuration.replaceEndpoints([MacRemoteEndpointConfiguration(
            id: "endpoint", name: "Fixture", baseURL: "https://fixture.invalid/v1",
            models: [MacRemoteModelConfiguration(id: "first", modelName: "first-model", displayName: "First"),
                     MacRemoteModelConfiguration(id: "second", modelName: "second-model", displayName: "Second")])])
        let store = MacDefaultChatModelStore(application: application)
        let preference = MacDefaultChatModelPresentation(store: store)
        var opened: [String?] = []
        let hosting = NSHostingView(rootView: VStack {
            MacSettingsDefaultChatModelSummary(preference: preference) { opened.append($0) }
        }.environment(\.accessibilityEnabled, true))
        hosting.frame = NSRect(x: 0, y: 0, width: 600, height: 300)
        let window = NSWindow(contentRect: hosting.frame, styleMask: [.titled], backing: .buffered, defer: false)
        window.isReleasedWhenClosed = false
        window.contentView = hosting
        window.makeKeyAndOrderFront(nil)
        defer { window.orderOut(nil); window.contentView = nil; window.close() }
        try await settle(hosting)

        let saved = try store.set(.profile("second"), expectedRevision: nil,
                                  candidates: preference.catalog.candidates)
        try await settle(hosting)
        XCTAssertEqual(preference.snapshot?.selection, .profile("second"))
        XCTAssertEqual(preference.resolution, .resolved(profileID: "second", origin: .default))
        XCTAssertTrue(preference.title.contains("Second"))
        let button = try XCTUnwrap(elements(hosting).first {
            $0.accessibilityIdentifier?() == "settings.default-chat-model.manage"
        })
        XCTAssertEqual(button.accessibilityPerformPress?(), true)
        XCTAssertEqual(opened.count, 1)
        XCTAssertEqual(opened.first!, "second")

        try configuration.replaceEndpoints([])
        NotificationCenter.default.post(name: .nolaMacConfigurationDidChange, object: nil)
        try await settle(hosting)
        XCTAssertEqual(preference.snapshot?.selection, .profile("second"))
        XCTAssertEqual(preference.resolution,
                       .unavailable(profileID: "second", origin: .default, reason: .missing))
        XCTAssertEqual(try store.snapshot().revision, saved.revision,
                       "Rendering a missing default must not rewrite the user's choice.")

        _ = try store.set(.automatic, expectedRevision: saved.revision, candidates: [])
        try await settle(hosting)
        XCTAssertEqual(preference.snapshot?.selection, .automatic)
    }

    private func settle(_ view: NSView) async throws {
        for _ in 0..<8 {
            try await Task.sleep(for: .milliseconds(20))
            view.layoutSubtreeIfNeeded()
        }
    }

    private func elements(_ view: NSView) -> [AnyObject] {
        var queue: [AnyObject] = [view]
        var result: [AnyObject] = []
        var visited: Set<ObjectIdentifier> = []
        var index = 0
        while index < queue.count, result.count < 1000 {
            let element = queue[index]
            index += 1
            guard visited.insert(ObjectIdentifier(element)).inserted else { continue }
            result.append(element)
            let children: [Any] = element.accessibilityChildren?() ?? []
            queue.append(contentsOf: children.map { $0 as AnyObject })
            if let nativeView = element as? NSView { queue.append(contentsOf: nativeView.subviews) }
        }
        return result
    }
}
