// SPDX-License-Identifier: Apache-2.0
import XCTest
@testable import Nola

final class MacModelImportBridgeTests: XCTestCase {
    private nonisolated static func readFrame(_ file: FileHandle) throws -> [String:Any] {
        var data = Data()
        while let byte = try file.read(upToCount: 1), !byte.isEmpty {
            if byte.first == 10 { return try XCTUnwrap(JSONSerialization.jsonObject(with:data) as? [String:Any]) }
            data.append(byte)
            if data.count > 4 * 1024 * 1024 { throw NSError(domain:"fixture", code:1) }
        }
        throw NSError(domain:"fixture", code:2)
    }
    private nonisolated static func writeFrame(_ value: [String:Any], to file: FileHandle) throws {
        var data = try JSONSerialization.data(withJSONObject:value)
        data.append(10)
        try file.write(contentsOf:data)
    }
    private nonisolated static func modelFixture() throws -> MacModelConnectionSnapshot.Model {
        try JSONDecoder().decode(MacModelConnectionSnapshot.Model.self, from:Data(#"{"model_id":"fixture-model","display_name":"Fixture","protocol":"responses","context_window":null,"output_limit":null,"modalities":[],"reasoning_levels":[],"execution_capabilities":null}"#.utf8))
    }
    func testProbePrepareRejectsUnboundModelReceiptOverPrivatePipe() async throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at:root) }
        let application = try MacApplicationPort(databaseURL:root.appendingPathComponent("fixture.sqlite"))
        let service = MacModelConnectionService(application:application)
        let snapshot = try service.create(name:"Fixture", endpoint:"https://fixture.invalid/v1", wireProtocol:"responses", secret:"")
        let incoming = Pipe(), outgoing = Pipe()
        let bridge = MacModelImportBridge(application:application, fixtureInput:incoming.fileHandleForWriting, fixtureOutput:outgoing.fileHandleForReading)
        defer { bridge.cancel() }
        let requestInput = incoming.fileHandleForReading, responseOutput = outgoing.fileHandleForWriting
        let worker = Task.detached {
            let request = try Self.readFrame(requestInput)
            XCTAssertEqual(request["type"] as? String, "probe_prepare")
            try Self.writeFrame(["type":"probe_prepare", "request_id":request["request_id"]!,
                "confirmation":["request_id":UUID().uuidString.lowercased(), "connection_id":snapshot.id,
                    "expected_revision":snapshot.revision, "account_id":NSNull(), "account_revision":NSNull(),
                    "credential_binding":String(repeating:"a", count:64), "model_id":"wrong-model",
                    "protocol":"responses", "max_requests":1, "acknowledge_possible_charges":false]], to:responseOutput)
        }
        do { _ = try await bridge.prepareProbe(snapshot:snapshot, model:Self.modelFixture()); XCTFail("Unbound model must be rejected") }
        catch {}
        try await worker.value
    }

    func testProbeConfirmationRequiresCostApprovalAndExactResultIdentity() async throws {
        for wrongIdentity in [false, true] {
            let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
            defer { try? FileManager.default.removeItem(at:root) }
            let application = try MacApplicationPort(databaseURL:root.appendingPathComponent("fixture.sqlite"))
            let service = MacModelConnectionService(application:application)
            let snapshot = try service.create(name:"Fixture", endpoint:"https://fixture.invalid/v1", wireProtocol:"responses", secret:"")
            let incoming = Pipe(), outgoing = Pipe()
            let bridge = MacModelImportBridge(application:application, fixtureInput:incoming.fileHandleForWriting, fixtureOutput:outgoing.fileHandleForReading)
            defer { bridge.cancel() }
            let snapshotData = try JSONEncoder().encode(snapshot)
            let requestInput = incoming.fileHandleForReading, responseOutput = outgoing.fileHandleForWriting
            let worker = Task.detached {
                let preparation = try Self.readFrame(requestInput)
                let id = UUID().uuidString.lowercased()
                try Self.writeFrame(["type":"probe_prepare", "request_id":preparation["request_id"]!,
                    "confirmation":["request_id":id, "connection_id":snapshot.id,
                        "expected_revision":snapshot.revision, "account_id":NSNull(), "account_revision":NSNull(),
                        "credential_binding":String(repeating:"a", count:64), "model_id":"fixture-model",
                        "protocol":"responses", "max_requests":1, "acknowledge_possible_charges":false]], to:responseOutput)
                let confirmation = try Self.readFrame(requestInput)
                XCTAssertEqual(confirmation["type"] as? String, "probe")
                let body = try XCTUnwrap(confirmation["request"] as? [String:Any])
                XCTAssertEqual(body["acknowledge_possible_charges"] as? Bool, true)
                XCTAssertEqual(body["request_id"] as? String, id)
                var result = try XCTUnwrap(JSONSerialization.jsonObject(with:snapshotData) as? [String:Any])
                result["revision"] = snapshot.revision + 1
                if wrongIdentity {
                    var document = result["document"] as! [String:Any]
                    var connection = document["connection"] as! [String:Any]
                    connection["id"] = "other-owner"
                    document["connection"] = connection
                    result["document"] = document
                }
                try Self.writeFrame(["type":"probe", "request_id":confirmation["request_id"]!, "snapshot":result], to:responseOutput)
            }
            let prepared = try await bridge.prepareProbe(snapshot:snapshot, model:Self.modelFixture())
            do { _ = try await bridge.confirmProbe(requestID:prepared.request_id, acknowledgePossibleCharges:false); XCTFail("Must require cost approval") }
            catch {}
            do {
                let result = try await bridge.confirmProbe(requestID:prepared.request_id, acknowledgePossibleCharges:true)
                XCTAssertFalse(wrongIdentity)
                XCTAssertEqual(result.id, snapshot.id)
            } catch { XCTAssertTrue(wrongIdentity) }
            try await worker.value
        }
    }

    func testProbeCredentialReadIsBoundToApprovedLocalAccountAndNonce() throws {
        let data = Data(#"{"revision":2,"document":{"format_version":1,"connection":{"id":"connection","display_name":"Fixture","base_url":"https://fixture.invalid/v1","protocols":["responses"],"account_id":"local-account","source":"manual","source_id":null,"enabled":true},"catalog":null}}"#.utf8)
        let snapshot = try JSONDecoder().decode(MacModelConnectionSnapshot.self, from: data)
        let approved: [String:Any] = ["connection_id":"connection", "endpoint":"https://fixture.invalid/v1",
            "protocol":"responses", "account_id":"local-account", "nonce":UUID().uuidString]
        XCTAssertTrue(MacModelImportBridge.matchesProbeCredentialBinding(approved, snapshot:snapshot, wireProtocol:"responses"))
        for (key, value) in [("connection_id","other"), ("endpoint","https://other.invalid/v1"),
            ("protocol","anthropic"), ("account_id","remote-account"), ("nonce","invalid")] {
            var changed = approved; changed[key] = value
            XCTAssertFalse(MacModelImportBridge.matchesProbeCredentialBinding(changed, snapshot:snapshot, wireProtocol:"responses"))
        }
        XCTAssertFalse(MacModelImportBridge.matchesProbeCredentialBinding(approved, snapshot:snapshot, wireProtocol:"anthropic"))
    }

    func testCredentialSinkRequiresApprovedConnectionEndpointProtocolAndAccount() throws {
        let data = Data(#"{"plan_id":"fixture","expires_at_ms":1,"candidates":[{"candidate_id":"candidate","connection":{"id":"connection","display_name":"Fixture","base_url":"https://fixture.invalid/v1","protocols":["responses"],"account_id":"existing-account","source":"codex","source_id":"fixture","enabled":true},"models":[],"credential_status":"available","issue":null,"existing_revision":1}]}"#.utf8)
        let candidate = try XCTUnwrap(JSONDecoder().decode(MacModelImportPlan.self, from: data).candidates.first)
        let approved: [String: Any] = ["connection_id": "connection", "endpoint": "https://fixture.invalid/v1",
            "protocols": ["responses"], "account_id": "existing-account"]
        XCTAssertTrue(MacModelImportBridge.matchesCredentialBinding(approved, candidate: candidate))
        for (key, value) in [("connection_id", "other"), ("endpoint", "https://other.invalid/v1"), ("account_id", "other-account")] {
            var changed = approved; changed[key] = value
            XCTAssertFalse(MacModelImportBridge.matchesCredentialBinding(changed, candidate: candidate))
        }
        var changed = approved; changed["protocols"] = ["chat_completions"]
        XCTAssertFalse(MacModelImportBridge.matchesCredentialBinding(changed, candidate: candidate))
    }
}
