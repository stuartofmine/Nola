// SPDX-License-Identifier: Apache-2.0
import XCTest
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

@MainActor final class MacSiwcSignInModelTests: XCTestCase {
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
        for _ in 0..<100 {
            if transport.complete() { break }
            await Task.yield()
        }
        await Task.yield()
        XCTAssertNil(model.flow)
        XCTAssertNil(model.error)
    }
}
