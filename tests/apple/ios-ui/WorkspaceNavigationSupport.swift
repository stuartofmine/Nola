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

    func rootTabButton(_ tab: String) -> XCUIElement {
        let compact = buttons["main.tab.\(tab)"]
        if compact.exists { return compact.firstMatch }

        let byID = buttons.matching(identifier: "tab.\(tab)")
        if byID.count > 0 {
            let children = byID.children(matching: .button).matching(identifier: "tab.\(tab)")
            return children.count > 0 ? children.firstMatch : byID.firstMatch
        }

        let localizedTitles: [String]
        switch tab {
        case "chat": localizedTitles = ["Chat", "对话"]
        case "activity": localizedTitles = ["Activity", "动态"]
        case "information": localizedTitles = ["Library", "Information", "资料库", "信息"]
        case "settings": localizedTitles = ["Settings", "设置"]
        default: localizedTitles = []
        }
        for title in localizedTitles {
            let tabBarButton = tabBars.buttons.matching(identifier: title)
            if tabBarButton.count > 0 { return tabBarButton.firstMatch }
            let button = buttons.matching(NSPredicate(format: "label == %@", title))
            if button.count > 0 { return button.firstMatch }
        }
        return compact.firstMatch
    }

    @discardableResult
    func waitForRootTab(_ tab: String, timeout: TimeInterval = 10) -> XCUIElement {
        let deadline = Date().addingTimeInterval(timeout)
        repeat {
            let compact = buttons["main.tab.\(tab)"].firstMatch
            if compact.exists { return compact }
            if buttons.matching(identifier: "tab.\(tab)").count > 0 {
                return rootTabButton(tab)
            }
            let fallback = rootTabButton(tab)
            if fallback.exists { return fallback }
            RunLoop.current.run(until: Date().addingTimeInterval(0.15))
        } while Date() < deadline
        let finalTarget = rootTabButton(tab)
        XCTAssertTrue(finalTarget.exists, "Missing root tab \(tab)")
        return finalTarget
    }

    func tapRootTab(_ tab: String, timeout: TimeInterval = 10) {
        waitForRootTab(tab, timeout: timeout).tap()
    }

    func openWorkspaceManagement(_ destination: String) {
        let close = buttons["workspace.management.close"]
        if close.exists { close.tap() }
        let rootTabVisible = buttons["main.tab.settings"].exists ||
            buttons.matching(identifier: "tab.settings").count > 0
        if textViews["chat.composer.input"].exists, !rootTabVisible {
            // Conversations deliberately hide the tab bar. Return through the
            // actual native back button before choosing another root tab.
            let compactBack = buttons["chat.navigation.back"]
            if compactBack.exists { compactBack.tap() }
            else { navigationBars.buttons.element(boundBy: 0).tap() }
            XCTAssertTrue(buttons["chat.conversations.new"].waitForExistence(timeout: 5))
        }
        if destination == "settings" || destination == "models" {
            tapRootTab("settings")
            if destination == "models" { buttons["settings.models"].tap() }
        } else {
            tapRootTab("information")
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
            let menu = buttons["model.setup.protocol"]
            for _ in 0..<4 where !menu.isHittable { swipeUp() }
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
