// SPDX-License-Identifier: Apache-2.0
import XCTest

final class IOSFirstLaunchTests: XCTestCase {
    private func launchWelcome() -> XCUIApplication {
        continueAfterFailure = false
        let app = XCUIApplication()
        app.launchArguments = ["-ui-testing", "-ui-testing-onboarding", "-AppleLanguages", "(en)", "-AppleLocale", "en_US"]
        app.launchEnvironment["NOLA_UI_TEST_RUN_ID"] = "first-launch-\(UUID())"
        app.launch()
        XCTAssertTrue(app.buttons["onboarding.chooseModel"].waitForExistence(timeout: 10))
        return app
    }

    func testClosingSetupReturnsToWelcomeAndSkippingSurvivesRelaunch() {
        let app = launchWelcome()
        defer { app.terminate() }
        app.buttons["onboarding.chooseModel"].tap()
        XCTAssertTrue(app.textFields["model.setup.endpoint"].waitForExistence(timeout: 5))
        app.buttons["model.setup.close"].tap()
        app.buttons["onboarding.skip"].tap()
        XCTAssertTrue(app.buttons["chat.conversations.new"].waitForExistence(timeout: 10))
        app.terminate()
        app.launch()
        XCTAssertTrue(app.buttons["chat.conversations.new"].waitForExistence(timeout: 10))
        XCTAssertFalse(app.buttons["onboarding.skip"].exists)
    }

    func testReopensUnfinishedSetupAndUsesSavedModelForFirstConversation() {
        let app = launchWelcome()
        defer { app.terminate() }
        app.buttons["onboarding.chooseModel"].tap()
        XCTAssertTrue(app.textFields["model.setup.endpoint"].waitForExistence(timeout: 5))
        app.terminate()
        app.launch()
        let endpoint = app.textFields["model.setup.endpoint"]
        XCTAssertTrue(endpoint.waitForExistence(timeout: 10))
        endpoint.tap()
        endpoint.typeText("https://first-chat.example.invalid/v1")
        let modelID = app.textFields["model.setup.modelID"]
        modelID.tap()
        modelID.typeText("first-chat-model\n")
        let save = app.buttons["model.setup.save"]
        for _ in 0..<6 where !save.isHittable { app.swipeUp() }
        XCTAssertTrue(save.isEnabled)
        save.tap()
        XCTAssertTrue(app.buttons["chat.conversations.new"].waitForExistence(timeout: 10))
        app.terminate()
        app.launch()
        let newChat = app.buttons["chat.conversations.new"]
        XCTAssertTrue(newChat.waitForExistence(timeout: 10))
        newChat.tap()
        let selected = app.descendants(matching: .any).matching(NSPredicate(format: "label CONTAINS %@", "first-chat-model")).firstMatch
        XCTAssertTrue(selected.waitForExistence(timeout: 8))
    }
}
