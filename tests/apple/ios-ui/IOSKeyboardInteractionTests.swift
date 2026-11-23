// SPDX-License-Identifier: Apache-2.0
import XCTest
import UIKit
import Darwin

@MainActor
final class IOSKeyboardInteractionTests: XCTestCase {
    private func launch() -> XCUIApplication {
        continueAfterFailure = false
        let app = XCUIApplication()
        app.launchArguments = ["-ui-testing", "-AppleLanguages", "(en)", "-AppleLocale", "en_US"]
        app.launchEnvironment["NOLA_UI_TEST_RUN_ID"] = "keyboard-" + UUID().uuidString
        app.launch()
        return app
    }

    private func capture(_ name: String, _ app: XCUIApplication) {
        let image = XCTAttachment(screenshot: app.screenshot())
        image.name = name
        image.lifetime = .keepAlways
        add(image)
    }

    private func tapBlank(_ app: XCUIApplication, x: CGFloat, y: CGFloat, name: String) {
        let keyboard = app.keyboards.firstMatch
        XCTAssertTrue(keyboard.waitForExistence(timeout: 4), name)
        capture(name + "-editing", app)
        app.coordinate(withNormalizedOffset: CGVector(dx: x, dy: y)).tap()
        let hidden = XCTNSPredicateExpectation(predicate: NSPredicate(format: "exists == false"), object: keyboard)
        XCTAssertEqual(XCTWaiter.wait(for: [hidden], timeout: 5), .completed, name)
        capture(name + "-dismissed", app)
    }

    func testBlankTapPreservesSearchAndConversationDrafts() {
        let app = launch()
        defer { app.terminate() }
        app.tapRootTab("chat")
        let search = app.textFields["chat.conversations.search"]
        XCTAssertTrue(search.waitForExistence(timeout: 8))
        search.tap()
        search.typeText("unmatched")
        XCTAssertTrue(app.staticTexts["No matching conversations"].waitForExistence(timeout: 5))
        tapBlank(app, x: 0.92, y: 0.43, name: "conversation-search")
        XCTAssertEqual(search.value as? String, "unmatched")
        app.buttons["chat.conversations.new"].tap()
        let input = app.textViews["chat.composer.input"]
        XCTAssertTrue(input.waitForExistence(timeout: 5), "One tap on New opens a writable conversation")
        input.tap()
        input.typeText("Keep this draft")
        tapBlank(app, x: 0.93, y: 0.32, name: "chat-draft")
        XCTAssertEqual(input.value as? String, "Keep this draft")
    }

    private func bindLoopbackPort() throws -> (socket: Int32, address: String) {
        let fd = socket(AF_INET, SOCK_STREAM, 0)
        guard fd >= 0 else { throw POSIXError(.ENFILE) }
        var local = sockaddr_in()
        local.sin_family = sa_family_t(AF_INET)
        local.sin_len = UInt8(MemoryLayout<sockaddr_in>.size)
        local.sin_addr.s_addr = inet_addr("127.0.0.1")
        var size = socklen_t(MemoryLayout<sockaddr_in>.size)
        let bound = withUnsafeMutablePointer(to: &local) { pointer in
            pointer.withMemoryRebound(to: sockaddr.self, capacity: 1) { address in
                Darwin.bind(fd, address, size) == 0 && getsockname(fd, address, &size) == 0
            }
        }
        guard bound else { Darwin.close(fd); throw POSIXError(.EADDRNOTAVAIL) }
        return (fd, "http://127.0.0.1:" + String(UInt16(bigEndian: local.sin_port)))
    }

    func testCancelSlowVerificationPreservesHostDraftInSettings() throws {
        let (socketFD, addressText) = try bindLoopbackPort()
        defer { Darwin.close(socketFD) }
        // Accept the TCP handshake but send no HTTP reply until cancellation.
        XCTAssertEqual(listen(socketFD, 1), 0)
        let app = launch()
        defer { app.terminate() }
        app.openModelCenterFromSettings()
        app.buttons["model.setup.source.2"].tap()
        let address = app.textFields["model.web_host.url_field"]
        let token = app.secureTextFields["model.web_host.token_field"]
        address.tap()
        address.typeText(addressText + "\n")
        token.typeText("dummy-token\n")
        let cancel = app.buttons["model.web_host.cancel_verification"]
        XCTAssertTrue(cancel.waitForExistence(timeout: 5))
        XCTAssertFalse(address.isEnabled)
        capture("verification-in-progress", app)
        cancel.tap()
        let connect = app.buttons["model.web_host.connect"]
        XCTAssertTrue(connect.waitForExistence(timeout: 4))
        XCTAssertTrue(connect.isEnabled)
        XCTAssertTrue(address.isEnabled)
        XCTAssertEqual(address.value as? String, addressText)
        XCTAssertNotEqual(token.value as? String, token.placeholderValue)
        XCTAssertFalse(app.alerts.firstMatch.exists)
        XCTAssertFalse(app.descendants(matching: .any)["model.web_host.validation_error"].firstMatch.exists)
        capture("cancelled-verification-retains-draft", app)
    }

    func testManualAddressEditDropsPastedTokenUntilNewSetupIsProvided() throws {
        let original = "http://127.0.0.1:1"
        let data = try JSONSerialization.data(withJSONObject: ["type": "nola.server.connection", "protocol_version": 1,
            "origin": original, "host_id": "nola-fixture-host", "access_token": "dummy-fixture-token"] as [String: Any])
        let app = launch()
        defer { app.terminate() }
        app.openModelCenterFromSettings()
        app.buttons["model.setup.source.2"].tap()
        let paste = app.buttons["model.web_host.paste_setup"]
        let address = app.textFields["model.web_host.url_field"]
        let token = app.secureTextFields["model.web_host.token_field"]
        XCUIDevice.shared.press(.home)
        UIPasteboard.general.setItems([["public.utf8-plain-text": String(decoding: data, as: UTF8.self)]],
                                      options: [.localOnly: true, .expirationDate: Date().addingTimeInterval(90)])
        let change = UIPasteboard.general.changeCount
        defer { if UIPasteboard.general.changeCount == change { UIPasteboard.general.setItems([], options: [.localOnly: true]) } }
        app.activate()
        let pasteReady = XCTNSPredicateExpectation(predicate: NSPredicate(format: "enabled == true AND hittable == true"), object: paste)
        XCTAssertEqual(XCTWaiter.wait(for: [pasteReady], timeout: 5), .completed)
        paste.tap()
        let pasted = XCTNSPredicateExpectation(predicate: NSPredicate(format: "value == %@", original), object: address)
        XCTAssertEqual(XCTWaiter.wait(for: [pasted], timeout: 5), .completed)
        let filled = try XCTUnwrap(token.value as? String)
        XCTAssertNotEqual(filled, token.placeholderValue)
        address.tap()
        address.typeText("2\n")
        XCTAssertEqual(address.value as? String, original + "2")
        XCTAssertNotEqual(token.value as? String, filled)
        let go = app.keyboards.buttons["Go"]
        if go.exists, go.isHittable, app.frame.contains(go.frame) {
            go.tap()
        } else {
            // With a hardware keyboard the secure field owns focus while the
            // software keyboard's accessibility nodes remain below the window.
            token.typeText("\n")
        }
        let error = app.descendants(matching: .any)["model.web_host.validation_error"].firstMatch
        XCTAssertTrue(error.waitForExistence(timeout: 5))
        XCTAssertTrue(error.label.contains("Enter the access token for this host"), error.label)
        capture("edited-endpoint-needs-own-token", app)
        paste.tap()
        let restored = XCTNSPredicateExpectation(predicate: NSPredicate(format: "value == %@", original), object: address)
        XCTAssertEqual(XCTWaiter.wait(for: [restored], timeout: 5), .completed)
        XCTAssertEqual(token.value as? String, filled)
    }

    func testConnectFocusesTheFieldThatNeedsCorrection() {
        let app = launch()
        defer { app.terminate() }
        app.tapRootTab("settings")
        app.buttons["settings.category.agent"].tap()
        app.buttons["Plugins"].tap()
        app.buttons["intelligence.harness.plugins"].tap()
        let connect = app.buttons["model.web_host.connect"]
        let address = app.textFields["model.web_host.url_field"]
        let token = app.secureTextFields["model.web_host.token_field"]
        XCTAssertTrue(connect.waitForExistence(timeout: 6))
        connect.tap()
        XCTAssertTrue(app.keyboards.firstMatch.waitForExistence(timeout: 4))
        app.typeText("http://127.0.0.1")
        XCTAssertEqual(address.value as? String, "http://127.0.0.1")
        connect.tap()
        let error = app.descendants(matching: .any)["model.web_host.validation_error"].firstMatch
        XCTAssertTrue(error.label.contains("Enter the access token"), error.label)
        app.typeText("fixture")
        XCTAssertEqual(address.value as? String, "http://127.0.0.1")
        XCTAssertNotEqual(token.value as? String, token.placeholderValue)
        capture("host-correct-token", app)
    }

    func testBlankTapAndFocusChangesInHostSheet() throws {
        // This test owns the port and never listens, so the connection must fail.
        let (fd, addressText) = try bindLoopbackPort()
        defer { Darwin.close(fd) }
        let app = launch()
        defer { app.terminate() }
        app.tapRootTab("settings")
        app.buttons["settings.category.agent"].tap()
        app.buttons["Plugins"].tap()
        app.buttons["intelligence.harness.plugins"].tap()
        let address = app.textFields["model.web_host.url_field"]
        let token = app.secureTextFields["model.web_host.token_field"]
        XCTAssertTrue(address.waitForExistence(timeout: 6))
        address.tap()
        address.typeText(addressText + "\n")
        token.typeText("fixture")
        XCTAssertTrue(app.keyboards.firstMatch.exists, "Next keeps the keyboard open for the token")
        let maskedToken = token.value as? String
        XCTAssertFalse(maskedToken?.isEmpty ?? true)
        let section = app.staticTexts["Connection"].firstMatch
        XCTAssertTrue(section.isHittable)
        // The bottom of a secure form can be covered by the system keyboard.
        // Tap the blank trailing space in the visible section header instead.
        tapBlank(app, x: 0.85, y: (section.frame.midY - app.frame.minY) / app.frame.height, name: "host-sheet")
        XCTAssertEqual(address.value as? String, addressText)
        XCTAssertEqual(token.value as? String, maskedToken)
        app.buttons["model.web_host.connect"].tap()
        let failure = app.descendants(matching: .any)["model.web_host.validation_error"].firstMatch
        XCTAssertTrue(failure.waitForExistence(timeout: 20), "Connection failure must leave an editable form")
        XCTAssertTrue(app.buttons["model.web_host.connect"].isEnabled)
        XCTAssertEqual(address.value as? String, addressText)
        capture("host-sheet-retry", app)
    }

    func testScrollInExpertEditorWithTheKeyboardOpen() {
        let app = launch()
        defer { app.terminate() }
        app.tapRootTab("settings")
        app.buttons["settings.category.agents"].tap()
        app.buttons["experts.template"].tap()
        app.buttons["Code writer"].tap()
        let details = app.buttons["experts.role.details"]
        for _ in 0..<4 where !details.isHittable { app.swipeUp() }
        details.tap()
        let name = app.textFields["experts.name"]
        for _ in 0..<6 where !name.isHittable { app.swipeDown() }
        name.tap()
        name.typeText(" audit")
        let draft = name.value as? String
        XCTAssertTrue(draft?.contains("audit") == true)
        let keyboard = app.keyboards.firstMatch
        XCTAssertTrue(keyboard.waitForExistence(timeout: 4))
        capture("expert-editor-before-scroll", app)
        let origin = app.coordinate(withNormalizedOffset: .zero)
        let x = app.frame.midX
        // The expanded role includes its prompt, tool pack, and skills below the editor.
        let high = origin.withOffset(CGVector(dx: x, dy: 178))
        let tab = app.rootTabButton("settings")
        let listBottom = tab.exists && tab.isHittable ? tab.frame.minY : app.frame.maxY
        let low = origin.withOffset(CGVector(dx: x, dy: min(keyboard.frame.minY - 116, listBottom - 60)))
        low.press(forDuration: 0.09, thenDragTo: high)
        capture("expert-editor-after-scroll", app)
        let hidden = XCTNSPredicateExpectation(predicate: NSPredicate(format: "exists == false"), object: keyboard)
        XCTAssertEqual(XCTWaiter.wait(for: [hidden], timeout: 5), .completed)
        let save = app.buttons["experts.save"]
        for _ in 0..<12 where !save.isHittable { app.swipeUp() }
        XCTAssertTrue(save.isHittable, "Scrolling must reveal the action below the expanded role")
        XCTAssertFalse(name.isHittable)
        capture("expert-editor-role-scrolled", app)
        for _ in 0..<12 where !name.isHittable { app.swipeDown() }
        XCTAssertEqual(name.value as? String, draft)
        name.tap()
        XCTAssertTrue(keyboard.waitForExistence(timeout: 4))
        let done = app.buttons["experts.keyboard.done"]
        XCTAssertTrue(done.isHittable)
        done.tap()
        XCTAssertTrue(keyboard.waitForNonExistence(timeout: 4))
        XCTAssertEqual(name.value as? String, draft)
    }

    private func hostFixture() throws -> (String, String) {
        let env = ProcessInfo.processInfo.environment
        guard let url = env["NOLA_TEST_PAIRING_URL"], let token = env["NOLA_TEST_PAIRING_TOKEN"],
              URL(string: url)?.host == "127.0.0.1", !token.isEmpty else {
            throw XCTSkip("Requires a disposable loopback Nola Host and its temporary token")
        }
        return (url, token)
    }

    func testCapabilityLibraryPairsAndReturnsFromLiveHostManagement() throws {
        let (url, tokenValue) = try hostFixture()
        let app = launch()
        defer { app.terminate() }
        app.tapRootTab("settings")
        app.buttons["settings.category.agent"].tap()
        let plugins = app.buttons["Plugins"]
        plugins.tap()
        app.buttons["intelligence.harness.plugins"].tap()
        let address = app.textFields["model.web_host.url_field"]
        let token = app.secureTextFields["model.web_host.token_field"]
        address.tap()
        address.typeText(url + "\n")
        token.typeText(tokenValue + "\n")

        let web = app.webViews.firstMatch
        let inventory = web.descendants(matching: .any).matching(NSPredicate(
            format: "label CONTAINS[c] %@ OR label CONTAINS %@ OR label CONTAINS %@ OR label CONTAINS %@",
            "No plugins yet", "还没有插件", "Install Pi or DeepSeek Harness", "安装 Pi 或 DeepSeek Harness"
        )).firstMatch
        let host = app.buttons.matching(NSPredicate(format: "identifier BEGINSWITH %@ AND label CONTAINS %@",
            "intelligence.harness.host.", url)).firstMatch
        for attempt in 0..<2 {
            if attempt > 0 { host.tap() }
            let inventoryLoaded = inventory.waitForExistence(timeout: 60)
            capture("live-plugin-library-" + String(attempt + 1), app)
            XCTAssertTrue(inventoryLoaded, "The inventory must come from the authenticated Host page")
            let close = app.buttons.matching(NSPredicate(format: "label IN %@", ["Done", "完成", "Close", "关闭"])).firstMatch
            XCTAssertTrue(close.waitForExistence(timeout: 5))
            close.tap()
            XCTAssertTrue(host.waitForExistence(timeout: 5))
            XCTAssertTrue(plugins.isSelected)
            XCTAssertFalse(address.exists)
        }
        capture("returned-to-paired-host", app)

        // Remove this run’s loopback identity and saved token through the UI.
        app.buttons["intelligence.harness.plugins"].tap()
        address.tap()
        address.typeText(url)
        app.coordinate(withNormalizedOffset: CGVector(dx: 0.90, dy: 0.13)).tap()
        let forget = app.buttons["model.web_host.forget_pairing"]
        for _ in 0..<3 where !forget.isHittable { app.swipeUp() }
        XCTAssertTrue(forget.isHittable)
        forget.tap()
        app.alerts["Forget this host pairing?"].buttons["Forget pairing"].tap()
        XCTAssertFalse(forget.exists)
    }

    func testFailedHostCredentialsCanBeCorrectedAgainstIsolatedServer() throws {
        let (fixture, tokenValue) = try hostFixture()
        let url = fixture.replacingOccurrences(of: "127.0.0.1", with: "localhost")
        UIPasteboard.general.setItems([], options: [.localOnly: true])
        let app = launch()
        defer { app.terminate() }
        app.openModelCenterFromSettings()
        app.buttons["model.setup.source.2"].tap()
        let address = app.textFields["model.web_host.url_field"]
        let token = app.secureTextFields["model.web_host.token_field"]
        XCTAssertTrue(address.waitForExistence(timeout: 5))
        let firstSteps = app.buttons["model.web_host.first_steps"]
        let connect = app.buttons["model.web_host.connect"]
        XCTAssertTrue(firstSteps.exists)
        let setupStep = app.staticTexts.containing(NSPredicate(format: "label CONTAINS[c] %@", "Copy Connection Setup")).firstMatch
        XCTAssertFalse(setupStep.exists)
        firstSteps.tap()
        XCTAssertTrue(setupStep.waitForExistence(timeout: 3))
        firstSteps.tap()
        XCTAssertFalse(setupStep.exists)
        XCTAssertEqual(connect.label, "Connect")
        let paste = app.buttons["model.web_host.paste_setup"]
        XCTAssertTrue(paste.exists, "The native paste action must stay visible on an empty clipboard")
        capture("host-empty-clipboard", app)
        let initialAddress = address.value as? String
        if paste.isEnabled { paste.tap() }
        XCTAssertEqual(address.value as? String, initialAddress)
        XCTAssertFalse(app.descendants(matching: .any)["model.web_host.checking"].firstMatch.exists)
        // Returning from the source app is part of copying desktop setup to Nola.
        XCUIDevice.shared.press(.home)
        UIPasteboard.general.setItems([["public.utf8-plain-text": "not a connection setup"]],
                                      options: [.localOnly: true, .expirationDate: Date().addingTimeInterval(60)])
        app.activate()
        let enabled = XCTNSPredicateExpectation(predicate: NSPredicate(format: "enabled == true"), object: paste)
        XCTAssertEqual(XCTWaiter.wait(for: [enabled], timeout: 5), .completed)
        paste.tap()
        let validation = app.descendants(matching: .any)["model.web_host.validation_error"].firstMatch
        XCTAssertTrue(validation.waitForExistence(timeout: 5))
        capture("host-invalid-clipboard", app)
        address.tap()
        address.typeText(url)
        tapBlank(app, x: 0.91, y: 0.25, name: "host-model-settings")
        XCTAssertEqual(address.value as? String, url)
        for _ in 0..<3 where !connect.isHittable { app.swipeUp() }
        XCTAssertEqual(connect.label, "Connect")
        token.tap()
        token.typeText("incorrect")
        if !connect.isHittable { app.keyboards.buttons["Go"].tap() } else { connect.tap() }
        let failure = app.descendants(matching: .any)["model.web_host.validation_error"].firstMatch
        XCTAssertTrue(failure.waitForExistence(timeout: 12))
        capture("host-rejected-token", app)
        let hostID = try XCTUnwrap(ProcessInfo.processInfo.environment["NOLA_TEST_PAIRING_ID"])
        let setup: [String: Any] = ["type": "nola.server.connection", "protocol_version": 1,
                                     "origin": url, "host_id": hostID, "access_token": tokenValue]
        let json = try JSONSerialization.data(withJSONObject: setup)
        XCUIDevice.shared.press(.home)
        UIPasteboard.general.setItems([["public.utf8-plain-text": String(decoding: json, as: UTF8.self)]],
                                      options: [.localOnly: true, .expirationDate: Date().addingTimeInterval(90)])
        app.activate()
        for _ in 0..<3 where !paste.isHittable { app.swipeDown() }
        paste.tap()
        let system = XCUIApplication(bundleIdentifier: "com.apple.springboard")
        let allow = system.buttons.matching(NSPredicate(format: "label IN %@", ["Allow Paste", "允许粘贴"])).firstMatch
        XCTAssertFalse(allow.exists, "The explicit system paste gesture must not need a second confirmation")
        XCTAssertFalse(failure.exists)
        for _ in 0..<3 where !connect.isHittable { app.swipeUp() }
        connect.tap()
        let verified = app.descendants(matching: .any)["model.web_host.verified"].firstMatch
        XCTAssertTrue(verified.waitForExistence(timeout: 12), "Retry must reach verified state on the same Host form")
        XCTAssertFalse(failure.exists)
        XCTAssertEqual(address.value as? String, url)
        XCTAssertEqual(connect.label, "Verify again")
        XCTAssertFalse(firstSteps.exists)
        connect.tap()
        XCTAssertTrue(verified.waitForExistence(timeout: 12), "Verify again must authenticate with the saved credential")
        XCTAssertFalse(failure.exists)
        capture("host-pairing-verified", app)
        let forget = app.buttons["model.web_host.forget_pairing"]
        for _ in 0..<4 where !forget.isHittable { app.swipeUp() }
        forget.tap()
        let confirmation = app.alerts["Forget this host pairing?"]
        XCTAssertTrue(confirmation.waitForExistence(timeout: 5))
        confirmation.buttons["Cancel"].firstMatch.tap()
        XCTAssertTrue(forget.isEnabled)
        forget.tap()
        confirmation.buttons["Forget pairing"].firstMatch.tap()
        let forgotten = XCTNSPredicateExpectation(predicate: NSPredicate(format: "exists == false"), object: forget)
        XCTAssertEqual(XCTWaiter.wait(for: [forgotten], timeout: 5), .completed)
        XCTAssertEqual(connect.label, "Connect")
        XCTAssertTrue(firstSteps.exists)
        connect.tap()
        XCTAssertTrue(failure.waitForExistence(timeout: 8), "Forgetting removes the stored pairing credential")
    }
}
