import XCTest

/// Opt-in native cloud acceptance. Experts are created and selected through
/// production controls; the relay only forwards the actual configured model.
final class IOSRealExpertAcceptanceTests: XCTestCase {
    func testNativeExpertPublicReasoningGrowsAndStops() throws {
        let environment = ProcessInfo.processInfo.environment
        guard let database = environment["NOLA_NATIVE_EXPERT_DATABASE"], let root = environment["NOLA_NATIVE_EXPERT_ROOT"],
              let endpoint = environment["NOLA_NATIVE_EXPERT_ENDPOINT"], let model = environment["NOLA_NATIVE_EXPERT_MODEL"],
              let secret = environment["NOLA_NATIVE_EXPERT_KEY"], let namespace = environment["NOLA_NATIVE_EXPERT_NAMESPACE"] else {
            throw XCTSkip("Requires an isolated Store and actual configured provider")
        }
        continueAfterFailure = false
        let app = XCUIApplication()
        app.launchArguments = ["-ui-testing", "-AppleLanguages", "(en)", "-AppleLocale", "en_US"]
        app.launchEnvironment.merge(["NOLA_UI_TEST_RUN_ID": namespace, "NOLA_REAL_IOS_UI_DATABASE_PATH": database,
            "NOLA_REAL_IOS_UI_FIXTURE_ROOT": root, "NOLA_REAL_IOS_UI_PREPARE_CREDENTIAL": "1",
            "NOLA_REAL_IOS_UI_CLOUD_ENDPOINT": endpoint, "NOLA_REAL_IOS_UI_CLOUD_MODEL": model,
            "NOLA_REAL_IOS_UI_CLOUD_KEY": secret]) { _, new in new }
        app.launch()
        defer { app.terminate() }
        XCTAssertTrue(app.tabBars.buttons["Chat"].waitForExistence(timeout: 15)); app.tabBars.buttons["Chat"].tap()
        XCTAssertTrue(app.buttons["chat.conversations.new"].waitForExistence(timeout: 10)); app.buttons["chat.conversations.new"].tap()
        app.buttons["chat.topbar.modelPicker"].tap()
        let modelChoice = app.buttons.matching(NSPredicate(format: "label CONTAINS %@", model)).firstMatch
        XCTAssertTrue(modelChoice.waitForExistence(timeout: 10)); modelChoice.tap()
        app.buttons["chat.topbar.more"].tap(); app.buttons["chat.experts.open"].tap(); app.buttons["experts.manage"].tap()
        app.buttons["experts.create"].tap()
        let name = "Native Public Reasoning Observer"
        let field = app.textFields["experts.name"]
        XCTAssertTrue(field.waitForExistence(timeout: 5)); field.tap(); field.typeText(name)
        let save = app.buttons["experts.save"]
        for _ in 0..<5 where !save.isHittable { app.collectionViews.firstMatch.swipeUp() }
        XCTAssertTrue(save.isHittable); save.tap()
        let agent = app.buttons.matching(NSPredicate(format: "identifier BEGINSWITH %@ AND label CONTAINS %@", "experts.edit.", name)).firstMatch
        for _ in 0..<5 where !agent.isHittable { app.collectionViews.firstMatch.swipeDown() }
        XCTAssertTrue(agent.exists)
        let expertID = String(agent.identifier.dropFirst("experts.edit.".count))
        app.buttons["experts.management.done"].tap()
        XCTAssertTrue(app.buttons["experts.select." + expertID].waitForExistence(timeout: 5)); app.buttons["experts.select." + expertID].tap()
        app.buttons["Done"].firstMatch.tap()
        let input = app.textViews["chat.composer.input"]
        input.tap(); input.typeText("Arrange digits 1 through 9 exactly once into a nine-digit number whose first n digits are divisible by n, for every n from one through nine. Explain the constraints and give a valid number.")
        app.buttons["chat.composer.send"].tap()
        let deadline = Date().addingTimeInterval(90)
        let row = expertReplyRows(app).firstMatch
        let activity = app.descendants(matching: .any).matching(NSPredicate(format: "identifier BEGINSWITH %@", "room.member.activity.")).firstMatch
        while Date() < deadline, !activity.exists, !row.exists { Thread.sleep(forTimeInterval: 0.2) }
        XCTAssertTrue(activity.exists || row.exists, "An admitted expert must show its real processing controls while waiting for provider bytes")
        if !row.exists { XCTAssertTrue(activity.buttons["Stop"].firstMatch.isHittable) }
        let bubble = row.descendants(matching: .any)["chat.assistant.bubble"].firstMatch
        let timeline = app.scrollViews["chat.expert.timeline"].firstMatch
        XCTAssertTrue(timeline.exists)
        func bounds(_ frame: CGRect) -> [String: Double] {
            ["x": Double(frame.minX), "y": Double(frame.minY), "width": Double(frame.width), "height": Double(frame.height)]
        }
        var samples: [[String: Any]] = []
        var firstReasoningCount: Int?
        var grew = false
        while Date() < deadline {
            // DisclosureGroup exposes its header and body as siblings. Only
            // the actual public reasoning StaticText is a content sample.
            if row.exists, let body = bubble.staticTexts.matching(identifier: "chat.assistant.thinking").allElementsBoundByIndex.filter({
                   let clipped = $0.frame.intersection(timeline.frame)
                   return $0.label.count >= 32 && $0.isHittable && !clipped.isNull && !clipped.isEmpty
               }).max(by: { $0.label.count < $1.label.count }) {
                let visible = body.frame.intersection(timeline.frame)
                let text = body.label
                let state = bubble.value as? String ?? "missing"
                if state == "running", firstReasoningCount == nil || text.count >= (firstReasoningCount ?? 0) + 32 {
                    let screenshot = samples.isEmpty ? "native-public-reasoning-first-visible" : "native-public-reasoning-grown-visible"
                    samples.append(["characters": text.count, "state": state, "screenshot": screenshot,
                        "bodyBounds": bounds(body.frame),
                        "viewportBounds": bounds(timeline.frame), "visibleIntersection": bounds(visible)])
                    capture(screenshot, app)
                    try write(samples, root: root, name: "native-public-reasoning-samples.json")
                    if let initial = firstReasoningCount, text.count >= initial + 32 { grew = true; break }
                    if firstReasoningCount == nil { firstReasoningCount = text.count }
                }
            }
            Thread.sleep(forTimeInterval: 0.4)
        }
        capture("native-public-reasoning-before-stop", app)
        XCTAssertTrue(grew, "Public reasoning must visibly grow while the original Run is still running")
        let stop = row.buttons["Stop"].firstMatch
        for _ in 0..<8 where !stop.isHittable { dragReplies(timeline, earlier: false) }
        XCTAssertTrue(stop.isHittable); stop.tap()
        let stopped = XCTNSPredicateExpectation(predicate: NSPredicate(format: "value == %@", "cancelled"), object: bubble)
        XCTAssertEqual(XCTWaiter.wait(for: [stopped], timeout: 45), .completed, "The UI must publish the exact canonical cancellation")
        capture("native-public-reasoning-exact-cancelled", app)
        try write(["passed": true, "expertID": expertID, "assistantRowID": row.identifier,
            "terminalState": bubble.value as? String ?? "missing", "samples": samples], root: root, name: "native-public-reasoning-result.json")
    }

    func testNativeExpertsStreamRecoverAndKeepConversationDrafts() throws {
        let environment = ProcessInfo.processInfo.environment
        guard let database = environment["NOLA_NATIVE_EXPERT_DATABASE"],
              let root = environment["NOLA_NATIVE_EXPERT_ROOT"],
              let endpoint = environment["NOLA_NATIVE_EXPERT_ENDPOINT"],
              let model = environment["NOLA_NATIVE_EXPERT_MODEL"],
              let secret = environment["NOLA_NATIVE_EXPERT_KEY"],
              let namespace = environment["NOLA_NATIVE_EXPERT_NAMESPACE"] else {
            throw XCTSkip("Requires an isolated canonical Store and actual configured provider")
        }
        continueAfterFailure = false
        let app = XCUIApplication()
        app.launchArguments = ["-ui-testing", "-AppleLanguages", "(en)", "-AppleLocale", "en_US"]
        app.launchEnvironment.merge([
            "NOLA_UI_TEST_RUN_ID": namespace,
            "NOLA_REAL_IOS_UI_DATABASE_PATH": database,
            "NOLA_REAL_IOS_UI_FIXTURE_ROOT": root,
            "NOLA_REAL_IOS_UI_PREPARE_CREDENTIAL": "1",
            "NOLA_REAL_IOS_UI_CLOUD_ENDPOINT": endpoint,
            "NOLA_REAL_IOS_UI_CLOUD_MODEL": model,
            "NOLA_REAL_IOS_UI_CLOUD_KEY": secret,
            "NOLA_REAL_IOS_UI_EXPERT_CAPTURE": "1",
        ]) { _, new in new }
        app.launch()
        defer { app.terminate() }
        let captureFixture = try XCTUnwrap(JSONSerialization.jsonObject(with: Data(contentsOf:
            URL(fileURLWithPath: root).appendingPathComponent("native-capture-fixture.json"))) as? [String: String])
        let templateName = try XCTUnwrap(captureFixture["templateName"])
        let chatTab = app.tabBars.buttons["Chat"]
        XCTAssertTrue(chatTab.waitForExistence(timeout: 15)); chatTab.tap()
        let newConversation = app.buttons["chat.conversations.new"]
        XCTAssertTrue(newConversation.waitForExistence(timeout: 10)); newConversation.tap()
        app.buttons["chat.topbar.modelPicker"].tap()
        let modelChoice = app.buttons.matching(NSPredicate(format: "label CONTAINS %@", model)).firstMatch
        XCTAssertTrue(modelChoice.waitForExistence(timeout: 10)); modelChoice.tap()

        app.buttons["chat.topbar.more"].tap()
        app.buttons["chat.experts.open"].tap()
        app.buttons["experts.manage"].tap()
        let expertNames = ["Native Alpha · 完整专家姓名与长期任务研究助理", "Native Beta · Independent Research Specialist"]
        app.buttons["experts.create"].tap()
        let firstExpertName = app.textFields["experts.name"]
        XCTAssertTrue(firstExpertName.waitForExistence(timeout: 5)); firstExpertName.tap(); firstExpertName.typeText(expertNames[0])
        let templates = app.buttons["experts.templates.manage"]
        for _ in 0..<5 where !templates.isHittable { app.collectionViews.firstMatch.swipeUp() }
        XCTAssertTrue(templates.isHittable); templates.tap()
        let addTemplate = app.buttons["prompt.templates.add"]
        XCTAssertTrue(addTemplate.waitForExistence(timeout: 5)); addTemplate.tap()
        let templateNameField = app.textFields["prompt.templates.editor.name"]
        XCTAssertTrue(templateNameField.waitForExistence(timeout: 5)); templateNameField.tap(); templateNameField.typeText(templateName)
        let templateBody = app.textViews["prompt.templates.editor.content"]
        for _ in 0..<5 where !templateBody.isHittable { app.collectionViews.firstMatch.swipeUp() }
        XCTAssertTrue(templateBody.isHittable); templateBody.tap(); templateBody.typeText(try XCTUnwrap(captureFixture["templateContent"]))
        capture("native-custom-template-created-in-editor", app)
        app.buttons["prompt.templates.editor.save"].tap()
        let savedTemplate = app.staticTexts[templateName].firstMatch
        for _ in 0..<5 where !savedTemplate.exists { app.collectionViews.firstMatch.swipeUp() }
        XCTAssertTrue(savedTemplate.waitForExistence(timeout: 5), "The real template directory must show the saved resource")
        capture("native-custom-template-saved-directory", app)
        XCTAssertTrue(app.buttons["experts.templates.done"].isHittable); app.buttons["experts.templates.done"].tap()
        XCTAssertEqual(app.textFields["experts.name"].value as? String, expertNames[0], "Template management must retain the expert edit draft")
        capture("native-template-library-return-kept-expert-draft", app)
        var expertIDs: [String] = []
        for (index, name) in expertNames.enumerated() {
            if index > 0 { app.buttons["experts.create"].tap() }
            let field = app.textFields["experts.name"]
            XCTAssertTrue(field.waitForExistence(timeout: 5))
            if index > 0 { field.tap(); field.typeText(name) }
            let templatePicker = app.buttons["experts.template"]
            for _ in 0..<5 where !templatePicker.isHittable { app.collectionViews.firstMatch.swipeUp() }
            XCTAssertTrue(templatePicker.isHittable); templatePicker.tap()
            let templateChoice = app.buttons.matching(NSPredicate(format: "label CONTAINS %@", templateName)).firstMatch
            let menu = try XCTUnwrap(app.collectionViews.allElementsBoundByIndex.filter(\.isHittable)
                .min(by: { $0.frame.width < $1.frame.width }), "The native template menu must be scrollable")
            for _ in 0..<8 where !templateChoice.exists || !templateChoice.isHittable { menu.swipeUp() }
            XCTAssertTrue(templateChoice.waitForExistence(timeout: 5)); templateChoice.tap()
            let save = app.buttons["experts.save"]
            for _ in 0..<5 where !save.isHittable { app.collectionViews.firstMatch.swipeUp() }
            capture("native-create-" + name.replacingOccurrences(of: " ", with: "-"), app)
            XCTAssertTrue(save.isHittable); save.tap()
            let row = app.buttons.matching(NSPredicate(format:
                "identifier BEGINSWITH %@ AND label CONTAINS %@", "experts.edit.", name)).firstMatch
            for _ in 0..<5 where !row.exists || !row.isHittable { app.collectionViews.firstMatch.swipeDown() }
            XCTAssertTrue(row.waitForExistence(timeout: 5))
            expertIDs.append(String(row.identifier.dropFirst("experts.edit.".count)))
        }
        app.buttons["experts.management.done"].tap()
        for id in expertIDs {
            let choice = app.buttons["experts.select." + id]
            XCTAssertTrue(choice.waitForExistence(timeout: 10)); choice.tap()
        }
        capture("native-two-experts-selected", app)
        app.buttons["Done"].firstMatch.tap()

        let nonce = "NATIVE_EXPERT_" + UUID().uuidString.replacingOccurrences(of: "-", with: "")
        let begin = nonce + "_BEGIN", end = nonce + "_END"
        let prompt = "Begin with \(begin). Write 48 numbered sentences about astronomy, each with 12 words. Finish with \(end). Do not use tools."
        let input = app.textViews["chat.composer.input"]
        XCTAssertTrue(input.waitForExistence(timeout: 5)); input.tap(); input.typeText(prompt)
        app.buttons["chat.composer.send"].tap()

        let bubbles = app.otherElements.matching(identifier: "chat.assistant.bubble")
        let deadline = Date().addingTimeInterval(180)
        var samples: [[String: Any]] = []
        var lastText = ""
        var grewBeforeTerminal = false
        while Date() < deadline {
            if app.descendants(matching: .any).matching(NSPredicate(format:
                "identifier BEGINSWITH %@", "room.submission.error.")).firstMatch.exists {
                capture("native-expert-production-failure", app)
                XCTFail("The production expert submission failed before incremental content")
                return
            }
            let current = bubbles.allElementsBoundByIndex.map { bubble in
                bubble.descendants(matching: .any)["chat.assistant.answer"]
                    .staticTexts.allElementsBoundByIndex.map(\.label).joined(separator: "\n")
            }.joined(separator: "\n")
            let running = bubbles.allElementsBoundByIndex.contains { ($0.value as? String) != "completed" }
            if current != lastText {
                samples.append(["stage": "before-cold", "characters": current.count,
                    "running": running, "hasBegin": current.contains(begin), "hasEnd": current.contains(end)])
                if lastText.contains(begin), current.count > lastText.count, running, !current.contains(end) {
                    grewBeforeTerminal = true
                    let visibleNames = app.staticTexts.matching(NSPredicate(format:
                        "identifier BEGINSWITH %@", "room.expert.name.")).allElementsBoundByIndex.filter(\.isHittable)
                    XCTAssertFalse(visibleNames.isEmpty, "The streaming expert must retain a full external identity")
                    for identity in visibleNames {
                        XCTAssertTrue(expertNames.contains(identity.label), "The name must be complete, never an ellipsis label")
                        XCTAssertEqual(app.staticTexts.matching(identifier: identity.identifier).count, 1, "A reply owns one visible full expert name")
                    }
                    capture("native-live-full-expert-name", app)
                    break
                }
                lastText = current
            }
            Thread.sleep(forTimeInterval: 0.5)
        }
        capture("native-experts-real-increment-before-cold", app)
        try write(samples, root: root, name: "native-ui-samples.json")
        XCTAssertTrue(grewBeforeTerminal, "Actual native expert text must grow before terminal; a final answer alone is insufficient.")
        let beforeCold = lastText
        app.terminate()
        app.launchEnvironment["NOLA_REAL_IOS_UI_PREPARE_CREDENTIAL"] = "0"
        app.launch()
        XCTAssertTrue(chatTab.waitForExistence(timeout: 15)); chatTab.tap()
        let rows = conversationRows(app)
        XCTAssertTrue(rows.firstMatch.waitForExistence(timeout: 10))
        let originalRowID = rows.firstMatch.identifier
        rows.firstMatch.tap()
        let finalDeadline = Date().addingTimeInterval(240)
        var completedPeers: [String: String] = [:]
        var completedTexts: [String: String] = [:]
        var page = 0
        while Date() < finalDeadline {
            if app.descendants(matching: .any).matching(NSPredicate(format:
                "identifier BEGINSWITH %@", "room.submission.error.")).firstMatch.exists {
                capture("native-expert-cold-production-failure", app)
                XCTFail("The production expert recovery failed")
                return
            }
            for row in expertReplyRows(app).allElementsBoundByIndex {
                let bubble = row.descendants(matching: .any)["chat.assistant.bubble"].firstMatch
                let name = row.staticTexts.matching(NSPredicate(format: "identifier BEGINSWITH %@", "room.expert.name.")).firstMatch
                if (bubble.value as? String) == "completed", name.exists,
                   row.staticTexts.allElementsBoundByIndex.contains(where: { $0.label.contains(end) }) {
                    completedPeers[row.identifier] = name.label
                    completedTexts[row.identifier] = bubble.descendants(matching: .any)["chat.assistant.answer"].firstMatch
                        .staticTexts.allElementsBoundByIndex.map(\.label).joined(separator: " ")
                    try write(completedTexts, root: root, name: "native-ui-complete-texts.json")
                }
            }
            if Set(completedPeers.values) == Set(expertNames), completedPeers.count == 2 { break }
            let timeline = app.scrollViews["chat.expert.timeline"].firstMatch
            dragReplies(timeline, earlier: page % 24 < 12)
            page += 1
        }
        capture("native-two-experts-cold-recovered-terminal", app)
        XCTAssertEqual(completedPeers.count, 2, "Both distinct canonical peer replies must be reachable through actual scrolling")
        XCTAssertEqual(Set(completedPeers.values), Set(expertNames))
        try verifyDraftIsolation(app, root: root, namespace: namespace, originalRowID: originalRowID)
        try write(["passed": true, "expertIDs": expertIDs, "originalRowID": originalRowID,
            "begin": begin, "end": end, "partialBeforeColdCharacters": beforeCold.count], root: root, name: "native-ui-result.json")
    }

    private struct PeerVerification: Decodable {
        struct Peer: Decodable { let messageID: String; let name: String; let text: String }
        let parentConversationID: String
        let peers: [Peer]
    }

    /// Continues a completed real-model fixture without issuing another model
    /// request. Every expected ID and text is an external current-Core read.


    private func dragReplies(_ timeline: XCUIElement, earlier: Bool) {
        timeline.coordinate(withNormalizedOffset: CGVector(dx: 0.8, dy: earlier ? 0.25 : 0.85))
            .press(forDuration: 0.05, thenDragTo: timeline.coordinate(withNormalizedOffset: CGVector(dx: 0.8, dy: earlier ? 0.85 : 0.25)))
    }

    private func normalized(_ text: String) -> String {
        text.split(whereSeparator: \.isWhitespace).joined(separator: " ")
    }

    private func expertReplyRows(_ app: XCUIApplication) -> XCUIElementQuery {
        app.descendants(matching: .any).matching(NSPredicate(format: "identifier BEGINSWITH %@", "room.expert.reply."))
    }

    private func verifyDraftIsolation(_ app: XCUIApplication, root: String, namespace: String, originalRowID: String) throws {
        let input = app.textViews["chat.composer.input"]
        let newConversation = app.buttons["chat.conversations.new"]
        let chatTab = app.tabBars.buttons["Chat"]
        let draftA = "UNSENT_A_" + namespace, draftB = "UNSENT_B_" + namespace
        input.tap(); input.typeText(draftA)
        backToList(app)
        newConversation.tap()
        XCTAssertTrue(input.waitForExistence(timeout: 5))
        XCTAssertFalse((input.value as? String ?? "").contains(draftA))
        input.tap(); input.typeText(draftB)
        backToList(app)
        let secondIDs = conversationRows(app).allElementsBoundByIndex.map(\.identifier).filter { $0 != originalRowID }
        XCTAssertEqual(secondIDs.count, 1)
        app.buttons[originalRowID].tap()
        XCTAssertEqual(input.value as? String, draftA)
        capture("native-original-owner-draft", app)
        backToList(app)
        app.buttons[secondIDs[0]].tap()
        XCTAssertEqual(input.value as? String, draftB)
        capture("native-other-owner-draft", app)
        app.terminate(); app.launch()
        XCTAssertTrue(chatTab.waitForExistence(timeout: 15)); chatTab.tap()
        app.buttons[originalRowID].tap()
        XCTAssertEqual(input.value as? String, draftA)
        XCTAssertTrue(app.scrollViews["chat.expert.timeline"].waitForExistence(timeout: 5))
        capture("native-experts-and-draft-restored", app)
    }

    private func conversationRows(_ app: XCUIApplication) -> XCUIElementQuery {
        app.buttons.matching(NSPredicate(format: "identifier MATCHES %@", "^chat\\.conversation\\.[0-9a-f-]{36}$"))
    }
    private func backToList(_ app: XCUIApplication) {
        app.navigationBars.buttons.element(boundBy: 0).tap()
        XCTAssertTrue(app.buttons["chat.conversations.new"].waitForExistence(timeout: 5))
    }
    private func capture(_ name: String, _ app: XCUIApplication) {
        let image = XCTAttachment(screenshot: app.screenshot()); image.name = name; image.lifetime = .keepAlways; add(image)
        let tree = XCTAttachment(string: app.debugDescription); tree.name = name + "-tree"; tree.lifetime = .keepAlways; add(tree)
    }
    private func write(_ value: Any, root: String, name: String) throws {
        try JSONSerialization.data(withJSONObject: value, options: [.prettyPrinted, .sortedKeys])
            .write(to: URL(fileURLWithPath: root).appendingPathComponent(name))
    }
}
