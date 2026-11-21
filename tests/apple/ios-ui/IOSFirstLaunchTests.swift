// SPDX-License-Identifier: Apache-2.0
import SQLite3
import XCTest

final class IOSFirstLaunchTests: XCTestCase {
    func testUnsupportedStoreShowsDiagnosticsAndRetryPreservesExistingData() throws {
        #if targetEnvironment(simulator)
        continueAfterFailure = false
        let root = URL(fileURLWithPath: "/tmp/nola-startup-ui-" + UUID().uuidString, isDirectory: true)
        try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
        let database = root.appendingPathComponent("fixture.db")
        var connection: OpaquePointer?
        XCTAssertEqual(sqlite3_open(database.path, &connection), SQLITE_OK)
        XCTAssertEqual(sqlite3_exec(connection, "CREATE TABLE preserved (value TEXT); INSERT INTO preserved VALUES ('original'); PRAGMA user_version=200;", nil, nil, nil), SQLITE_OK)
        sqlite3_close(connection)
        let app = XCUIApplication()
        defer { app.terminate(); try? FileManager.default.removeItem(at: root) }
        app.launchArguments = ["-ui-testing", "-AppleLanguages", "(en)", "-nola.appearance", "dark"]
        app.launchEnvironment["NOLA_UI_TEST_RUN_ID"] = root.lastPathComponent
        app.launchEnvironment["NOLA_REAL_IOS_UI_DATABASE_PATH"] = database.path
        app.launch()
        let retry = app.buttons["startup.retry"]
        XCTAssertTrue(retry.waitForExistence(timeout: 10))
        XCTAssertGreaterThanOrEqual(retry.frame.height, 44)
        let diagnostic = app.staticTexts["startup.diagnostic"]
        XCTAssertFalse(diagnostic.exists)
        app.buttons["Technical details"].tap()
        XCTAssertTrue(diagnostic.label.contains("Unsupported database format"))
        let shot = XCTAttachment(screenshot: app.screenshot())
        shot.name = "unsupported-format-recovery"; shot.lifetime = .keepAlways; add(shot)
        retry.tap()
        XCTAssertTrue(diagnostic.waitForExistence(timeout: 5))
        XCTAssertEqual(sqlite3_open_v2(database.path, &connection, SQLITE_OPEN_READONLY, nil), SQLITE_OK)
        defer { sqlite3_close(connection) }
        var statement: OpaquePointer?
        XCTAssertEqual(sqlite3_prepare_v2(connection, "SELECT value FROM preserved", -1, &statement, nil), SQLITE_OK)
        defer { sqlite3_finalize(statement) }
        XCTAssertEqual(sqlite3_step(statement), SQLITE_ROW)
        XCTAssertEqual(sqlite3_column_text(statement, 0).map { String(cString: $0) }, "original")
        #else
        throw XCTSkip("Requires a disposable simulator database")
        #endif
    }

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
        let start = ContinuousClock.now
        app.buttons["onboarding.chooseModel"].tap()
        XCTAssertTrue(app.textFields["model.setup.endpoint"].waitForExistence(timeout: 5))
        XCTAssertLessThan(start.duration(to: .now), .seconds(45),
                          "Opening API setup must not stall on the offscreen Host pasteboard control")
        XCTAssertFalse(app.buttons["model.web_host.paste_setup"].exists)
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
