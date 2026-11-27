// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Live iOS Simulator XCUITest driving unsupported-DB diagnostics and real LAN model chat + all 4 root tabs with PNG evidence.

import SQLite3
import UIKit
import XCTest

final class IOSRealModelE2ETests: XCTestCase {
    private struct RealLLMConfig {
        let endpoint: String
        let model: String
        let apiKey: String
    }

    private func loadRealLLMConfig() throws -> RealLLMConfig {
        let env = ProcessInfo.processInfo.environment
        let fixtureURL = URL(fileURLWithPath: "/tmp/nola-real-llm-e2e.json")
        let fixture = (try? Data(contentsOf: fixtureURL))
            .flatMap { try? JSONSerialization.jsonObject(with: $0) as? [String: String] }

        let envKey = env["NOLA_REAL_LLM_API_KEY"] ?? env["NOLA_PRIMARY_API_KEY"]
        let apiKey = (envKey?.isEmpty == false ? envKey : nil) ?? fixture?["apiKey"] ?? ""
        guard !apiKey.isEmpty else {
            throw XCTSkip("Requires NOLA_REAL_LLM_API_KEY, NOLA_PRIMARY_API_KEY, or /tmp/nola-real-llm-e2e.json")
        }
        let endpoint = (env["NOLA_REAL_LLM_ENDPOINT"].flatMap { $0.isEmpty ? nil : $0 })
            ?? fixture?["endpoint"]
            ?? "http://192.168.31.18:8081/v1"
        let model = (env["NOLA_REAL_LLM_MODEL"].flatMap { $0.isEmpty ? nil : $0 })
            ?? fixture?["model"]
            ?? "qwen3.8-flash-next-iq3_s"
        return RealLLMConfig(endpoint: endpoint, model: model, apiKey: apiKey)
    }

    private func evidenceDirectory() -> URL {
        let base = ProcessInfo.processInfo.environment["NOLA_E2E_EVIDENCE_DIR"]
            .flatMap { $0.isEmpty ? nil : $0 }
            ?? "/tmp/nola-real-e2e-evidence"
        let dir = URL(fileURLWithPath: base, isDirectory: true).appendingPathComponent("ios", isDirectory: true)
        try? FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
        return dir
    }

    private func saveEvidenceScreenshot(_ filename: String) {
        let screenshot = XCUIScreen.main.screenshot()
        let attachment = XCTAttachment(screenshot: screenshot)
        attachment.name = filename
        attachment.lifetime = .keepAlways
        add(attachment)
        let target = evidenceDirectory().appendingPathComponent(filename)
        try? screenshot.pngRepresentation.write(to: target, options: .atomic)
    }

    // MARK: - Test 1: Unsupported / legacy store shows diagnostics without destroying data

    func testUnsupportedStoreShowsDiagnostics() throws {
        #if targetEnvironment(simulator)
        continueAfterFailure = false
        let root = URL(fileURLWithPath: "/tmp/nola-ios-startup-ui-" + UUID().uuidString, isDirectory: true)
        try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
        let database = root.appendingPathComponent("fixture.db")

        var connection: OpaquePointer?
        XCTAssertEqual(sqlite3_open(database.path, &connection), SQLITE_OK)
        XCTAssertEqual(
            sqlite3_exec(connection, "CREATE TABLE preserved (value TEXT); INSERT INTO preserved VALUES ('original'); PRAGMA user_version=1;", nil, nil, nil),
            SQLITE_OK
        )
        sqlite3_close(connection)

        let app = XCUIApplication()
        defer {
            app.terminate()
            try? FileManager.default.removeItem(at: root)
        }
        app.launchArguments = ["-ui-testing", "-AppleLanguages", "(en)", "-nola.appearance", "dark"]
        app.launchEnvironment["NOLA_UI_TEST_RUN_ID"] = root.lastPathComponent
        app.launchEnvironment["NOLA_REAL_IOS_UI_DATABASE_PATH"] = database.path
        app.launch()

        let retry = app.buttons["startup.retry"]
        XCTAssertTrue(retry.waitForExistence(timeout: 12), "Startup recovery screen must appear for unsupported DB")
        let diagnostic = app.staticTexts["startup.diagnostic"]
        if !diagnostic.exists, app.buttons["Technical details"].exists {
            app.buttons["Technical details"].tap()
        }
        XCTAssertTrue(diagnostic.waitForExistence(timeout: 5), "Technical details must reveal startup diagnostic")
        saveEvidenceScreenshot("ios-00-unsupported-db.png")

        retry.tap()
        XCTAssertTrue(retry.waitForExistence(timeout: 5))

        // Confirm original SQLite data was preserved untouched.
        var roConn: OpaquePointer?
        XCTAssertEqual(sqlite3_open_v2(database.path, &roConn, SQLITE_OPEN_READONLY, nil), SQLITE_OK)
        defer { sqlite3_close(roConn) }
        var statement: OpaquePointer?
        XCTAssertEqual(sqlite3_prepare_v2(roConn, "SELECT value FROM preserved", -1, &statement, nil), SQLITE_OK)
        defer { sqlite3_finalize(statement) }
        XCTAssertEqual(sqlite3_step(statement), SQLITE_ROW)
        XCTAssertEqual(sqlite3_column_text(statement, 0).map { String(cString: $0) }, "original")
        #else
        throw XCTSkip("Requires a disposable simulator database")
        #endif
    }

    // MARK: - Test 2: Real LAN model setup, real chat reply, and all 4 tabs + subpages

    func testRealModelSetupChatAndAllTabs() throws {
        let config = try loadRealLLMConfig()
        continueAfterFailure = false

        let app = XCUIApplication()
        defer { app.terminate() }
        app.launchArguments = ["-ui-testing", "-AppleLanguages", "(en)", "-AppleLocale", "en_US"]
        app.launchEnvironment["NOLA_UI_TEST_RUN_ID"] = "ios-real-e2e-\(UUID().uuidString)"
        app.launch()

        // 1. Open Settings -> Models and configure the real LAN model
        tapRootTab("settings", in: app)
        let modelsButton = app.buttons["settings.models"]
        XCTAssertTrue(modelsButton.waitForExistence(timeout: 10))
        for _ in 0..<4 where !modelsButton.isHittable { app.swipeUp() }
        modelsButton.tap()

        let endpointField = app.textFields["model.setup.endpoint"]
        XCTAssertTrue(endpointField.waitForExistence(timeout: 8))
        endpointField.tap()
        endpointField.typeText(config.endpoint)

        let keyField = app.secureTextFields["model.setup.key"]
        if keyField.waitForExistence(timeout: 3) {
            pasteSecret(config.apiKey, into: keyField, in: app)
        }

        let modelField = app.textFields["model.setup.modelID"]
        XCTAssertTrue(modelField.waitForExistence(timeout: 5))
        modelField.tap()
        modelField.typeText(config.model)

        let saveButton = app.buttons["model.setup.save"]
        for _ in 0..<5 where !saveButton.isHittable { app.swipeUp() }
        XCTAssertTrue(saveButton.isEnabled, "Model setup Save button must be enabled")
        saveButton.tap()
        XCTAssertTrue(
            app.staticTexts["Saved and set as default · not tested"].waitForExistence(timeout: 12),
            "Configured real model must confirm save"
        )
        saveEvidenceScreenshot("ios-01-models.png")

        if app.navigationBars.buttons.element(boundBy: 0).exists {
            app.navigationBars.buttons.element(boundBy: 0).tap()
        }

        // 2. Navigate to Chat, create new conversation, send real prompt, wait up to 90s for real reply
        tapRootTab("chat", in: app)
        let newChat = app.buttons["chat.conversations.new"]
        XCTAssertTrue(newChat.waitForExistence(timeout: 10))
        newChat.tap()

        let picker = app.buttons["chat.topbar.modelPicker"]
        if picker.waitForExistence(timeout: 5) {
            picker.tap()
            let choice = app.buttons.matching(NSPredicate(format: "label CONTAINS %@", config.model)).firstMatch
            if choice.waitForExistence(timeout: 5) {
                choice.tap()
            } else if app.buttons["Close"].exists {
                app.buttons["Close"].tap()
            }
        }

        let input = app.textViews["chat.composer.input"]
        XCTAssertTrue(input.waitForExistence(timeout: 10))
        input.tap()
        input.typeText("请用一句话回答 1+1 等于几，并在结尾输出 OK_IOS_REAL_MODEL。不要调用工具。")

        let sendButton = app.buttons["chat.composer.send"]
        XCTAssertTrue(sendButton.waitForExistence(timeout: 5))
        sendButton.tap()

        let replyMarker = app.otherElements.matching(identifier: "chat.assistant.bubble")
            .descendants(matching: .staticText)
            .matching(NSPredicate(format: "label CONTAINS %@", "OK_IOS_REAL_MODEL"))
            .firstMatch
        XCTAssertTrue(replyMarker.waitForExistence(timeout: 90), "Expected real LAN model reply containing OK_IOS_REAL_MODEL")

        let terminal = XCTNSPredicateExpectation(
            predicate: NSPredicate(format: "exists == true AND label != %@", "Stop generation"),
            object: sendButton
        )
        XCTAssertEqual(XCTWaiter.wait(for: [terminal], timeout: 30), .completed)
        saveEvidenceScreenshot("ios-02-chat-real-reply.png")

        // Return to conversation list before traversing tabs
        let back = app.buttons["chat.navigation.back"]
        if back.exists {
            back.tap()
        } else {
            app.navigationBars.buttons.element(boundBy: 0).tap()
        }
        XCTAssertTrue(newChat.waitForExistence(timeout: 8))

        // 3. Activity tab
        tapRootTab("activity", in: app)
        XCTAssertTrue(app.buttons["activity.plans"].waitForExistence(timeout: 8))
        saveEvidenceScreenshot("ios-03-activity.png")

        // 4. Library / Information tab + Memory subpage check
        tapRootTab("information", in: app)
        let knowledge = app.buttons["information.knowledge"]
        XCTAssertTrue(knowledge.waitForExistence(timeout: 8))
        saveEvidenceScreenshot("ios-04-library.png")
        let memoryEntry = app.buttons["information.memory"]
        if memoryEntry.exists && memoryEntry.isHittable {
            memoryEntry.tap()
            RunLoop.current.run(until: Date().addingTimeInterval(0.6))
            if app.navigationBars.buttons.element(boundBy: 0).exists {
                app.navigationBars.buttons.element(boundBy: 0).tap()
            }
        }

        // 5. Settings tab + Capabilities (Skills) & Experts subpages
        tapRootTab("settings", in: app)
        XCTAssertTrue(app.buttons["settings.category.general"].waitForExistence(timeout: 8))
        saveEvidenceScreenshot("ios-05-settings.png")

        for subpageID in ["settings.category.agent", "settings.category.agents"] {
            let row = app.buttons[subpageID]
            if row.exists && row.isHittable {
                row.tap()
                RunLoop.current.run(until: Date().addingTimeInterval(0.6))
                if app.navigationBars.buttons.element(boundBy: 0).exists {
                    app.navigationBars.buttons.element(boundBy: 0).tap()
                }
            }
        }
    }

    // MARK: - Navigation & secret-safe paste helpers

    private func rootTabButton(_ tab: String, in app: XCUIApplication) -> XCUIElement {
        let compact = app.buttons["main.tab.\(tab)"].firstMatch
        if compact.exists { return compact }
        let byID = app.buttons.matching(identifier: "tab.\(tab)").firstMatch
        if byID.exists { return byID }
        let titles: [String: [String]] = [
            "chat": ["Chat", "对话"],
            "activity": ["Activity", "动态"],
            "information": ["Library", "Information", "资料库", "信息"],
            "settings": ["Settings", "设置"],
        ]
        for title in titles[tab] ?? [] {
            let tabBarButton = app.tabBars.buttons[title].firstMatch
            if tabBarButton.exists { return tabBarButton }
        }
        return compact
    }

    private func tapRootTab(_ tab: String, in app: XCUIApplication, timeout: TimeInterval = 10) {
        let deadline = Date().addingTimeInterval(timeout)
        repeat {
            let button = rootTabButton(tab, in: app)
            if button.exists && button.isHittable {
                button.tap()
                return
            }
            RunLoop.current.run(until: Date().addingTimeInterval(0.15))
        } while Date() < deadline
        let fallback = rootTabButton(tab, in: app)
        XCTAssertTrue(fallback.exists, "Missing root tab \(tab)")
        fallback.tap()
    }

    /// Pastes credentials via UIPasteboard so XCTest never logs the secret as a typeText argument.
    private func pasteSecret(_ secret: String, into field: XCUIElement, in app: XCUIApplication) {
        UIPasteboard.general.string = secret
        defer { UIPasteboard.general.items = [] }
        field.tap()
        field.press(forDuration: 1.0)
        let pasteItem = app.menuItems["Paste"].firstMatch
        let pasteButton = app.buttons["Paste"].firstMatch
        if pasteItem.waitForExistence(timeout: 3) {
            pasteItem.tap()
        } else if pasteButton.waitForExistence(timeout: 3) {
            pasteButton.tap()
        }
        let allowPaste = app.buttons["Allow Paste"]
        if allowPaste.waitForExistence(timeout: 1) {
            allowPaste.tap()
        }
    }
}
