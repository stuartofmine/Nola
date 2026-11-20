// SPDX-License-Identifier: Apache-2.0
import CoreGraphics
import SQLite3
import XCTest

/// Real app navigation with a fresh namespace; no provider, permission grants or inference.
final class IOSWorkspaceNavigationSmokeTests: XCTestCase {
    func testModelDraftSurvivesSourceChangesAndRequiresExplicitDiscard() throws {
        continueAfterFailure = false
        let app = XCUIApplication()
        app.launchArguments = ["-ui-testing", "-AppleLanguages", "(en)", "-AppleLocale", "en_US"]
        app.launchEnvironment["NOLA_UI_TEST_RUN_ID"] = "model-draft-\(UUID().uuidString)"
        app.launch()
        defer { app.terminate() }
        func step(_ name: String, _ perform: () -> Void) {
            XCTContext.runActivity(named: name) { activity in
                func capture(_ phase: String) {
                    // Accessibility can see the destination during the slide animation.
                    var frame = app.screenshot()
                    let deadline = Date().addingTimeInterval(5)
                    var stable = false
                    repeat {
                        RunLoop.current.run(until: Date().addingTimeInterval(0.3))
                        let next = app.screenshot()
                        stable = next.pngRepresentation == frame.pngRepresentation
                        frame = next
                    } while !stable && Date() < deadline
                    let image = XCTAttachment(screenshot: frame)
                    image.name = "\(name)-\(phase)"
                    image.lifetime = .keepAlways
                    activity.add(image)
                    XCTAssertTrue(stable, "The captured screen was still changing: \(name)-\(phase)")
                }
                capture("before")
                perform()
                capture("after")
            }
        }
        let endpoint = app.textFields["model.setup.endpoint"]
        step("01-open-model-settings") {
            app.waitForRootTab("settings")
            app.openModelCenterFromSettings()
            XCTAssertTrue(endpoint.waitForExistence(timeout: 5))
        }
        step("02-enter-endpoint") {
            endpoint.tap()
            endpoint.typeText("https://draft.example.invalid/v1\n")
            XCTAssertEqual(endpoint.value as? String, "https://draft.example.invalid/v1")
        }
        step("03-switch-source") {
            app.buttons["model.setup.source.2"].tap()
            XCTAssertFalse(endpoint.exists)
            XCTAssertTrue(app.buttons["model.setup.close"].isHittable)
        }
        step("04-close-with-unsaved-draft") {
            app.buttons["model.setup.close"].tap()
            XCTAssertTrue(app.alerts["Discard unsaved changes?"].waitForExistence(timeout: 5))
        }
        func discardAlertAction(identifier: String, label: String) -> XCUIElement {
            let alert = app.alerts["Discard unsaved changes?"]
            XCTAssertTrue(alert.waitForExistence(timeout: 5))
            XCTAssertEqual(app.alerts.count, 1, "Only one discard decision may be presented")
            let actions = alert.buttons.matching(NSPredicate(
                format: "identifier == %@ AND label == %@", identifier, label
            ))
            XCTAssertGreaterThan(actions.count, 0, "The exact alert action must exist")
            // The SwiftUI/UIKit bridge may expose an outer Button and one
            // direct Button child for a single action. A descendant-containing
            // query includes the outer node itself on this runtime; direct
            // children identify the unique actionable leaf without ambiguity.
            let children = actions.children(matching: .button).matching(NSPredicate(
                format: "identifier == %@ AND label == %@", identifier, label
            ))
            if children.count > 0 {
                XCTAssertEqual(actions.count, 2, "Expected exactly one wrapper and its action child")
                XCTAssertEqual(children.count, 1, "Separate duplicate actions must fail the test")
                return children.element
            }
            XCTAssertEqual(actions.count, 1)
            return actions.element
        }
        step("05-keep-editing") {
            discardAlertAction(identifier: "model.setup.keep_editing", label: "Keep editing").tap()
            XCTAssertTrue(app.buttons["model.setup.source.0"].waitForExistence(timeout: 5))
            XCTAssertFalse(app.alerts.firstMatch.exists)
        }
        step("06-restore-endpoint") {
            app.buttons["model.setup.source.0"].tap()
            XCTAssertEqual(endpoint.value as? String, "https://draft.example.invalid/v1")
        }
        step("07-request-discard") {
            app.buttons["model.setup.close"].tap()
            XCTAssertTrue(app.alerts["Discard unsaved changes?"].waitForExistence(timeout: 5))
        }
        step("08-confirm-discard") {
            discardAlertAction(identifier: "model.setup.discard", label: "Discard changes").tap()
            XCTAssertTrue(app.buttons["settings.models"].waitForExistence(timeout: 5))
        }
        step("09-reopen-model-settings") {
            app.buttons["settings.models"].tap()
            XCTAssertTrue(endpoint.waitForExistence(timeout: 5))
            XCTAssertNotEqual(endpoint.value as? String, "https://draft.example.invalid/v1")
        }
    }

    func testSystemPromptWhitespaceSurvivesSaveAndReopenWithoutInference() throws {
        continueAfterFailure = false
        let app = XCUIApplication()
        app.launchArguments = ["-ui-testing", "-AppleLanguages", "(en)", "-AppleLocale", "en_US"]
        app.launchEnvironment["NOLA_UI_TEST_RUN_ID"] = "prompt-whitespace-\(UUID().uuidString)"
        app.launch()
        defer { app.terminate() }
        app.waitForRootTab("settings")
        app.openModelCenterFromSettings()
        let endpoint = app.textFields["model.setup.endpoint"]
        XCTAssertTrue(endpoint.waitForExistence(timeout: 5))
        endpoint.tap(); endpoint.typeText("https://prompt.example.invalid/v1")
        let model = app.textFields["model.setup.modelID"]
        model.tap(); model.typeText("prompt-whitespace-model\n")
        let save = app.buttons["model.setup.save"]
        for _ in 0..<6 where !save.isHittable { app.swipeUp() }
        save.tap()
        XCTAssertTrue(app.staticTexts["Saved and set as default · not tested"].waitForExistence(timeout: 10))
        let row = app.buttons.matching(NSPredicate(format: "identifier BEGINSWITH %@", "model.row.")).firstMatch
        for _ in 0..<6 where !row.isHittable { app.swipeUp() }
        XCTAssertTrue(row.exists); row.tap()
        let parameters = app.buttons.matching(NSPredicate(format: "identifier BEGINSWITH %@", "model.parameters.")).firstMatch
        for _ in 0..<4 where !parameters.isHittable { app.swipeUp() }
        XCTAssertTrue(parameters.exists); parameters.tap()
        let prompt = app.textViews["model-parameters-system-prompt"]
        for _ in 0..<10 where !prompt.isHittable { app.swipeUp() }
        XCTAssertTrue(prompt.exists)
        let exact = "  Preserve indentation\n    second line\n\n"
        prompt.tap(); prompt.typeText(exact)
        let parameterSave = app.buttons["model-parameters-save"]
        XCTAssertTrue(parameterSave.isHittable, "Parameter Save must remain tappable while editing text")
        XCTAssertGreaterThanOrEqual(parameterSave.frame.height, 44)
        parameterSave.tap()
        XCTAssertTrue(parameters.waitForExistence(timeout: 5)); parameters.tap()
        for _ in 0..<10 where !prompt.isHittable { app.swipeUp() }
        XCTAssertEqual(prompt.value as? String, exact)
        captureAudit("system-prompt-whitespace-reopened", in: app)
    }

    func testSkillAndMCPEditorsSaveAndDiscardThroughActualControls() throws {
        continueAfterFailure = false
        let app = XCUIApplication()
        app.launchArguments = ["-ui-testing", "-AppleLanguages", "(en)", "-AppleLocale", "en_US"]
        app.launchEnvironment["NOLA_UI_TEST_RUN_ID"] = "capability-editors-\(UUID().uuidString)"
        app.launch()
        defer { app.terminate() }
        app.tapRootTab("settings")
        app.buttons["settings.category.agent"].tap()
        let add = app.buttons["intelligence.capabilities.add"]
        XCTAssertTrue(add.waitForExistence(timeout: 5))
        XCTAssertGreaterThanOrEqual(add.frame.height, 44, "Add Skill must expose a full-height native touch target")
        let importSkill = app.buttons["intelligence.capabilities.importSkill"]
        XCTAssertTrue(importSkill.exists)
        XCTAssertGreaterThanOrEqual(importSkill.frame.height, 44, "Import Skill must expose a full-height native touch target")
        add.tap()
        let name = app.textFields["intelligence.capabilities.editor.name"]
        XCTAssertTrue(name.waitForExistence(timeout: 5))
        name.tap(); name.typeText("Audit Skill")
        app.buttons["intelligence.capabilities.editor.save"].tap()
        let editSkill = app.buttons.matching(NSPredicate(format: "identifier BEGINSWITH %@ AND identifier ENDSWITH %@",
            "intelligence.capabilities.skill.", ".edit")).firstMatch
        XCTAssertTrue(editSkill.waitForExistence(timeout: 5)); editSkill.tap()
        XCTAssertEqual(name.value as? String, "Audit Skill")
        captureAudit("skill-saved-and-reopened", in: app)
        name.tap(); name.typeText(" unsaved")
        app.buttons["intelligence.capabilities.editor.cancel"].tap()
        let keepEditing = app.buttons["Keep editing"]
        XCTAssertTrue(keepEditing.waitForExistence(timeout: 3))
        keepEditing.tap()
        XCTAssertEqual(name.value as? String, "Audit Skill unsaved")
        app.buttons["intelligence.capabilities.editor.cancel"].tap()
        XCTAssertTrue(app.buttons["Discard"].waitForExistence(timeout: 3))
        app.buttons["Discard"].tap()
        XCTAssertTrue(editSkill.waitForExistence(timeout: 5)); editSkill.tap()
        XCTAssertEqual(name.value as? String, "Audit Skill")
        captureAudit("skill-unsaved-changes-discarded", in: app)
        app.buttons["intelligence.capabilities.editor.cancel"].tap()
        let mcp = app.segmentedControls["intelligence.capabilities.kind"].buttons.matching(
            NSPredicate(format: "label BEGINSWITH %@", "MCP")).firstMatch
        XCTAssertTrue(mcp.exists); mcp.tap()
        XCTAssertGreaterThanOrEqual(add.frame.height, 44, "Add MCP must keep the same touch target")
        add.tap()
        XCTAssertTrue(name.waitForExistence(timeout: 5)); name.tap(); name.typeText("Audit MCP")
        let endpoint = app.textFields["intelligence.capabilities.editor.endpoint"]
        endpoint.tap(); endpoint.typeText("https://mcp.example.invalid/mcp")
        app.buttons["intelligence.capabilities.editor.save"].tap()
        let editMCP = app.buttons.matching(NSPredicate(format: "identifier BEGINSWITH %@ AND identifier ENDSWITH %@",
            "intelligence.capabilities.mcp.", ".edit")).firstMatch
        XCTAssertTrue(editMCP.waitForExistence(timeout: 5)); editMCP.tap()
        XCTAssertEqual(name.value as? String, "Audit MCP")
        XCTAssertEqual(endpoint.value as? String, "https://mcp.example.invalid/mcp")
        captureAudit("mcp-saved-and-reopened", in: app)
    }

    func testBuiltInExpertRoleAndMentionWithoutInference() throws {
        continueAfterFailure = false
        let app = XCUIApplication()
        app.launchArguments = ["-ui-testing", "-AppleLanguages", "(en)", "-AppleLocale", "en_US"]
        app.launchEnvironment["NOLA_UI_TEST_RUN_ID"] = "expert-role-\(UUID().uuidString)"
        app.launch()
        defer { app.terminate() }
        app.tapRootTab("settings")
        app.buttons["settings.category.agents"].tap()
        let template = app.buttons["experts.template"]
        XCTAssertTrue(template.waitForExistence(timeout: 5)); template.tap()
        let writer = app.buttons["Code writer"]
        XCTAssertTrue(writer.waitForExistence(timeout: 5)); writer.tap()
        let name = app.textFields["experts.name"]
        XCTAssertEqual(name.value as? String, "Code writer")
        let reasoning = app.buttons["experts.reasoning"]
        XCTAssertTrue(reasoning.exists); reasoning.tap()
        XCTAssertTrue(app.buttons["Inherit model setting"].waitForExistence(timeout: 3))
        XCTAssertFalse(app.buttons["None"].exists, "An unknown model must not offer an invented none level")
        app.buttons["Inherit model setting"].tap()
        let details = app.buttons["experts.role.details"]
        for _ in 0..<4 where !details.isHittable { app.swipeUp() }
        XCTAssertTrue(details.exists); details.tap()
        XCTAssertTrue(app.staticTexts["read_file"].exists)
        XCTAssertTrue(app.staticTexts["Code implementation"].exists)
        details.tap()
        let save = app.buttons["experts.save"]
        for _ in 0..<6 where !save.isHittable { app.swipeUp() }
        XCTAssertTrue(save.isHittable); save.tap()
        let expert = app.buttons.matching(NSPredicate(format: "identifier BEGINSWITH %@ AND label CONTAINS %@", "experts.edit.", "Code writer"))
        for _ in 0..<6 where !expert.element.isHittable { app.swipeDown() }
        XCTAssertEqual(expert.count, 1)
        let expertID = String(expert.element.identifier.dropFirst("experts.edit.".count))
        app.buttons["experts.management.done"].tap()
        app.tapRootTab("chat")
        let newChat = app.buttons["chat.conversations.new"]
        XCTAssertTrue(newChat.waitForExistence(timeout: 5)); newChat.tap()
        // The product's explicit New action creates a canonical owner before
        // any message. Capture that baseline independently of @ selection.
        app.buttons["chat.navigation.back"].tap()
        let rows = app.buttons.matching(NSPredicate(format: "identifier MATCHES %@", "chat\\.conversation\\.[0-9a-fA-F-]{36}"))
        XCTAssertTrue(rows.element.waitForExistence(timeout: 5))
        XCTAssertEqual(rows.count, 1)
        let ownerRowID = rows.element.identifier
        rows.element.tap()
        let input = app.textViews["chat.composer.input"]
        XCTAssertTrue(input.waitForExistence(timeout: 5)); input.tap(); input.typeText("@Code")
        let mention = app.buttons["chat.mention." + expertID]
        XCTAssertTrue(mention.waitForExistence(timeout: 5))
        let pickerShot = XCTAttachment(screenshot: app.screenshot())
        pickerShot.name = "ios-inline-mention-picker"; pickerShot.lifetime = .keepAlways; add(pickerShot)
        mention.tap()
        let completed = NSPredicate(format: "value CONTAINS[c] %@", "@Code writer")
        expectation(for: completed, evaluatedWith: input)
        waitForExpectations(timeout: 5)
        XCTAssertFalse(app.buttons["chat.expert.selected." + expertID].exists)
        let screenshot = XCTAttachment(screenshot: app.screenshot())
        screenshot.name = "builtin-role-selected-by-mention-without-inference"
        screenshot.lifetime = .keepAlways
        add(screenshot)
        app.buttons["chat.navigation.back"].tap()
        XCTAssertTrue(app.buttons[ownerRowID].waitForExistence(timeout: 5))
        XCTAssertEqual(rows.count, 1, "Mention selection must keep the same single ordinary owner")
    }

    func testExpertChildHistoriesStayUnderParentAndOldContextIsReadOnly() throws {
        guard let fixturePath = ProcessInfo.processInfo.environment["NOLA_CHILD_HISTORY_FIXTURE"],
              let data = FileManager.default.contents(atPath: fixturePath),
              let fixture = try JSONSerialization.jsonObject(with: data) as? [String: String],
              let database = fixture["database"], database.contains("nola-ios-child-history-fixture-"),
              let parent = fixture["parentID"], let old = fixture["oldID"],
              let current = fixture["currentID"] else {
            throw XCTSkip("Requires an isolated Core-authored parent/old/current child fixture")
        }
        continueAfterFailure = false
        let app = XCUIApplication()
        app.launchArguments = ["-ui-testing", "-AppleLanguages", "(en)", "-AppleLocale", "en_US"]
        app.launchEnvironment["NOLA_UI_TEST_RUN_ID"] = "child-history-\(UUID().uuidString)"
        app.launchEnvironment["NOLA_REAL_IOS_UI_DATABASE_PATH"] = database
        app.launch()
        defer { app.terminate() }
        app.tapRootTab("chat")
        let parentRow = app.buttons["chat.conversation." + parent.lowercased()]
        XCTAssertTrue(parentRow.waitForExistence(timeout: 5))
        let ordinaryRows = app.buttons.matching(NSPredicate(format: "identifier MATCHES %@", "chat\\.conversation\\.[0-9a-fA-F-]{36}"))
        XCTAssertEqual(ordinaryRows.count, 1)
        XCTAssertFalse(app.buttons["chat.child." + old].exists, "Collapsed branches cannot eagerly expose or load child histories")
        let toggle = app.buttons["chat.children.toggle." + parent.lowercased()]
        XCTAssertTrue(toggle.waitForExistence(timeout: 5)); XCTAssertGreaterThanOrEqual(toggle.frame.height, 44)
        toggle.tap()
        let oldRow = app.buttons["chat.child." + old]
        let currentRow = app.buttons["chat.child." + current]
        XCTAssertTrue(oldRow.waitForExistence(timeout: 5)); XCTAssertTrue(currentRow.exists)
        XCTAssertEqual(ordinaryRows.count, 1)
        let treeShot = XCTAttachment(screenshot: app.screenshot()); treeShot.name = "expert-child-history-tree"
        treeShot.lifetime = .keepAlways; add(treeShot)
        oldRow.tap()
        XCTAssertTrue(app.otherElements["chat.child.history"].waitForExistence(timeout: 5))
        XCTAssertTrue(app.staticTexts["Read-only history"].waitForExistence(timeout: 5))
        XCTAssertFalse(app.buttons["chat.child.continue"].exists)
        XCTAssertFalse(app.textViews["chat.composer.input"].exists)
        let oldShot = XCTAttachment(screenshot: app.screenshot()); oldShot.name = "expert-old-child-read-only"
        oldShot.lifetime = .keepAlways; add(oldShot)
        app.buttons["chat.child.done"].tap()
        XCTAssertTrue(currentRow.waitForExistence(timeout: 5)); currentRow.tap()
        let continueButton = app.buttons["chat.child.continue"]
        XCTAssertTrue(continueButton.waitForExistence(timeout: 5))
        XCTAssertFalse(app.textViews["chat.composer.input"].exists)
        continueButton.tap()
        let editor = app.textViews["chat.composer.input"]
        XCTAssertTrue(editor.waitForExistence(timeout: 10))
        expectation(for: NSPredicate(format: "value CONTAINS %@", "@History expert"), evaluatedWith: editor)
        waitForExpectations(timeout: 5)
        let continuedShot = XCTAttachment(screenshot: app.screenshot())
        continuedShot.name = "expert-current-context-returned-main-selection"
        continuedShot.lifetime = .keepAlways; add(continuedShot)
        app.buttons["chat.navigation.back"].tap()
        XCTAssertTrue(parentRow.waitForExistence(timeout: 5)); XCTAssertEqual(ordinaryRows.count, 1)
    }

    private func openExpertFixture() throws -> (app: XCUIApplication, database: URL) {
        #if targetEnvironment(simulator)
        continueAfterFailure = false
        let root = URL(fileURLWithPath: "/tmp/nola-ios-effort-fixture-" + UUID().uuidString, isDirectory: true)
        try FileManager.default.createDirectory(at: root, withIntermediateDirectories: false)
        let database = root.appendingPathComponent("state.db")
        let app = XCUIApplication()
        addTeardownBlock { app.terminate(); try FileManager.default.removeItem(at: root) }
        app.launchArguments = ["-ui-testing", "-AppleLanguages", "(en)", "-AppleLocale", "en_US"]
        app.launchEnvironment["NOLA_UI_TEST_RUN_ID"] = root.lastPathComponent
        app.launchEnvironment["NOLA_REAL_IOS_UI_DATABASE_PATH"] = database.path
        app.launch()
        app.waitForRootTab("settings")
        app.terminate()
        var connection: OpaquePointer?
        XCTAssertEqual(sqlite3_open_v2(database.path, &connection, SQLITE_OPEN_READWRITE, nil), SQLITE_OK)
        defer { sqlite3_close(connection) }
        let fixture = """
        BEGIN;
        INSERT INTO model_profiles(id,display_name,provider,model_name,runtime,source_json,parameters_json,selected,enabled,revision,created_at_ms,updated_at_ms)
        VALUES ('restricted','Declared effort fixture','openai','restricted','remote',
          '{"baseURL":"https://fixture.invalid/v1","wireProtocol":"responses","executionCapabilities":{"toolCalls":true,"imageInput":false,"reasoning":true,"samplingParameters":false},"reasoning":{"control":"effort","efforts":["low","high"]}}',
          '{}',1,1,1,1,1);
        INSERT OR REPLACE INTO product_settings(key,value_json,revision,updated_at_ms) VALUES
          ('chat.default_model','{"version":1,"selection":{"kind":"profile","profile_id":"restricted"}}',1,1),
          ('persistent_agent_directory','{"agents":[{"id":"77e34d10-a83b-4dbb-b175-20492a3a76f2","name":"Saved expert","description":"","revision":1,"availability":"active","defaults":{"modelSelectionID":"configured:restricted","selectedPromptTemplateID":"9f7d0000-4e6f-4c61-8000-000000000001","expertReasoningEffort":"high","modelOverrides":{"configured:restricted":{"temperature":0.7,"topP":1.0},"configured:other":{"temperature":0.25}}}}]}',1,1);
        COMMIT;
        """
        XCTAssertEqual(sqlite3_exec(connection, fixture, nil, nil, nil), SQLITE_OK, String(cString: sqlite3_errmsg(connection)))
        app.launch()
        return (app, database)
        #else
        throw XCTSkip("Requires a disposable simulator database")
        #endif
    }

    private func storedExpert(named name: String, database: URL) throws -> [String: Any] {
        var connection: OpaquePointer?
        XCTAssertEqual(sqlite3_open_v2(database.path, &connection, SQLITE_OPEN_READONLY, nil), SQLITE_OK)
        defer { sqlite3_close(connection) }
        var statement: OpaquePointer?
        let sql = "SELECT value_json FROM product_settings WHERE key='persistent_agent_directory'"
        XCTAssertEqual(sqlite3_prepare_v2(connection, sql, -1, &statement, nil), SQLITE_OK)
        defer { sqlite3_finalize(statement) }
        XCTAssertEqual(sqlite3_step(statement), SQLITE_ROW)
        let json = String(cString: try XCTUnwrap(sqlite3_column_text(statement, 0)))
        let directory = try XCTUnwrap(JSONSerialization.jsonObject(with: Data(json.utf8)) as? [String: [[String: Any]]])
        return try XCTUnwrap(directory["agents"]?.first { $0["name"] as? String == name })
    }

    private func expertDefaults(named name: String, database: URL) throws -> [String: Any] {
        try XCTUnwrap(storedExpert(named: name, database: database)["defaults"] as? [String: Any])
    }

    func testExpertRestoresOnlySelectedModelDefaultsOnSave() throws {
        let (app, database) = try openExpertFixture()
        app.tapRootTab("settings")
        app.buttons["settings.category.agents"].tap()
        let expert = app.buttons["experts.edit.77e34d10-a83b-4dbb-b175-20492a3a76f2"]
        XCTAssertTrue(expert.waitForExistence(timeout: 5))
        expert.tap()
        let original = try expertDefaults(named: "Saved expert", database: database)
        let reset = app.buttons["experts.model.restoreDefaults"]
        tapVisible(reset, in: app)
        XCTAssertFalse(reset.exists)
        XCTAssertEqual(try expertDefaults(named: "Saved expert", database: database) as NSDictionary, original as NSDictionary)
        tapVisible(app.buttons["experts.save"], in: app)
        let oldOverrides = try XCTUnwrap(original["modelOverrides"] as? [String: Any])
        var expected = original
        expected["modelOverrides"] = oldOverrides.filter { $0.key != "configured:restricted" }
        XCTAssertEqual(try expertDefaults(named: "Saved expert", database: database) as NSDictionary, expected as NSDictionary)
        XCTAssertEqual(try storedExpert(named: "Saved expert", database: database)["description"] as? String, "")
        tapVisible(expert, in: app)
        XCTAssertFalse(reset.exists)
        captureAudit("expert-restored-selected-model-only", in: app)
    }

    func testExpertDeclaredReasoningLevelPersistsWithoutInference() throws {
        let (app, database) = try openExpertFixture()
        app.tapRootTab("settings")
        app.buttons["settings.category.agents"].tap()
        let model = app.buttons["experts.model"]
        XCTAssertFalse(app.buttons["experts.create"].exists, "A new expert already has an open editor")
        XCTAssertTrue(model.waitForExistence(timeout: 5))
        let effort = app.buttons["experts.reasoning"]
        XCTAssertTrue(effort.waitForExistence(timeout: 5)); effort.tap()
        XCTAssertTrue(app.buttons["Low"].waitForExistence(timeout: 3))
        XCTAssertTrue(app.buttons["High"].exists, "Inherited choices must come from the Core app default declaration")
        app.buttons["Inherit model setting"].tap()
        model.tap(); app.buttons["Declared effort fixture"].tap()
        effort.tap()
        XCTAssertTrue(app.buttons["Low"].waitForExistence(timeout: 3))
        XCTAssertTrue(app.buttons["High"].exists)
        XCTAssertFalse(app.buttons["None"].exists)
        XCTAssertFalse(app.buttons["Medium"].exists, "Only the actual low/high declaration can produce options")
        app.buttons["High"].tap()
        let name = app.textFields["experts.name"]
        let expertName = "Effort fixture expert " + String(UUID().uuidString.prefix(8))
        name.tap(); name.typeText(expertName)
        let keyboardDone = app.buttons["experts.keyboard.done"]
        XCTAssertTrue(keyboardDone.waitForExistence(timeout: 3)); keyboardDone.tap()
        expectation(for: NSPredicate(format: "exists == false"), evaluatedWith: app.keyboards.element)
        waitForExpectations(timeout: 3)
        let save = app.buttons["experts.save"]
        for _ in 0..<6 where !save.isHittable { app.swipeUp() }
        XCTAssertTrue(save.isHittable); save.tap()
        let expert = app.buttons.matching(NSPredicate(format: "identifier BEGINSWITH %@ AND label CONTAINS %@", "experts.edit.", expertName))
        for _ in 0..<6 where !expert.element.isHittable { app.swipeDown() }
        XCTAssertEqual(expert.count, 1); expert.element.tap()
        XCTAssertTrue(effort.waitForExistence(timeout: 5))
        XCTAssertTrue(effort.label.contains("High") || (effort.value as? String)?.contains("High") == true)
        let high = try expertDefaults(named: expertName, database: database)
        XCTAssertEqual(high as NSDictionary, ["modelSelectionID": "configured:restricted", "expertReasoningEffort": "high"] as NSDictionary)
        XCTAssertFalse(app.buttons["experts.model.restoreDefaults"].exists)
        let highScreenshot = XCTAttachment(screenshot: app.screenshot())
        highScreenshot.name = "expert-declared-high-persisted-after-reopen"
        highScreenshot.lifetime = .keepAlways
        add(highScreenshot)
        effort.tap(); app.buttons["Inherit model setting"].tap()
        for _ in 0..<6 where !save.isHittable { app.swipeUp() }
        XCTAssertTrue(save.isHittable); save.tap()
        for _ in 0..<6 where !expert.element.isHittable { app.swipeDown() }
        expert.element.tap()
        XCTAssertTrue(effort.waitForExistence(timeout: 5))
        XCTAssertTrue(effort.label.contains("Inherit model setting") || (effort.value as? String)?.contains("Inherit model setting") == true)
        XCTAssertEqual(try expertDefaults(named: expertName, database: database) as NSDictionary,
            ["modelSelectionID": "configured:restricted"] as NSDictionary)
        let screenshot = XCTAttachment(screenshot: app.screenshot())
        screenshot.name = "expert-declared-effort-inheritance-restored"
        screenshot.lifetime = .keepAlways
        add(screenshot)
    }

    private func waitAndTapRootTab(_ tab: String, in app: XCUIApplication, timeout: TimeInterval = 5) {
        app.tapRootTab(tab, timeout: timeout)
    }

    func testConversationListHeaderRemainsLegibleAfterReturningFromWorkspaceTabs() throws {
        try checkConversationListHeader(language: "en", locale: "en_US", dark: false, titleLabel: "Chat")
    }

    func testChineseDarkConversationListHeaderRemainsLegibleAfterReturningFromWorkspaceTabs() throws {
        try checkConversationListHeader(language: "zh-Hans", locale: "zh_CN", dark: true, titleLabel: "对话")
    }

    func testChineseWorkspaceFiltersAndComposerInDarkAppearance() throws {
        try checkChineseWorkspaceFiltersAndComposer(theme: "dark")
    }

    func testChineseWorkspaceFiltersAndComposerInLightAppearance() throws {
        try checkChineseWorkspaceFiltersAndComposer(theme: "light")
    }

    private func checkChineseWorkspaceFiltersAndComposer(theme: String) throws {
        continueAfterFailure = false
        let app = XCUIApplication()
        app.launchArguments = ["-ui-testing", "-AppleLanguages", "(zh-Hans)", "-AppleLocale", "zh_CN",
                               "-nola.appearance", theme]
        app.launchEnvironment["NOLA_UI_TEST_RUN_ID"] = "workspace-filters-\(UUID().uuidString)"
        app.launch()
        defer { app.terminate() }
        app.tapRootTab("chat")
        func capture(_ name: String) {
            let attachment = XCTAttachment(screenshot: app.screenshot())
            attachment.name = "workspace-zh-\(theme)-\(name)"
            attachment.lifetime = .keepAlways
            add(attachment)
        }
        let newChat = app.buttons["chat.conversations.new"]
        XCTAssertTrue(newChat.waitForExistence(timeout: 5))
        capture("empty-list")
        newChat.tap()
        let input = app.textViews["chat.composer.input"]
        let attach = app.buttons["chat.composer.attach"]
        let send = app.buttons["chat.composer.send"]
        let back = app.buttons["chat.navigation.back"]
        XCTAssertTrue(input.waitForExistence(timeout: 5))
        XCTAssertTrue(input.isHittable && attach.isHittable)
        capture("composer")
        input.tap()
        let draft = "整理下周的读书笔记"
        input.typeText(draft)
        XCTAssertEqual(input.value as? String, draft)
        let keyboard = app.keyboards.firstMatch
        XCTAssertTrue(keyboard.waitForExistence(timeout: 5))
        XCTAssertLessThanOrEqual(input.frame.maxY, keyboard.frame.minY + 1)
        XCTAssertTrue(send.isHittable)
        capture("composer-draft")
        back.tap()

        let row = app.buttons.matching(NSPredicate(format: "identifier BEGINSWITH %@", "chat.conversation.")).firstMatch
        XCTAssertTrue(row.waitForExistence(timeout: 5))
        let owner = row.identifier
        let waiting = app.buttons["等待你回应"]
        let all = app.buttons["全部"]
        XCTAssertTrue(waiting.isHittable && all.isHittable)
        capture("filter-all")
        for _ in 0..<2 {
            waiting.tap()
            XCTAssertFalse(row.exists)
            XCTAssertTrue(app.staticTexts["没有匹配的会话"].waitForExistence(timeout: 5))
            all.tap()
            XCTAssertTrue(app.buttons[owner].waitForExistence(timeout: 5))
        }
        waiting.tap()
        capture("filter-waiting")
        all.tap()
        for (tab, anchor) in [("activity", "activity.plans"), ("information", "information.knowledge"),
                              ("settings", "settings.category.general")] {
            app.tapRootTab(tab)
            XCTAssertTrue(app.buttons[anchor].waitForExistence(timeout: 5))
            XCTAssertTrue(app.rootTabButton(tab).isSelected)
            capture(tab)
        }
        app.tapRootTab("chat")
        XCTAssertTrue(app.buttons[owner].waitForExistence(timeout: 5))
        capture("returned-list")
        app.buttons[owner].tap()
        XCTAssertTrue(input.waitForExistence(timeout: 5))
        XCTAssertEqual(input.value as? String, draft)
    }

    private func openShortMessageFixture(lowercaseOwner: Bool) throws -> XCUIApplication {
        #if targetEnvironment(simulator)
        continueAfterFailure = false
        let key = "A" + UUID().uuidString.dropFirst()
        let owner = lowercaseOwner ? key.lowercased() : key
        let directory = URL(fileURLWithPath: "/tmp/nola-ios-bubble-" + UUID().uuidString, isDirectory: true)
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: false)
        let database = directory.appendingPathComponent("nola.db")
        let app = XCUIApplication()
        addTeardownBlock {
            app.terminate()
            try FileManager.default.removeItem(at: directory)
        }
        app.launchArguments = ["-ui-testing", "-AppleLanguages", "(zh-Hans)", "-AppleLocale", "zh_CN",
                               "-nola.appearance", "dark"]
        app.launchEnvironment["NOLA_UI_TEST_RUN_ID"] = directory.lastPathComponent
        app.launchEnvironment["NOLA_REAL_IOS_UI_DATABASE_PATH"] = database.path
        app.launch()
        app.waitForRootTab("chat")
        app.terminate()
        // Let the product create the schema. Seed only this temporary store, with
        // its exact owner spelling and the two completed messages displayed below.
        try seedShortMessages(at: database, owner: owner)
        app.launch()
        app.tapRootTab("chat")
        let row = app.buttons["chat.conversation.\(owner.lowercased())"]
        XCTAssertTrue(row.waitForExistence(timeout: 5))
        row.tap()
        return app
        #else
        throw XCTSkip("Requires an isolated simulator database")
        #endif
    }

    private func seedShortMessages(at database: URL, owner: String) throws {
        var db: OpaquePointer?
        let opened = sqlite3_open_v2(database.path, &db, SQLITE_OPEN_READWRITE, nil)
        defer { sqlite3_close(db) }
        XCTAssertEqual(opened, SQLITE_OK)
        let sql = """
        BEGIN;
        INSERT INTO conversations(id,title,revision,created_at_ms,updated_at_ms)
            VALUES ('\(owner)','读书笔记',1,1,2);
        INSERT INTO messages(id,conversation_id,role,status,ordinal,created_at_ms,completed_at_ms)
            VALUES ('greeting','\(owner)','user','completed',1,1,1),
                   ('reply','\(owner)','assistant','completed',2,2,2);
        INSERT INTO message_parts(id,message_id,kind,ordinal,state,content_text,created_at_ms,updated_at_ms)
            VALUES ('greeting-text','greeting','text',1,'completed','你好',1,1),
                   ('reply-text','reply','text',1,'completed','你好！有什么我可以帮忙的吗？',2,2);
        COMMIT;
        """
        let result = sqlite3_exec(db, sql, nil, nil, nil)
        XCTAssertEqual(result, SQLITE_OK, String(cString: sqlite3_errmsg(db)))
    }

    func testLowercaseConversationOwnerOpensExactReadOnlyHistory() throws {
        let app = try openShortMessageFixture(lowercaseOwner: true)
        let reader = app.descendants(matching: .any)["chat.history.exactReader"].firstMatch
        XCTAssertTrue(reader.waitForExistence(timeout: 5))
        let reply = app.descendants(matching: .any).matching(NSPredicate(format: "label CONTAINS %@",
                                                                         "有什么我可以帮忙的吗")).firstMatch
        XCTAssertTrue(reply.waitForExistence(timeout: 5))
        XCTAssertFalse(app.textViews["chat.composer.input"].exists)
        XCTAssertFalse(app.buttons.matching(NSPredicate(format: "identifier BEGINSWITH %@", "chat.message.edit.")).firstMatch.exists)
        let attachment = XCTAttachment(screenshot: app.screenshot())
        attachment.name = "workspace-zh-dark-exact-owner-history"
        attachment.lifetime = .keepAlways
        add(attachment)
        app.buttons["完成"].tap()
        XCTAssertTrue(app.textFields["chat.conversations.search"].waitForExistence(timeout: 5))
    }

    func testShortUserMessageStaysCompactInConversation() throws {
        let app = try openShortMessageFixture(lowercaseOwner: false)
        let back = app.buttons["chat.navigation.back"]
        XCTAssertTrue(back.waitForExistence(timeout: 5))
        let bubbles = app.staticTexts.matching(identifier: "你好")
        let bodyVisible = NSPredicate { _, _ in
            bubbles.allElementsBoundByIndex.contains { $0.frame.midY > back.frame.maxY }
        }
        XCTAssertEqual(XCTWaiter.wait(for: [XCTNSPredicateExpectation(predicate: bodyVisible, object: app)], timeout: 5),
                       .completed)
        let title = try XCTUnwrap(bubbles.allElementsBoundByIndex.first { $0.frame.midY > back.frame.maxY })
        let screenshot = app.screenshot()
        let attachment = XCTAttachment(screenshot: screenshot)
        attachment.name = "workspace-zh-dark-short-message"
        attachment.lifetime = .keepAlways
        add(attachment)
        let width = try renderedBubbleWidth(in: screenshot, at: title.frame.midY, viewport: app.frame)
        let metrics = XCTAttachment(string: "viewport=\(app.frame), message=\(title.frame), renderedBubbleWidth=\(width)")
        metrics.name = "workspace-zh-dark-short-message-geometry"
        metrics.lifetime = .keepAlways
        add(metrics)
        XCTAssertTrue(app.frame.contains(title.frame))
        XCTAssertGreaterThan(width, 40, "The two glyphs must have horizontal padding")
        XCTAssertLessThan(width, app.frame.width / 3, "The short bubble must hug its content")
        XCTAssertTrue(app.textViews["chat.composer.input"].isHittable)
    }

    private func renderedBubbleWidth(in screenshot: XCUIScreenshot, at y: CGFloat, viewport: CGRect) throws -> CGFloat {
        let image = try XCTUnwrap(screenshot.image.cgImage)
        let scale = CGFloat(image.width) / viewport.width
        let row = try XCTUnwrap(image.cropping(to: CGRect(x: 0, y: floor((y - viewport.minY) * scale),
                                                         width: CGFloat(image.width), height: 1)))
        let context = try XCTUnwrap(CGContext(data: nil, width: row.width, height: 1, bitsPerComponent: 8,
                                              bytesPerRow: row.width * 4, space: CGColorSpaceCreateDeviceRGB(),
                                              bitmapInfo: CGImageAlphaInfo.noneSkipLast.rawValue))
        context.draw(row, in: CGRect(x: 0, y: 0, width: row.width, height: 1))
        let pixels = try XCTUnwrap(context.data).assumingMemoryBound(to: UInt8.self)
        // Only the user bubble occupies this scanline. Its leading margin gives a
        // local canvas sample in either appearance, including on historical builds.
        let occupied = (0..<row.width).filter { x in
            (0..<3).contains { channel in abs(Int(pixels[4 * x + channel]) - Int(pixels[channel])) >= 6 }
        }
        let first = try XCTUnwrap(occupied.first)
        let last = try XCTUnwrap(occupied.last)
        return CGFloat(last - first + 1) / scale
    }

    private func checkConversationListHeader(language: String, locale: String, dark: Bool, titleLabel: String) throws {
        continueAfterFailure = false
        let app = XCUIApplication()
        let theme = dark ? "dark" : "light"
        let prefix = "chat-header-\(language)-\(theme)"
        app.launchArguments = ["-ui-testing", "-AppleLanguages", "(\(language))", "-AppleLocale", locale,
                               "-nola.appearance", theme]
        app.launchEnvironment["NOLA_UI_TEST_RUN_ID"] = "chat-header-\(UUID().uuidString)"
        app.launch()
        defer { app.terminate() }
        app.tapRootTab("chat")
        let newChat = app.buttons["chat.conversations.new"]
        XCTAssertTrue(newChat.waitForExistence(timeout: 5))
        newChat.tap()
        let conversationBack = app.buttons["chat.navigation.back"]
        XCTAssertTrue(conversationBack.waitForExistence(timeout: 5))
        conversationBack.tap()

        let search = app.textFields["chat.conversations.search"]
        let firstRow = app.buttons.matching(NSPredicate(format: "identifier BEGINSWITH %@", "chat.conversation.")).firstMatch
        func capture(_ name: String) -> XCUIScreenshot {
            let screenshot = app.screenshot()
            let attachment = XCTAttachment(screenshot: screenshot)
            attachment.name = "\(prefix)-\(name)"
            attachment.lifetime = .keepAlways
            add(attachment)
            return screenshot
        }
        func inspectHeader(_ name: String) throws -> (frame: CGRect, contrast: Double) {
            XCTAssertTrue(search.waitForExistence(timeout: 5))
            XCTAssertTrue(firstRow.waitForExistence(timeout: 5))
            let screenshot = capture(name)
            if name == "before" {
                try XCUIElementPixelAssertions.assertScreenshot(screenshot, label: "\(prefix)-baseline", isDark: dark)
            }
            XCTAssertTrue(app.rootTabButton("chat").isSelected)
            let title = try XCTUnwrap(app.staticTexts.matching(identifier: titleLabel).allElementsBoundByIndex
                .first { $0.frame.midY < search.frame.minY })
            for element in [title, search, newChat, firstRow] {
                XCTAssertTrue(app.frame.contains(element.frame), "\(name): clipped \(element.identifier)")
            }
            XCTAssertTrue(search.isHittable && newChat.isHittable && firstRow.isHittable)
            XCTAssertLessThanOrEqual(title.frame.maxY, search.frame.minY)
            XCTAssertLessThanOrEqual(search.frame.maxY, firstRow.frame.minY)
            let appStatusBar = app.statusBars.firstMatch
            let statusBar = appStatusBar.exists ? appStatusBar : XCUIApplication(bundleIdentifier: "com.apple.springboard").statusBars.firstMatch
            if statusBar.exists { XCTAssertGreaterThanOrEqual(title.frame.minY, statusBar.frame.maxY) }
            let contrast = try titleEdgeContrast(in: screenshot, frame: title.frame, viewport: app.frame)
            let statusFrame = statusBar.exists ? String(describing: statusBar.frame) : "unavailable"
            let metrics = XCTAttachment(string: "statusBar=\(statusFrame), title=\(title.frame), search=\(search.frame), firstRow=\(firstRow.frame), titleEdgeContrast=\(contrast)")
            metrics.name = "\(prefix)-\(name)-metrics"
            metrics.lifetime = .keepAlways
            add(metrics)
            return (title.frame, contrast)
        }

        let baseline = try inspectHeader("before")
        var contrasts: [(tab: String, value: Double)] = []
        for (tab, entry) in [("information", "information.knowledge"), ("activity", "activity.plans"),
                             ("settings", "settings.category.general")] {
            app.tapRootTab(tab)
            let button = app.buttons[entry]
            XCTAssertTrue(button.waitForExistence(timeout: 5))
            XCTAssertTrue(app.rootTabButton(tab).isSelected)
            button.tap()
            let back = app.navigationBars.buttons.firstMatch
            XCTAssertTrue(back.waitForExistence(timeout: 5))
            _ = capture("\(tab)-detail")
            back.tap()
            XCTAssertTrue(button.waitForExistence(timeout: 5))
            app.tapRootTab("chat")
            let returned = try inspectHeader("after-\(tab)")
            XCTAssertEqual(returned.frame.minY, baseline.frame.minY, accuracy: 1, tab)
            contrasts.append((tab, returned.contrast))
        }
        firstRow.tap()
        XCTAssertTrue(conversationBack.waitForExistence(timeout: 5))
        XCTAssertGreaterThan(baseline.contrast, 40, "The heading must already be legible on first display")
        for sample in contrasts {
            XCTAssertGreaterThanOrEqual(sample.value, max(40, baseline.contrast * 0.8),
                                        "\(sample.tab): returning softened or obscured the chat heading")
        }
    }

    /// Scroll-edge blur fades vertically. A few sharp pixels in the lower strokes
    /// can survive even when the top half of the heading is unreadable.
    private func titleEdgeContrast(in screenshot: XCUIScreenshot, frame: CGRect, viewport: CGRect) throws -> Double {
        let screen = try XCTUnwrap(screenshot.image.cgImage)
        let scaleX = CGFloat(screen.width) / viewport.width
        let scaleY = CGFloat(screen.height) / viewport.height
        let bounds = frame.offsetBy(dx: -viewport.minX, dy: -viewport.minY)
            .applying(CGAffineTransform(scaleX: scaleX, y: scaleY)).integral
        let title = try XCTUnwrap(screen.cropping(to: bounds))
        let width = title.width, height = title.height
        let context = try XCTUnwrap(CGContext(data: nil, width: width, height: height, bitsPerComponent: 8,
                                              bytesPerRow: width, space: CGColorSpaceCreateDeviceGray(),
                                              bitmapInfo: CGImageAlphaInfo.none.rawValue))
        context.draw(title, in: CGRect(x: 0, y: 0, width: width, height: height))
        let pixels = try XCTUnwrap(context.data).assumingMemoryBound(to: UInt8.self)
        var rowContrasts: [Double] = []
        for y in 0..<height {
            let row = UnsafeBufferPointer(start: pixels + y * width, count: width)
            guard let light = row.max(), let dark = row.min(), Int(light) - Int(dark) >= 64 else { continue }
            let strongest = zip(row.dropFirst(), row)
                .map { abs(Int($0.0) - Int($0.1)) }.sorted(by: >).prefix(4)
            guard !strongest.isEmpty else { continue }
            rowContrasts.append(Double(strongest.reduce(0, +)) / Double(strongest.count))
        }
        guard !rowContrasts.isEmpty else { return 0 }
        return rowContrasts.sorted()[rowContrasts.count / 4]
    }

    func testCompactTabBarRespondsAtBothEdgesOfEveryTab() throws {
        continueAfterFailure = false
        let app = XCUIApplication()
        app.launchArguments = ["-ui-testing", "-AppleLanguages", "(en)", "-AppleLocale", "en_US"]
        app.launchEnvironment["NOLA_UI_TEST_RUN_ID"] = "compact-tab-targets-\(UUID().uuidString)"
        app.launch()
        defer { app.terminate() }
        app.waitForRootTab("chat")
        let tabs = ["chat", "activity", "information", "settings"]
        let buttons = tabs.map { app.buttons["main.tab.\($0)"] }
        try XCTSkipUnless(buttons[0].exists, "Requires the custom compact-width tab bar")
        let destinations = [app.textFields["chat.conversations.search"], app.buttons["activity.plans"],
                            app.buttons["information.knowledge"], app.buttons["settings.models"]]
        let centers = buttons.map { $0.frame.midX }
        let slotWidth = centers[1] - centers[0]
        XCTAssertGreaterThan(slotWidth, 44)
        let origin = app.coordinate(withNormalizedOffset: .zero)
        for direction in [-1, 1] {
            // Start with Activity so the first tap must change the visible page.
            for index in [1, 2, 3, 0] {
                let button = buttons[index]
                XCTAssertFalse(button.isSelected)
                let x = centers[index] + CGFloat(direction) * (slotWidth / 2 - 5)
                let offset = CGVector(dx: x - app.frame.minX, dy: button.frame.midY - app.frame.minY)
                origin.withOffset(offset).tap()
                XCTAssertTrue(destinations[index].waitForExistence(timeout: 5))
                XCTAssertTrue(destinations[index].isHittable, "\(tabs[index]) edge \(direction)")
                XCTAssertTrue(button.isSelected)
            }
        }
    }

    func testConversationSearchFieldAcceptsTapsAcrossItsBackground() throws {
        continueAfterFailure = false
        let app = XCUIApplication()
        app.launchArguments = ["-ui-testing", "-AppleLanguages", "(en)", "-AppleLocale", "en_US"]
        app.launchEnvironment["NOLA_UI_TEST_RUN_ID"] = "search-target-\(UUID().uuidString)"
        app.launch()
        defer { app.terminate() }
        app.tapRootTab("chat")
        let newChat = app.buttons["chat.conversations.new"]
        XCTAssertTrue(newChat.waitForExistence(timeout: 5))
        newChat.tap()
        let back = app.buttons["chat.navigation.back"]
        XCTAssertTrue(back.waitForExistence(timeout: 5))
        back.tap()
        let row = app.buttons.matching(NSPredicate(format: "identifier BEGINSWITH %@", "chat.conversation.")).firstMatch
        let search = app.textFields["chat.conversations.search"]
        XCTAssertTrue(row.waitForExistence(timeout: 5))
        XCTAssertTrue(search.exists)
        let viewport = app.frame
        let centerY = search.frame.midY
        let inset: CGFloat = viewport.width < 359 ? 16 : 20
        let points = [("magnifier", CGPoint(x: viewport.minX + inset + 22, y: centerY)),
                      ("top-padding", CGPoint(x: viewport.midX, y: centerY - 19))]
        let origin = app.coordinate(withNormalizedOffset: .zero)
        var missed: [String] = []
        for (name, point) in points {
            XCTAssertFalse(app.keyboards.firstMatch.exists)
            origin.withOffset(CGVector(dx: point.x - viewport.minX, dy: point.y - viewport.minY)).tap()
            let focused = app.keyboards.firstMatch.waitForExistence(timeout: 3)
            if focused {
                app.typeText("quartz")
                XCTAssertTrue(app.staticTexts["No matching conversations"].waitForExistence(timeout: 5))
                XCTAssertFalse(row.exists)
            } else { missed.append(name) }
            let attachment = XCTAttachment(screenshot: app.screenshot())
            attachment.name = "conversation-search-hit-\(name)"
            attachment.lifetime = .keepAlways
            add(attachment)
            if focused {
                let clear = app.buttons["chat.conversations.search.clear"]
                XCTAssertTrue(clear.isHittable)
                XCTAssertGreaterThanOrEqual(clear.frame.height, 44)
                clear.tap()
                XCTAssertTrue(row.waitForExistence(timeout: 5))
                row.tap()
                XCTAssertTrue(back.waitForExistence(timeout: 5))
                back.tap()
                XCTAssertTrue(search.waitForExistence(timeout: 5))
            }
        }
        XCTAssertTrue(missed.isEmpty, "Search taps did not focus the field at \(missed.joined(separator: ", "))")
    }

    func testSearchFindsUnsentDraftAndOpensItsConversation() throws {
        continueAfterFailure = false
        let app = XCUIApplication()
        app.launchArguments = ["-ui-testing", "-AppleLanguages", "(en)", "-AppleLocale", "en_US"]
        app.launchEnvironment["NOLA_UI_TEST_RUN_ID"] = "draft-search-\(UUID().uuidString)"
        app.launch()
        defer { app.terminate() }
        app.tapRootTab("chat")

        let newChat = app.buttons["chat.conversations.new"]
        let input = app.textViews["chat.composer.input"]
        let back = app.buttons["chat.navigation.back"]
        let search = app.textFields["chat.conversations.search"]
        let rows = app.buttons.matching(NSPredicate(format: "identifier BEGINSWITH %@", "chat.conversation."))
        func capture(_ step: String) {
            let attachment = XCTAttachment(screenshot: app.screenshot())
            attachment.name = "unsent-draft-search-\(step)"
            attachment.lifetime = .keepAlways
            add(attachment)
        }
        func createDraft(_ text: String) {
            XCTAssertTrue(newChat.waitForExistence(timeout: 5))
            newChat.tap()
            XCTAssertTrue(input.waitForExistence(timeout: 5))
            input.tap()
            input.typeText(text)
            XCTAssertEqual(input.value as? String, text)
            back.tap()
            XCTAssertTrue(search.waitForExistence(timeout: 5))
        }

        let query = "quartz"
        let original = "Review quartz samples tomorrow"
        createDraft(original)
        XCTAssertTrue(rows.firstMatch.waitForExistence(timeout: 5))
        let originalID = rows.firstMatch.identifier
        createDraft("Pack the conference bag")
        XCTAssertEqual(rows.count, 2)
        let otherID = try XCTUnwrap(rows.allElementsBoundByIndex.map(\.identifier).first { $0 != originalID })
        capture("both-conversations")

        search.tap()
        search.typeText(query)
        let originalRow = app.buttons[originalID]
        XCTAssertTrue(originalRow.waitForExistence(timeout: 5))
        XCTAssertFalse(app.buttons[otherID].exists)
        XCTAssertEqual(rows.count, 1)
        capture("filtered")
        originalRow.tap()
        XCTAssertTrue(input.waitForExistence(timeout: 5))
        XCTAssertEqual(input.value as? String, original)
        capture("reopened")
        back.tap()
        XCTAssertTrue(search.waitForExistence(timeout: 5))
        XCTAssertEqual(search.value as? String, query)
        search.tap()
        search.typeText(String(repeating: XCUIKeyboardKey.delete.rawValue, count: query.count))
        XCTAssertTrue(app.buttons[otherID].waitForExistence(timeout: 5))
        XCTAssertEqual(rows.count, 2)
        capture("search-cleared")
    }

    func testManagementPagesAndNavigationInBothThemesWithoutModelAccount() throws {
        continueAfterFailure = false
        let app = XCUIApplication()
        app.launchArguments = ["-ui-testing", "-AppleLanguages", "(en)", "-AppleLocale", "en_US"]
        app.launchEnvironment["NOLA_UI_TEST_RUN_ID"] = "navigation-\(UUID().uuidString)"
        app.launch()
        defer { app.terminate() }
        app.waitForRootTab("chat")

        func tap(_ identifier: String) {
            let button = app.buttons[identifier]
            XCTAssertTrue(button.waitForExistence(timeout: 5), identifier)
            for _ in 0..<5 where !button.isHittable { app.swipeUp() }
            for _ in 0..<5 where !button.isHittable { app.swipeDown() }
            XCTAssertTrue(button.isHittable, identifier)
            button.tap()
        }
        func capture(_ name: String, isDark: Bool) throws {
            // Accessibility nodes can exist before the native push settles.
            // Keep real animations enabled and capture their completed state.
            Thread.sleep(forTimeInterval: 0.6)
            let screenshot = app.screenshot()
            try XCUIElementPixelAssertions.assertScreenshot(screenshot, label: name, isDark: isDark)
            let attachment = XCTAttachment(data: screenshot.pngRepresentation, uniformTypeIdentifier: "public.png")
            attachment.name = name
            attachment.lifetime = .keepAlways
            add(attachment)
        }
        func backToSettings() {
            let back = app.navigationBars.buttons.element(boundBy: 0)
            XCTAssertTrue(back.waitForExistence(timeout: 3))
            back.tap()
            XCTAssertTrue(app.buttons["settings.models"].waitForExistence(timeout: 3))
        }

        for theme in ["Light", "Dark"] {
            let isDark = theme == "Dark"
            waitAndTapRootTab("settings", in: app)
            tap("settings.category.general")
            tap("settings.appearance")
            let choice = app.buttons[theme]
            XCTAssertTrue(choice.waitForExistence(timeout: 3))
            choice.tap()
            try capture("appearance-\(theme)", isDark: isDark)
            backToSettings()

            tap("settings.category.agents")
            XCTAssertTrue(app.buttons["experts.management.done"].waitForExistence(timeout: 5))
            try capture("experts-\(theme)", isDark: isDark)
            tap("experts.management.done")

            for identifier in ["settings.models", "settings.category.general", "settings.category.agent", "settings.category.privacy"] {
                tap(identifier)
                try capture("\(identifier)-\(theme)", isDark: isDark)
                if identifier == "settings.category.general" {
                    let update = app.buttons["Check for updates"]
                    for _ in 0..<6 where !update.isHittable { app.swipeUp() }
                    XCTAssertTrue(update.isHittable, "Software updates remain reachable in General")
                    try capture("general-lower-\(theme)", isDark: isDark)
                }
                backToSettings()
            }
            for destination in ["knowledge", "memory", "sources"] {
                waitAndTapRootTab("information", in: app)
                tap("information.\(destination)")
                try capture("library-\(destination)-\(theme)", isDark: isDark)
                let back = app.navigationBars.buttons.element(boundBy: 0)
                XCTAssertTrue(back.waitForExistence(timeout: 3))
                back.tap()
                XCTAssertTrue(app.buttons["information.knowledge"].waitForExistence(timeout: 3))
            }
            for tab in ["chat", "activity", "information", "settings"] {
                waitAndTapRootTab(tab, in: app)
                try capture("primary-\(tab)-\(theme)", isDark: isDark)
            }
        }
    }

    func testPlansRemainReachableFromActivityWithoutASettingsShortcut() throws {
        continueAfterFailure = false
        let app = XCUIApplication()
        app.launchArguments = ["-ui-testing", "-AppleLanguages", "(en)", "-AppleLocale", "en_US"]
        app.launchEnvironment["NOLA_UI_TEST_RUN_ID"] = "plans-navigation-\(UUID().uuidString)"
        app.launch()
        defer { app.terminate() }
        app.waitForRootTab("activity")
        for theme in ["Light", "Dark"] {
            let isDark = theme == "Dark"
            // Exercise the real setting instead of relying on the simulator's
            // appearance or a process argument overridden by saved preferences.
            waitAndTapRootTab("settings", in: app)
            let general = app.buttons["settings.category.general"]
            XCTAssertTrue(general.waitForExistence(timeout: 5)); general.tap()
            let appearance = app.buttons["settings.appearance"]
            XCTAssertTrue(appearance.waitForExistence(timeout: 5)); appearance.tap()
            let choice = app.buttons[theme]
            XCTAssertTrue(choice.waitForExistence(timeout: 5)); choice.tap()
            let settingsBack = app.navigationBars.buttons.element(boundBy: 0)
            XCTAssertTrue(settingsBack.waitForExistence(timeout: 3)); settingsBack.tap()
            XCTAssertTrue(app.buttons["settings.models"].waitForExistence(timeout: 5))

            waitAndTapRootTab("activity", in: app)
            let plans = app.buttons["activity.plans"]
            let search = app.textFields["activity.search"]
            XCTAssertTrue(plans.waitForExistence(timeout: 5))
            XCTAssertTrue(search.waitForExistence(timeout: 5))
            let viewport = app.windows.firstMatch.frame
            XCTAssertTrue(viewport.contains(plans.frame), "\(theme): Plans entry must not be clipped")
            XCTAssertTrue(viewport.contains(search.frame), "\(theme): Search field must not be clipped")
            XCTAssertGreaterThan(search.frame.width, 0)
            XCTAssertGreaterThan(search.frame.height, 0)
            XCTAssertTrue(plans.isHittable)
            XCTAssertTrue(search.isHittable)
            let screenshot = app.screenshot()
            try XCUIElementPixelAssertions.assertScreenshot(
                screenshot,
                label: "activity-plans-entry-and-search-\(theme)",
                isDark: isDark
            )
            let activityScreenshot = XCTAttachment(screenshot: screenshot)
            activityScreenshot.name = "activity-plans-entry-and-search-\(theme)"
            activityScreenshot.lifetime = .keepAlways
            add(activityScreenshot)
            plans.tap()
            XCTAssertTrue(app.staticTexts["No automations yet"].waitForExistence(timeout: 5))
            let createInChat = app.buttons["Create in chat"]
            XCTAssertTrue(createInChat.isHittable)
            createInChat.tap()
            XCTAssertTrue(app.textViews["chat.composer.input"].waitForExistence(timeout: 5),
                          "Create in chat opens a writable conversation immediately")
            captureAudit("plan-created-conversation-" + theme, in: app)
            app.buttons["chat.navigation.back"].tap()
            waitAndTapRootTab("activity", in: app)
            let back = app.navigationBars.buttons.element(boundBy: 0)
            XCTAssertTrue(back.waitForExistence(timeout: 3)); back.tap()
            XCTAssertTrue(plans.waitForExistence(timeout: 3))
        }
    }

}

extension IOSWorkspaceNavigationSmokeTests {
    private func launchSettingsAudit(_ name: String) -> XCUIApplication {
        continueAfterFailure = false
        let app = XCUIApplication()
        app.launchArguments = ["-ui-testing", "-AppleLanguages", "(en)", "-AppleLocale", "en_US", "-nola.appearance", "dark"]
        app.launchEnvironment["NOLA_UI_TEST_RUN_ID"] = name + "-" + UUID().uuidString
        app.launch()
        return app
    }

    private func tapVisible(_ element: XCUIElement, in app: XCUIApplication) {
        for _ in 0..<5 where !element.isHittable { app.swipeUp() }
        for _ in 0..<5 where !element.isHittable { app.swipeDown() }
        XCTAssertTrue(element.isHittable, element.description)
        element.tap()
    }

    private func captureAudit(_ name: String, in app: XCUIApplication) {
        let attachment = XCTAttachment(screenshot: app.screenshot())
        attachment.name = name
        attachment.lifetime = .keepAlways
        add(attachment)
    }

    func testLibraryHeadingRemainsSharpAfterReturningFromSourcesInBothThemes() throws {
        for theme in ["light", "dark"] {
            let app = XCUIApplication()
            app.launchArguments = ["-ui-testing", "-AppleLanguages", "(en)", "-AppleLocale", "en_US", "-nola.appearance", theme]
            app.launchEnvironment["NOLA_UI_TEST_RUN_ID"] = "library-heading-" + UUID().uuidString
            app.launch()
            defer { app.terminate() }
            app.openWorkspaceManagement("sources")
            app.navigationBars.buttons.firstMatch.tap()
            app.tapRootTab("chat")
            app.tapRootTab("information")
            XCTAssertTrue(app.buttons["information.knowledge"].waitForExistence(timeout: 5))
            let heading = app.staticTexts["information.heading"].firstMatch
            XCTAssertTrue(heading.exists)
            let screenshot = app.screenshot()
            let sharpness = try titleEdgeContrast(in: screenshot, frame: heading.frame, viewport: app.frame)
            XCTAssertGreaterThan(sharpness, 40, "The top strokes of the Library title must remain readable after navigation")
            captureAudit("library-heading-returned-" + theme, in: app)
        }
    }

    func testRegularWidthWorkspacePagesReturnToAnExistingConversation() throws {
        let app = launchSettingsAudit("ipad-workspace")
        defer { app.terminate() }
        try XCTSkipUnless(app.frame.width >= 700, "Requires a regular-width iPad window")
        app.tapRootTab("chat")
        let create = app.buttons["chat.conversations.new"]
        XCTAssertTrue(create.waitForExistence(timeout: 5))
        create.tap()
        let back = app.buttons["chat.navigation.back"]
        XCTAssertTrue(back.waitForExistence(timeout: 5))
        back.tap()
        let rows = app.buttons.matching(NSPredicate(format: "identifier MATCHES %@", "chat\\.conversation\\.[0-9a-fA-F-]{36}"))
        XCTAssertEqual(rows.count, 1)
        let owner = rows.element.identifier
        for (tab, control) in [("chat", create), ("activity", app.buttons["activity.plans"]),
                               ("information", app.buttons["information.knowledge"]),
                               ("settings", app.buttons["settings.category.general"])] {
            app.tapRootTab(tab)
            XCTAssertTrue(control.waitForExistence(timeout: 5))
            XCTAssertTrue(control.isHittable)
            XCTAssertTrue(app.frame.contains(control.frame))
            captureAudit("ipad-workspace-" + tab, in: app)
        }
        app.tapRootTab("chat")
        app.buttons[owner].tap()
        XCTAssertTrue(app.textViews["chat.composer.input"].waitForExistence(timeout: 5))
        captureAudit("ipad-workspace-restored-conversation", in: app)
    }

    func testDataOverviewNotificationTargetAndDarkEmptyState() throws {
        let app = launchSettingsAudit("notifications")
        defer { app.terminate() }
        app.openWorkspaceManagement("sources")
        let bell = app.buttons["data.notifications"]
        XCTAssertTrue(bell.waitForExistence(timeout: 5))
        // UIKit exposes the toolbar's visual button frame. Exercise the outer
        // edge of its 44-point touch region from the actual window instead.
        let toolbarPoint = CGVector(dx: bell.frame.midX, dy: bell.frame.midY + 21)
        app.coordinate(withNormalizedOffset: .zero).withOffset(toolbarPoint).tap()
        XCTAssertTrue(app.navigationBars["Notifications"].waitForExistence(timeout: 5))
        try XCUIElementPixelAssertions.assertScreenshot(app.screenshot(), label: "empty-notifications", isDark: true)
        captureAudit("data-notifications", in: app)
        app.navigationBars.buttons.element(boundBy: 0).tap()
        XCTAssertTrue(app.buttons["data.center.open"].waitForExistence(timeout: 5))
    }

    func testDeviceDataSourceAccessSettingsSurviveReopening() {
        let app = launchSettingsAudit("device-policy")
        defer { app.terminate() }
        app.openWorkspaceManagement("sources")
        tapVisible(app.buttons["data.center.open"], in: app)
        for source in ["contacts", "calendars", "reminders", "health"] {
            tapVisible(app.buttons["data.center.source." + source], in: app)
            let action = app.buttons["data.source.primaryAction"]
            XCTAssertTrue(action.waitForExistence(timeout: 5))
            XCTAssertGreaterThanOrEqual(action.frame.height, 44)
            captureAudit("device-" + source, in: app)
            let menu = app.buttons["data.source.access.policy"]
            tapVisible(menu, in: app)
            app.buttons["Blocked"].tap()
            XCTAssertEqual(app.staticTexts["data.source.policy.summary"].label, "Never exposed to conversations or automations.")
            app.navigationBars.buttons.element(boundBy: 0).tap()
            tapVisible(app.buttons["data.center.source." + source], in: app)
            let restoredSummary = app.staticTexts["data.source.policy.summary"]
            XCTAssertTrue(restoredSummary.waitForExistence(timeout: 5))
            XCTAssertEqual(restoredSummary.label, "Never exposed to conversations or automations.")
            tapVisible(menu, in: app)
            app.buttons["Per conversation"].tap()
            app.navigationBars.buttons.element(boundBy: 0).tap()
        }
    }

    func testDataSourceEditorKeepsDraftUntilDiscardConfirmed() {
        let app = launchSettingsAudit("source-editor")
        defer { app.terminate() }
        app.openWorkspaceManagement("sources")
        tapVisible(app.buttons["data.center.open"], in: app)
        tapVisible(app.buttons.matching(NSPredicate(format: "label == %@", "Add source")).firstMatch, in: app)
        let name = app.textFields["data.center.editor.name"]
        XCTAssertTrue(name.waitForExistence(timeout: 5))
        name.tap()
        name.typeText("Local audit")
        app.buttons["data.center.editor.cancel"].tap()
        let alert = app.alerts["Discard changes?"]
        XCTAssertTrue(alert.waitForExistence(timeout: 5))
        alert.buttons["Keep editing"].tap()
        XCTAssertEqual(name.value as? String, "Local audit")
        captureAudit("source-editor-kept-draft", in: app)
        app.buttons["data.center.editor.cancel"].tap()
        alert.buttons["Discard"].tap()
        let closed = XCTNSPredicateExpectation(predicate: NSPredicate(format: "exists == false"), object: name)
        XCTAssertEqual(XCTWaiter.wait(for: [closed], timeout: 5), .completed)
        XCTAssertTrue(app.buttons["data.center.source.contacts"].exists)
    }

    func testPrivacyDisclosureAndResetConfirmationCanReturnToSettings() {
        let app = launchSettingsAudit("privacy")
        defer { app.terminate() }
        app.tapRootTab("settings")
        app.buttons["settings.category.privacy"].tap()
        tapVisible(app.buttons["Privacy"], in: app)
        XCTAssertTrue(app.navigationBars["Privacy"].waitForExistence(timeout: 5))
        captureAudit("privacy-disclosure", in: app)
        app.navigationBars.buttons.element(boundBy: 0).tap()
        let reset = app.buttons["settings.privacy.clear"]
        tapVisible(reset, in: app)
        let alert = app.alerts["Clear All Data"]
        XCTAssertTrue(alert.waitForExistence(timeout: 5))
        captureAudit("privacy-reset-confirm", in: app)
        alert.buttons["Cancel"].tap()
        XCTAssertFalse(alert.exists)
        app.navigationBars.buttons.element(boundBy: 0).tap()
        XCTAssertTrue(app.buttons["settings.category.privacy"].waitForExistence(timeout: 5))
    }
}
