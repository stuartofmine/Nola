// SPDX-License-Identifier: Apache-2.0
import XCTest
import UIKit
@testable import Nola

@MainActor
final class ChatNavigationGestureTests: XCTestCase {
    func testCompactChatRestoresAndReleasesTheNativeBackGesture() throws {
        let navigation = UINavigationController(rootViewController: UIViewController())
        navigation.loadViewIfNeeded()
        let gesture = try XCTUnwrap(navigation.interactivePopGestureRecognizer)
        let original = gesture.delegate
        let enabled = gesture.isEnabled
        let chat = ChatNavigationGestureBridge.Controller()
        navigation.pushViewController(chat, animated: false)
        chat.viewDidAppear(false)
        XCTAssertTrue(gesture.delegate === chat)
        XCTAssertTrue(gesture.isEnabled)
        XCTAssertTrue(chat.gestureRecognizerShouldBegin(gesture))
        chat.viewWillDisappear(false)
        XCTAssertTrue(gesture.delegate === chat, "An in-progress swipe must keep its recognizer enabled")
        chat.viewDidAppear(false) // cancelled interactive return
        XCTAssertTrue(gesture.delegate === chat)
        chat.viewDidDisappear(false)
        XCTAssertTrue(gesture.delegate === original)
        XCTAssertEqual(gesture.isEnabled, enabled)
        // Returning to this chat installs the same native gesture again.
        chat.viewDidAppear(false)
        XCTAssertTrue(gesture.delegate === chat)
        chat.viewDidDisappear(false)
    }

    func testRootChatDoesNotInterceptSystemNavigation() throws {
        let chat = ChatNavigationGestureBridge.Controller()
        let navigation = UINavigationController(rootViewController: chat)
        navigation.loadViewIfNeeded()
        let gesture = try XCTUnwrap(navigation.interactivePopGestureRecognizer)
        XCTAssertFalse(chat.gestureRecognizerShouldBegin(gesture))
    }
}
