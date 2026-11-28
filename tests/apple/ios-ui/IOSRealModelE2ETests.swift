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

        func assistantText(containing marker: String) -> XCUIElement {
            app.otherElements.matching(identifier: "chat.assistant.bubble")
                .descendants(matching: .staticText)
                .matching(NSPredicate(format: "label CONTAINS %@", marker)).firstMatch
        }
        func waitForIdle(file: StaticString = #filePath, line: UInt = #line) {
            let ready = XCTNSPredicateExpectation(
                predicate: NSPredicate(format: "exists == true AND label != %@", "Stop generation"), object: sendButton
            )
            XCTAssertEqual(XCTWaiter.wait(for: [ready], timeout: 30), .completed, file: file, line: line)
        }
        XCTAssertTrue(assistantText(containing: "OK_IOS_REAL_MODEL").waitForExistence(timeout: 90))
        waitForIdle()
        saveEvidenceScreenshot("ios-02-chat-real-reply.png")
        let editButtons = app.buttons.matching(NSPredicate(format: "identifier BEGINSWITH %@", "chat.message.edit."))
        let originalEditID = editButtons.firstMatch.identifier
        XCTAssertFalse(originalEditID.isEmpty)
        let back = app.buttons["chat.navigation.back"]
        let conversationRows = app.buttons.matching(NSPredicate(format: "identifier BEGINSWITH %@", "chat.conversation."))

        // Stop an actual streaming reply, then submit again on the same conversation.
        input.tap()
        input.typeText("Write a long story with 80 paragraphs. Start with IOS_STOP_TEST.")
        sendButton.tap()
        let stop = app.buttons.matching(identifier: "chat.composer.send")
            .matching(NSPredicate(format: "label == %@", "Stop generation")).firstMatch
        XCTAssertTrue(stop.waitForExistence(timeout: 15))
        stop.tap()
        back.tap()
        let originalRow = conversationRows.firstMatch
        XCTAssertTrue(originalRow.waitForExistence(timeout: 10))
        let conversationID = originalRow.identifier
        tapRootTab("activity", in: app)
        XCTAssertTrue(app.buttons["activity.plans"].waitForExistence(timeout: 8))
        tapRootTab("chat", in: app)
        app.buttons[conversationID].tap()
        waitForIdle()
        saveEvidenceScreenshot("ios-stop-reopen.png")
        input.tap()
        input.typeText("Reply only OK_IOS_AFTER_STOP.")
        sendButton.tap()
        XCTAssertTrue(assistantText(containing: "OK_IOS_AFTER_STOP").waitForExistence(timeout: 60))
        waitForIdle()
        saveEvidenceScreenshot("ios-chat-after-stop.png")

        // Editing the first user message must remove both later turns and preserve the conversation.
        let originalEdit = app.buttons[originalEditID]
        for _ in 0..<6 where !originalEdit.isHittable { app.swipeDown() }
        XCTAssertTrue(originalEdit.isHittable)
        originalEdit.tap()
        let editor = app.textViews["chat.message.editor"]
        XCTAssertTrue(editor.waitForExistence(timeout: 8))
        let originalText = editor.coordinate(withNormalizedOffset: CGVector(dx: 0.18, dy: 0.12))
        originalText.tap()
        originalText.press(forDuration: 1.0)
        saveEvidenceScreenshot("ios-edit-touch-menu.png")
        func selectionAction(_ title: String) -> XCUIElement {
            let menu = app.menuItems[title].firstMatch
            return menu.exists ? menu : app.buttons[title].firstMatch
        }
        let selectAll = selectionAction("Select All")
        if !selectAll.exists, selectionAction("Select").exists { selectionAction("Select").tap() }
        if !selectAll.exists, selectionAction("Show more items").exists { selectionAction("Show more items").tap() }
        XCTAssertTrue(selectionAction("Select All").waitForExistence(timeout: 5))
        saveEvidenceScreenshot("ios-edit-original-open.png")
        selectionAction("Select All").tap()
        let keyboard = app.keyboards.firstMatch
        XCTAssertTrue(keyboard.waitForExistence(timeout: 5))
        let firstKey = keyboard.keys.matching(NSPredicate(format: "label ==[c] %@", "r")).firstMatch
        XCTAssertTrue(firstKey.waitForExistence(timeout: 5))
        firstKey.tap()
        XCTAssertEqual((editor.value as? String)?.lowercased(), "r")
        editor.typeText("eply only OK_IOS_EDITED_ROOT.")
        XCTAssertEqual((editor.value as? String)?.lowercased(), "reply only ok_ios_edited_root.")
        let saveEdit = app.buttons["chat.message.editor.save"]
        XCTAssertTrue(saveEdit.isEnabled)
        saveEdit.tap()
        XCTAssertTrue(assistantText(containing: "OK_IOS_EDITED_ROOT").waitForExistence(timeout: 90))
        waitForIdle()
        saveEvidenceScreenshot("ios-edited-root.png")

        app.terminate()
        app.launch()
        if back.exists { back.tap() }
        tapRootTab("chat", in: app)
        let row = app.buttons[conversationID]
        XCTAssertTrue(row.waitForExistence(timeout: 10))
        row.tap()
        XCTAssertTrue(assistantText(containing: "OK_IOS_EDITED_ROOT").waitForExistence(timeout: 15))
        XCTAssertFalse(assistantText(containing: "OK_IOS_REAL_MODEL").exists)
        XCTAssertFalse(assistantText(containing: "OK_IOS_AFTER_STOP").exists)
        XCTAssertEqual(editButtons.count, 1)
        saveEvidenceScreenshot("ios-edited-root-relaunched.png")

        // Return to conversation list before traversing tabs
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
