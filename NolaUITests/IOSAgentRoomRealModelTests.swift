// SPDX-License-Identifier: Apache-2.0
import XCTest

/// Opt-in real inference through the shipped model configuration and group composer.
/// Pass values with TEST_RUNNER_NOLA_AGENT_ROOM_UI_* when invoking xcodebuild.
final class IOSAgentRoomRealModelTests: XCTestCase {
    func testExistingHistoryAndRealRepliesFromTwoIndependentRecipients() throws {
        continueAfterFailure = false
        let environment = ProcessInfo.processInfo.environment
        guard let fixture = environment["NOLA_AGENT_ROOM_UI_FIXTURE_PATH"],
              let endpoint = environment["NOLA_AGENT_ROOM_UI_BASE_URL"],
              let model = environment["NOLA_AGENT_ROOM_UI_MODEL"] else {
            throw XCTSkip("Real room inference requires an isolated fixture, base URL and model in the test environment.")
        }
        let agents = [environment["NOLA_AGENT_ROOM_UI_FIRST_AGENT"] ?? "a",
                      environment["NOLA_AGENT_ROOM_UI_SECOND_AGENT"] ?? "b"]
        let artifactDirectory = environment["NOLA_AGENT_ROOM_UI_ARTIFACT_DIR"].map {
            URL(fileURLWithPath: $0, isDirectory: true)
        } ?? FileManager.default.temporaryDirectory
        let root = artifactDirectory.resolvingSymlinksInPath()
            .appendingPathComponent("nola-room-real-ui-\(UUID())", isDirectory: true)
        try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
        let database = root.appendingPathComponent("nola.db")
        try FileManager.default.copyItem(at: URL(fileURLWithPath: fixture), to: database)
        let databaseAttachment = XCTAttachment(string: database.path)
        databaseAttachment.name = "isolated-canonical-database-path"
        databaseAttachment.lifetime = .keepAlways
        add(databaseAttachment)
        let app = XCUIApplication()
        app.launchArguments = ["-ui-testing", "-AppleLanguages", "(en)", "-AppleLocale", "en_US"]
        app.launchEnvironment["NOLA_REAL_IOS_UI_DATABASE_PATH"] = database.path
        app.launchEnvironment["NOLA_UI_TEST_RUN_ID"] = UUID().uuidString
        app.launchEnvironment["NOLA_REAL_IOS_UI_PREPARE_CREDENTIAL"] = "1"
        app.launchEnvironment["NOLA_REAL_IOS_UI_FIXTURE_ROOT"] = root.path
        app.launchEnvironment["NOLA_REAL_IOS_UI_CLOUD_ENDPOINT"] = endpoint
        app.launchEnvironment["NOLA_REAL_IOS_UI_CLOUD_MODEL"] = model
        app.launchEnvironment["NOLA_REAL_IOS_UI_CLOUD_KEY"] = environment["NOLA_AGENT_ROOM_UI_API_KEY"] ?? ""
        app.launch()
        defer { app.terminate() }

        let chat = app.buttons["tab.chat"]
        XCTAssertTrue(chat.waitForExistence(timeout: 15))
        XCTAssertTrue(chat.isHittable)
        chat.tap()
        XCTAssertTrue(chat.isSelected)
        let rooms = app.buttons["Agent chats"].firstMatch
        XCTAssertTrue(rooms.waitForExistence(timeout: 5))
        rooms.tap()
        let isolation = app.buttons[environment["NOLA_AGENT_ROOM_UI_ROOM"] ?? "Isolation"].firstMatch
        XCTAssertTrue(isolation.waitForExistence(timeout: 5))
        isolation.tap()
        captureSafeButtons(app, stage: "room-opened-directly")
        let membersClosed = XCTNSPredicateExpectation(
            predicate: NSPredicate(format: "exists == false"), object: app.buttons["Done"].firstMatch)
        XCTAssertEqual(XCTWaiter.wait(for: [membersClosed], timeout: 5), .completed,
            "Opening an existing room must go directly to the conversation")
        let recipient = app.buttons["room.recipient"]
        XCTAssertTrue(recipient.waitForExistence(timeout: 5))
        XCTAssertTrue(app.otherElements["chat.assistant.bubble"].firstMatch.waitForExistence(timeout: 5))
        capture("room-existing-real-history")

        // Keep the CLI's warmed contexts intact. Mobile execution uses newly
        // admitted contexts created through the same product UI as a user.
        app.buttons["chat.conversation.settings"].tap()
        app.buttons["All agent chats"].tap()
        let freshTitle = "iOS acceptance \(UUID().uuidString.prefix(8))"
        let name = app.textFields["Chat name"]
        XCTAssertTrue(name.waitForExistence(timeout: 5))
        name.tap()
        name.typeText(freshTitle)
        app.buttons["Create chat"].tap()
        for agent in agents {
            let add = app.buttons[agent].firstMatch
            XCTAssertTrue(add.waitForExistence(timeout: 5))
            add.tap()
        }
        capture("room-fresh-two-members")
        app.buttons["Done"].firstMatch.tap()

        var markers: [String] = []
        for (index, agent) in agents.enumerated() {
            recipient.tap()
            captureSafeButtons(app, stage: "recipient-\(index)-after-tap")
            let choice = app.collectionViews.buttons[agent].firstMatch
            XCTAssertTrue(choice.waitForExistence(timeout: 5))
            choice.tap()
            let selected = XCTNSPredicateExpectation(predicate: NSPredicate(format: "label == %@", agent), object: recipient)
            XCTAssertEqual(XCTWaiter.wait(for: [selected], timeout: 5), .completed)
            let input = app.textViews["chat.composer.input"]
            XCTAssertTrue(input.waitForExistence(timeout: 5))
            let marker = "IOSROOM\(index)_\(UUID().uuidString.replacingOccurrences(of: "-", with: ""))"
            markers.append(marker)
            input.tap()
            input.typeText("Reply with exactly \(marker). Do not use tools or change any data.")
            let send = app.buttons["chat.composer.send"]
            XCTAssertTrue(send.isEnabled)
            send.tap()
            // Match only completed assistant content, never the user prompt.
            let replyExists = { self.hasCommittedAssistantReply(marker, in: app) }
            let failure = app.descendants(matching: .any).matching(NSPredicate(
                format: "identifier BEGINSWITH %@", "room.submission.error.")).firstMatch
            var capturedOutcomeTree = false
            let outcome = XCTNSPredicateExpectation(predicate: NSPredicate { _, _ in
                if !capturedOutcomeTree,
                   self.completedAssistantBubbles(in: app).count > index || failure.exists {
                    self.captureAssistantTree(app, name: "assistant-first-outcome-\(index)")
                    capturedOutcomeTree = true
                }
                return replyExists() || failure.exists
            }, object: nil)
            let outcomeResult = XCTWaiter.wait(for: [outcome], timeout: 180)
            captureAssistantTree(app, name: "assistant-final-outcome-\(index)")
            XCTAssertEqual(outcomeResult, .completed)
            if failure.exists {
                capture("room-real-submission-failed-\(index)")
                XCTFail("Room submission failed: \(failure.label)")
            }
            XCTAssertTrue(replyExists(), "The selected recipient must commit its own real reply")
            let streaming = app.otherElements.matching(NSPredicate(
                format: "identifier == %@ AND value == %@", "chat.assistant.bubble", "streaming"))
            let finished = XCTNSPredicateExpectation(predicate: NSPredicate { _, _ in streaming.count == 0 }, object: nil)
            XCTAssertEqual(XCTWaiter.wait(for: [finished], timeout: 60), .completed)
            capture("room-real-recipient-\(index)-completed")
        }
        app.terminate()
        app.launchEnvironment.removeValue(forKey: "NOLA_REAL_IOS_UI_PREPARE_CREDENTIAL")
        app.launchEnvironment.removeValue(forKey: "NOLA_REAL_IOS_UI_CLOUD_KEY")
        app.launch()
        XCTAssertTrue(rooms.waitForExistence(timeout: 15))
        rooms.tap()
        let freshRoom = app.buttons[freshTitle].firstMatch
        XCTAssertTrue(freshRoom.waitForExistence(timeout: 5))
        freshRoom.tap()
        app.buttons["chat.conversation.settings"].tap()
        for agent in agents {
            XCTAssertTrue(app.staticTexts[agent].firstMatch.waitForExistence(timeout: 5),
                "Canonical member must survive process restart")
        }
        app.buttons["Done"].firstMatch.tap()
        XCTAssertTrue(recipient.waitForExistence(timeout: 5))
        captureAssistantTree(app, name: "assistant-after-restart")
        for marker in markers.reversed() {
            let replyExists = { self.hasCommittedAssistantReply(marker, in: app) }
            for _ in 0..<8 where !replyExists() { app.swipeDown() }
            XCTAssertTrue(replyExists(),
                "Each committed assistant reply must survive process restart")
        }
        capture("room-real-history-after-process-restart")
        XCTAssertEqual(app.buttons.matching(identifier: "room.recipient").count, 1)
        XCTAssertFalse(app.descendants(matching: .any).matching(NSPredicate(
            format: "identifier BEGINSWITH %@", "room.member.activity.")).firstMatch.exists,
            "Idle and completed members must not occupy a persistent activity strip")
    }

    private func hasCommittedAssistantReply(_ marker: String, in app: XCUIApplication) -> Bool {
        // Paragraph phrases and block Markdown expose different AX internals.
        // The completed assistant boundary is stable for both rendering paths.
        for bubble in completedAssistantBubbles(in: app).allElementsBoundByIndex {
            for element in bubble.descendants(matching: .any).allElementsBoundByIndex {
                for label in [element.label, element.value as? String ?? ""] {
                    let normalized = label.replacingOccurrences(of: "\u{00AD}", with: "")
                        .replacingOccurrences(of: "-", with: "")
                        .components(separatedBy: .whitespacesAndNewlines).joined()
                    if normalized == marker { return true }
                }
            }
        }
        return false
    }

    private func completedAssistantBubbles(in app: XCUIApplication) -> XCUIElementQuery {
        app.descendants(matching: .any).matching(NSPredicate(
            format: "identifier == %@ AND value == %@", "chat.assistant.bubble", "completed"))
    }

    private func captureAssistantTree(_ app: XCUIApplication, name: String) {
        let bubbles = app.descendants(matching: .any).matching(identifier: "chat.assistant.bubble")
        let lines = bubbles.allElementsBoundByIndex.enumerated().flatMap { index, bubble in
            ([bubble] + bubble.descendants(matching: .any).allElementsBoundByIndex).map { element in
                "bubble=\(index), type=\(element.elementType.rawValue), identifier=\(element.identifier), label=\(element.label), value=\(element.value as? String ?? "")"
            }
        }
        // Restricted to assistant bubbles: excludes composer, configuration and credentials.
        let attachment = XCTAttachment(string: lines.joined(separator: "\n"))
        attachment.name = name
        attachment.lifetime = .keepAlways
        add(attachment)
    }

    private func capture(_ name: String) {
        let attachment = XCTAttachment(screenshot: XCUIScreen.main.screenshot())
        attachment.name = name
        attachment.lifetime = .keepAlways
        add(attachment)
    }

    /// Deliberately excludes text fields, values and arbitrary hierarchy dumps.
    private func captureSafeButtons(_ element: XCUIElement, stage: String) {
        let buttons = element.descendants(matching: .button).allElementsBoundByIndex
        let summary = buttons.map {
            "label=\($0.label), identifier=\($0.identifier), frame=\($0.frame), hittable=\($0.isHittable)"
        }.joined(separator: "\n")
        let attachment = XCTAttachment(string: summary)
        attachment.name = "safe-buttons-\(stage)"
        attachment.lifetime = .keepAlways
        add(attachment)
    }
}
