// SPDX-License-Identifier: Apache-2.0
import XCTest
import SwiftUI
import AppKit
@testable import Nola

final class MacSiwcAccountBridgeTests: XCTestCase {
    private nonisolated static func read(_ file: FileHandle) throws -> [String: Any] {
        var bytes = Data()
        while let byte = try file.read(upToCount: 1), !byte.isEmpty {
            if byte.first == 10 { return try XCTUnwrap(JSONSerialization.jsonObject(with: bytes) as? [String: Any]) }
            bytes.append(byte)
        }
        throw NSError(domain: "fixture", code: 1)
    }
    private nonisolated static func write(_ frame: [String: Any], _ file: FileHandle) throws {
        var bytes = try JSONSerialization.data(withJSONObject: frame); bytes.append(10)
        try file.write(contentsOf: bytes)
    }
    func testPrivatePipeBindsOwnerAndNeverAcceptsForeignFlowOrAuthorizationURL() async throws {
        for variant in ["valid", "owner", "url"] {
            let input = Pipe(), output = Pipe()
            let bridge = MacSiwcAccountBridge(fixtureInput: input.fileHandleForWriting, fixtureOutput: output.fileHandleForReading)
            defer { bridge.close() }
            let owner = UUID().uuidString.lowercased(), account = UUID().uuidString.lowercased()
            let reader = input.fileHandleForReading, writer = output.fileHandleForWriting
            let worker = Task.detached {
                let hello = try Self.read(reader)
                try Self.write(["type": "hello", "request_id": hello["request_id"]!, "owner_id": owner, "protocol": 1], writer)
                let request = try Self.read(reader)
                let body = try XCTUnwrap(request["request"] as? [String: Any])
                XCTAssertEqual(body["account_id"] as? String, account)
                XCTAssertEqual(body["acknowledge_authorization"] as? Bool, true)
                try Self.write(["type": "flow", "request_id": request["request_id"]!, "flow": [
                    "owner_id": variant == "owner" ? UUID().uuidString : owner,
                    "account_id": account, "flow_id": UUID().uuidString, "expected_account_revision": NSNull(),
                    "phase": "awaiting_authorization", "authorization_url": variant == "url" ? "https://evil.invalid/authorize" : "https://auth.openai.com/authorize?state=fixture",
                    "error": NSNull(), "private_session_may_be_retained": false,
                    "account": NSNull(), "inference_performed": false]], writer)
            }
            do {
                let flow = try await bridge.start(accountID: account, revision: nil, name: "Fixture")
                XCTAssertEqual(variant, "valid")
                XCTAssertEqual(flow.account_id, account)
            } catch { XCTAssertNotEqual(variant, "valid") }
            try await worker.value
        }
    }
    func testBrowserURLRejectsCredentialUserInfoAndNonHTTPS() {
        for url in ["http://auth.openai.com/authorize", "https://auth.openai.com.evil.invalid/authorize",
                    "https://user@auth.openai.com/authorize", "https://auth.openai.com:444/authorize"] {
            XCTAssertNil(MacSiwcAccountBridge.authorizationURL(url))
        }
    }
    func testErrorsUseFixedAllowlistWithoutEchoingSourceContent() {
        XCTAssertNotEqual(MacSiwcBridgeFailure.sanitized("native_siwc_refresh_result_unknown").localizedDescription,
            MacSiwcBridgeFailure.sanitized("native_siwc_core_commit_conflict_private_session_retained").localizedDescription)
        XCTAssertEqual(MacSiwcBridgeFailure.sanitized("token=private-provider-response").localizedDescription,
            MacSiwcBridgeFailure.unavailable.localizedDescription)
        XCTAssertFalse(MacSiwcBridgeFailure.sanitized("token=private-provider-response").localizedDescription.contains("token="))
    }
    func testManagementUsesExactAccountAndSessionRevisionWithoutInference() async throws {
        let input = Pipe(), output = Pipe()
        let bridge = MacSiwcAccountBridge(fixtureInput: input.fileHandleForWriting, fixtureOutput: output.fileHandleForReading)
        defer { bridge.close() }
        let id = UUID().uuidString.lowercased(), owner = UUID().uuidString.lowercased()
        let account = MacModelAccountSnapshot(revision: 7, account: .init(id: id, display_name: "Fixture",
            source: "nola_chatgpt", credential_ref: "siwc:" + id, state: "ready"))
        let reader = input.fileHandleForReading, writer = output.fileHandleForWriting
        let worker = Task.detached {
            let hello = try Self.read(reader)
            try Self.write(["type": "hello", "request_id": hello["request_id"]!, "owner_id": owner, "protocol": 1], writer)
            let frame = try Self.read(reader)
            XCTAssertEqual(frame["type"] as? String, "sign_out")
            let body = try XCTUnwrap(frame["request"] as? [String: Any])
            XCTAssertEqual(body["account_id"] as? String, id)
            XCTAssertEqual(body["expected_account_revision"] as? Int, 7)
            XCTAssertEqual(body["expected_session_revision"] as? Int, 4)
            XCTAssertEqual(body["acknowledge_sign_out"] as? Bool, true)
            let value = try JSONSerialization.jsonObject(with: JSONEncoder().encode(account))
            try Self.write(["type": "account", "request_id": frame["request_id"]!, "result": [
                "owner_id": owner, "account_id": id, "account": value, "connection": NSNull(),
                "session": ["account_id": id, "session_revision": 5, "signed_in": false,
                    "plan_usage_authorized": false, "reauthorization_required": true, "revocation_unconfirmed": true],
                "inference_performed": false]], writer)
        }
        let result = try await bridge.manage("sign_out", account: account, sessionRevision: 4, acknowledgeSignOut: true)
        XCTAssertEqual(result.session?.signed_in, false)
        XCTAssertEqual(result.session?.revocation_unconfirmed, true)
        try await worker.value
    }
    func testCatalogUsesFreshRevisionsAndRejectsForeignOrUnsupportedConnections() async throws {
        for variant in ["valid", "foreign", "version"] {
            let input = Pipe(), output = Pipe()
            let bridge = MacSiwcAccountBridge(fixtureInput: input.fileHandleForWriting, fixtureOutput: output.fileHandleForReading)
            defer { bridge.close() }
            let id = UUID().uuidString.lowercased(), owner = UUID().uuidString.lowercased()
            let account = MacModelAccountSnapshot(revision: 7, account: .init(id: id, display_name: "Fixture",
                source: "nola_chatgpt", credential_ref: "siwc:" + id, state: "ready"))
            let updated = MacModelAccountSnapshot(revision: 8, account: account.account)
            let reader = input.fileHandleForReading, writer = output.fileHandleForWriting
            let worker = Task.detached {
                let hello = try Self.read(reader)
                try Self.write(["type": "hello", "request_id": hello["request_id"]!, "owner_id": owner, "protocol": 1], writer)
                for action in ["account_status", "models"] {
                    let frame = try Self.read(reader)
                    XCTAssertEqual(frame["type"] as? String, action)
                    let body = try XCTUnwrap(frame["request"] as? [String: Any])
                    XCTAssertEqual(body["expected_account_revision"] as? Int, action == "models" ? 8 : 7)
                    if action == "models" { XCTAssertEqual(body["expected_session_revision"] as? Int, 11) }
                    else { XCTAssertTrue(body["expected_session_revision"] is NSNull) }
                    let connection: [String: Any] = ["revision": 1, "document": ["format_version": variant == "version" ? 2 : 1,
                        "connection": ["id": "siwc-" + id, "display_name": "Fixture", "base_url": "https://chatgpt.com",
                            "protocols": ["siwc_responses"], "account_id": variant == "foreign" ? UUID().uuidString : id,
                            "source": "nola_chatgpt", "enabled": false]]]
                    try Self.write(["type": "account", "request_id": frame["request_id"]!, "result": [
                        "owner_id": owner, "account_id": id,
                        "account": try JSONSerialization.jsonObject(with: JSONEncoder().encode(updated)),
                        "session": ["account_id": id, "session_revision": 11, "signed_in": true,
                            "plan_usage_authorized": true, "reauthorization_required": false, "revocation_unconfirmed": false],
                        "connection": action == "models" ? connection as Any : NSNull(), "inference_performed": false]], writer)
                }
            }
            do {
                let connectionID = try await bridge.discoverModels(account: account)
                XCTAssertEqual(variant, "valid")
                XCTAssertEqual(connectionID, "siwc-" + id)
            } catch { XCTAssertNotEqual(variant, "valid") }
            try await worker.value
        }
    }
}

private nonisolated final class DelayedSiwcFixture: MacSiwcAccountTransport, @unchecked Sendable {
    private let lock = NSLock()
    private var continuation: CheckedContinuation<MacSiwcFlow, Error>?
    private var selected = ""
    func start(accountID: String, revision: Int64?, name: String) async throws -> MacSiwcFlow {
        try await withCheckedThrowingContinuation { value in
            lock.lock(); selected = accountID; continuation = value; lock.unlock()
        }
    }
    func update(_ action: String) async throws -> MacSiwcFlow { throw NSError(domain: "Unexpected automatic action", code: 1) }
    func discoverModels(account: MacModelAccountSnapshot) async throws -> String { throw NSError(domain: "Unexpected discovery", code: 1) }
    func close() {}
    func hasStarted() -> Bool { lock.lock(); defer { lock.unlock() }; return continuation != nil }
    func complete() -> Bool {
        lock.lock(); let pending = continuation; continuation = nil; let account = selected; lock.unlock()
        guard let pending else { return false }
        pending.resume(returning: MacSiwcFlow(owner_id: UUID().uuidString, flow_id: UUID().uuidString,
            account_id: account, expected_account_revision: nil, phase: "authorized", authorization_url: nil,
            error: nil, private_session_may_be_retained: false, account: nil, inference_performed: false))
        return true
    }
}

private nonisolated final class GuidedSiwcFixture: MacSiwcAccountTransport, @unchecked Sendable {
    private let lock = NSLock()
    private var accountID = ""
    private var states = ["awaiting_authorization", "awaiting_authorization", "authorized"]
    private var actions: [String] = []
    var failSave = false
    var failDiscovery = false
    var startWithoutURL = false
    var finishPhase = "committed"
    var startFailure: MacSiwcBridgeFailure?
    func calls() -> [String] { lock.withLock { actions } }
    func start(accountID: String, revision: Int64?, name: String) async throws -> MacSiwcFlow {
        lock.withLock { self.accountID = accountID; actions.append("start") }
        if let startFailure { throw startFailure }
        return value("awaiting_authorization", includeURL: !startWithoutURL)
    }
    func update(_ action: String) async throws -> MacSiwcFlow {
        let phase = lock.withLock {
            actions.append(action)
            return action == "status" ? (states.isEmpty ? "authorized" : states.removeFirst()) : action == "cancel" ? "cancelled" : finishPhase
        }
        if action == "finish", failSave { throw MacSiwcBridgeFailure.privateSessionRetained }
        return value(phase)
    }
    func discoverModels(account: MacModelAccountSnapshot) async throws -> String {
        lock.withLock { actions.append("models") }
        if failDiscovery { throw MacSiwcBridgeFailure.authorizationNetwork }
        return "fixture-connection"
    }
    func close() {}
    private func value(_ phase: String, includeURL: Bool = true) -> MacSiwcFlow {
        let id = lock.withLock { accountID }
        return MacSiwcFlow(owner_id: "fixture-owner", flow_id: "fixture-flow", account_id: id,
            expected_account_revision: nil, phase: phase,
            authorization_url: phase == "awaiting_authorization" && includeURL ? "https://auth.openai.com/authorize?state=fixture" : nil,
            error: nil, private_session_may_be_retained: false,
            account: phase == "committed" ? MacModelAccountSnapshot(revision: 1,
                account: .init(id: id, display_name: "Fixture", source: "nola_chatgpt", credential_ref: "siwc:" + id, state: "ready")) : nil,
            inference_performed: false)
    }
}

@MainActor final class MacSiwcSignInModelTests: XCTestCase {
    private func waitUntilIdle(_ model: MacSiwcSignInModel) async throws {
        for _ in 0..<300 {
            if !model.running { return }
            try await Task.sleep(for: .milliseconds(10))
        }
        XCTFail("Sign-in did not settle")
    }
    func testSignInDismissalWaitsForCancellationAcknowledgement() async throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: root) }
        let app = try MacApplicationPort(databaseURL: root.appendingPathComponent("fixture.sqlite"))
        let transport = GuidedSiwcFixture()
        let model = MacSiwcSignInModel(account: nil, openBrowser: { _ in true },
            pollInterval: .seconds(30), makeBridge: { _ in transport })
        XCTAssertFalse(model.requiresConfirmedDismissal)
        model.begin(application: app, name: "Fixture")
        XCTAssertTrue(model.requiresConfirmedDismissal)
        for _ in 0..<100 {
            if model.flow?.phase == "awaiting_authorization" { break }
            try await Task.sleep(for: .milliseconds(1))
        }
        XCTAssertEqual(model.flow?.phase, "awaiting_authorization")
        XCTAssertTrue(model.requiresConfirmedDismissal)
        model.cancel()
        XCTAssertTrue(model.requiresConfirmedDismissal, "Closing must wait for the private bridge acknowledgement")
        try await waitUntilIdle(model)
        XCTAssertTrue(model.cancellationConfirmed)
        XCTAssertFalse(model.requiresConfirmedDismissal)
        XCTAssertEqual(transport.calls().filter { $0 == "cancel" }.count, 1)
        XCTAssertFalse(transport.calls().contains("finish"))
        model.close()
    }

    func testGuidedSignInOpensBrowserOnceSavesOnceAndLoadsModelsOnExplicitChoice() async throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: root) }
        let app = try MacApplicationPort(databaseURL: root.appendingPathComponent("fixture.sqlite"))
        let transport = GuidedSiwcFixture()
        var opened: [URL] = []
        let model = MacSiwcSignInModel(account: nil, openBrowser: { opened.append($0); return true },
            pollInterval: .milliseconds(1), makeBridge: { _ in transport })
        model.begin(application: app, name: "Fixture")
        try await waitUntilIdle(model)
        XCTAssertEqual(opened.count, 1, "status polling must not reopen browser tabs")
        XCTAssertEqual(opened.first?.host, "auth.openai.com")
        XCTAssertEqual(model.flow?.phase, "committed")
        XCTAssertEqual(transport.calls().filter { $0 == "finish" }.count, 1)
        XCTAssertFalse(transport.calls().contains("models"), "sign-in alone must not discover or select models")
        model.chooseModels()
        try await waitUntilIdle(model)
        XCTAssertEqual(model.connectionID, "fixture-connection")
        XCTAssertEqual(transport.calls().filter { $0 == "models" }.count, 1)
        model.close()
    }
    func testBrowserFailureAllowsExplicitReopenAndSaveFailureIsNotRetriedAutomatically() async throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: root) }
        let app = try MacApplicationPort(databaseURL: root.appendingPathComponent("fixture.sqlite"))
        let transport = GuidedSiwcFixture(); transport.failSave = true
        var attempts = 0
        let model = MacSiwcSignInModel(account: nil, openBrowser: { _ in attempts += 1; return attempts > 1 },
            pollInterval: .milliseconds(50), makeBridge: { _ in transport })
        model.begin(application: app, name: "Fixture")
        for _ in 0..<100 {
            if model.browserOpenFailed { break }
            try await Task.sleep(for: .milliseconds(1))
        }
        XCTAssertTrue(model.browserOpenFailed)
        model.openAuthorizationPage()
        XCTAssertFalse(model.browserOpenFailed)
        try await waitUntilIdle(model)
        XCTAssertEqual(attempts, 2)
        XCTAssertNotNil(model.error)
        XCTAssertEqual(model.flow?.phase, "authorized")
        XCTAssertEqual(transport.calls().filter { $0 == "finish" }.count, 1)
        model.close()
    }
    func testDiscoveryFailureKeepsSavedAccountAndDoesNotRestartAuthorization() async throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: root) }
        let app = try MacApplicationPort(databaseURL: root.appendingPathComponent("fixture.sqlite"))
        let transport = GuidedSiwcFixture(); transport.failDiscovery = true
        let model = MacSiwcSignInModel(account: nil, openBrowser: { _ in true },
            pollInterval: .milliseconds(1), makeBridge: { _ in transport })
        model.begin(application: app, name: "Fixture")
        try await waitUntilIdle(model)
        model.chooseModels()
        try await waitUntilIdle(model)
        XCTAssertEqual(model.flow?.phase, "committed")
        XCTAssertNotNil(model.error)
        XCTAssertNil(model.connectionID)
        XCTAssertEqual(transport.calls().filter { $0 == "start" }.count, 1)
        XCTAssertEqual(transport.calls().filter { $0 == "finish" }.count, 1)
        model.close()
    }
    func testCatalogRetryCreatesFreshTransportWithoutRepeatingSignIn() async throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: root) }
        let app = try MacApplicationPort(databaseURL: root.appendingPathComponent("fixture.sqlite"))
        let signIn = GuidedSiwcFixture(), failedCatalog = GuidedSiwcFixture(), recoveredCatalog = GuidedSiwcFixture()
        failedCatalog.failDiscovery = true
        var created = 0
        let model = MacSiwcSignInModel(account: nil, openBrowser: { _ in true },
            pollInterval: .milliseconds(1), makeBridge: { _ in
                created += 1
                return created == 1 ? signIn : created == 2 ? failedCatalog : recoveredCatalog
            })
        model.begin(application: app, name: "Fixture")
        try await waitUntilIdle(model)
        model.chooseModels(); try await waitUntilIdle(model)
        XCTAssertNotNil(model.error)
        XCTAssertEqual(model.flow?.phase, "committed")
        model.chooseModels(); try await waitUntilIdle(model)
        XCTAssertNil(model.error)
        XCTAssertEqual(model.connectionID, "fixture-connection")
        XCTAssertEqual(created, 3)
        XCTAssertEqual(signIn.calls().filter { $0 == "start" }.count, 1)
        XCTAssertFalse(recoveredCatalog.calls().contains("start"))
        model.close()
    }
    func testDelayedBrowserLinkStillOpensAndUnconfirmedSaveNeverShowsSuccess() async throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: root) }
        let app = try MacApplicationPort(databaseURL: root.appendingPathComponent("fixture.sqlite"))
        let transport = GuidedSiwcFixture(); transport.startWithoutURL = true; transport.finishPhase = "authorized"
        var opened = 0
        let model = MacSiwcSignInModel(account: nil, openBrowser: { _ in opened += 1; return true },
            pollInterval: .milliseconds(1), makeBridge: { _ in transport })
        model.begin(application: app, name: "Fixture")
        try await waitUntilIdle(model)
        XCTAssertEqual(opened, 1)
        XCTAssertEqual(model.flow?.phase, "authorized")
        XCTAssertNotNil(model.error)
        XCTAssertEqual(transport.calls().filter { $0 == "finish" }.count, 1)
        model.close()
    }
    func testUncertainStartRequiresAccountReviewInsteadOfBlindRetry() async throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: root) }
        let app = try MacApplicationPort(databaseURL: root.appendingPathComponent("fixture.sqlite"))
        let transport = GuidedSiwcFixture(); transport.startFailure = .unavailable
        let model = MacSiwcSignInModel(account: nil, openBrowser: { _ in XCTFail("No verified browser link"); return false },
            pollInterval: .milliseconds(1), makeBridge: { _ in transport })
        model.begin(application: app, name: "Fixture")
        try await waitUntilIdle(model)
        XCTAssertFalse(model.retrySignInAllowed)
        XCTAssertNotNil(model.error)
        XCTAssertNil(model.flow)
        model.close()
    }
    func testRealSignInPrimaryActionsConnectThenOpenModelSelection() async throws {
        let previousPolicy = NSApp.activationPolicy()
        NSApp.setActivationPolicy(.regular)
        defer { NSApp.setActivationPolicy(previousPolicy) }
        NSApp.activate()
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: root) }
        let app = try MacApplicationPort(databaseURL: root.appendingPathComponent("fixture.sqlite"))
        let transport = GuidedSiwcFixture()
        var browserCount = 0, selected: String?
        let model = MacSiwcSignInModel(account: nil, openBrowser: { _ in browserCount += 1; return true },
            pollInterval: .milliseconds(1), makeBridge: { _ in transport })
        let controller = NSHostingController(rootView: MacSiwcSignInView(application: app, account: nil, model: model, saved: { selected = $0 }))
        let view = controller.view
        view.frame = NSRect(x: 0, y: 0, width: 460, height: 600)
        let window = NSWindow(contentRect: view.bounds, styleMask: [.titled], backing: .buffered, defer: false)
        window.isReleasedWhenClosed = false; window.contentViewController = controller; window.makeKeyAndOrderFront(nil)
        defer { model.close(); window.orderOut(nil); window.contentViewController = nil; window.close() }
        func button(_ identifier: String) -> AnyObject? {
            var queue: [AnyObject] = [window, view], visited = Set<ObjectIdentifier>()
            while !queue.isEmpty && visited.count < 4000 {
                let element = queue.removeFirst()
                guard visited.insert(ObjectIdentifier(element)).inserted else { continue }
                let label = identifier == "mac-chatgpt-sign-in"
                    ? String(localized: "Sign in with ChatGPT", table: "ModelCenter")
                    : String(localized: "Choose models", table: "ModelCenter")
                let role = (element.accessibilityRole?() ?? nil)?.rawValue
                if element.accessibilityIdentifier?() == identifier
                    || (role == "AXButton" && (element.accessibilityLabel?() == label || element.accessibilityTitle?() == label)) { return element }
                let children: [Any] = (element.accessibilityChildren?() ?? nil) ?? []
                queue.append(contentsOf: (children + NSAccessibility.unignoredChildren(from: children)).map { $0 as AnyObject })
                if let v = element as? NSView { queue.append(contentsOf: v.subviews) }
            }
            return nil
        }
        func pressPrimary(_ identifier: String) throws {
            if let element = button(identifier) {
                XCTAssertEqual(element.accessibilityPerformPress?(), true)
                return
            }
            // AppKit may defer SwiftUI's AX descendants until an external AX
            // client connects. Exercise the real default button through its
            // keyboard equivalent in that case, never call the model action.
            let enter = try XCTUnwrap(NSEvent.keyEvent(with: .keyDown, location: .zero,
                modifierFlags: [], timestamp: ProcessInfo.processInfo.systemUptime,
                windowNumber: window.windowNumber, context: nil, characters: "\r",
                charactersIgnoringModifiers: "\r", isARepeat: false, keyCode: 36))
            if !window.performKeyEquivalent(with: enter) { window.sendEvent(enter) }
        }
        for _ in 0..<4 {
            view.layoutSubtreeIfNeeded(); view.displayIfNeeded()
            try await Task.sleep(for: .milliseconds(30)); await Task.yield()
        }
        try pressPrimary("mac-chatgpt-sign-in")
        await Task.yield()
        try await waitUntilIdle(model)
        for _ in 0..<4 {
            view.layoutSubtreeIfNeeded(); view.displayIfNeeded()
            try await Task.sleep(for: .milliseconds(30)); await Task.yield()
        }
        XCTAssertEqual(browserCount, 1)
        XCTAssertEqual(model.flow?.phase, "committed")
        try pressPrimary("mac-chatgpt-choose-models")
        await Task.yield()
        try await waitUntilIdle(model)
        for _ in 0..<30 { if selected != nil { break }; try await Task.sleep(for: .milliseconds(10)) }
        XCTAssertEqual(selected, "fixture-connection", "The next action must open model selection, not return to account settings")
    }
    func testSignInPagesFitBothAppearancesWithReadableText() async throws {
        let previousPolicy = NSApp.activationPolicy()
        NSApp.setActivationPolicy(.regular)
        defer { NSApp.setActivationPolicy(previousPolicy) }
        NSApp.activate()
        let root = FileManager.default.temporaryDirectory.appendingPathComponent("nola-signin-" + UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: root) }
        let app = try MacApplicationPort(databaseURL: root.appendingPathComponent("fixture.sqlite"))
        let captures = FileManager.default.temporaryDirectory.appendingPathComponent("NolaInteractionScreenshots", isDirectory: true)
        try FileManager.default.createDirectory(at: captures, withIntermediateDirectories: true)
        for dark in [false, true] {
            for phase in ["initial", "awaiting_authorization", "committed", "failed"] {
                let model = MacSiwcSignInModel(account: nil, openBrowser: { _ in XCTFail("Rendering must not open a browser"); return false })
                if phase != "initial" {
                    model.flow = MacSiwcFlow(owner_id: "fixture", flow_id: "fixture", account_id: "fixture", expected_account_revision: nil,
                        phase: phase, authorization_url: phase == "awaiting_authorization" ? "https://auth.openai.com/authorize?state=fixture" : nil,
                        error: nil, private_session_may_be_retained: false, account: nil, inference_performed: false)
                }
                model.browserOpenFailed = phase == "awaiting_authorization"
                if phase == "failed" { model.error = MacSiwcBridgeFailure.authorizationNetwork.localizedDescription }
                let view = NSHostingView(rootView: MacSiwcSignInView(application: app, account: nil, model: model)
                    .environment(\.colorScheme, dark ? .dark : .light)
                    .environment(\.dynamicTypeSize, .accessibility1)
                    .background(Color(nsColor: .windowBackgroundColor)))
                let size = view.fittingSize
                XCTAssertEqual(size.width, 460, accuracy: 1)
                XCTAssertLessThanOrEqual(size.height, 650, "The sheet must fit a compact desktop window: \(phase)")
                view.frame = NSRect(origin: .zero, size: size)
                let window = NSWindow(contentRect: view.bounds, styleMask: [.titled], backing: .buffered, defer: false)
                window.isReleasedWhenClosed = false; window.contentView = view
                window.appearance = NSAppearance(named: dark ? .darkAqua : .aqua)
                window.makeKeyAndOrderFront(nil)
                view.layoutSubtreeIfNeeded(); view.displayIfNeeded()
                try await Task.sleep(for: .milliseconds(100))
                if let bitmap = view.bitmapImageRepForCachingDisplay(in: view.bounds) {
                    view.cacheDisplay(in: view.bounds, to: bitmap)
                    let data = try XCTUnwrap(bitmap.representation(using: .png, properties: [:]))
                    try data.write(to: captures.appendingPathComponent("signin-\(phase)-\(dark ? "dark" : "light").png"))
                } else { XCTFail("Could not render the sign-in page") }
                window.orderOut(nil); window.contentView = nil; window.close(); model.close()
            }
        }
    }
    func testClosingOwnerRejectsLateAuthorizationWithoutFinishing() async throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: root) }
        let application = try MacApplicationPort(databaseURL: root.appendingPathComponent("fixture.sqlite"))
        let transport = DelayedSiwcFixture()
        let model = MacSiwcSignInModel(account: nil, makeBridge: { _ in transport })
        model.begin(application: application, name: "Fixture")
        // Let start suspend on the actual async boundary, then invalidate the owner.
        for _ in 0..<100 { if transport.hasStarted() { break }; await Task.yield() }
        XCTAssertTrue(transport.hasStarted())
        model.close()
        XCTAssertFalse(model.running)
        for _ in 0..<100 {
            if transport.complete() { break }
            await Task.yield()
        }
        await Task.yield()
        XCTAssertNil(model.flow)
        XCTAssertNil(model.error)
    }
}
