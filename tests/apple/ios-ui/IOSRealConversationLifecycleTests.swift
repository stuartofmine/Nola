import XCTest
import CoreGraphics
import UIKit

/// Opt-in real-provider acceptance. No model replies or execution events are fixtures.
final class IOSRealConversationLifecycleTests: XCTestCase {
    private var app: XCUIApplication!

    private func configuredCredential() throws -> String? {
        if let path = ProcessInfo.processInfo.environment["NOLA_UI_REPRO_CREDENTIAL_FILE"] {
            let url = URL(fileURLWithPath: path)
            defer { try? FileManager.default.removeItem(at: url) }
            return try String(contentsOf: url, encoding: .utf8)
        }
        return ProcessInfo.processInfo.environment["NOLA_UI_REPRO_API_KEY"]
    }

    private func launchAndImport(verifyModelDraftAcrossTabs: Bool = false) throws -> String {
        guard let endpoint = ProcessInfo.processInfo.environment["NOLA_UI_REPRO_ENDPOINT"],
              let importedModel = ProcessInfo.processInfo.environment["NOLA_UI_REPRO_MODEL"] else {
            throw XCTSkip("Requires an explicitly configured real provider relay")
        }
        continueAfterFailure = false
        app = XCUIApplication()
        app.launchArguments = ["-ui-testing", "-AppleLanguages", "(en)", "-AppleLocale", "en_US"]
        app.launchEnvironment["NOLA_UI_TEST_RUN_ID"] = ProcessInfo.processInfo.environment["NOLA_UI_EXISTING_LOCAL_NAMESPACE"] ?? "real-lifecycle-\(UUID().uuidString)"
        if let database = ProcessInfo.processInfo.environment["NOLA_UI_REAL_DATABASE_PATH"] {
            app.launchEnvironment["NOLA_REAL_IOS_UI_DATABASE_PATH"] = database
        }
        app.launch()
        let chatTab = app.buttons["main.tab.chat"]
        XCTAssertTrue(chatTab.waitForExistence(timeout: 10))
        if ProcessInfo.processInfo.environment["NOLA_UI_REUSE_CONFIGURED_MODELS"] == "1" {
            guard ProcessInfo.processInfo.environment["NOLA_UI_EXISTING_LOCAL_NAMESPACE"] != nil else {
                throw NSError(domain: "NolaA2UIAcceptance", code: 4)
            }
            chatTab.tap()
            return importedModel
        }
        app.openModelCenterFromSettings()
        let configuredKey = try configuredCredential()
        let setupStarted = Date()
        app.configureServiceModel(endpoint: endpoint, modelID: importedModel,
            wireProtocol: ProcessInfo.processInfo.environment["NOLA_UI_REPRO_PROTOCOL"], beforeSave: {
                guard verifyModelDraftAcrossTabs else { return }
                self.app.textFields["model.setup.modelID"].typeText("\n")
                self.app.buttons["main.tab.chat"].tap()
                XCTAssertTrue(self.app.buttons["chat.conversations.new"].waitForExistence(timeout: 5))
                self.app.buttons["main.tab.settings"].tap()
                let address = self.app.textFields["model.setup.endpoint"]
                XCTAssertTrue(address.waitForExistence(timeout: 5), "Returning must retain the model configuration navigation stack")
                XCTAssertEqual(address.value as? String, endpoint)
                XCTAssertEqual(self.app.textFields["model.setup.modelID"].value as? String, importedModel)
                self.capture("model-setup-draft-restored-after-tab-switch")
            }) { credential in
            if let key = configuredKey, !key.isEmpty {
                self.pasteUsingSystemMenu(key, into: credential)
            }
        }
        if verifyModelDraftAcrossTabs {
            XCTAssertLessThan(Date().timeIntervalSince(setupStarted), 60,
                "Model setup must not wait for a hidden native tab bar animation to become idle")
        }
        chatTab.tap()
        return importedModel
    }

    func testRealModelSetupThenChat() throws {
        guard ProcessInfo.processInfo.environment["NOLA_UI_REUSE_CONFIGURED_MODELS"] != "1" else {
            XCTFail("Model setup acceptance must enter the actual configuration form")
            return
        }
        let model = try launchAndImport(verifyModelDraftAcrossTabs: true)
        app.buttons["chat.conversations.new"].tap()
        chooseModel(model)
        send("Reply exactly MODEL_SETUP_OK. Do not use tools.")
        assertReply("MODEL_SETUP_OK")
        waitForTerminal()
        capture("model-setup-real-reply")
    }

    func testRealLocalColdStopDuringOldLeaseAndSendAgain() throws {
        let model = try launchAndImport()
        app.buttons["chat.conversations.new"].tap()
        chooseModel(model)
        let marker = "IOS_LOCAL_COLD_" + UUID().uuidString.prefix(8)
        send("Start with \(marker), then write 150 numbered paragraphs about astronomy, at least 60 words each. Do not use tools.")
        assertReply(marker)
        waitForRunning()
        returnToList()
        let row = app.buttons.matching(NSPredicate(format: "identifier BEGINSWITH %@", "chat.conversation.")).firstMatch
        XCTAssertTrue(row.exists)
        let conversationID = row.identifier
        capture("local-cold-before-process-exit")
        app.terminate()
        app.launch()
        let tab = app.buttons["tab.chat"]
        XCTAssertTrue(tab.waitForExistence(timeout: 15)); tab.tap()
        let restored = app.buttons[conversationID]
        XCTAssertTrue(restored.waitForExistence(timeout: 15)); restored.tap()
        waitForRunning()
        capture("local-cold-waiting-old-lease")
        let button = app.buttons["chat.composer.send"]
        button.tap()
        let stopped = XCTNSPredicateExpectation(predicate: NSPredicate(format: "exists == true AND label != %@", "Stop generation"), object: button)
        XCTAssertEqual(XCTWaiter.wait(for: [stopped], timeout: 60), .completed,
            "Cold Stop must not wait for the previous delivery lease to expire")
        capture("local-cold-stop-settled")
        send("Reply exactly IOS_LOCAL_AFTER_COLD_STOP=842.")
        assertReply("IOS_LOCAL_AFTER_COLD_STOP=842")
        waitForTerminal()
        capture("local-cold-stop-next-turn-completed")
    }

    func testRealKeyboardDismissalAndCommittedHistorySurviveTabNavigation() throws {
        let model = try launchAndImport()
        app.buttons["chat.conversations.new"].tap()
        chooseModel(model)
        send("Write eight numbered paragraphs about astronomy, 40 words each. End with VIEWPORT_REPLY_DONE.")
        assertReply("VIEWPORT_REPLY_DONE")
        waitForTerminal()
        let input = app.textViews["chat.composer.input"]
        input.tap(); input.typeText("Unsent draft survives navigation")
        let keyboard = app.keyboards.firstMatch
        XCTAssertTrue(keyboard.waitForExistence(timeout: 5))
        XCTAssertLessThanOrEqual(input.frame.maxY, keyboard.frame.minY + 2)
        XCTAssertTrue(app.buttons["chat.composer.send"].isHittable)
        XCTAssertFalse(app.buttons["chat.composer.dismissKeyboard"].exists)
        capture("real-keyboard-visible-above-dock")
        // Start inside visible transcript, not the AX scroll frame's safe-area
        // or keyboard extension, which can cover the entire screen on iOS 27.
        app.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.3))
            .press(forDuration: 0.1, thenDragTo: app.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.6)))
        let hidden = XCTNSPredicateExpectation(predicate: NSPredicate(format: "exists == false"), object: keyboard)
        XCTAssertEqual(XCTWaiter.wait(for: [hidden], timeout: 5), .completed)
        returnToList()
        let row = app.buttons.matching(NSPredicate(format: "identifier BEGINSWITH %@", "chat.conversation.")).firstMatch
        let owner = row.identifier
        app.buttons["main.tab.activity"].tap()
        app.buttons["main.tab.chat"].tap()
        app.buttons[owner].tap()
        XCTAssertEqual(input.value as? String, "Unsent draft survives navigation")
        assertReply("VIEWPORT_REPLY_DONE")
        capture("real-keyboard-draft-and-committed-history-restored")
    }

    func testDraftKeyboardMenusAndColdRestoreRemainConversationOwned() throws {
        let model = try launchAndImport()
        app.buttons["chat.conversations.new"].tap()
        chooseModel(model)
        let input = app.textViews["chat.composer.input"]
        let draft = "草稿归属测试\nReply exactly DRAFT_RESTORED_OK."
        input.tap(); input.typeText(draft)
        XCTAssertTrue(app.keyboards.firstMatch.waitForExistence(timeout: 5))
        XCTAssertLessThanOrEqual(input.frame.maxY, app.keyboards.firstMatch.frame.minY + 2)
        app.buttons["chat.conversation.details"].tap()
        XCTAssertTrue(app.buttons["chat.work_tools.close"].waitForExistence(timeout: 5))
        app.buttons["chat.work_tools.close"].tap()
        XCTAssertEqual(input.value as? String, draft, "Opening the conversation tools must not discard a draft")
        capture("draft-after-keyboard-and-work-tools-sheet")
        returnToList(usingEdgeGesture: true)
        let original = app.buttons.matching(NSPredicate(format: "identifier BEGINSWITH %@", "chat.conversation.")).firstMatch
        XCTAssertTrue(original.waitForExistence(timeout: 5))
        let originalID = original.identifier
        app.buttons["chat.conversations.new"].tap()
        XCTAssertNotEqual(input.value as? String, draft, "A new conversation must not inherit another draft")
        returnToList()
        app.buttons[originalID].tap()
        XCTAssertEqual(input.value as? String, draft)
        app.terminate(); app.launch()
        XCTAssertTrue(app.buttons[originalID].waitForExistence(timeout: 15))
        app.buttons[originalID].tap()
        XCTAssertEqual(input.value as? String, draft, "Cold launch must restore the exact multiline draft")
        capture("draft-restored-after-cold-launch")
        app.buttons["chat.composer.send"].tap()
        assertReply("DRAFT_RESTORED_OK")
        waitForTerminal()
        capture("draft-restored-real-message-completed")
        let copy = app.buttons.matching(NSPredicate(format: "identifier BEGINSWITH %@", "chat.message.copy.")).firstMatch
        let edit = app.buttons.matching(NSPredicate(format: "identifier BEGINSWITH %@", "chat.message.edit.")).firstMatch
        XCTAssertTrue(copy.waitForExistence(timeout: 5))
        XCTAssertTrue(edit.waitForExistence(timeout: 5))
        // AX coordinate transforms can report 44pt as 43.99999999999997.
        let minimumHit = 44.0 - 0.01
        XCTAssertGreaterThanOrEqual(copy.frame.width, minimumHit)
        XCTAssertGreaterThanOrEqual(copy.frame.height, minimumHit)
        XCTAssertGreaterThanOrEqual(edit.frame.height, minimumHit)
        XCTAssertFalse(copy.frame.intersects(edit.frame), "Message actions must preserve distinct hit regions")
        XCTAssertLessThanOrEqual(edit.frame.maxX, app.windows.firstMatch.frame.maxX)
    }

    func testRealStopCannotAdmitOverlappingFollowup() throws {
        let model = try launchAndImport()
        app.buttons["chat.conversations.new"].tap()
        chooseModel(model)
        let marker = "CANCEL_OWNERSHIP_" + UUID().uuidString
        send("Write 120 numbered paragraphs about astronomy, at least 60 words each. Start with \(marker).")
        waitForForwardedRequest(containing: marker)
        waitForRunning()
        capture("real-running-controls-painted")
        app.buttons["chat.composer.send"].tap()
        waitForTerminal()
        let next = "Reply exactly CANCEL_FOLLOWUP_OK."
        send(next)
        let pending = app.staticTexts.matching(NSPredicate(format: "label CONTAINS %@", "The previous task has not finished")).firstMatch
        if pending.waitForExistence(timeout: 3) {
            XCTAssertEqual(app.textViews["chat.composer.input"].value as? String, next,
                "An unresolved stop must preserve the unsent followup")
            capture("real-cancellation-unconfirmed-blocks-followup")
        } else {
            assertReply("CANCEL_FOLLOWUP_OK")
            waitForTerminal()
            capture("real-cancellation-confirmed-allows-followup")
        }
        XCTAssertFalse(app.staticTexts.matching(NSPredicate(format: "label CONTAINS %@", "cannot overlap an active Run")).firstMatch.exists)
    }

    func testExpertMentionOnUntitledConversationSendsRealReply() throws {
        let environment = ProcessInfo.processInfo.environment
        guard let database = environment["NOLA_UI_REAL_DATABASE_PATH"],
              let model = environment["NOLA_UI_REPRO_MODEL"] else {
            throw XCTSkip("Requires an isolated Store with Arithmetic QA and a real model")
        }
        continueAfterFailure = false
        app = XCUIApplication()
        app.launchArguments = ["-ui-testing", "-AppleLanguages", "(en)", "-AppleLocale", "en_US"]
        app.launchEnvironment["NOLA_REAL_IOS_UI_DATABASE_PATH"] = database
        app.launchEnvironment["NOLA_UI_TEST_RUN_ID"] = "expert-mention-\(UUID().uuidString)"
        app.launch()
        defer { app.terminate() }
        let newConversation = app.buttons["chat.conversations.new"]
        if !newConversation.waitForExistence(timeout: 5) {
            let chat = app.tabBars.buttons["Chat"]
            XCTAssertTrue(chat.waitForExistence(timeout: 10), app.debugDescription)
            chat.tap()
        }
        XCTAssertTrue(newConversation.waitForExistence(timeout: 10)); newConversation.tap()
        chooseModel(model)
        app.buttons["chat.topbar.more"].tap()
        XCTAssertTrue(app.buttons["chat.inspector.resources"].exists)
        XCTAssertTrue(app.buttons["chat.conversation.settings"].exists)
        app.buttons["Experts"].tap()
        app.buttons["Done"].tap()
        XCTAssertFalse(app.buttons["chat.controls.permission"].exists, "Saved permission state must not occupy the composer dock")
        let input = app.textViews["chat.composer.input"]
        XCTAssertTrue(input.waitForExistence(timeout: 10)); input.tap(); input.typeText("@Arith")
        let candidate = app.buttons["chat.mention.arithmetic-qa"]
        XCTAssertTrue(candidate.waitForExistence(timeout: 10), app.debugDescription)
        XCTAssertFalse(app.buttons["chat.mention.language-qa"].exists)
        candidate.tap()
        XCTAssertFalse(candidate.exists)
        capture("expert-mention-selected-before-first-message")
        send("Compute 17 times 19. Reply with EXPERT_RESULT= followed by the result. Do not use tools.")
        let result = app.staticTexts.matching(NSPredicate(format: "label CONTAINS %@", "EXPERT_RESULT=323")).firstMatch
        XCTAssertTrue(result.waitForExistence(timeout: 180), app.debugDescription)
        XCTAssertFalse(app.staticTexts.matching(NSPredicate(format: "label CONTAINS %@", "invalid room identity or title")).firstMatch.exists)
        let question = app.staticTexts.matching(NSPredicate(format: "label BEGINSWITH %@", "Compute 17 times 19.")).firstMatch
        XCTAssertTrue(question.exists, "Admitted parent message must appear immediately, without reopening the conversation")
        XCTAssertLessThan(question.frame.minY, result.frame.minY, "The reply must follow its parent")
        capture("expert-first-message-real-reply")
        send("Write 24 numbered short facts about prime numbers, one per line. Start with TIMELINE_SECOND_BEGIN and finish with TIMELINE_SECOND_END. Do not use tools.")
        let end = app.staticTexts.matching(NSPredicate(format: "label == %@", "TIMELINE_SECOND_END")).firstMatch
        let begin = app.staticTexts.matching(NSPredicate(format: "label BEGINSWITH %@", "TIMELINE_SECOND_BEGIN")).firstMatch
        XCTAssertTrue(begin.waitForExistence(timeout: 180), app.debugDescription)
        XCTAssertFalse(end.exists, "The first public delta must be visible before the final marker arrives")
        capture("expert-long-reply-incremental")
        XCTAssertTrue(end.waitForExistence(timeout: 180), app.debugDescription)
        waitForTerminal()
        capture("expert-long-reply-bottom")
        let timeline = app.scrollViews.matching(identifier: "chat.expert.timeline").firstMatch
        XCTAssertTrue(timeline.exists)
        timeline.swipeDown()
        let latest = app.buttons["chat.scroll.latest"]
        XCTAssertTrue(latest.waitForExistence(timeout: 5), app.debugDescription)
        capture("expert-long-reply-reading-history")
        latest.tap()
        XCTAssertTrue(end.isHittable)
        capture("expert-long-reply-returned-to-bottom")
    }

    func testRealA2UIFormEditSubmitRestoreAndIsolation() throws {
        let model: String
        if let namespace = ProcessInfo.processInfo.environment["NOLA_UI_EXISTING_LOCAL_NAMESPACE"],
           let owner = ProcessInfo.processInfo.environment["NOLA_UI_RESUME_A2UI_CONVERSATION"] {
            model = try XCTUnwrap(ProcessInfo.processInfo.environment["NOLA_UI_REPRO_MODEL"])
            app = XCUIApplication()
            app.launchArguments = ["-ui-testing", "-AppleLanguages", "(en)", "-AppleLocale", "en_US"]
            app.launchEnvironment["NOLA_UI_TEST_RUN_ID"] = namespace
            app.launch()
            XCTAssertTrue(app.tabBars.firstMatch.waitForExistence(timeout: 15))
            app.tabBars.buttons["Chat"].tap()
            let row = app.buttons.matching(NSPredicate(format: "identifier ==[c] %@", "chat.conversation." + owner)).firstMatch
            XCTAssertTrue(row.waitForExistence(timeout: 15))
            row.tap()
            if ProcessInfo.processInfo.environment["NOLA_UI_RESUME_A2UI_WAIT_FOR_RUNNING"] == "1" {
                waitForRunning()
            }
            try waitForTerminalWithUiApprovals()
        } else {
            model = try launchAndImport()
            app.buttons["chat.conversations.new"].tap()
            chooseModel(model)
            send("Use render_ui to publish one A2UI 0.9.1 form, surfaceId acceptance-form. Include Text title A2UI_ACCEPTANCE, TextField id name labeled Acceptance name bound to /name initially empty, Button id submit with variant primary labeled Submit acceptance with submit event and name context bound to /name, and Text id status initially Awaiting input. When a later UI action arrives, read the actual submitted name and update this SAME surface with status text ACK followed by that name, preserving the input and button. Do not invent a submitted value. Publish now.")
        }
        let field = app.textFields["a2ui.field.name"]
        try waitForUI(field)
        waitForTerminal()
        let surface = app.otherElements.matching(NSPredicate(format: "identifier BEGINSWITH %@", "a2ui.surface.")).firstMatch
        XCTAssertTrue(surface.exists)
        let surfaceIdentity = surface.identifier
        guard !surfaceIdentity.isEmpty else {
            XCTFail("Matched Surface must expose its exact identity")
            return
        }
        let exactSurface = app.otherElements.matching(NSPredicate(format: "identifier == %@", surfaceIdentity)).firstMatch
        revealInVisibleTranscript(field)
        capture("a2ui-form-visible-before-edit")
        let token = "ORCHID" + UUID().uuidString.replacingOccurrences(of: "-", with: "").prefix(8)
        field.tap()
        if let current = field.value as? String, !current.isEmpty, current != field.placeholderValue {
            field.typeText(String(repeating: XCUIKeyboardKey.delete.rawValue, count: current.count))
        }
        field.typeText(token)
        // Finish input composition, then wait for Core's asynchronous edit
        // preview to enable the actual generated action before tapping once.
        swipeVisibleTranscript(up: true)
        let submit = app.buttons["a2ui.action.submit"]
        revealInVisibleTranscript(submit)
        let actionReady = XCTNSPredicateExpectation(
            predicate: NSPredicate(format: "exists == true AND enabled == true AND hittable == true"), object: submit)
        XCTAssertEqual(XCTWaiter.wait(for: [actionReady], timeout: 15), .completed)
        capture("a2ui-action-enabled-before-submit")
        submit.tap()
        waitForRunning()
        let acknowledgement = exactSurface.descendants(matching: .staticText).matching(NSPredicate(format: "label == %@", "ACK " + token)).firstMatch
        try waitForTerminalWithUiApprovals()
        revealInVisibleTranscript(acknowledgement)
        XCTAssertTrue(acknowledgement.waitForExistence(timeout: 10))
        XCTAssertEqual(app.textFields.matching(identifier: "a2ui.field.name").count, 1)
        XCTAssertEqual(app.buttons.matching(identifier: "a2ui.action.submit").count, 1)
        XCTAssertTrue(exactSurface.exists, "Update must retain the original Surface generation")
        capture("a2ui-real-edited-submitted-updated")
        returnToList()
        let row = app.buttons.matching(NSPredicate(format: "identifier BEGINSWITH %@", "chat.conversation.")).firstMatch
        let originalID = row.identifier
        app.terminate(); app.launch()
        XCTAssertTrue(app.buttons[originalID].waitForExistence(timeout: 15))
        app.buttons[originalID].tap()
        XCTAssertTrue(acknowledgement.waitForExistence(timeout: 20))
        XCTAssertTrue(exactSurface.exists)
        capture("a2ui-real-cold-restored")
        returnToList()
        app.buttons["chat.conversations.new"].tap()
        XCTAssertFalse(app.textFields["a2ui.field.name"].exists)
        XCTAssertFalse(acknowledgement.exists)
        chooseModel(model)
        send("Have I submitted a form name in THIS conversation? If none, reply exactly ISOLATED_NONE. Do not invent a name.")
        assertReply("ISOLATED_NONE")
        waitForTerminal()
        XCTAssertFalse(assistantText(token).exists)
        capture("a2ui-real-new-conversation-isolated")
    }

    private func waitForTerminalWithUiApprovals(timeout: TimeInterval = 240) throws {
        let deadline = Date().addingTimeInterval(timeout)
        var lastReveal = Date.distantPast
        let send = app.buttons["chat.composer.send"]
        XCTAssertTrue(send.waitForExistence(timeout: 15))
        while Date() < deadline {
            if send.label != "Stop generation" { return }
            let approval = app.buttons["chat.permission.allow_once"].firstMatch
            if approval.exists {
                guard app.descendants(matching: .any)["chat.permission.tool.render_ui"].firstMatch.exists else {
                    XCTFail("Only render_ui is authorized in retained form recovery")
                    throw NSError(domain: "NolaA2UIAcceptance", code: 5)
                }
                revealInVisibleTranscript(approval)
                capture("a2ui-retained-render-ui-permission")
                approval.tap()
            } else {
                let latest = app.buttons["chat.scroll.latest"]
                if latest.exists && latest.isHittable { latest.tap() }
                else if Date().timeIntervalSince(lastReveal) >= 8 {
                    // Lazy native rows may omit an offscreen permission from
                    // AX until a real scroll reveals the bottom of the row.
                    swipeVisibleTranscript(up: true)
                    lastReveal = Date()
                }
            }
            Thread.sleep(forTimeInterval: 0.5)
        }
        XCTFail("Retained A2UI Run did not reach a terminal state")
        throw NSError(domain: "NolaA2UIAcceptance", code: 6)
    }

    private func visibleTranscriptBounds() -> (top: CGFloat, bottom: CGFloat) {
        let top = app.frame.minY + 104
        let composerTop = app.textViews["chat.composer.input"].frame.minY
        return (top, max(top + 80, min(composerTop - 12, app.frame.maxY - 160)))
    }

    private func swipeVisibleTranscript(up: Bool) {
        // On iOS 27 the AX scroll frame can extend underneath the keyboard.
        // Ground gestures in the visible region above the actual composer.
        let bounds = visibleTranscriptBounds()
        let height = bounds.bottom - bounds.top
        let startY = bounds.top + height * (up ? 0.78 : 0.22)
        let endY = bounds.top + height * (up ? 0.22 : 0.78)
        let origin = app.coordinate(withNormalizedOffset: .zero)
        // Keep the gesture outside TextField contents. Dragging from their
        // center selects text instead of moving the transcript on iOS 27.
        let gutterX = app.frame.minX + 8
        origin.withOffset(CGVector(dx: gutterX, dy: startY))
            .press(forDuration: 0.1, thenDragTo: origin.withOffset(CGVector(dx: gutterX, dy: endY)))
    }

    private func revealInVisibleTranscript(_ element: XCUIElement) {
        for _ in 0..<20 {
            if !element.exists {
                swipeVisibleTranscript(up: false)
                continue
            }
            let bounds = visibleTranscriptBounds()
            let frame = element.frame
            if element.isHittable && frame.minY >= bounds.top && frame.maxY <= bounds.bottom { return }
            swipeVisibleTranscript(up: frame.maxY > bounds.bottom || frame.minY >= bounds.top)
        }
        XCTAssertTrue(element.isHittable, "Generated control must be reachable inside the visible transcript")
    }

    private func waitForUI(_ element: XCUIElement, timeout: TimeInterval = 240) throws {
        let deadline = Date().addingTimeInterval(timeout)
        while Date() < deadline {
            if element.exists { return }
            if app.buttons["chat.error.details"].exists {
                capture("a2ui-real-execution-error")
                XCTFail("Generated UI execution failed: \(app.debugDescription)")
                throw NSError(domain: "NolaA2UIAcceptance", code: 1)
            }
            let approval = app.buttons["chat.permission.allow_once"].firstMatch
            let uiPermission = app.descendants(matching: .any)["chat.permission.tool.render_ui"].firstMatch
            if approval.exists {
                guard uiPermission.exists else {
                    capture("a2ui-unexpected-permission")
                    XCTFail("Unexpected permission request; only render_ui is authorized")
                    throw NSError(domain: "NolaA2UIAcceptance", code: 3)
                }
                if !approval.isHittable { revealInVisibleTranscript(approval) }
                if approval.isHittable { approval.tap() }
            } else {
                let latest = app.buttons["chat.scroll.latest"]
                if latest.exists && latest.isHittable { latest.tap() }
            }
            Thread.sleep(forTimeInterval: 0.5)
        }
        XCTFail("Expected real generated UI did not appear: \(app.debugDescription)")
        throw NSError(domain: "NolaA2UIAcceptance", code: 2)
    }

    private func openRetainedUnknownPermissionConversation() throws -> String {
        _ = try launchAndImport()
        let owner = try XCTUnwrap(ProcessInfo.processInfo.environment["NOLA_UI_RESUME_LONG_CONVERSATION"])
        let rowID = "chat.conversation." + owner
        let retained = app.buttons.matching(NSPredicate(format: "identifier ==[c] %@", rowID)).firstMatch
        XCTAssertTrue(retained.waitForExistence(timeout: 15))
        let exactID = retained.identifier
        retained.tap()
        return exactID
    }

    private func keepUnknownBlockedUsingNativeDecision() {
        let card = app.descendants(matching: .any)["chat.recovery.unknown_effect"].firstMatch
        XCTAssertTrue(card.waitForExistence(timeout: 45), app.debugDescription)
        let keep = app.buttons["chat.recovery.keep_unknown_blocked"]
        let ready = XCTNSPredicateExpectation(predicate: NSPredicate(format: "exists == true AND enabled == true AND hittable == true"), object: keep)
        XCTAssertEqual(XCTWaiter.wait(for: [ready], timeout: 15), .completed)
        capture("unknown-permission-exact-recovery-decision")
        keep.tap()
        let dialog = app.sheets["Review Recovery Decision"]
        XCTAssertTrue(dialog.waitForExistence(timeout: 10))
        let confirm = dialog.buttons.matching(identifier: "chat.recovery.confirm_keep_unknown_blocked").firstMatch
        XCTAssertTrue(confirm.waitForExistence(timeout: 10))
        capture("unknown-permission-preserve-and-stop-confirmation")
        confirm.tap()
        let blocked = app.staticTexts["The task is closed as blocked. The previous operation’s outcome remains unknown."]
        XCTAssertTrue(blocked.waitForExistence(timeout: 30), app.debugDescription)
        capture("unknown-permission-closed-as-blocked")
    }

    func testRetainedUnknownPermissionCanStopWithoutRetryAndColdRestore() throws {
        try assertUnknownPermissionStopAndColdRestore()
    }

    private func assertUnknownPermissionStopAndColdRestore() throws {
        let originalID = try openRetainedUnknownPermissionConversation()
        keepUnknownBlockedUsingNativeDecision()
        returnToList()
        app.terminate(); app.launch()
        XCTAssertTrue(app.tabBars.firstMatch.waitForExistence(timeout: 15))
        app.tabBars.buttons["Chat"].tap()
        XCTAssertTrue(app.buttons[originalID].waitForExistence(timeout: 15))
        app.buttons[originalID].tap()
        XCTAssertNotEqual(app.buttons["chat.composer.send"].label, "Stop generation")
        XCTAssertFalse(app.buttons["chat.recovery.confirm_keep_unknown_blocked"].exists)
        let committed = app.staticTexts["A2UI_STEP_2"]
        revealInVisibleTranscript(committed)
        XCTAssertTrue(committed.exists, "Safe blocked-stop must retain the previously committed Surface")
        capture("unknown-permission-blocked-cold-restored")
    }

    func testRealFollowUpAfterRetainedUnknownWasStopped() throws {
        _ = try openRetainedUnknownPermissionConversation()
        XCTAssertFalse(app.buttons["chat.recovery.keep_unknown_blocked"].exists)
        send("Reply exactly IOS_AFTER_SAFE_STOP=842. Do not use tools or retry earlier work.")
        assertReply("IOS_AFTER_SAFE_STOP=842")
        waitForTerminal()
        capture("unknown-permission-new-message-completed-after-safe-stop")
    }

    func testRealPermissionWaitColdRecoveryHasSafeStopAndCanSendAgain() throws {
        let model = try launchAndImport()
        app.buttons["chat.conversations.new"].tap()
        chooseModel(model)
        send("Use render_ui to publish one tiny native astronomy card, surfaceId cold-permission. Include Text title COLD_PERMISSION_CARD and one useful sentence about telescopes. Use A2UI v0.9.1 and the tool’s catalog schema exactly. No other tools.")
        let approval = app.buttons["chat.permission.allow_once"].firstMatch
        XCTAssertTrue(approval.waitForExistence(timeout: 180))
        XCTAssertTrue(app.descendants(matching: .any)["chat.permission.tool.render_ui"].firstMatch.exists)
        capture("cold-permission-live-question-not-approved")
        returnToList()
        let row = app.buttons.matching(NSPredicate(format: "identifier BEGINSWITH %@", "chat.conversation.")).firstMatch
        let originalID = row.identifier
        app.terminate(); app.launch()
        XCTAssertTrue(app.tabBars.firstMatch.waitForExistence(timeout: 15))
        app.tabBars.buttons["Chat"].tap()
        XCTAssertTrue(app.buttons[originalID].waitForExistence(timeout: 15))
        app.buttons[originalID].tap()
        let recovery = app.descendants(matching: .any)["chat.recovery.unknown_effect"].firstMatch
        let sendButton = app.buttons["chat.composer.send"]
        let deadline = Date().addingTimeInterval(60)
        while Date() < deadline && !recovery.exists && sendButton.label != "Stop generation" {
            Thread.sleep(forTimeInterval: 0.5)
        }
        XCTAssertFalse(recovery.exists, "An unanswered Core question must not become an unknown side effect")
        XCTAssertEqual(sendButton.label, "Stop generation",
            "A recovered permission question must remain stoppable")
        capture("cold-permission-unstarted-question-is-stoppable")
        sendButton.tap()
        let settled = Date().addingTimeInterval(60)
        while Date() < settled && sendButton.label == "Stop generation" && !recovery.exists {
            Thread.sleep(forTimeInterval: 0.5)
        }
        XCTAssertFalse(recovery.exists)
        XCTAssertNotEqual(sendButton.label, "Stop generation")
        capture("cold-permission-stopped-through-native-composer")
        send("Reply exactly IOS_AFTER_COLD_PERMISSION_STOP=842. Do not use tools or retry earlier work.")
        assertReply("IOS_AFTER_COLD_PERMISSION_STOP=842")
        waitForTerminal()
        capture("cold-permission-new-message-completed")
    }

    func testRealA2UILongUpdatesSurviveNavigationAndCancellation() throws {
        let model = try launchAndImport()
        if let owner = ProcessInfo.processInfo.environment["NOLA_UI_RESUME_LONG_CONVERSATION"] {
            let retained = app.buttons.matching(NSPredicate(format: "identifier ==[c] %@", "chat.conversation." + owner)).firstMatch
            XCTAssertTrue(retained.waitForExistence(timeout: 15))
            retained.tap()
            capture("a2ui-long-retained-after-upstream-error")
            if ProcessInfo.processInfo.environment["NOLA_UI_RESUME_LONG_WAIT_FOR_RUNNING"] != "1" {
            send("Continue the existing long-acceptance astronomy lesson after the upstream connection failure. The initial publication and A2UI_STEP_1 orbital motion update already succeeded. Preserve the same Surface and perform only the FOUR remaining separate render_ui updates: stellar evolution, telescope optics, astronomical distances, observation uncertainty, 120-160 useful words each in body. Set status A2UI_STEP_2, A2UI_STEP_3, A2UI_STEP_4, A2UI_STEP_5 respectively. Every message must use version v0.9.1. Use the latest successful publication current_revision for expected_revision; increment only from each successful receipt. Do not recreate the Surface or repeat step 1. End normally after step 5.")
            }

        } else {
            app.buttons["chat.conversations.new"].tap()
            chooseModel(model)
        send("Use only render_ui to build a real native astronomy lesson surfaceId long-acceptance. Follow the tool schema exactly: every messages entry MUST contain version=v0.9.1 and exactly one payload; createSurface catalogId MUST be https://nola.app/a2ui/v0.9.1/catalog.json. Initial call uses expected_revision=0 and creates a root Column with Text title A2UI_LONG, Text body, and Text status A2UI_STARTED. Then make FIVE separate sequential successful render_ui updates to this SAME surface: body explains orbital motion, stellar evolution, telescope optics, astronomical distances, then observation uncertainty (120-160 distinct useful words per topic); status is A2UI_STEP_1 through A2UI_STEP_5. Each update MUST use expected_revision equal to the current_revision returned by the previous successful publication; never use 0 for an existing surface, and never recreate it. Preserve title and node IDs. After step 5 end normally.")
        }
        try waitForUI(app.staticTexts["A2UI_LONG"])
        waitForRunning()
        returnToList(usingEdgeGesture: true)
        let original = app.buttons.matching(NSPredicate(format: "identifier BEGINSWITH %@", "chat.conversation.")).firstMatch
        XCTAssertTrue(original.waitForExistence(timeout: 10))
        let originalID = original.identifier
        original.tap()
        try waitForTerminalWithUiApprovals(timeout: 720)
        revealInVisibleTranscript(app.staticTexts["A2UI_STEP_5"])
        XCTAssertTrue(app.staticTexts["A2UI_STEP_5"].exists)
        capture("a2ui-long-updates-completed-after-navigation")
        returnToList()
        app.buttons[originalID].tap()
        XCTAssertTrue(app.staticTexts["A2UI_STEP_5"].waitForExistence(timeout: 15))
        send("Review the existing long-acceptance astronomy lesson in FOUR separate render_ui calls on the same surface: first identify common misconceptions about orbital motion, second compare evidence from telescope observations, third explain measurement uncertainty, fourth give practical beginner observation criteria. Each call should write 100-150 words of distinct useful analysis into body and status A2UI_CANCEL_STEP followed by the step number. Preserve the identity and use its actual revision. Do not use any other tools.")
        // Cancel a genuine active follow-up before approving any new tool.
        // A model may be streaming or awaiting render_ui authorization; both
        // must keep the last committed Surface available after Stop.
        waitForRunning()
        capture("a2ui-long-active-followup-before-cancel")
        returnToList(usingEdgeGesture: true)
        XCTAssertTrue(app.buttons[originalID].waitForExistence(timeout: 10))
        app.buttons[originalID].tap()
        XCTAssertEqual(app.buttons["chat.composer.send"].label, "Stop generation")
        app.buttons["chat.composer.send"].tap()
        waitForTerminal()
        XCTAssertFalse(app.buttons["chat.error.details"].exists)
        revealInVisibleTranscript(app.staticTexts["A2UI_STEP_5"])
        XCTAssertTrue(app.staticTexts["A2UI_STEP_5"].exists, "Cancellation preserves the last committed Surface")
        capture("a2ui-long-explicit-cancellation")
        returnToList()
        app.terminate(); app.launch()
        XCTAssertTrue(app.tabBars.firstMatch.waitForExistence(timeout: 15))
        app.tabBars.buttons["Chat"].tap()
        XCTAssertTrue(app.buttons[originalID].waitForExistence(timeout: 15))
        app.buttons[originalID].tap()
        revealInVisibleTranscript(app.staticTexts["A2UI_STEP_5"])
        XCTAssertTrue(app.staticTexts["A2UI_STEP_5"].exists)
        XCTAssertNotEqual(app.buttons["chat.composer.send"].label, "Stop generation")
        capture("a2ui-long-cancelled-cold-restored")
        send("Continue the same long-acceptance astronomy lesson after cancellation. Make exactly ONE render_ui update on its current revision, preserving title and identity. Write a concise useful comparison of optical and radio observations in body, and set status to A2UI_RESUMED. Follow v0.9.1 exactly. Do not use other tools or repeat the cancelled four-step plan.")
        waitForRunning()
        try waitForTerminalWithUiApprovals()
        revealInVisibleTranscript(app.staticTexts["A2UI_RESUMED"])
        XCTAssertTrue(app.staticTexts["A2UI_RESUMED"].exists)
        capture("a2ui-long-next-run-completed-after-cancel")
    }

    func testRealProviderConversationIsolationAndLongRunNavigation() throws {
        let model = try launchAndImport()
        app.buttons["chat.conversations.new"].tap()
        chooseModel(model)
        send("Remember the private test token ORCHID731. Reply exactly FIRST_ACK ORCHID731.")
        assertReply("FIRST_ACK ORCHID731")
        waitForTerminal()
        send("What token did I ask you to remember? Reply SECOND_ACK followed by that token.")
        assertReply("SECOND_ACK ORCHID731")
        waitForTerminal()
        capture("real-multiturn-confirmed")

        let naturalMarker = "NATURAL_TASK_ACK_" + UUID().uuidString
        send("Write 12 numbered paragraphs explaining astronomy for beginners, at least 40 words each. Start with \(naturalMarker). End with NATURAL_TASK_DONE. Complete this answer without asking questions.")
        waitForForwardedRequest(containing: naturalMarker)
        waitForRunning()
        returnToList(usingEdgeGesture: true)
        let naturalOwner = app.buttons.matching(NSPredicate(format: "identifier BEGINSWITH %@", "chat.conversation.")).firstMatch
        XCTAssertTrue(naturalOwner.waitForExistence(timeout: 10))
        XCTAssertTrue(naturalOwner.label.contains("Running"), "A naturally completing task keeps its owner while the page is absent")
        capture("real-natural-task-in-list")
        naturalOwner.tap()
        waitForTerminal()
        assertReply("NATURAL_TASK_DONE")
        capture("real-natural-task-completed")

        let longMarker = "LONG_RUNNING_ACK_" + UUID().uuidString
        send("Write 120 numbered paragraphs explaining astronomy for beginners. Each paragraph should have at least 60 words. Start with \(longMarker). Continue until all 120 are complete.")
        waitForForwardedRequest(containing: longMarker)
        waitForRunning()
        XCTAssertEqual(app.buttons["chat.composer.send"].label, "Stop generation")
        returnToList(usingEdgeGesture: true)
        let original = app.buttons.matching(NSPredicate(format: "identifier BEGINSWITH %@", "chat.conversation.")).firstMatch
        XCTAssertTrue(original.waitForExistence(timeout: 5))
        let originalID = original.identifier
        XCTAssertTrue(original.label.contains("Running"), "The conversation list must expose the active run status")
        XCTAssertFalse(original.label.contains("No messages yet"), "An admitted running task must not be labeled as an empty conversation")
        capture("real-long-run-in-list")
        original.tap()
        XCTAssertTrue(app.textViews["chat.composer.input"].waitForExistence(timeout: 10))
        XCTAssertEqual(app.buttons["chat.composer.send"].label, "Stop generation", "Leaving the conversation must not cancel its execution")
        capture("real-long-run-restored")
        app.buttons["chat.composer.send"].tap()
        waitForTerminal()
        XCTAssertFalse(app.staticTexts["Model request failed"].exists, "Explicit user cancellation must not be presented as a provider failure")
        capture("real-long-run-cancelled")

        send("Reply exactly POST_CANCEL_ACK ORCHID731.")
        assertReply("POST_CANCEL_ACK ORCHID731")
        waitForTerminal()
        capture("real-send-after-cancellation")

        returnToList()
        app.buttons["chat.conversations.new"].tap()
        chooseModel(model)
        send("Have I given you a private test token in THIS conversation? If none, reply exactly ISOLATED_NONE. Do not invent a token.")
        assertReply("ISOLATED_NONE")
        waitForTerminal()
        XCTAssertFalse(assistantText("ORCHID731").exists, "New conversation must not display or inherit the first conversation's private token")
        capture("real-new-conversation-isolated")
        returnToList()
        app.buttons[originalID].tap()
        let earlier = assistantText("SECOND_ACK ORCHID731")
        for _ in 0..<30 where !earlier.exists { app.scrollViews["chat.timeline.viewport"].swipeDown() }
        XCTAssertTrue(earlier.waitForExistence(timeout: 10), "Original committed history must survive switching conversations")
        capture("real-original-conversation-restored")
    }

    private func pairIsolatedHostAndStartConversation() throws {
        guard let endpoint = ProcessInfo.processInfo.environment["NOLA_UI_HOST_ENDPOINT"],
              let token = ProcessInfo.processInfo.environment["NOLA_UI_HOST_TOKEN"] else {
            throw XCTSkip("Requires an explicitly configured isolated real Host")
        }
        continueAfterFailure = false
        app = XCUIApplication()
        app.launchArguments = ["-ui-testing", "-AppleLanguages", "(en)", "-AppleLocale", "en_US"]
        app.launchEnvironment["NOLA_UI_TEST_RUN_ID"] = "real-host-lifecycle-\(UUID().uuidString)"
        if let database = ProcessInfo.processInfo.environment["NOLA_UI_HOST_DATABASE_PATH"] {
            app.launchEnvironment["NOLA_REAL_IOS_UI_DATABASE_PATH"] = database
        }
        app.launch()
        let tabs = app.tabBars.firstMatch
        XCTAssertTrue(tabs.waitForExistence(timeout: 10))
        app.openModelCenterFromSettings()
        app.buttons["model.setup.source.2"].tap()
        let url = app.textFields["model.web_host.url_field"]
        XCTAssertTrue(url.waitForExistence(timeout: 5))
        url.tap()
        url.typeText(String(repeating: XCUIKeyboardKey.delete.rawValue, count: (url.value as? String)?.count ?? 0) + endpoint)
        XCTAssertEqual(url.value as? String, endpoint)
        let credential = app.secureTextFields["model.web_host.token_field"]
        pasteUsingSystemMenu(token, into: credential)
        app.buttons["model.web_host.connect"].tap()
        let paired = XCTNSPredicateExpectation(predicate: NSPredicate(format: "exists == false"), object: app.buttons["model.web_host.connect"])
        XCTAssertEqual(XCTWaiter.wait(for: [paired], timeout: 30), .completed, "Pairing must dismiss its form")
        capture("host-paired-return")
        let pairingTree = XCTAttachment(string: app.debugDescription)
        pairingTree.name = "host-paired-tree"; pairingTree.lifetime = .keepAlways; add(pairingTree)
        XCTAssertTrue(app.staticTexts["Host paired and saved. Choose it when you want to run this conversation on your Mac."].waitForExistence(timeout: 8))
        // Pairing codes must not trigger the system website-password flow.
        // Dismissing it in the test would hide a product navigation obstruction.
        let possibleSystemPrompt = app.buttons["以后"].exists || app.buttons["Save Password"].exists
        if possibleSystemPrompt {
            let screenshot = XCTAttachment(screenshot: XCUIScreen.main.screenshot())
            screenshot.name = "pairing-prompt-at-assertion"; screenshot.lifetime = .keepAlways; add(screenshot)
            let hierarchy = XCTAttachment(string: app.debugDescription)
            hierarchy.name = "pairing-prompt-at-assertion-tree"; hierarchy.lifetime = .keepAlways; add(hierarchy)
            let system = XCTAttachment(string: XCUIApplication(bundleIdentifier: "com.apple.springboard").debugDescription)
            system.name = "pairing-prompt-system-tree"; system.lifetime = .keepAlways; add(system)
        }
        XCTAssertFalse(possibleSystemPrompt,
            "Host pairing must not ask the OS to save a website password")
        // Pairing must leave the live navigation usable, without a relaunch
        // hiding a stuck keyboard or system credential presentation.
        let pairedChatTab = app.buttons["tab.chat"]
        let pairedNavigation = XCTNSPredicateExpectation(predicate: NSPredicate(format: "isHittable == true"), object: pairedChatTab)
        XCTAssertEqual(XCTWaiter.wait(for: [pairedNavigation], timeout: 10), .completed)
        pairedChatTab.tap()
        XCTAssertTrue(app.buttons["chat.conversations.new"].waitForExistence(timeout: 10))
        capture("host-paired-live-navigation")
        // Also verify pairing survives a cold process boundary.
        app.terminate()
        app.launch()
        capture("host-pairing-cold-restored")
        let chatTab = app.buttons["tab.chat"]
        let navigationReady = XCTNSPredicateExpectation(predicate: NSPredicate(format: "isHittable == true"), object: chatTab)
        XCTAssertEqual(XCTWaiter.wait(for: [navigationReady], timeout: 10), .completed)
        chatTab.tap()
        XCTAssertTrue(app.buttons["chat.conversations.new"].waitForExistence(timeout: 10))
        app.buttons["chat.conversations.new"].tap()
        chooseModel("Host")
    }

    func testRealHostProgressReadsSourcePlanAndPreservesRefresh() throws {
        try pairIsolatedHostAndStartConversation()
        send("Use the available planning tool to persist exactly two pending steps in this conversation: IOS_SOURCE_PLAN_ALPHA inspect source evidence; IOS_SOURCE_PLAN_BETA review the result. Do not execute the steps yet. After the plan tool succeeds, reply exactly IOS_SOURCE_PLAN_READY. Do not claim persistence without a successful tool result.")
        assertReply("IOS_SOURCE_PLAN_READY")
        waitForTerminal()
        app.buttons["chat.topbar.more"].tap()
        app.buttons["chat.inspector.progress"].tap()
        let sourceStep = app.staticTexts["Inspect source evidence"]
        XCTAssertTrue(sourceStep.waitForExistence(timeout: 30), "Progress must read the paired source plan")
        XCTAssertTrue(sourceStep.isHittable)
        XCTAssertTrue(app.staticTexts["Review the result"].isHittable)
        XCTAssertFalse(app.staticTexts["Progress could not be loaded."].exists)
        capture("paired-source-progress")
        // Exercise the existing periodic refresh while keeping committed facts visible.
        for _ in 0..<8 {
            XCTAssertTrue(sourceStep.isHittable, "Refreshing the same owner must preserve visible source facts")
            XCTAssertFalse(app.activityIndicators.firstMatch.exists)
            Thread.sleep(forTimeInterval: 0.5)
        }
        capture("paired-source-progress-after-refresh")
    }



    func testRealHostDelegatesTwoAgentsAndRestoresResults() throws {
        try pairIsolatedHostAndStartConversation()
        let delegationMarker = "IOS_DELEGATION_" + UUID().uuidString.replacingOccurrences(of: "-", with: "")
        send("Please actually delegate this task to exactly two child agents using the available agent tools. Each child must first use an available shell tool to run sleep 20, so we can verify navigation while work is still running. Agent one: then compute 17 times 19, return CHILD_ONE=323. Agent two: then compute 23 times 29, return CHILD_TWO=667. Run the children concurrently, wait for both real child results, then give the final reply beginning \(delegationMarker) with both results. Do not claim delegation unless the child executions really happened.")
        waitForRunning()
        returnToList(usingEdgeGesture: true)
        let runningRow = app.buttons.matching(NSPredicate(format: "identifier BEGINSWITH %@", "chat.conversation.")).firstMatch
        XCTAssertTrue(runningRow.waitForExistence(timeout: 5))
        XCTAssertTrue(runningRow.label.contains("Running"))
        let runningConversationID = runningRow.identifier
        capture("real-host-long-task-list")
        runningRow.tap()
        XCTAssertTrue(app.textViews["chat.composer.input"].waitForExistence(timeout: 10))
        let delegatedReply = assistantText(delegationMarker)
        let delegationDeadline = Date().addingTimeInterval(300)
        while !delegatedReply.exists && Date() < delegationDeadline {
            let approval = app.buttons["chat.permission.allow_once"].firstMatch
            if approval.exists && approval.isEnabled {
                if !approval.isHittable { app.swipeUp() }
                if approval.isHittable {
                    capture("real-host-delegated-permission")
                    if approval.exists && approval.isEnabled && approval.isHittable { approval.tap() }
                }
            }
            Thread.sleep(forTimeInterval: 0.5)
        }
        XCTAssertTrue(delegatedReply.exists, "Delegation must finish after real permission decisions")
        waitForTerminal()
        assertReply("CHILD_ONE=323")
        assertReply("CHILD_TWO=667")
        let process = app.buttons["chat.execution.process.toggle"].firstMatch
        if process.exists { process.tap() }
        let tasks = app.descendants(matching: .any).matching(NSPredicate(format: "identifier BEGINSWITH %@", "chat.task."))
        let taskIDs = Set(tasks.allElementsBoundByIndex.map(\.identifier))
        XCTAssertGreaterThanOrEqual(taskIDs.count, 2, "A model sentence is insufficient: two distinct delegated task projections must be visible")
        capture("real-host-two-agents-completed")
        returnToList()
        let row = app.buttons[runningConversationID]
        XCTAssertTrue(row.waitForExistence(timeout: 5)); row.tap()
        assertReply(delegationMarker)
        capture("real-host-agent-results-restored")

        // Exceed the Core's recent-message tail using actual committed turns.
        // The maintenance request must execute at the paired Host and return
        // its real completion evidence before Application publishes an archive.
        for index in 1...4 {
            let marker = "HOST_MAINTENANCE_TURN_\(index)"
            send("Reply exactly \(marker). Keep the earlier child results in this conversation.")
            assertReply(marker)
            waitForTerminal()
        }
        if !app.buttons["chat.conversation.settings"].exists { app.buttons["chat.topbar.more"].tap() }
        app.buttons["chat.conversation.settings"].tap()
        let context = app.buttons["chat.more_settings.context"]
        XCTAssertTrue(context.waitForExistence(timeout: 10)); context.tap()
        let compact = app.buttons["Run compact"]
        for _ in 0..<12 where !compact.isHittable { app.swipeUp() }
        XCTAssertTrue(compact.isHittable, app.debugDescription)
        compact.tap()
        let settled = XCTNSPredicateExpectation(predicate: NSPredicate(format: "enabled == true"), object: compact)
        XCTAssertEqual(XCTWaiter.wait(for: [settled], timeout: 200), .completed)
        let completed = app.staticTexts.matching(NSPredicate(format: "label BEGINSWITH %@", "Host context compacted")).firstMatch
        // Source receipt is displayed beside the action; no local archive is fabricated.
        for _ in 0..<4 where !completed.exists { app.swipeUp() }
        XCTAssertTrue(completed.waitForExistence(timeout: 10), app.debugDescription)
        capture("real-host-manual-compaction-completed")
    }

    private func reopenExistingHostConversation(openConversation: Bool = true) throws {
        guard let database = ProcessInfo.processInfo.environment["NOLA_UI_HOST_DATABASE_PATH"],
              let conversation = ProcessInfo.processInfo.environment["NOLA_UI_EXISTING_HOST_CONVERSATION_ID"] else {
            throw XCTSkip("Requires the isolated database and conversation from a real Host run")
        }
        continueAfterFailure = false
        app = XCUIApplication()
        app.launchArguments = ["-ui-testing", "-AppleLanguages", "(en)", "-AppleLocale", "en_US"]
        // Installing a new test build relocates the simulator app container.
        // Reuse the original relative test namespace, not its obsolete absolute path.
        let directory = URL(fileURLWithPath: database).deletingLastPathComponent().lastPathComponent
        let prefix = "nola-ios-xctest-host-"
        XCTAssertTrue(directory.hasPrefix(prefix))
        app.launchEnvironment["NOLA_UI_TEST_RUN_ID"] = String(directory.dropFirst(prefix.count))
        app.launch()
        let row = app.buttons["chat.conversation.\(conversation.lowercased())"]
        if !row.waitForExistence(timeout: 5) {
            let tab = app.tabBars.buttons["Chat"]
            XCTAssertTrue(tab.waitForExistence(timeout: 15), app.debugDescription)
            tab.tap()
        }
        XCTAssertTrue(row.waitForExistence(timeout: 15), app.debugDescription)
        if openConversation {
            row.tap()
            XCTAssertTrue(app.textViews["chat.composer.input"].waitForExistence(timeout: 15))
        }
    }





    func testExistingHostStreamsThenStopsLongReply() throws {
        try reopenExistingHostConversation()
        let marker = "IOS_REMOTE_STOP_" + UUID().uuidString.prefix(8)
        send("Start your answer with \(marker). Then write 150 numbered paragraphs, at least 60 words each, explaining astronomy. Do not use tools.")
        assertReply(marker)
        let button = app.buttons["chat.composer.send"]
        XCTAssertEqual(button.label, "Stop generation", "A completed reply cannot prove cancellation")
        capture("existing-host-streaming-before-stop")
        button.tap()
        let stopped = XCTNSPredicateExpectation(predicate: NSPredicate(format: "exists == true AND label != %@", "Stop generation"), object: button)
        XCTAssertEqual(XCTWaiter.wait(for: [stopped], timeout: 60), .completed)
        capture("existing-host-streaming-after-stop")
    }



    private func stopExistingPendingGeneration() throws {
        try reopenExistingHostConversation()
        waitForRunning()
        capture("existing-host-pending-before-stop")
        app.buttons["chat.composer.send"].tap()
        let button = app.buttons["chat.composer.send"]
        let stopped = XCTNSPredicateExpectation(predicate: NSPredicate(format: "exists == true AND label != %@", "Stop generation"), object: button)
        XCTAssertEqual(XCTWaiter.wait(for: [stopped], timeout: 60), .completed)
        capture("existing-host-after-stop")
        // Read-only companion Store audit must confirm the exact original Run
        // has a committed cancelled terminal seal; button state alone is insufficient.
    }

    func testExistingLocalColdRunStopsAndSendsAgain() throws {
        try stopExistingPendingGeneration()
        send("Reply exactly IOS_LOCAL_AFTER_COLD_STOP=842.")
        assertReply("IOS_LOCAL_AFTER_COLD_STOP=842")
        waitForTerminal()
        capture("existing-local-cold-stop-next-turn-completed")
    }

    func testExistingLocalConfirmedColdCancellationAllowsNextMessage() throws {
        try reopenExistingHostConversation()
        waitForTerminal()
        send("Reply exactly IOS_LOCAL_AFTER_COLD_STOP=842.")
        assertReply("IOS_LOCAL_AFTER_COLD_STOP=842")
        waitForTerminal()
        capture("local-confirmed-cold-cancel-next-message")
    }

    func testRealEmbeddedChildPermissionSurvivesLeaseRenewal() throws {
        let model = try launchAndImport()
        app.buttons["chat.conversations.new"].tap()
        chooseModel(model)
        send("Actually use spawn_agent to create exactly one child with permission_mode=ask. The child must use render_ui to publish a small Text card containing IOS_EMBEDDED_CHILD_APPROVED=391, then return that exact marker. Wait for the real child result. Do not claim delegation or tool execution if it did not occur.")
        verifyEmbeddedChildApproval()
    }

    func testExistingEmbeddedChildPermissionColdRecovery() throws {
        guard let database = ProcessInfo.processInfo.environment["NOLA_UI_REAL_DATABASE_PATH"],
              let conversation = ProcessInfo.processInfo.environment["NOLA_UI_EXISTING_HOST_CONVERSATION_ID"],
              let namespace = ProcessInfo.processInfo.environment["NOLA_UI_EXISTING_LOCAL_NAMESPACE"] else {
            throw XCTSkip("Requires the exact isolated failed embedded child database")
        }
        continueAfterFailure = false
        app = XCUIApplication()
        app.launchArguments = ["-ui-testing", "-AppleLanguages", "(en)", "-AppleLocale", "en_US"]
        app.launchEnvironment["NOLA_REAL_IOS_UI_DATABASE_PATH"] = database
        app.launchEnvironment["NOLA_UI_TEST_RUN_ID"] = namespace
        if let key = ProcessInfo.processInfo.environment["NOLA_UI_REPRO_API_KEY"], !key.isEmpty {
            let run = try XCTUnwrap(ProcessInfo.processInfo.environment["NOLA_UI_EXISTING_RUN_ID"])
            let assistant = try XCTUnwrap(ProcessInfo.processInfo.environment["NOLA_UI_EXISTING_ASSISTANT_ID"])
            app.launchEnvironment["NOLA_REAL_IOS_UI_RESTORE_FROZEN_CREDENTIAL"] = "1"
            app.launchEnvironment["NOLA_REAL_IOS_UI_CLOUD_KEY"] = key
            app.launchEnvironment["NOLA_REAL_IOS_UI_FROZEN_CONVERSATION"] = conversation
            app.launchEnvironment["NOLA_REAL_IOS_UI_FROZEN_RUN"] = run
            app.launchEnvironment["NOLA_REAL_IOS_UI_FROZEN_ASSISTANT"] = assistant
        }
        app.launch()
        let row = app.buttons["chat.conversation.\(conversation.lowercased())"]
        if !row.waitForExistence(timeout: 5) {
            let tab = app.tabBars.buttons["Chat"]
            XCTAssertTrue(tab.waitForExistence(timeout: 15), app.debugDescription)
            tab.tap()
        }
        XCTAssertTrue(row.waitForExistence(timeout: 20), app.debugDescription)
        row.tap()
        verifyEmbeddedChildApproval()
    }

    private func verifyEmbeddedChildApproval() {
        let deadline = Date().addingTimeInterval(300)
        var delayedChildApproval = false
        while Date() < deadline {
            let approval = app.buttons["chat.permission.allow_once"].firstMatch
            if approval.exists && approval.isHittable {
                let childRationale = app.staticTexts.containing(NSPredicate(format: "label CONTAINS %@", "子任务 ")).firstMatch
                if childRationale.exists && !delayedChildApproval {
                    capture("embedded-child-before-long-permission-wait")
                    // Outlast the original delivery lease while the shared driver renews it.
                    RunLoop.current.run(until: Date().addingTimeInterval(65))
                    XCTAssertTrue(approval.exists && approval.isEnabled, app.debugDescription)
                    delayedChildApproval = true
                }
                approval.tap()
            }
            if delayedChildApproval && app.buttons["chat.composer.send"].label != "Stop generation" { break }
            RunLoop.current.run(until: Date().addingTimeInterval(0.5))
        }
        XCTAssertTrue(delayedChildApproval, "The actual embedded child must request user approval")
        let permissionGone = XCTNSPredicateExpectation(predicate: NSPredicate(format: "exists == false"),
            object: app.buttons["chat.permission.allow_once"].firstMatch)
        XCTAssertEqual(XCTWaiter.wait(for: [permissionGone], timeout: 30), .completed,
            "An unconfirmed submission must never count as a completed approval")
        XCTAssertFalse(app.staticTexts["chat.permission.submission_error"].exists)
        assertReply("IOS_EMBEDDED_CHILD_APPROVED=391")
        waitForTerminal()
        capture("embedded-child-approved-after-renewal")
        // Companion exact Store audit must additionally prove child/parent sealed,
        // permission response committed once, and source type remains embedded.
    }

    func testRealHostReadOnlyChildResultNavigation() throws {
        try pairIsolatedHostAndStartConversation()
        send("Use spawn_agent to create exactly one child with permission_mode=read_only. The child must independently calculate 17 times 23 and return IOS_CHILD_RESULT=391. Wait for the actual child completion and report that exact result; do not invent a child execution.")
        waitForRunning()
        assertReply("IOS_CHILD_RESULT=391")
        waitForTerminal()
        let result = app.buttons.matching(NSPredicate(format: "identifier BEGINSWITH %@", "chat.task.openResult.")).firstMatch
        for _ in 0..<8 where !result.isHittable { app.swipeUp() }
        XCTAssertTrue(result.waitForExistence(timeout: 15), app.debugDescription)
        result.tap()
        XCTAssertTrue(app.descendants(matching: .any)["chat.task.result.readOnly"].waitForExistence(timeout: 20), app.debugDescription)
        XCTAssertFalse(app.textViews["chat.composer.input"].exists)
        XCTAssertTrue(app.staticTexts.containing(NSPredicate(format: "label CONTAINS %@", "IOS_CHILD_RESULT=391"))
            .firstMatch.waitForExistence(timeout: 30), app.debugDescription)
        capture("real-source-child-result-readonly")
        app.navigationBars.buttons.element(boundBy: 0).tap()
        XCTAssertTrue(app.textViews["chat.composer.input"].waitForExistence(timeout: 10))
        capture("real-source-child-result-returned-parent")
    }



    func testExistingHostRunsTwoReadOnlyAgents() throws {
        try reopenExistingHostConversation()
        let marker = "IOS_READONLY_PARALLEL_" + UUID().uuidString.replacingOccurrences(of: "-", with: "")
        send("For this new task, create exactly two independent child agents with spawn_agent. Set permission_mode=read_only on BOTH tool calls, using the tool schema. Submit both spawn calls together before any wait_task. Do not use shell tools or request writable permission. Child one independently explains why 31 times 37 is 1147, then returns READONLY_ONE=1147. Child two independently explains why 41 times 43 is 1763, then returns READONLY_TWO=1763. Wait for both actual child results and finish with \(marker), both actual task IDs and both results. Do not reuse earlier tasks or claim concurrency if admission fails.")
        waitForRunning()
        capture("read-only-children-running")
        assertReply(marker)
        waitForTerminal()
        assertReply("READONLY_ONE=1147")
        assertReply("READONLY_TWO=1763")
        capture("read-only-children-real-results")
        // A companion read-only Store audit must compare started/ended times,
        // immutable parent ownership and permission_mode. Model prose alone
        // does not establish concurrent execution.
    }

    func testExistingHostCompactsAndRecallsOriginalChildResults() throws {
        try reopenExistingHostConversation()
        for index in 1...4 {
            let marker = "IOS_COMPACT_TURN_\(index)_" + UUID().uuidString.prefix(8)
            send("Reply exactly \(marker).")
            assertReply(marker)
            waitForTerminal()
        }
        try compactAndRecallOriginalChildResults()
    }

    func testExistingHostCompactsCommittedHistoryAndRecallsOriginalChildResults() throws {
        try reopenExistingHostConversation()
        try compactAndRecallOriginalChildResults()
    }

    private func compactAndRecallOriginalChildResults() throws {
        guard let firstTask = ProcessInfo.processInfo.environment["NOLA_UI_EXPECTED_CHILD_ONE_ID"],
              let secondTask = ProcessInfo.processInfo.environment["NOLA_UI_EXPECTED_CHILD_TWO_ID"] else {
            throw XCTSkip("Requires actual original child IDs for post-compaction recall")
        }
        app.buttons["chat.topbar.more"].tap()
        app.buttons["chat.conversation.settings"].tap()
        let context = app.buttons["chat.more_settings.context"]
        XCTAssertTrue(context.waitForExistence(timeout: 10)); context.tap()
        let compact = app.buttons["Run compact"]
        for _ in 0..<12 where !compact.isHittable { app.swipeUp() }
        XCTAssertTrue(compact.isHittable)
        let reload = app.buttons["Reload context state"]
        if reload.isHittable {
            reload.tap()
            let reloaded = XCTNSPredicateExpectation(predicate: NSPredicate(format: "enabled == true"), object: reload)
            XCTAssertEqual(XCTWaiter.wait(for: [reloaded], timeout: 30), .completed)
        }
        compact.tap()
        let completed = app.staticTexts.matching(NSPredicate(format: "label BEGINSWITH %@", "Host context compacted")).firstMatch
        let compactSettled = XCTNSPredicateExpectation(predicate: NSPredicate(format: "enabled == true"), object: compact)
        XCTAssertEqual(XCTWaiter.wait(for: [compactSettled], timeout: 200), .completed)
        for _ in 0..<4 where !completed.exists { app.swipeUp() }
        XCTAssertTrue(completed.waitForExistence(timeout: 10), "The paired Host must return real compaction evidence: \(app.debugDescription)")
        capture("existing-host-compaction-receipt")
        app.navigationBars.buttons["Cancel"].firstMatch.tap()
        XCTAssertTrue(app.textViews["chat.composer.input"].waitForExistence(timeout: 10))
        let recallMarker = "IOS_AFTER_COMPACT_" + UUID().uuidString.prefix(8)
        send("Begin your reply with \(recallMarker). Recall our ORIGINAL two child agents that each ran sleep 20. Give each exact task ID and its returned CHILD_ONE or CHILD_TWO value from this conversation. Do not substitute the later read-only tasks. If you cannot recall, say so.")
        assertReply(recallMarker)
        waitForTerminal()
        let replies = app.otherElements.matching(identifier: "chat.assistant.bubble").allElementsBoundByIndex.filter {
            $0.descendants(matching: .staticText).matching(NSPredicate(format: "label BEGINSWITH %@", recallMarker)).firstMatch.exists
        }
        XCTAssertEqual(replies.count, 1, "Recall must be checked in the new response, not older visible history")
        let recalled = replies.first?.descendants(matching: .staticText).allElementsBoundByIndex.map(\.label).joined(separator: "\n") ?? ""
        for fact in [firstTask, secondTask, "CHILD_ONE=323", "CHILD_TWO=667"] {
            XCTAssertTrue(recalled.contains(fact), "New post-compaction reply omitted \(fact)")
        }
        capture("existing-host-original-facts-after-compaction")
    }

    private func returnToList(usingEdgeGesture: Bool = false) {
        if usingEdgeGesture {
            let start = app.coordinate(withNormalizedOffset: CGVector(dx: 0.005, dy: 0.5))
            let end = app.coordinate(withNormalizedOffset: CGVector(dx: 0.85, dy: 0.5))
            start.press(forDuration: 0.1, thenDragTo: end)
        } else {
            app.buttons["chat.navigation.back"].tap()
        }
        XCTAssertTrue(app.buttons["chat.conversations.new"].waitForExistence(timeout: 5))
    }

    private func assistantText(_ text: String) -> XCUIElement {
        app.otherElements.matching(identifier: "chat.assistant.bubble").descendants(matching: .staticText).matching(NSPredicate(format: "label CONTAINS %@", text)).firstMatch
    }

    private func assertReply(_ text: String) {
        let result = assistantText(text)
        XCTAssertTrue(result.waitForExistence(timeout: 240), "Expected real assistant reply: \(text)\n\(app.debugDescription)")
    }

    private func chooseModel(_ name: String) {
        app.buttons["chat.topbar.modelPicker"].tap()
        let choice = app.buttons.matching(NSPredicate(format: "label == %@", name)).firstMatch
        XCTAssertTrue(choice.waitForExistence(timeout: 5), app.debugDescription)
        choice.tap()
    }

    private func send(_ prompt: String) {
        let input = app.textViews["chat.composer.input"]
        XCTAssertTrue(input.waitForExistence(timeout: 10))
        if prompt.count > 160 {
            pasteUsingSystemMenu(prompt, into: input)
            XCTAssertEqual(input.value as? String, prompt)
        } else {
            input.tap(); input.typeText(prompt)
        }
        app.buttons["chat.composer.send"].tap()
    }

    /// Uses the same system edit menu as a person pasting a pairing token or a
    /// long task. No application model or input state is injected, and XCTest
    /// does not log the credential as a typeText argument.
    private func pasteUsingSystemMenu(_ text: String, into field: XCUIElement) {
        UIPasteboard.general.string = text
        defer { UIPasteboard.general.items = [] }
        field.tap()
        field.press(forDuration: 1.1)
        let paste = app.menuItems["Paste"].firstMatch
        let pasteButton = app.buttons["Paste"].firstMatch
        if paste.waitForExistence(timeout: 3) {
            paste.tap()
        } else {
            XCTAssertTrue(pasteButton.waitForExistence(timeout: 3), "System paste action must be available")
            pasteButton.tap()
        }
        let allowPaste = app.buttons["Allow Paste"]
        if allowPaste.waitForExistence(timeout: 1) { allowPaste.tap() }
    }

    /// Reads the transparent local test relay's observation, never a fabricated model event.
    private func waitForForwardedRequest(containing marker: String) {
        guard let endpoint = ProcessInfo.processInfo.environment["NOLA_UI_REPRO_ENDPOINT"],
              var url = URLComponents(string: endpoint) else { XCTFail("Missing real relay"); return }
        url.path = "/__nola_test/forwarded"
        let deadline = Date().addingTimeInterval(30)
        while Date() < deadline {
            if let endpoint = url.url, let data = try? Data(contentsOf: endpoint),
               let body = String(data: data, encoding: .utf8), body.contains(marker) { return }
            Thread.sleep(forTimeInterval: 0.2)
        }
        XCTFail("The long task must reach the real upstream before navigation and cancellation")
    }

    private func waitForRunning() {
        let button = app.buttons["chat.composer.send"]
        let running = XCTNSPredicateExpectation(predicate: NSPredicate(format: "exists == true AND label == %@", "Stop generation"), object: button)
        XCTAssertEqual(XCTWaiter.wait(for: [running], timeout: 30), .completed)
    }

    private func waitForTerminal() {
        let button = app.buttons["chat.composer.send"]
        XCTAssertTrue(button.waitForExistence(timeout: 5))
        let done = XCTNSPredicateExpectation(predicate: NSPredicate(format: "exists == true AND label != %@", "Stop generation"), object: button)
        XCTAssertEqual(XCTWaiter.wait(for: [done], timeout: 240), .completed)
    }

    private func controlPaintRange(_ frame: CGRect, in screenshot: XCUIScreenshot, window: CGRect) -> Int {
        guard let image = screenshot.image.cgImage else { return 0 }
        let scale = CGFloat(image.width) / window.width
        let visible = frame.intersection(window)
        guard !visible.isNull, visible.width > 0, visible.height > 0,
              let crop = image.cropping(to: CGRect(x: (visible.minX - window.minX) * scale,
                y: (visible.minY - window.minY) * scale, width: visible.width * scale, height: visible.height * scale)) else {
            return 0
        }
        var pixels = [UInt8](repeating: 0, count: 64 * 64 * 4)
        return pixels.withUnsafeMutableBytes { bytes in
            guard let context = CGContext(data: bytes.baseAddress, width: 64, height: 64,
                bitsPerComponent: 8, bytesPerRow: 64 * 4, space: CGColorSpaceCreateDeviceRGB(),
                bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue) else { return 0 }
            context.draw(crop, in: CGRect(x: 0, y: 0, width: 64, height: 64))
            let data = bytes.bindMemory(to: UInt8.self)
            let levels = stride(from: 0, to: data.count, by: 4).map {
                (Int(data[$0]) + Int(data[$0 + 1]) + Int(data[$0 + 2])) / 3
            }
            return (levels.max() ?? 0) - (levels.min() ?? 0)
        }
    }

    private func capture(_ name: String) {
        var screenshot = app.screenshot()
        var regions = ""
        // AX exposes destination geometry before UIKit finishes painting a push
        // and SwiftUI updates the bottom inset. Require matching frames AND
        // painted pixels within a bounded wait; do not disable the animations.
        let painted = XCTNSPredicateExpectation(predicate: NSPredicate { [self] _, _ in
            let controls = ["chat.topbar.modelPicker", "chat.composer.send"].compactMap { identifier -> XCUIElement? in
                let control = app.buttons[identifier]
                return control.exists && control.isHittable ? control : nil
            }
            let frames = controls.map(\.frame)
            let window = app.windows.firstMatch.frame
            screenshot = app.screenshot()
            let ranges = frames.map { controlPaintRange($0, in: screenshot, window: window) }
            regions = zip(controls, zip(frames, ranges)).map { "\($0.0.identifier): \($0.1.0), range=\($0.1.1)" }.joined(separator: "\n")
            return frames == controls.map(\.frame) && ranges.allSatisfy { $0 > 40 }
        }, object: nil)
        let result = XCTWaiter.wait(for: [painted], timeout: 5)
        let image = XCTAttachment(screenshot: screenshot)
        image.name = name; image.lifetime = .keepAlways; add(image)
        let tree = XCTAttachment(string: app.debugDescription)
        tree.name = name + "-accessibility"; tree.lifetime = .keepAlways; add(tree)
        if result != .completed {
            let diagnostic = XCTAttachment(string: regions)
            diagnostic.name = name + "-pixel-region"; diagnostic.lifetime = .keepAlways; add(diagnostic)
        }
        XCTAssertEqual(result, .completed, "\(name): visible controls must settle and paint within five seconds")
    }
}
