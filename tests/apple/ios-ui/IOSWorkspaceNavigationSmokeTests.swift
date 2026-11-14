// SPDX-License-Identifier: Apache-2.0
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
        XCTAssertTrue(app.buttons["main.tab.settings"].waitForExistence(timeout: 10))
        app.openModelCenterFromSettings()
        let endpoint = app.textFields["model.setup.endpoint"]
        XCTAssertTrue(endpoint.waitForExistence(timeout: 5))
        endpoint.tap()
        endpoint.typeText("https://draft.example.invalid/v1\n")
        app.buttons["model.setup.source.2"].tap()
        XCTAssertTrue(app.buttons["model.setup.close"].waitForExistence(timeout: 5))
        app.buttons["model.setup.close"].tap()
        let confirmationEvidence = XCTAttachment(string: app.debugDescription)
        confirmationEvidence.name = "model-draft-close-accessibility"
        confirmationEvidence.lifetime = .keepAlways
        add(confirmationEvidence)
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
        let keep = discardAlertAction(identifier: "model.setup.keep_editing", label: "Keep editing")
        keep.tap()
        app.buttons["model.setup.source.0"].tap()
        XCTAssertEqual(endpoint.value as? String, "https://draft.example.invalid/v1")
        app.buttons["model.setup.close"].tap()
        let discard = discardAlertAction(identifier: "model.setup.discard", label: "Discard changes")
        discard.tap()
        XCTAssertTrue(app.buttons["settings.models"].waitForExistence(timeout: 5))
        app.buttons["settings.models"].tap()
        XCTAssertTrue(endpoint.waitForExistence(timeout: 5))
        XCTAssertNotEqual(endpoint.value as? String, "https://draft.example.invalid/v1")
    }

    func testSystemPromptWhitespaceSurvivesSaveAndReopenWithoutInference() throws {
        continueAfterFailure = false
        let app = XCUIApplication()
        app.launchArguments = ["-ui-testing", "-AppleLanguages", "(en)", "-AppleLocale", "en_US"]
        app.launchEnvironment["NOLA_UI_TEST_RUN_ID"] = "prompt-whitespace-\(UUID().uuidString)"
        app.launch()
        defer { app.terminate() }
        XCTAssertTrue(app.buttons["main.tab.settings"].waitForExistence(timeout: 10))
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
    }

    func testSkillAndMCPEditorsSaveAndDiscardThroughActualControls() throws {
        continueAfterFailure = false
        let app = XCUIApplication()
        app.launchArguments = ["-ui-testing", "-AppleLanguages", "(en)", "-AppleLocale", "en_US"]
        app.launchEnvironment["NOLA_UI_TEST_RUN_ID"] = "capability-editors-\(UUID().uuidString)"
        app.launch()
        defer { app.terminate() }
        XCTAssertTrue(app.buttons["main.tab.settings"].waitForExistence(timeout: 10))
        app.buttons["main.tab.settings"].tap()
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
        name.tap(); name.typeText(" unsaved")
        app.buttons["intelligence.capabilities.editor.cancel"].tap()
        XCTAssertTrue(app.buttons["Discard"].waitForExistence(timeout: 3))
        app.buttons["Discard"].tap()
        XCTAssertTrue(editSkill.waitForExistence(timeout: 5)); editSkill.tap()
        XCTAssertEqual(name.value as? String, "Audit Skill")
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
    }

    func testBuiltInExpertRoleAndMentionWithoutInference() throws {
        continueAfterFailure = false
        let app = XCUIApplication()
        app.launchArguments = ["-ui-testing", "-AppleLanguages", "(en)", "-AppleLocale", "en_US"]
        app.launchEnvironment["NOLA_UI_TEST_RUN_ID"] = "expert-role-\(UUID().uuidString)"
        app.launch()
        defer { app.terminate() }
        XCTAssertTrue(app.buttons["main.tab.settings"].waitForExistence(timeout: 10))
        app.buttons["main.tab.settings"].tap()
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
        app.buttons["main.tab.chat"].tap()
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
        XCTAssertTrue(mention.waitForExistence(timeout: 5)); mention.tap()
        let completed = NSPredicate(format: "value == %@", "")
        expectation(for: completed, evaluatedWith: input)
        waitForExpectations(timeout: 5)
        XCTAssertTrue(app.buttons["chat.expert.selected." + expertID].waitForExistence(timeout: 5))
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
              let current = fixture["currentID"], let agentID = fixture["agentID"] else {
            throw XCTSkip("Requires an isolated Core-authored parent/old/current child fixture")
        }
        continueAfterFailure = false
        let app = XCUIApplication()
        app.launchArguments = ["-ui-testing", "-AppleLanguages", "(en)", "-AppleLocale", "en_US"]
        app.launchEnvironment["NOLA_UI_TEST_RUN_ID"] = "child-history-\(UUID().uuidString)"
        app.launchEnvironment["NOLA_REAL_IOS_UI_DATABASE_PATH"] = database
        app.launch()
        defer { app.terminate() }
        let chat = app.buttons["main.tab.chat"]
        XCTAssertTrue(chat.waitForExistence(timeout: 10)); chat.tap()
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
        let selectedExpert = app.buttons["chat.expert.selected." + agentID]
        XCTAssertTrue(selectedExpert.waitForExistence(timeout: 10))
        XCTAssertTrue(selectedExpert.label.contains("History expert"), "The frozen member name must be available without loading the directory")
        XCTAssertGreaterThanOrEqual(selectedExpert.frame.height, 44)
        XCTAssertTrue(app.textViews["chat.composer.input"].exists)
        let continuedShot = XCTAttachment(screenshot: app.screenshot())
        continuedShot.name = "expert-current-context-returned-main-selection"
        continuedShot.lifetime = .keepAlways; add(continuedShot)
        app.buttons["chat.navigation.back"].tap()
        XCTAssertTrue(parentRow.waitForExistence(timeout: 5)); XCTAssertEqual(ordinaryRows.count, 1)
    }

    func testExpertDeclaredReasoningLevelPersistsWithoutInference() throws {
        guard let database = ProcessInfo.processInfo.environment["NOLA_EXPERT_EFFORT_DATABASE"],
              database.contains("ios-effort-fixture-"), FileManager.default.fileExists(atPath: database) else {
            throw XCTSkip("Requires the isolated Core-authored low/high model fixture")
        }
        continueAfterFailure = false
        let app = XCUIApplication()
        app.launchArguments = ["-ui-testing", "-AppleLanguages", "(en)", "-AppleLocale", "en_US"]
        app.launchEnvironment["NOLA_UI_TEST_RUN_ID"] = "expert-effort-\(UUID().uuidString)"
        app.launchEnvironment["NOLA_REAL_IOS_UI_DATABASE_PATH"] = database
        app.launch()
        defer { app.terminate() }
        XCTAssertTrue(app.buttons["main.tab.settings"].waitForExistence(timeout: 10))
        app.buttons["main.tab.settings"].tap()
        app.buttons["settings.category.agents"].tap()
        let model = app.buttons["experts.model"]
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
        let screenshot = XCTAttachment(screenshot: app.screenshot())
        screenshot.name = "expert-declared-effort-inheritance-restored"
        screenshot.lifetime = .keepAlways
        add(screenshot)
    }

    func testManagementPagesAndNavigationInBothThemesWithoutModelAccount() throws {
        continueAfterFailure = false
        let app = XCUIApplication()
        app.launchArguments = ["-ui-testing", "-AppleLanguages", "(en)", "-AppleLocale", "en_US"]
        app.launchEnvironment["NOLA_UI_TEST_RUN_ID"] = "navigation-\(UUID().uuidString)"
        app.launch()
        defer { app.terminate() }
        XCTAssertTrue(app.buttons["main.tab.chat"].waitForExistence(timeout: 10))

        func tap(_ identifier: String) {
            let button = app.buttons[identifier]
            XCTAssertTrue(button.waitForExistence(timeout: 5), identifier)
            for _ in 0..<5 where !button.isHittable { app.swipeUp() }
            for _ in 0..<5 where !button.isHittable { app.swipeDown() }
            XCTAssertTrue(button.isHittable, identifier)
            button.tap()
        }
        func capture(_ name: String) {
            // Accessibility nodes can exist before the native push settles.
            // Keep real animations enabled and capture their completed state.
            Thread.sleep(forTimeInterval: 0.6)
            let attachment = XCTAttachment(data: app.screenshot().pngRepresentation, uniformTypeIdentifier: "public.png")
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
            tap("main.tab.settings")
            tap("settings.category.general")
            tap("settings.appearance")
            let choice = app.buttons[theme]
            XCTAssertTrue(choice.waitForExistence(timeout: 3))
            choice.tap()
            capture("appearance-\(theme)")
            backToSettings()

            tap("settings.category.agents")
            XCTAssertTrue(app.buttons["experts.management.done"].waitForExistence(timeout: 5))
            capture("experts-\(theme)")
            tap("experts.management.done")

            for identifier in ["settings.models", "settings.category.general", "settings.category.agent", "settings.category.privacy"] {
                tap(identifier)
                capture("\(identifier)-\(theme)")
                if identifier == "settings.category.general" {
                    let update = app.buttons["Check for updates"]
                    for _ in 0..<6 where !update.isHittable { app.swipeUp() }
                    XCTAssertTrue(update.isHittable, "Software updates remain reachable in General")
                    capture("general-lower-\(theme)")
                }
                backToSettings()
            }
            for destination in ["knowledge", "memory", "sources"] {
                tap("main.tab.information")
                tap("information.\(destination)")
                capture("library-\(destination)-\(theme)")
                let back = app.navigationBars.buttons.element(boundBy: 0)
                XCTAssertTrue(back.waitForExistence(timeout: 3))
                back.tap()
                XCTAssertTrue(app.buttons["information.knowledge"].waitForExistence(timeout: 3))
            }
            for tab in ["chat", "activity", "information", "settings"] {
                tap("main.tab.\(tab)")
                capture("primary-\(tab)-\(theme)")

            }
        }
    }

    func testPlansRemainReachableFromActivityWithoutASettingsShortcut() {
        continueAfterFailure = false
        let app = XCUIApplication()
        app.launchArguments = ["-ui-testing", "-AppleLanguages", "(en)", "-AppleLocale", "en_US"]
        app.launchEnvironment["NOLA_UI_TEST_RUN_ID"] = "plans-navigation-\(UUID().uuidString)"
        app.launch()
        defer { app.terminate() }
        XCTAssertTrue(app.buttons["main.tab.activity"].waitForExistence(timeout: 10))
        for theme in ["Light", "Dark"] {
            // Exercise the real setting instead of relying on the simulator's
            // appearance or a process argument overridden by saved preferences.
            app.buttons["main.tab.settings"].tap()
            let general = app.buttons["settings.category.general"]
            XCTAssertTrue(general.waitForExistence(timeout: 5)); general.tap()
            let appearance = app.buttons["settings.appearance"]
            XCTAssertTrue(appearance.waitForExistence(timeout: 5)); appearance.tap()
            let choice = app.buttons[theme]
            XCTAssertTrue(choice.waitForExistence(timeout: 5)); choice.tap()
            let settingsBack = app.navigationBars.buttons.element(boundBy: 0)
            XCTAssertTrue(settingsBack.waitForExistence(timeout: 3)); settingsBack.tap()
            XCTAssertTrue(app.buttons["settings.models"].waitForExistence(timeout: 5))

            app.buttons["main.tab.activity"].tap()
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
            let activityScreenshot = XCTAttachment(screenshot: app.screenshot())
            activityScreenshot.name = "activity-plans-entry-and-search-\(theme)"
            activityScreenshot.lifetime = .keepAlways
            add(activityScreenshot)
            plans.tap()
            XCTAssertTrue(app.staticTexts["No automations yet"].waitForExistence(timeout: 5))
            XCTAssertTrue(app.buttons["Create in chat"].exists)
            let back = app.navigationBars.buttons.element(boundBy: 0)
            XCTAssertTrue(back.waitForExistence(timeout: 3)); back.tap()
            XCTAssertTrue(plans.waitForExistence(timeout: 3))
        }
    }

}
