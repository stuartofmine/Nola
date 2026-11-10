// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Implements IOSRealHostChatTests logic in the project module.
// - Interfaces:
//   - Internal implementation details only.
// - APIs:
//   - none
//
//
//  IOSRealHostChatTests.swift
//  NolaUITests
//
//  Real-provider Host chat acceptance. Offline UI cases have been removed.
//

import XCTest

final class IOSRealHostChatTests: XCTestCase {
    private var app: XCUIApplication!

    override func setUpWithError() throws {
        continueAfterFailure = false
        app = XCUIApplication()
    }

    override func tearDownWithError() throws {
        app.terminate()
        app = nil
    }

    private func assertTab(_ identifier: String, fallbackLabel: String, screen: String) {
        var button = tabButton(identifier, fallbackLabel: fallbackLabel)
        XCTAssertTrue(button.waitForExistence(timeout: 8), "Missing tab: \(identifier)")
        button.tap()
        let destination = element(screen)
        if !destination.waitForExistence(timeout: 3) {
            // A first-run system permission alert can consume the synthesized tab tap
            // after XCTest dismisses the alert. Retry the same navigation once, while
            // still requiring the destination screen contract to become visible. A
            // SwiftUI tab transition can invalidate the original XCUIElement handle,
            // so resolve it again before retrying.
            button = tabButton(identifier, fallbackLabel: fallbackLabel)
            XCTAssertTrue(button.waitForExistence(timeout: 3), "Missing tab after interrupted navigation: \(identifier)")
            button.tap()
        }
        XCTAssertTrue(destination.waitForExistence(timeout: 8), "Missing screen: \(screen)")
    }

    private func tabButton(_ identifier: String, fallbackLabel: String) -> XCUIElement {
        let globallyIdentified = app.buttons.matching(identifier: identifier).firstMatch
        if globallyIdentified.exists {
            return globallyIdentified
        }
        let compactTab = app.tabBars.buttons[fallbackLabel].firstMatch
        return compactTab.exists ? compactTab : app.buttons[fallbackLabel].firstMatch
    }

    private func waitForPrimaryNavigation() -> Bool {
        if app.tabBars.firstMatch.waitForExistence(timeout: 3) {
            return true
        }
        return app.buttons["Chat"].firstMatch.waitForExistence(timeout: 12)
    }

    private func element(_ identifier: String) -> XCUIElement {
        app.descendants(matching: .any)[identifier]
    }
}

extension IOSRealHostChatTests {

    func testRealHostRendersToolAndMarkdownInProductionChat() throws {
        let environment = ProcessInfo.processInfo.environment
        guard environment["NOLA_REAL_IOS_UI_E2E"] == "1" else {
            throw XCTSkip("Set NOLA_REAL_IOS_UI_E2E=1 to run the real iOS rendered chat test.")
        }
        let hostURL = try XCTUnwrap(environment["NOLA_REAL_WEB_AGENT_HOST_URL"])
        let hostToken = try XCTUnwrap(environment["NOLA_REAL_WEB_AGENT_HOST_TOKEN"])
        let hostID = try XCTUnwrap(environment["NOLA_REAL_WEB_AGENT_HOST_ID"])
        let model = try XCTUnwrap(environment["NOLA_REAL_LLM_MODEL"])
        let sourcePath = try XCTUnwrap(environment["NOLA_REAL_IOS_UI_SOURCE_PATH"])
        let marker = try XCTUnwrap(environment["NOLA_REAL_IOS_UI_MARKER"])
        let timelinePath = try XCTUnwrap(environment["NOLA_REAL_IOS_UI_TIMELINE_PATH"])
        let screenshotPath = try XCTUnwrap(environment["NOLA_REAL_IOS_UI_SCREENSHOT_PATH"])
        let accessibilityPath = try XCTUnwrap(environment["NOLA_REAL_IOS_UI_ACCESSIBILITY_PATH"])
        let transcriptPath = try XCTUnwrap(environment["NOLA_REAL_IOS_UI_TRANSCRIPT_PATH"])
        let databasePath = URL(fileURLWithPath: timelinePath, isDirectory: false)
            .deletingLastPathComponent()
            .appendingPathComponent("ios-product.db", isDirectory: false)
            .path

        let app = XCUIApplication()
        app.launchArguments = [
            "-ui-testing",
            "-ui-testing-real-chat-host",
            "-AppleLanguages", "(en)",
            "-AppleLocale", "en_US",
        ]
        app.launchEnvironment["NOLA_REAL_WEB_AGENT_HOST_URL"] = hostURL
        app.launchEnvironment["NOLA_REAL_WEB_AGENT_HOST_TOKEN"] = hostToken
        app.launchEnvironment["NOLA_REAL_WEB_AGENT_HOST_ID"] = hostID
        app.launchEnvironment["NOLA_REAL_LLM_MODEL"] = model
        app.launchEnvironment["NOLA_REAL_IOS_UI_DATABASE_PATH"] = databasePath
        app.launchEnvironment["NOLA_REAL_IOS_UI_RESET"] = "1"
        app.launch()
        let chatTab = app.tabBars.buttons["Chat"]
        XCTAssertTrue(chatTab.waitForExistence(timeout: 10))
        chatTab.tap()

        if app.buttons["chat.conversations.new"].waitForExistence(timeout: 2) { app.buttons["chat.conversations.new"].tap() }
        let input = app.textViews["chat.composer.input"]
        XCTAssertTrue(input.waitForExistence(timeout: 8))

        let more = app.buttons["chat.topbar.more"]
        XCTAssertTrue(more.waitForExistence(timeout: 12))
        let executionPicker = app.buttons["chat.executionBackendPicker"].firstMatch

        input.tap()
        let prompt = "Use the read_file tool to read \(sourcePath). Then answer with a Markdown level-two heading named Verified and three complete bullet points explaining the source line, the exact marker, and why the answer is grounded."
        input.typeText(prompt)
        let send = app.buttons["chat.composer.send"]
        XCTAssertTrue(send.waitForExistence(timeout: 2))
        let sendStartedAt = Date()
        send.tap()

        let assistantBubble = app.descendants(matching: .any)["chat.assistant.bubble"]
        XCTAssertTrue(
            assistantBubble.waitForExistence(timeout: 1),
            "The production assistant bubble must exist before the first real Host event."
        )
        let firstBubbleDelay = Date().timeIntervalSince(sendStartedAt) * 1_000
        var samples: [[String: Any]] = []
        var lastFingerprint = ""
        var terminalErrorDetected = false
        var terminalAndStable = false
        var stableTerminalFingerprint: String?
        var stableTerminalSampleCount = 0
        var expandedExecutionProcess = false
        let toolCards = app.buttons.matching(
            NSPredicate(format: "identifier BEGINSWITH %@", "chat.assistant.tool.")
        )
        // Accessibility snapshots synchronously visit SwiftUI on the app's
        // main actor. Dense whole-tree polling can therefore starve the very
        // stream this test is observing. Sample the stable bubble subtree at
        // human-visible cadence and query the concrete tool-card contract.
        Thread.sleep(forTimeInterval: 1.5)
        // This is a real two-request model round trip with an actual file
        // read between them. Local inference can spend tens of seconds on
        // each request; keep a bounded end-to-end window for both attempts.
        let samplingDeadline = Date().addingTimeInterval(120)
        while Date() < samplingDeadline {
            let processToggle = app.buttons["chat.execution.process.toggle"].firstMatch
            if !expandedExecutionProcess, processToggle.exists, processToggle.isHittable {
                processToggle.tap()
                expandedExecutionProcess = (processToggle.value as? String) == "Expanded"
            }
            let answer = app.descendants(matching: .any)["chat.assistant.answer"]
            let text = answer.exists
                ? answer.staticTexts.allElementsBoundByIndex.map(\.label).joined(separator: "\n")
                : ""
            let headingCount = answer.exists
                ? answer.descendants(matching: .any)
                    .matching(identifier: "chat.assistant.markdown.heading.2").count
                : 0
            let listItemCount = answer.exists
                ? answer.descendants(matching: .any).matching(
                    NSPredicate(format: "identifier BEGINSWITH %@", "chat.assistant.markdown.")
                ).count
                : 0
            let bubbleCount = app.descendants(matching: .any)
                .matching(identifier: "chat.assistant.bubble").count
            let toolVisible = toolCards.firstMatch.exists
            let markerVisible = text.contains(marker)
            let bubbleCompleted = (assistantBubble.value as? String) == "completed"
            let fingerprint = "\(text.count):\(headingCount):\(listItemCount):\(bubbleCount):\(toolVisible)"
            if fingerprint != lastFingerprint {
                samples.append([
                    "stage": "assistant",
                    "elapsedMs": Date().timeIntervalSince(sendStartedAt) * 1_000,
                    "assistantCharacters": text.count,
                    "assistantText": text,
                    "running": !bubbleCompleted,
                    "textLength": text.count,
                    "headingCount": headingCount,
                    "markdownNodeCount": listItemCount,
                    "bubbleCount": bubbleCount,
                    "toolVisible": toolVisible,
                    "markerVisible": markerVisible,
                    "bubbleCompleted": bubbleCompleted,
                ])
                lastFingerprint = fingerprint
            }
            terminalErrorDetected = text.localizedCaseInsensitiveContains("unknown error")
                || text.localizedCaseInsensitiveContains("remote conversation")
                || text.localizedCaseInsensitiveContains("couldn’t be completed")
            if terminalErrorDetected {
                break
            }
            if bubbleCompleted && markerVisible && headingCount > 0 && toolVisible && text.count > 120 {
                if stableTerminalFingerprint == fingerprint {
                    stableTerminalSampleCount += 1
                } else {
                    stableTerminalFingerprint = fingerprint
                    stableTerminalSampleCount = 1
                }
                terminalAndStable = stableTerminalSampleCount >= 3
            } else {
                stableTerminalFingerprint = nil
                stableTerminalSampleCount = 0
            }
            if terminalAndStable {
                break
            }
            // The Host emits semantic phrases rather than raw provider tokens.
            // Poll often enough to observe those human-visible stages without
            // dense, unbounded whole-tree snapshots that starve SwiftUI.
            Thread.sleep(forTimeInterval: 0.2)
        }
        let toolsVisible = toolCards.firstMatch.exists
        let answerNodes = app.descendants(matching: .any)
            .matching(identifier: "chat.assistant.answer")
        let answer = answerNodes.firstMatch
        let answerVisible = answer.exists
        let headingNodes = answer.descendants(matching: .any)
            .matching(identifier: "chat.assistant.markdown.heading.2")
        let heading = headingNodes.firstMatch
        let markerNodes = answer.staticTexts
            .matching(NSPredicate(format: "label CONTAINS %@", marker))
        let headingVisible = heading.exists && heading.label.contains("Verified")
        let markerVisible = markerNodes.firstMatch.exists
        let markerCount = markerNodes.count
        let answerCount = answerNodes.count
        let headingCount = headingNodes.count
        let positiveLengths = samples.compactMap { sample -> Int? in
            let value = sample["textLength"] as? Int ?? 0
            return value > 0 ? value : nil
        }
        let evidenceDirectory = URL(fileURLWithPath: timelinePath).deletingLastPathComponent()
        try FileManager.default.createDirectory(
            at: evidenceDirectory,
            withIntermediateDirectories: true
        )
        let initialFinalText = answer.staticTexts.allElementsBoundByIndex
            .map(\.label)
            .joined(separator: "\n")
        let chatScreenshot = XCUIScreen.main.screenshot()
        try chatScreenshot.pngRepresentation.write(to: URL(fileURLWithPath: screenshotPath))
        try app.debugDescription.write(
            to: URL(fileURLWithPath: accessibilityPath),
            atomically: true,
            encoding: .utf8
        )
        samples.append([
            "stage": "evidence-captured",
            "elapsedMs": Date().timeIntervalSince(sendStartedAt) * 1_000,
            "assistantCharacters": initialFinalText.count,
            "assistantText": initialFinalText,
            "running": true,
            "toolVisible": toolsVisible,
            "answerVisible": answerVisible,
            "headingVisible": headingVisible,
            "markerVisible": markerVisible,
            "bubbleCount": app.descendants(matching: .any)
                .matching(identifier: "chat.assistant.bubble").count,
        ])
        try JSONSerialization.data(
            withJSONObject: samples,
            options: [.prettyPrinted, .sortedKeys]
        ).write(to: URL(fileURLWithPath: timelinePath))
        try JSONSerialization.data(
            withJSONObject: [
                ["role": "user", "text": prompt],
                ["role": "assistant", "text": initialFinalText],
            ],
            options: [.prettyPrinted, .sortedKeys]
        ).write(to: URL(fileURLWithPath: transcriptPath))
        let attachment = XCTAttachment(screenshot: chatScreenshot)
        attachment.name = "Real iOS Agent Host rendered chat"
        attachment.lifetime = .keepAlways
        add(attachment)
        XCTAssertTrue(toolsVisible, "The real read_file trace was not rendered")
        XCTAssertTrue(answerVisible, "The real assistant answer was not rendered")
        XCTAssertTrue(
            terminalAndStable,
            "Evidence capture must wait for a terminal canonical bubble and stable rendered text."
        )
        more.tap()
        XCTAssertTrue(executionPicker.waitForExistence(timeout: 5))
        XCTAssertTrue(waitForExecutionUnit("Nola", in: executionPicker))
        executionPicker.tap()
        let codexOption = app.descendants(matching: .any)
            .matching(NSPredicate(format: "label BEGINSWITH %@", "Codex"))
            .firstMatch
        let claudeOption = app.descendants(matching: .any)
            .matching(NSPredicate(format: "label BEGINSWITH %@", "Claude Code"))
            .firstMatch
        XCTAssertTrue(codexOption.waitForExistence(timeout: 5), "The installed Codex execution unit is missing from the real Host menu.")
        XCTAssertTrue(claudeOption.waitForExistence(timeout: 5), "The installed Claude Code execution unit is missing from the real Host menu.")
        XCTAssertTrue(codexOption.isHittable, "An installed Codex execution unit must be selectable.")
        codexOption.tap()
        XCTAssertTrue(codexOption.waitForNonExistence(timeout: 5))
        more.tap()
        XCTAssertTrue(executionPicker.waitForExistence(timeout: 5))
        XCTAssertTrue(waitForExecutionUnit("Codex", in: executionPicker))
        executionPicker.tap()
        let nolaOption = app.descendants(matching: .any)
            .matching(NSPredicate(format: "label BEGINSWITH %@", "Nola"))
            .firstMatch
        XCTAssertTrue(nolaOption.waitForExistence(timeout: 5))
        nolaOption.tap()
        XCTAssertTrue(nolaOption.waitForNonExistence(timeout: 5))
        more.tap()
        XCTAssertTrue(executionPicker.waitForExistence(timeout: 5))
        XCTAssertTrue(waitForExecutionUnit("Nola", in: executionPicker))
        app.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.45)).tap()

        app.terminate()
        app.launchEnvironment["NOLA_REAL_IOS_UI_RESET"] = "0"
        app.launch()
        let restoredChatTab = app.tabBars.buttons["Chat"]
        if restoredChatTab.waitForExistence(timeout: 5) { restoredChatTab.tap() }
        if app.buttons["chat.conversations.new"].waitForExistence(timeout: 5) {
            let restoredConversation = app.buttons.matching(NSPredicate(format: "identifier MATCHES %@", "^chat\\.conversation\\.[0-9a-f-]{36}$")).firstMatch
            XCTAssertTrue(restoredConversation.waitForExistence(timeout: 5))
            restoredConversation.tap()
        }
        let restoredAnswers = app.descendants(matching: .any)
            .matching(identifier: "chat.assistant.answer")
        let restoredHeading = restoredAnswers.firstMatch.descendants(matching: .any)[
            "chat.assistant.markdown.heading.2"
        ]
        let restoredMarker = app.staticTexts
            .matching(NSPredicate(format: "label CONTAINS %@", marker))
        let conversationRestored = restoredHeading.waitForExistence(timeout: 10)
            && restoredMarker.firstMatch.waitForExistence(timeout: 10)
            && restoredAnswers.count == 1
        app.openModelCenterFromSettings()
        let messagingCard = app.descendants(matching: .any)["model.host.messaging.card"]
        XCTAssertTrue(
            messagingCard.waitForExistence(timeout: 12),
            "A connected Host must expose messaging-channel state in the model center."
        )
        XCTAssertTrue(app.buttons["model.host.messaging.manage"].firstMatch.waitForExistence(timeout: 5))
        let bubbleStable = samples.allSatisfy { ($0["bubbleCount"] as? Int) == 1 }
        let progressive = Set(positiveLengths).count >= 3
        let passed = firstBubbleDelay < 3_000
            && terminalAndStable
            && headingVisible
            && markerVisible
            && answerCount == 1
            && headingCount == 1
            && bubbleStable
            && progressive
            && conversationRestored
        samples.append([
            "stage": "assistant",
            "elapsedMs": Date().timeIntervalSince(sendStartedAt) * 1_000,
            "assistantCharacters": initialFinalText.count,
            "assistantText": initialFinalText,
            "running": false,
            "passed": passed,
            "firstBubbleDelayMs": firstBubbleDelay,
            "marker": marker,
            "headingVisible": headingVisible,
            "markerVisible": markerVisible,
            "markerCount": markerCount,
            "answerCount": answerCount,
            "headingCount": headingCount,
            "bubbleStable": bubbleStable,
            "conversationRestored": conversationRestored,
            "terminalAndStable": terminalAndStable,
        ])
        try JSONSerialization.data(
            withJSONObject: samples,
            options: [.prettyPrinted, .sortedKeys]
        ).write(to: URL(fileURLWithPath: timelinePath))
        XCTAssertFalse(terminalErrorDetected, "The canonical Host projection failed before rendering the expected answer: \(initialFinalText)")
        XCTAssertTrue(terminalAndStable, "The real Host answer never reached a stable terminal UI state")
        XCTAssertTrue(headingVisible, "The Markdown heading is not accessible")
        XCTAssertTrue(markerVisible, "The real tool result is not accessible")
        XCTAssertEqual(answerCount, 1, "The real streamed answer created duplicate bubbles")
        XCTAssertEqual(headingCount, 1, "The Markdown answer was rendered twice")
        XCTAssertLessThan(firstBubbleDelay, 3_000)
        XCTAssertTrue(progressive, "The production UI did not expose progressive semantic text stages")
        XCTAssertTrue(bubbleStable, "Waiting, streaming, and terminal states must retain one bubble")
        XCTAssertTrue(conversationRestored, "The completed conversation was not restored after relaunch")
    }

    private func waitForExecutionUnit(_ expected: String, in picker: XCUIElement) -> Bool {
        let deadline = Date().addingTimeInterval(5)
        while Date() < deadline {
            if picker.label.contains(expected) { return true }
            Thread.sleep(forTimeInterval: 0.05)
        }
        return false
    }
}
