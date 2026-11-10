// SPDX-License-Identifier: Apache-2.0
#if DEBUG
import XCTest
import CryptoKit
@testable import Nola

@MainActor
final class IOSRemoteAuxiliaryInvocationTests: XCTestCase {
    func testRemoteReceiptIsObservedWithoutLocalPhysicalAttempt() async throws { try await exercise(.success) }
    func testLostInvokeResponseInspectsOriginalAttemptWithoutResend() async throws { try await exercise(.lostResponse) }
    func testUnknownPhysicalOutcomeRemainsRecoverableWithoutResend() async throws { try await exercise(.unknown) }
    func testForeignHostReceiptCannotPublishOutput() async throws { try await exercise(.foreignHost) }
    func testCancellationRetainsTheSameSourceAttemptAndMeasuredUsage() async throws {
        guard IOSApplicationRuntime.isAvailable else { throw XCTSkip("NolaCore runtime unavailable") }
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: directory) }
        let ledger = try IOSApplicationRuntime(databaseURL: directory.appendingPathComponent("cancel.sqlite"))
        let source = AuxiliaryFixture(mode: .waitForCancellation)
        let provider = await WebAgentProvider(config: .init(baseURL:URL(string:"http://127.0.0.1:8123")!,
            token:"auxiliary-fixture",expectedHostId:AuxiliaryFixture.host)) { try await source.response($0) }
        let id = UUID()
        let invocation = LLMSummaryInvocation(ownerID:"cancel-account",operationID:id,provider:provider).usingLedger(ledger)
        let input = ModelInput(dateRange:.init(start:"2026-10-15",end:"2026-10-16"),candidates:[],userPrefs:[:])
        let task = Task { try await invocation.generate(input:input,timeout:20) }
        guard try await source.waitUntilInvoked() else {
            // If admission failed, surface that actual error instead of
            // waiting forever for a physical dispatch that cannot happen.
            _ = try await task.value
            return XCTFail("Auxiliary fixture did not reach invoke within its bounded wait")
        }
        task.cancel()
        do { _ = try await task.value; XCTFail("Cancellation must not return output") }
        catch is CancellationError {}
        let receipt = try ledger.executeAuxiliaryInvocation([
            "type":.string("inspect_source"),"local_owner":.object(["kind":.string("account"),"account_id":.string("cancel-account")]),
            "source_host_id":.string(AuxiliaryFixture.host),"operation_id":.string(id.uuidString.lowercased())
        ])
        XCTAssertEqual(receipt.attempt?.id,"\(id.uuidString.lowercased()):physical")
        XCTAssertEqual(receipt.attempt?.state,"cancelled")
        guard case .object(let usage)? = receipt.attempt?.usage else { return XCTFail("Cancellation usage missing") }
        XCTAssertEqual(usage["coverage"],.string("known"))
    }

    func testNextBackgroundOccurrenceCannotBypassUnknownWithNewOperationID() async throws {
        try await exerciseWorkerSlot(settleBeforeNext: false)
    }
    func testNewLogicalRequestWaitsForOldTerminalButDoesNotReceiveOldOutput() async throws {
        try await exerciseWorkerSlot(settleBeforeNext: true)
    }

    private func exerciseWorkerSlot(settleBeforeNext: Bool) async throws {
        guard IOSApplicationRuntime.isAvailable else { throw XCTSkip("NolaCore runtime unavailable") }
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: directory) }
        let ledger = try IOSApplicationRuntime(databaseURL: directory.appendingPathComponent("worker.sqlite"))
        let source = AuxiliaryFixture(mode: .unknown)
        let provider = await WebAgentProvider(config: .init(baseURL: URL(string:"http://127.0.0.1:8123")!,
            token:"auxiliary-fixture", expectedHostId:AuxiliaryFixture.host)) { try await source.response($0) }
        let originalID = UUID()
        let original = LLMSummaryInvocation(ownerID:"worker-account", operationID:originalID,
            provider:provider).usingLedger(ledger)
        let input = ModelInput(dateRange:.init(start:"2026-10-15",end:"2026-10-16"),candidates:[],userPrefs:["message":"first occurrence"])
        do { _ = try await original.generate(input:input,timeout:20); XCTFail("Expected pending source") }
        catch is LLMRemoteAuxiliaryOutcomeUnknown {}
        if settleBeforeNext { await source.settlePending() }
        let next = LLMSummaryInvocation(ownerID:"worker-account", operationID:UUID(),provider:provider).usingLedger(ledger)
        let nextInput = ModelInput(dateRange:input.dateRange,candidates:[],userPrefs:["message":"different occurrence and different input"])
        do {
            let output = try await next.generate(input:nextInput,timeout:20)
            XCTAssertTrue(settleBeforeNext)
            XCTAssertEqual(output.explain,"source-only-physical-output")
        } catch let blocked as LLMRemoteAuxiliaryScopeBlocked {
            XCTAssertFalse(settleBeforeNext)
            XCTAssertEqual(blocked.operationID,originalID.uuidString.lowercased())
        }
        let counts = await source.counts()
        XCTAssertEqual(counts.invokes,settleBeforeNext ? 2 : 1)
    }

    private func exercise(_ mode: AuxiliaryFixture.Mode) async throws {
        guard IOSApplicationRuntime.isAvailable else { throw XCTSkip("NolaCore runtime unavailable") }
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: directory) }
        let ledger = try IOSApplicationRuntime(databaseURL: directory.appendingPathComponent("local.sqlite"))
        let source = AuxiliaryFixture(mode: mode)
        let provider = await WebAgentProvider(config: .init(baseURL: URL(string: "http://127.0.0.1:8123")!,
            token: "auxiliary-fixture", expectedHostId: AuxiliaryFixture.host)) { try await source.response($0) }
        let operation = UUID()
        let service = ModelService(provider: provider)
        service.installTestingAuxiliaryLedger(ledger)
        let invocation = LLMSummaryInvocation(ownerID: "local-account", operationID: operation,
            provider: service, scope: .account("local-account")).usingLedger(ledger)
        let input = ModelInput(dateRange: .init(start: "2026-10-15", end: "2026-10-16"), candidates: [],
            userPrefs: ["message":"Summarize the supplied events"])
        do {
            let physical = try await invocation.generateReceipt(input: input, timeout: 20)
            let output = physical.output
            XCTAssertEqual(physical.finishReason, "stop")
            XCTAssertEqual(physical.hasToolCalls, false)
            XCTAssertTrue(mode == .success || mode == .lostResponse)
            XCTAssertEqual(output.explain, "source-only-physical-output")
        } catch is LLMRemoteAuxiliaryOutcomeUnknown {
            XCTAssertEqual(mode, .unknown)
        } catch {
            XCTAssertEqual(mode, .foreignHost, "Unexpected failure: \(error)")
        }
        let counts = await source.counts()
        XCTAssertEqual(counts.invokes, 1)
        if mode == .lostResponse || mode == .unknown { XCTAssertEqual(counts.inspects, 1) }
        // No local Admit/Start was ever issued: only the explicit source receipt
        // mirror exists. The source operation cannot be inspected as a local one.
        XCTAssertThrowsError(try ledger.executeAuxiliaryInvocation([
            "type": .string("inspect"), "operation_id": .string(operation.uuidString.lowercased())
        ]))
        let observed = try ledger.executeAuxiliaryInvocation([
            "type": .string("inspect_source"), "local_owner": .object(["kind": .string("account"), "account_id": .string("local-account")]),
            "source_host_id": .string(AuxiliaryFixture.host), "operation_id": .string(operation.uuidString.lowercased())
        ])
        XCTAssertEqual(observed.operation.attemptsStarted, mode == .foreignHost ? 0 : 1)
        if mode == .success || mode == .lostResponse {
            XCTAssertEqual(observed.attempt?.state, "succeeded")
            guard case .object(let usage)? = observed.attempt?.usage else { return XCTFail("Usage receipt missing") }
            XCTAssertEqual(usage["coverage"], .string("known"))
        }
        if mode == .success {
            await source.reviseTerminalUsage()
            _ = try await service.refreshAuxiliarySource(scope:.account("local-account"),
                sourceHostID:AuxiliaryFixture.host, operationID:operation.uuidString.lowercased())
            let updated = try ledger.executeAuxiliaryInvocation([
                "type":.string("inspect_source"),"local_owner":.object(["kind":.string("account"),"account_id":.string("local-account")]),
                "source_host_id":.string(AuxiliaryFixture.host),"operation_id":.string(operation.uuidString.lowercased())
            ])
            guard case .object(let usage)? = updated.attempt?.usage,
                  case .object(let counters)? = usage["normalized"] else { return XCTFail("Revised usage missing") }
            XCTAssertEqual(counters["output_tokens"],.number(11))
            let refreshedCounts = await source.counts()
            XCTAssertEqual(refreshedCounts.invokes,1,"A late usage refresh must not dispatch the model")
            XCTAssertEqual(refreshedCounts.inspects,1)
        }
    }
}

private actor AuxiliaryFixture {
    enum Mode: Equatable, Sendable { case success, lostResponse, unknown, foreignHost, waitForCancellation }
    static let host = "host-auxiliary-fixture-identity"
    private var mode: Mode
    private var operation: [String: Any] = [:]
    private var attempt: [String: Any]?
    private var invokes = 0
    private var inspects = 0
    private var invokeWaiters: [CheckedContinuation<Void, Never>] = []
    private var cancellationWaiter: CheckedContinuation<Void, Never>?
    init(mode: Mode) { self.mode = mode }
    func waitUntilInvoked() async throws -> Bool {
        for _ in 0..<200 {
            if invokes > 0 { return true }
            try await Task.sleep(nanoseconds: 10_000_000)
        }
        return false
    }
    func settlePending() {
        mode = .success
        operation["state"] = "succeeded"; operation["revision"] = 3; operation["active_attempt_id"] = NSNull()
        attempt?["state"] = "succeeded"; attempt?["revision"] = 2; attempt?["completed_at_ms"] = 3
        attempt?["output"] = ["items":[],"explain":"old-output-must-not-be-returned"]
    }
    func reviseTerminalUsage() {
        operation["revision"] = 4; attempt?["revision"] = 3
        attempt?["usage"] = ["coverage":"known","provider_raw":["prompt_tokens":42,"completion_tokens":11],
            "normalized":["input_tokens":42,"output_tokens":11,"cached_input_tokens":NSNull(),"cache_write_input_tokens":NSNull()]]
    }
    func counts() -> (invokes: Int, inspects: Int) { (invokes, inspects) }
    func response(_ request: URLRequest) async throws -> WebAgentTransportResponse {
        let encrypted = try JSONSerialization.jsonObject(with: XCTUnwrap(request.httpBody)) as! [String: Any]
        let body = try WebAgentProvider.decryptMobileJSON(encrypted, token: "auxiliary-fixture")
        let path = request.url!.lastPathComponent
        if path == "prepare" {
            return try response(request, value: ["source_host_id":Self.host,"profile_id":"actual-host-profile","revision":7])
        }
        let id = try XCTUnwrap(body["operation_id"] as? String)
        let physical = "\(id):physical"
        XCTAssertEqual(body["attempt_id"] as? String, physical)
        if path == "admit" {
            attempt = nil
            let inputJSON = try XCTUnwrap(body["input_json"] as? String)
            let digest = SHA256.hash(data: Data(inputJSON.utf8)).map { String(format:"%02x", $0) }.joined()
            operation = ["id":id,"owner":["kind":"source_authority","source_host_id":Self.host,"authority_id":"verified-auth-epoch"],
                "scenario":"event_summary","configuration":["provider":"configured:actual-host-profile","model":"real-model",
                    "configuration_revision":7,"configuration_digest":String(repeating:"a",count:64),"input_digest":digest,
                    "max_attempts":1,"maximum_duration_ms":body["maximum_duration_ms"]!,
                    "max_output_tokens_per_attempt":8192,"total_output_token_budget":8192],
                "state":"ready","attempts_started":0,"active_attempt_id":NSNull(),
                "cancellation_requested_at_ms":NSNull(),"created_at_ms":1,"revision":1]
        } else if path == "invoke" {
            invokes += 1
            operation["state"] = "running"; operation["attempts_started"] = 1
            operation["active_attempt_id"] = physical; operation["revision"] = 2
            attempt = ["id":physical,"operation_id":id,"predecessor_id":NSNull(),"state":"running",
                "provider":"configured:actual-host-profile","model":"real-model","failure":NSNull(),"output":NSNull(),
                "usage":["coverage":"unknown","provider_raw":NSNull(),"normalized":NSNull()],
                "created_at_ms":2,"completed_at_ms":NSNull(),"revision":1]
            if mode != .unknown && mode != .waitForCancellation {
                operation["state"] = "succeeded"; operation["revision"] = 3; operation["active_attempt_id"] = NSNull()
                attempt?["state"] = "succeeded"; attempt?["revision"] = 2; attempt?["completed_at_ms"] = 3
                attempt?["output"] = ["items":[],"explain":"source-only-physical-output","finish_reason":"stop","has_tool_calls":false]
                attempt?["usage"] = ["coverage":"known","provider_raw":["prompt_tokens":42,"completion_tokens":7],
                    "normalized":["input_tokens":42,"output_tokens":7,"cached_input_tokens":NSNull(),"cache_write_input_tokens":NSNull()]]
            }
            invokeWaiters.forEach { $0.resume() }; invokeWaiters.removeAll()
            if mode == .waitForCancellation { await withCheckedContinuation { cancellationWaiter = $0 } }
            if mode == .lostResponse || mode == .unknown { throw URLError(.networkConnectionLost) }
        } else if path == "inspect" { inspects += 1 }
        else if path == "cancel" {
            operation["cancellation_requested_at_ms"] = 4; operation["revision"] = 4
            operation["state"] = "cancelled"; operation["active_attempt_id"] = NSNull()
            attempt?["state"] = "cancelled"; attempt?["revision"] = 2; attempt?["completed_at_ms"] = 4
            attempt?["usage"] = ["coverage":"known","provider_raw":["prompt_tokens":42,"completion_tokens":2],
                "normalized":["input_tokens":42,"output_tokens":2,"cached_input_tokens":NSNull(),"cache_write_input_tokens":NSNull()]]
            cancellationWaiter?.resume(); cancellationWaiter = nil
        }
        else { throw URLError(.unsupportedURL) }
        return try response(request, value: ["source_host_id":mode == .foreignHost && path == "invoke" ? "foreign-host" : Self.host,
            "operation_id":id,"attempt_id":physical,"receipt":["operation":operation,"attempt":attempt as Any? ?? NSNull(),"dispatch_permitted":false]])
    }
    private func response(_ request: URLRequest, value: [String: Any]) throws -> WebAgentTransportResponse {
        var encrypted = try WebAgentProvider.encryptMobileJSON(value, token: "auxiliary-fixture")
        encrypted["type"] = "encrypted"
        let data = try JSONSerialization.data(withJSONObject: encrypted)
        return .init(bytes: AsyncThrowingStream { stream in data.forEach { stream.yield($0) }; stream.finish() },
            response: HTTPURLResponse(url: request.url!, statusCode:200, httpVersion:nil,
                headerFields:["Content-Type":"application/json"])!)
    }
}
#endif
