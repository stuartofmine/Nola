// SPDX-License-Identifier: Apache-2.0
import SwiftUI
import UIKit
import XCTest
@testable import Nola

@MainActor
final class IOSCompactTabContainerTests: XCTestCase {
    private enum Tab { case chat, settings }

    func testCachedRootRespectsHardwareSafeAreaAndOnlyRefreshesChangedContentRevision() async throws {
        let scene = try XCTUnwrap(UIApplication.shared.connectedScenes.compactMap { $0 as? UIWindowScene }.first)
        let controller = IOSCompactTabContainer<Tab>.Controller()
        controller.additionalSafeAreaInsets = .init(top: 24, left: 0, bottom: 12, right: 0)
        let window = UIWindow(windowScene: scene)
        window.frame = CGRect(x: 0, y: 0, width: 390, height: 844)
        window.rootViewController = controller; window.makeKeyAndVisible()
        defer { window.isHidden = true; window.rootViewController = nil; window.windowScene = nil }
        controller.select(.chat, content: AnyView(TabLabelProbe(text: "initial")), revision: 0)
        try await Task.sleep(for: .milliseconds(100))
        window.layoutIfNeeded()
        let host = try XCTUnwrap(controller.children.first)
        XCTAssertGreaterThan(controller.view.safeAreaInsets.top, 20)
        XCTAssertGreaterThanOrEqual(host.view.frame.minY, controller.view.safeAreaInsets.top - 0.5)
        XCTAssertLessThanOrEqual(host.view.frame.maxY, controller.view.bounds.maxY - controller.view.safeAreaInsets.bottom + 0.5)
        func label(_ view: UIView) -> UILabel? {
            if let label = view as? UILabel { return label }
            return view.subviews.lazy.compactMap(label).first
        }
        XCTAssertEqual(label(host.view)?.text, "initial")
        controller.select(.chat, content: AnyView(TabLabelProbe(text: "unrelated render")), revision: 0)
        try await Task.sleep(for: .milliseconds(50))
        XCTAssertEqual(label(host.view)?.text, "initial")
        controller.select(.chat, content: AnyView(TabLabelProbe(text: "theme changed")), revision: 1)
        try await Task.sleep(for: .milliseconds(50))
        XCTAssertEqual(label(host.view)?.text, "theme changed")
    }

    func testReturningToActivityKeepsItsSnapshotWithoutReloading() async throws {
        let scene = try XCTUnwrap(UIApplication.shared.connectedScenes.compactMap { $0 as? UIWindowScene }.first)
        let controller = IOSCompactTabContainer<Tab>.Controller()
        let window = UIWindow(windowScene: scene); window.rootViewController = controller; window.makeKeyAndVisible()
        defer { window.isHidden = true; window.rootViewController = nil; window.windowScene = nil }
        var loads = 0
        let activity = AnyView(AppleActivityView(load: { _ in
            loads += 1
            return try JSONDecoder().decode(AppleActivityPage.self, from: Data(#"{"entries":[],"next_cursor":null}"#.utf8))
        }, detail: { _ in nil }, openSource: { _ in }))
        controller.select(.chat, content: activity, revision: 0)
        for _ in 0..<50 where loads == 0 { try await Task.sleep(for: .milliseconds(20)) }
        XCTAssertEqual(loads, 1)
        controller.select(.settings, content: AnyView(Text("Settings")), revision: 0)
        try await Task.sleep(for: .milliseconds(50))
        controller.select(.chat, content: activity, revision: 0)
        try await Task.sleep(for: .milliseconds(300))
        XCTAssertEqual(loads, 1)
    }

    func testOnlySelectedRootIsAttachedAndReturningRetainsItsController() {
        let container = IOSCompactTabContainer<Tab>.Controller()
        container.select(.chat, content: AnyView(Text("Chat")))
        let chat = container.children.first
        XCTAssertEqual(container.children.count, 1)
        container.select(.settings, content: AnyView(Text("Settings")))
        XCTAssertEqual(container.children.count, 1)
        XCTAssertNil(chat?.parent)
        XCTAssertFalse(container.children.first === chat)
        container.select(.chat, content: AnyView(Text("Updated chat")))
        XCTAssertTrue(container.children.first === chat)
        XCTAssertTrue(chat?.parent === container)
    }

    func testUpdatingCurrentRootDoesNotReplaceItsViewOrCreateSystemTabBar() {
        let container = IOSCompactTabContainer<Tab>.Controller()
        container.select(.chat, content: AnyView(Text("Chat")))
        let host = container.children.first
        container.select(.chat, content: AnyView(Text("Updated")))
        XCTAssertTrue(container.children.first === host)
        XCTAssertEqual(container.children.count, 1)
        func containsTabBar(_ view: UIView) -> Bool {
            view is UITabBar || view.subviews.contains(where: containsTabBar)
        }
        XCTAssertFalse(containsTabBar(container.view))
        XCTAssertTrue((host as? UIHostingController<AnyView>)?.safeAreaRegions == .container)
    }
}

private struct TabLabelProbe: UIViewRepresentable {
    let text: String
    func makeUIView(context: Context) -> UILabel { UILabel() }
    func updateUIView(_ label: UILabel, context: Context) { label.text = text }
}
