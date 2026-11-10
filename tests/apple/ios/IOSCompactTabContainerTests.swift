// SPDX-License-Identifier: Apache-2.0
import SwiftUI
import UIKit
import XCTest
@testable import Nola

@MainActor
final class IOSCompactTabContainerTests: XCTestCase {
    private enum Tab { case chat, settings }

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
