import XCTest

/// Sends through the configured real provider and verifies rendered, durable replies.
final class IOSRealProviderChatTests: XCTestCase {
    func testRealImportedModelConversationStaysVisible() throws {
        guard let endpoint = ProcessInfo.processInfo.environment["NOLA_UI_REPRO_ENDPOINT"],
              let importedModel = ProcessInfo.processInfo.environment["NOLA_UI_REPRO_MODEL"] else {
            throw XCTSkip("Requires an explicitly configured real model endpoint")
        }
        let app = XCUIApplication()
        app.launchArguments = ["-ui-testing", "-AppleLanguages", "(en)", "-AppleLocale", "en_US"]
        app.launchEnvironment["NOLA_UI_TEST_RUN_ID"] = "real-import-\(UUID().uuidString)"
        app.launch()
        let tabs = app.tabBars.firstMatch
        XCTAssertTrue(tabs.waitForExistence(timeout: 10))
        app.openModelCenterFromSettings()
        app.configureServiceModel(endpoint: endpoint, modelID: importedModel)
        tabs.buttons["Chat"].tap()
        if app.buttons["chat.conversations.new"].exists { app.buttons["chat.conversations.new"].tap() }
        app.buttons["chat.topbar.modelPicker"].tap()
        let pickerTree = XCTAttachment(string: app.debugDescription)
        pickerTree.name = "imported-model-picker"; pickerTree.lifetime = .keepAlways; add(pickerTree)
        let choice = app.descendants(matching: .any).matching(NSPredicate(format: "label == %@", importedModel)).firstMatch
        XCTAssertTrue(choice.waitForExistence(timeout: 5), app.debugDescription)
        choice.tap()
        if app.buttons["chat.conversations.new"].waitForExistence(timeout: 3) { app.buttons["chat.conversations.new"].tap() }
        let input = app.textViews["chat.composer.input"]
        XCTAssertTrue(input.waitForExistence(timeout: 10))
        input.tap()
        input.typeText("Explain why the sky is blue in one short sentence. Begin with SKY_REVIEW.")
        app.buttons["chat.composer.send"].tap()
        let reply = app.staticTexts.matching(NSPredicate(format: "label CONTAINS %@ AND NOT label CONTAINS %@", "SKY_REVIEW", "Begin with")).firstMatch
        XCTAssertTrue(reply.waitForExistence(timeout: 180), "Real imported model must produce an answer")
        let shot = XCTAttachment(screenshot: app.screenshot())
        shot.name = "real-import-first-reply"; shot.lifetime = .keepAlways; add(shot)
        let finished = XCTNSPredicateExpectation(predicate: NSPredicate(format: "label != %@", "Stop generation"), object: app.buttons["chat.composer.send"])
        XCTAssertEqual(XCTWaiter.wait(for: [finished], timeout: 180), .completed)
        input.tap()
        input.typeText("Now explain red sunsets in one short sentence. Begin with SUNSET_REVIEW.")
        app.buttons["chat.composer.send"].tap()
        let second = app.staticTexts.matching(NSPredicate(format: "label CONTAINS %@ AND NOT label CONTAINS %@", "SUNSET_REVIEW", "Begin with")).firstMatch
        XCTAssertTrue(second.waitForExistence(timeout: 180), "Follow-up must render a second real answer")
        let secondFinished = XCTNSPredicateExpectation(predicate: NSPredicate(format: "label != %@", "Stop generation"), object: app.buttons["chat.composer.send"])
        XCTAssertEqual(XCTWaiter.wait(for: [secondFinished], timeout: 180), .completed)
        app.navigationBars.buttons.element(boundBy: 0).tap()
        tabs.buttons.element(boundBy: 1).tap()
        tabs.buttons.element(boundBy: 0).tap()
        app.buttons.matching(NSPredicate(format: "identifier BEGINSWITH %@", "chat.conversation.")).firstMatch.tap()
        XCTAssertTrue(second.waitForExistence(timeout: 10), "Committed reply must survive tab navigation")
        let restored = XCTAttachment(screenshot: app.screenshot())
        restored.name = "real-import-followup-restored"; restored.lifetime = .keepAlways; add(restored)
    }
}
