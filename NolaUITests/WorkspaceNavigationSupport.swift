import XCTest

extension XCUIApplication {
    private func recordModelSetupStage(_ stage: String) {
        guard let root = ProcessInfo.processInfo.environment["NOLA_UI_SETUP_EVIDENCE_ROOT"] else { return }
        let path = URL(fileURLWithPath: root).appendingPathComponent("setup-stages.ndjson")
        guard let data = try? JSONSerialization.data(withJSONObject: ["stage": stage,
            "time": Date().timeIntervalSince1970]) else { return }
        if !FileManager.default.fileExists(atPath: path.path) { FileManager.default.createFile(atPath: path.path, contents: nil) }
        guard let handle = try? FileHandle(forWritingTo: path) else { return }
        defer { try? handle.close() }
        try? handle.seekToEnd(); try? handle.write(contentsOf: data + Data([10]))
    }

    func openWorkspaceManagement(_ destination: String) {
        let close = buttons["workspace.management.close"]
        if close.exists { close.tap() }
        if textViews["chat.composer.input"].exists, !tabBars.firstMatch.exists {
            // Conversations deliberately hide the tab bar. Return through the
            // actual native back button before choosing another root tab.
            let compactBack = buttons["chat.navigation.back"]
            if compactBack.exists { compactBack.tap() }
            else { navigationBars.buttons.element(boundBy: 0).tap() }
            XCTAssertTrue(buttons["chat.conversations.new"].waitForExistence(timeout: 5))
        }
        if destination == "settings" || destination == "models" {
            let compact = buttons["main.tab.settings"]
            let settings = tabBars.buttons["Settings"]
            if compact.exists { compact.tap() }
            else if settings.exists { settings.tap() }
            else { tabBars.buttons["设置"].tap() }
            if destination == "models" { buttons["settings.models"].tap() }
        } else {
            let compact = buttons["main.tab.information"]
            let information = tabBars.buttons["Information"]
            if compact.exists { compact.tap() }
            else if information.exists { information.tap() }
            else { tabBars.buttons["信息"].tap() }
            let entry = buttons["information.\(destination)"]
            XCTAssertTrue(entry.waitForExistence(timeout: 5))
            entry.tap()
        }
    }

    func openModelCenterFromSettings() {
        openWorkspaceManagement("settings")
        let models = buttons["settings.models"]
        XCTAssertTrue(models.waitForExistence(timeout: 5))
        for _ in 0..<4 where !models.isHittable { swipeUp() }
        models.tap()
    }
    func configureServiceModel(endpoint: String, modelID: String, wireProtocol: String? = nil,
        beforeSave: (() -> Void)? = nil, fillCredential: ((XCUIElement) -> Void)? = nil) {
        let address = textFields["model.setup.endpoint"]
        XCTAssertTrue(address.waitForExistence(timeout: 5))
        recordModelSetupStage("endpoint.tap")
        address.tap()
        recordModelSetupStage("endpoint.type")
        address.typeText(endpoint)
        recordModelSetupStage("endpoint.typed")
        fillCredential?(secureTextFields["model.setup.key"])
        recordModelSetupStage("credential.finished")
        let model = textFields["model.setup.modelID"]
        model.tap()
        recordModelSetupStage("model.focused")
        model.typeText(modelID)
        recordModelSetupStage("model.typed")
        beforeSave?()
        if let wireProtocol, wireProtocol != "chat_completions" {
            let labels = ["responses": "Responses", "anthropic": "Anthropic Messages"]
            guard let label = labels[wireProtocol] else {
                XCTFail("Unsupported explicitly requested test protocol")
                return
            }
            let advanced = buttons["Advanced settings"]
            for _ in 0..<4 where !advanced.isHittable { swipeUp() }
            recordModelSetupStage("advanced.open")
            advanced.tap()
            let menu = buttons.matching(NSPredicate(format: "label CONTAINS %@", "Chat Completions")).firstMatch
            XCTAssertTrue(menu.waitForExistence(timeout: 5))
            menu.tap()
            buttons[label].tap()
        }
        let save = buttons["model.setup.save"]
        for _ in 0..<4 where !save.isHittable { swipeUp() }
        XCTAssertTrue(save.isEnabled)
        recordModelSetupStage("save.submit")
        save.tap()
        recordModelSetupStage("save.submitted")
        XCTAssertTrue(staticTexts["Saved and set as default · not tested"].waitForExistence(timeout: 10))
        navigationBars.buttons.element(boundBy: 0).tap()
        recordModelSetupStage("setup.finished")
    }

}
