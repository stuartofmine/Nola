// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Consolidated iOS host output admission, process observation, unknown-effect recovery, remote auxiliary invocation, mobile transport policy, remote host model catalog/tool plan/URL validation, execution backend persistence, conversation ownership, and goal control outbox suite.

import CryptoKit
import Darwin
import Foundation
import SQLite3
import Testing
import XCTest
@testable import Nola

@MainActor
final class IOSHostOutputAdmissionTests: XCTestCase {
    func testHTTPExplicitAdmissionPreservesValidatedRoute() throws {
        try withRuntime { runtime, _ in
            let route = httpRoute()
            let result = try runtime.admitHostOutput(routes: [limits(route)],
                preference: .object(["explicit": .unsignedInteger(2_048)]))
            let object = try object(result)
            XCTAssertEqual(object["routes"], .array([limits(route)]))
            let resolved = try self.object(try XCTUnwrap(object["resolved"]))
            XCTAssertEqual(resolved["source"], .string("explicit_preference"))
            XCTAssertEqual(try number(resolved["tokens"]), 2_048)
            XCTAssertGreaterThan(try number(object["version"]), 0)
        }
    }

    func testNativeDefaultAndKnownCapAreResolvedBySharedDomain() throws {
        try withRuntime { runtime, _ in
            let route: AgentCoreJSONValue = .object([
                "kind": .string("native"), "adapter": .string("mlx"),
                "adapter_contract_revision": .unsignedInteger(1), "model_id": .string("fixture-native"),
                "profile_ref": .null,
            ])
            let defaultResult = try object(runtime.admitHostOutput(routes: [limits(route)], preference: .string("default")))
            let product = try object(try XCTUnwrap(defaultResult["resolved"]))
            XCTAssertEqual(product["source"], .string("product_default"))
            XCTAssertEqual(try number(product["tokens"]), 16_384)
            let cappedResult = try object(runtime.admitHostOutput(routes: [limits(route, output: 4_096)],
                preference: .string("default")))
            let capped = try object(try XCTUnwrap(cappedResult["resolved"]))
            XCTAssertEqual(capped["source"], .string("known_capability_default"))
            XCTAssertEqual(try number(capped["tokens"]), 4_096)
            XCTAssertThrowsError(try runtime.admitHostOutput(routes: [limits(route, output: 0)],
                preference: .string("default")))
        }
    }

    func testMalformedSecretSourceAndProtocolAreRejectedWithoutStorageWrites() throws {
        try withRuntime { runtime, database in
            try withWriterFence(database) {
                let paths = [database, database.appendingPathExtension("pending-commands.json"),
                    URL(fileURLWithPath: database.path + "-wal")]
                let before = paths.map { try? Data(contentsOf: $0) }
                _ = try runtime.admitHostOutput(routes: [limits(httpRoute())], preference: .string("default"))
                var fields = try object(httpRoute())
                fields["credential_source"] = .object(["kind": .string("none"), "account": .string("not-allowed")])
                XCTAssertThrowsError(try runtime.admitHostOutput(routes: [limits(.object(fields))], preference: .string("default")))
                fields = try object(httpRoute())
                fields["credential_source"] = .object(["kind": .string("ephemeral")])
                XCTAssertThrowsError(try runtime.admitHostOutput(routes: [limits(.object(fields))], preference: .string("default")),
                    "Empty credential digest cannot claim a nonempty ephemeral secret")
                fields = try object(httpRoute())
                fields["protocol"] = .string("unsupported")
                XCTAssertThrowsError(try runtime.admitHostOutput(routes: [limits(.object(fields))], preference: .string("default")))
                XCTAssertEqual(paths.map { try? Data(contentsOf: $0) }, before,
                    "Pure admission must not write the database, WAL or pending mutation file")
            }
        }
    }

    private func httpRoute() -> AgentCoreJSONValue {
        .object([
            "kind": .string("http"), "protocol": .string("responses"), "siwc_account": .null,
            "output_token_field": .string("max_output_tokens"), "provider": .string("openai"),
            "endpoint": .string("https://fixture.invalid/v1"), "endpoint_query_sha256": .null,
            "endpoint_query_source": .object(["kind": .string("none")]), "model": .string("fixture-model"),
            "credential_sha256": .string("e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"),
            "credential_source": .object(["kind": .string("none")]),
        ])
    }

    private func limits(_ route: AgentCoreJSONValue, output: UInt64? = nil) -> AgentCoreJSONValue {
        .object(["route": route, "max_input_tokens": .null, "max_total_tokens": .null,
            "max_output_tokens": output.map(AgentCoreJSONValue.unsignedInteger) ?? .null])
    }

    private func object(_ value: AgentCoreJSONValue) throws -> [String: AgentCoreJSONValue] {
        guard case .object(let fields) = value else { throw IOSApplicationRuntimeError.invalidResponse }
        return fields
    }

    private func number(_ value: AgentCoreJSONValue?) throws -> UInt64 {
        switch value {
        case .unsignedInteger(let value): return value
        case .signedInteger(let value): return try XCTUnwrap(UInt64(exactly: value))
        case .number(let value): return try XCTUnwrap(UInt64(exactly: value))
        default: throw IOSApplicationRuntimeError.invalidResponse
        }
    }

    private func withWriterFence(_ database: URL, _ body: () throws -> Void) throws {
        var connection: OpaquePointer?
        let opened = sqlite3_open_v2(database.path, &connection, SQLITE_OPEN_READWRITE | SQLITE_OPEN_FULLMUTEX, nil)
        defer { sqlite3_close(connection) }
        func requireSuccess(_ code: Int32) throws {
            guard code == SQLITE_OK else {
                throw NSError(domain: "HostOutputAdmissionFixture.SQLite", code: Int(code),
                    userInfo: [NSLocalizedDescriptionKey: String(cString: sqlite3_errmsg(connection))])
            }
        }
        try requireSuccess(opened)
        try requireSuccess(sqlite3_busy_timeout(connection, 5_000))
        // Fence unrelated archive-publication writers before taking the byte
        // oracle. A query that actually needs a write cannot succeed behind
        // this independent connection's writer lock; readers remain available.
        try requireSuccess(sqlite3_exec(connection, "BEGIN IMMEDIATE", nil, nil, nil))
        defer { XCTAssertEqual(sqlite3_exec(connection, "ROLLBACK", nil, nil, nil), SQLITE_OK) }
        try body()
    }

    private func withRuntime(_ body: (IOSApplicationRuntime, URL) throws -> Void) throws {
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: directory) }
        let database = directory.appendingPathComponent("application.db")
        try body(IOSApplicationRuntime(databaseURL: database), database)
    }
}

@MainActor
struct IOSPreparedHostOutputAdmissionTests {
    @Test
    func exactEncodedQueryAndCommonURLNormalizationReachSharedAdmission() throws {
        try withRuntime { runtime in
            for (input, endpoint, query) in [
                ("HTTPS://EXAMPLE.COM:443", "https://example.com/", Optional<String>.none),
                ("http://EXAMPLE.COM:80/v1?", "http://example.com/v1", ""),
                ("https://example.com:8443/proxy%2Ftenant/v1?a=%2F+b&a=2", "https://example.com:8443/proxy%2Ftenant/v1", "a=%2F+b&a=2"),
                ("https://example.com/%25/v1", "https://example.com/%25/v1", nil)
            ] {
                let config = configuration(input)
                let route = try IOSPreparedHostOutputAdmission.httpRoute(configuration: config, provider: "fixture")
                guard case .object(let fields) = route else { Issue.record("Expected HTTP route"); return }
                #expect(fields["endpoint"] == .string(endpoint))
                #expect(fields["endpoint_query_sha256"] == (query.map { .string(sha($0)) } ?? .null))
                #expect(fields["endpoint_query_source"] == .object(["kind": .string(query == nil ? "none" : "ephemeral")]))
                let admitted = try IOSPreparedHostOutputAdmission.admit(route: route, outputTokens: 64, using: runtime)
                let raw = String(decoding: try JSONEncoder().encode(admitted), as: UTF8.self)
                #expect(!raw.contains("private-key"))
                #expect(!raw.contains("a=%2F+b"))
            }
        }
    }

    @Test
    func malformedOrNonCanonicalRoutesFailWithoutDisclosingMaterials() throws {
        try withRuntime { runtime in
            for input in ["https://user:private-password@example.com/v1", "https://example.com/v1#private-fragment",
                          "file:///private-model", "https://example.com/a/../v1"] {
                do {
                    let route = try IOSPreparedHostOutputAdmission.httpRoute(configuration: configuration(input), provider: "fixture")
                    _ = try IOSPreparedHostOutputAdmission.admit(route: route, outputTokens: 64, using: runtime)
                    Issue.record("Expected route rejection")
                } catch {
                    let message = String(describing: error)
                    #expect(!message.contains("private-password"))
                    #expect(!message.contains("private-fragment"))
                    #expect(!message.contains("private-key"))
                }
            }
        }
    }

    @Test
    func outputFieldUsesProviderRuleAndUnknownCapsStayUnknown() throws {
        try withRuntime { runtime in
            for wire in [ModelWireProtocol.chatCompletions, .responses] {
                var config = configuration("https://example.com/v1")
                config.wireProtocol = wire
                config.executionCapabilities.chatOutputTokenField = .maxCompletionTokens
                let route = try IOSPreparedHostOutputAdmission.httpRoute(configuration: config, provider: "fixture")
                guard case .object(let fields) = route else { Issue.record("Expected route"); return }
                #expect(fields["output_token_field"] == .string(wire == .responses ? "max_output_tokens" : "max_completion_tokens"))
                let admission = try IOSPreparedHostOutputAdmission.admit(route: route, outputTokens: 64, using: runtime)
                guard case .object(let object) = admission, case .array(let routes)? = object["routes"],
                      case .object(let limits) = routes[0] else { Issue.record("Expected limits"); return }
                #expect(limits["max_input_tokens"] == .null)
                #expect(limits["max_output_tokens"] == .null)
                #expect(limits["max_total_tokens"] == .null)
                #expect(throws: Error.self) {
                    try IOSPreparedHostOutputAdmission.requireExact(admission, route: route, outputTokens: 128)
                }
            }
        }
    }

    private func configuration(_ url: String) -> OpenAIProvider.Config {
        .init(baseURL: URL(string: url)!, apiKey: "private-key", model: "fixture", maxCompletionTokens: 64)
    }

    private func sha(_ value: String) -> String {
        SHA256.hash(data: Data(value.utf8)).map { String(format: "%02x", $0) }.joined()
    }

    private func withRuntime(_ body: (IOSApplicationRuntime) throws -> Void) throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: root) }
        try body(IOSApplicationRuntime(databaseURL: root.appendingPathComponent("application.db")))
    }
}

final class IOSUnknownEffectRecoveryTests: XCTestCase {
    @MainActor
    func testRetainedPermissionUnknownRecoveryIsPresentedWithoutExecutingWork() async throws {
        guard let source = ProcessInfo.processInfo.environment["NOLA_RECOVERY_SNAPSHOT"] else {
            throw XCTSkip("Requires the retained synthetic permission-recovery snapshot")
        }
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent("recovery-regression-\(UUID().uuidString)")
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: directory) }
        let database = directory.appendingPathComponent("fixture.sqlite")
        try FileManager.default.copyItem(atPath: source, toPath: database.path)
        let runtime = try IOSApplicationRuntime(databaseURL: database)
        let conversation = UUID(uuidString: "52237609-5B53-4FD8-A9AD-39B823E0B3BF")!
        let run = "b61f5bb9-0fa4-42d9-baff-2d1cddb55da2"
        let direct = try XCTUnwrap(runtime.inspectUnknownEffectRecovery(
            conversationID: conversation.uuidString, runID: run, assistantMessageID: run + ":assistant"))
        XCTAssertEqual(direct.targets.count, 1)
        let container = ServiceContainer(applicationRuntime: runtime)
        let manager = ChatManager(container: container, restorePersistedState: false)
        container.chatManager = manager
        manager.currentConversationId = conversation
        await manager.discoverUnknownEffectRecovery()
        let settled = Date().addingTimeInterval(2)
        while manager.unknownEffectRecoveryChecking && Date() < settled {
            try await Task.sleep(nanoseconds: 10_000_000)
        }
        XCTAssertNotNil(manager.unknownEffectRecovery,
            "Core has an exact unknown delivery; selection must expose its recovery decision")
        XCTAssertEqual(manager.unknownEffectRecovery?.runID, run)
        XCTAssertEqual(manager.unknownEffectRecovery?.phase, .inspected)
        XCTAssertFalse(manager.canBeginNewMessage)
    }

    @MainActor
    func testRetainedUnknownStopIsIdempotentAcrossReopenAndPreservesUnknownEvidence() throws {
        guard let source = ProcessInfo.processInfo.environment["NOLA_RECOVERY_SNAPSHOT"] else {
            throw XCTSkip("Requires the retained synthetic permission-recovery snapshot")
        }
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent("recovery-stop-\(UUID().uuidString)")
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: directory) }
        let database = directory.appendingPathComponent("fixture.sqlite")
        try FileManager.default.copyItem(atPath: source, toPath: database.path)
        let runtime = try IOSApplicationRuntime(databaseURL: database)
        let conversation = "52237609-5B53-4FD8-A9AD-39B823E0B3BF"
        let run = "b61f5bb9-0fa4-42d9-baff-2d1cddb55da2"
        let inspection = try XCTUnwrap(runtime.inspectUnknownEffectRecovery(
            conversationID: conversation, runID: run, assistantMessageID: run + ":assistant"))
        XCTAssertEqual(inspection.independent_pending_effects, 0)
        let decision = try inspection.confirmed(choice: .keepUnknownBlocked,
            reason: "Stop the isolated fixture while preserving its unknown outcome")
        let requestID = UUID().uuidString.lowercased()
        let reply = try runtime.resolveUnknownEffectRecovery(decision, requestID: requestID)
        XCTAssertEqual(reply.outcome, .unknownPreservedBlocked)
        XCTAssertEqual(try runtime.resolveUnknownEffectRecovery(decision, requestID: requestID).outcome,
            .unknownPreservedBlocked)
        let reopened = try IOSApplicationRuntime(databaseURL: database)
        XCTAssertEqual(try reopened.resolveUnknownEffectRecovery(decision, requestID: requestID).outcome,
            .unknownPreservedBlocked)
        XCTAssertNil(try reopened.pendingUnknownEffectRecovery(conversationID: conversation))
        XCTAssertNil(try reopened.inspectUnknownEffectRecovery(conversationID: conversation,
            runID: run, assistantMessageID: run + ":assistant"))
        let identity = try IOSCanonicalAgentTurnIdentity(recoveringSubmissionID: run, assistantMessageID: run + ":assistant")
        let application = IOSCanonicalAgentApplication(store: reopened, descriptor: .init(
            conversationID: conversation, conversationTitle: "Fixture", identity: identity,
            userText: "", userContentJSON: nil, contextCheckpointID: nil, boundRunID: run))
        let frozen = try application.loadExactRunInvocation()
        let seal = try XCTUnwrap(application.committedTerminalSeal(coreSessionID: XCTUnwrap(frozen.coreSessionID)))
        XCTAssertEqual(seal.result.value.kind, .blocked)
        guard case .object(let resultFields) = seal.result.value.payload,
              case .array(let refs)? = resultFields["blocking_fact_refs"] else {
            return XCTFail("A blocked result must retain its canonical blocking evidence")
        }
        XCTAssertFalse(refs.isEmpty)
        let conflicting = try inspection.confirmed(choice: .authorizeRiskRetry,
            reason: "Stale test decision must not change the closed owner", target: inspection.targets[0])
        let rejected = try reopened.resolveUnknownEffectRecovery(conflicting, requestID: UUID().uuidString.lowercased())
        XCTAssertTrue(rejected.outcome == .rejected || rejected.outcome == .conflict)
    }

    func testConfirmationCannotFollowSelectionOrFreshInspection() {
        let conversation = UUID()
        let inspection = UUID()
        func view(_ conversationID: UUID, _ inspectionID: UUID,
                  _ phase: IOSUnknownEffectRecoveryPresentation.Phase = .inspected) -> IOSUnknownEffectRecoveryPresentation {
            .init(conversationID: conversationID, inspectionID: inspectionID, runID: "run",
                assistantMessageID: "assistant", attemptIDs: ["attempt"], canRetry: true, canKeepBlocked: true, phase: phase)
        }
        let captured = view(conversation, inspection)
        XCTAssertTrue(captured.acceptsConfirmation(captured))
        XCTAssertFalse(view(UUID(), inspection).acceptsConfirmation(captured))
        XCTAssertFalse(view(conversation, UUID()).acceptsConfirmation(captured))
        XCTAssertFalse(view(conversation, inspection, .outcomePending).acceptsConfirmation(captured))
    }

    private let target = IOSUnknownEffectRecoveryTarget(effect_id: "effect", request_id: "request",
        attempt_id: "attempt", worker_generation: "worker", lease_owner: "owner", lease_token: "token",
        expected_attempt_revision: 3)

    private func inspection(independent: UInt64 = 0) -> IOSUnknownEffectRecoveryInspection {
        .init(conversation_id: "conversation", run_id: "run", assistant_message_id: "assistant",
            invocation_config_digest: "digest", targets: [target], retry_allowed: true, independent_pending_effects: independent)
    }

    func testConfirmedDecisionFreezesFencesIdentityAndTime() throws {
        let decision = try inspection().confirmed(choice: .authorizeRiskRetry, reason: "I accept the risk",
            target: target, resolutionID: "resolution", nowMs: 42)
        let decoded = try JSONDecoder().decode(IOSUnknownEffectRecoveryDecision.self,
            from: JSONEncoder().encode(decision))
        XCTAssertEqual(decoded, decision)
        XCTAssertEqual(decoded.targets, [target])
        XCTAssertEqual(decoded.resolved_at_ms, 42)
        XCTAssertEqual(decoded.resolution_id, "resolution")
    }

    func testUnknownTargetAndIndependentWorkCannotBeAuthorizedAway() throws {
        XCTAssertThrowsError(try inspection().confirmed(choice: .authorizeRiskRetry, reason: "risk"))
        XCTAssertThrowsError(try inspection(independent: 1).confirmed(choice: .keepUnknownBlocked, reason: "stop"))
        XCTAssertThrowsError(try inspection().confirmed(choice: .keepUnknownBlocked, reason: " "))
        XCTAssertThrowsError(try inspection().confirmed(choice: .keepUnknownBlocked, reason: "stop", resolutionID: " bad"))
    }

    func testPendingEnvelopeRetainsOriginalInputNotFreshConfirmation() throws {
        let decision = try inspection().confirmed(choice: .authorizeRiskRetry, reason: "risk",
            target: target, resolutionID: "fixed-resolution", nowMs: 123)
        let input = try JSONSerialization.jsonObject(with: JSONEncoder().encode(decision))
        let envelope: [String: Any] = ["request_id": "fixed-request", "client_id": "ios-application",
            "protocol_version": IOSApplicationRuntime.expectedServiceProtocol, "command": [
            "type": "resolve_unknown_effect_recovery", "arguments": ["input": input]]]
        let first = try IOSApplicationRuntime.unknownEffectIntent(envelope)
        let persisted = try JSONSerialization.jsonObject(with:
            JSONSerialization.data(withJSONObject: envelope, options: [.sortedKeys])) as! [String: Any]
        XCTAssertEqual(try IOSApplicationRuntime.unknownEffectIntent(persisted), first)
        XCTAssertEqual(first?.requestID, "fixed-request")
        XCTAssertEqual(first?.decision.resolved_at_ms, 123)
        var foreign = envelope
        foreign["client_id"] = "remote-mirror"
        XCTAssertThrowsError(try IOSApplicationRuntime.unknownEffectIntent(foreign))
        XCTAssertNil(try IOSApplicationRuntime.unknownEffectIntent(["command": ["type": "other"]]))
    }

    func testReplyMustMatchExactDecisionAndChoice() throws {
        let decision = try inspection().confirmed(choice: .keepUnknownBlocked, reason: "stop",
            resolutionID: "resolution", nowMs: 1)
        try IOSUnknownEffectRecoveryReply(outcome: .unknownPreservedBlocked,
            resolution_id: "resolution", run_id: "run", code: nil).validate(for: decision)
        XCTAssertThrowsError(try IOSUnknownEffectRecoveryReply(outcome: .retryAuthorized,
            resolution_id: "resolution", run_id: "run", code: nil).validate(for: decision))
        XCTAssertThrowsError(try IOSUnknownEffectRecoveryReply(outcome: .unknownPreservedBlocked,
            resolution_id: "resolution", run_id: "other", code: nil).validate(for: decision))
    }
}

#if DEBUG
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

final class RemoteHostModelCatalogTests: XCTestCase {
    @MainActor func testPromptVoiceDoesNotBecomeReasoningTierDuringHostPreparation() async throws {
        let provider = WebAgentProvider(config: .init(baseURL: URL(string: "http://127.0.0.1:9")!,
            token: "fixture-token", expectedHostId: "host", clientId: "client")) { request in
            let object: [String: Any]
            if request.httpMethod == "GET", request.url?.path.hasSuffix("/agent/settings") == true {
                XCTAssertEqual(request.url?.query, "view=models")
                object = ["host_identity": ["id": "host"],
                    "model_profiles": [["profile_id": "p", "revision": 3, "display_name": "Model", "model_name": "upstream"]],
                    "default_chat_model_resolution": ["kind": "resolved", "profile_id": "p", "origin": "default"],
                    "default_chat_model_revision": 7]
            } else if request.url?.path.hasSuffix("/chat/conversations/c1/configuration") == true {
                var encrypted = try WebAgentProvider.encryptMobileJSON(["source_host_id": "host", "result": [
                    "protocol_version": 1, "source_host_id": "host", "client_id": "client", "conversation_id": "conversation",
                    "configuration_revision": NSNull(), "parent_message_id": NSNull(),
                ]], token: "fixture-token")
                encrypted["type"] = "encrypted"
                object = encrypted
            } else {
                XCTAssertTrue(request.url?.path.hasSuffix("/agent/tool-plan/prepare") == true)
                let now = Int64(Date().timeIntervalSince1970 * 1000)
                let plan: [String: Any] = ["schema_version": 1,
                    "reference": ["manifest_id": "plan", "host_id": "host", "client_id": "client", "session_id": "conversation", "catalog_revision": String(repeating: "a", count: 64)],
                    "issued_at_ms": now, "expires_at_ms": now + 60_000, "tools": []]
                var encrypted = try WebAgentProvider.encryptMobileJSON(["source_host_id": "host", "result": plan], token: "fixture-token")
                encrypted["type"] = "encrypted"
                object = encrypted
            }
            let data = try JSONSerialization.data(withJSONObject: object)
            return WebAgentTransportResponse(bytes: AsyncThrowingStream {
                for byte in data { $0.yield(byte) }; $0.finish()
            }, response: HTTPURLResponse(url: request.url!, statusCode: 200, httpVersion: nil, headerFields: nil)!)
        }
        for style in PromptResponseStyle.allCases {
            let prepared = try await provider.prepareHostToolPlan(["conversationId": "conversation", "response_style": style.rawValue])
            XCTAssertEqual(prepared["response_style"] as? String, style.rawValue)
            let intent = try XCTUnwrap(prepared["model_intent"] as? [String: Any])
            XCTAssertEqual(intent["tier"] as? String, "balanced")
            XCTAssertEqual(intent["profile_id"] as? String, "p")
        }
    }

    func testSubmissionFrontierRejectsRebasedParentAndForeignOwner() throws {
        func frontier(host: String = "host", revision: Int64? = 7, parent: String? = "observed-parent") -> AppleSubmissionFrontier {
            .init(protocol_version: 1, source_host_id: host, client_id: "client", conversation_id: "conversation",
                  configuration_revision: revision, parent_message_id: parent)
        }
        try frontier().validate(host: "host", client: "client", conversation: "conversation", observedParent: "observed-parent")
        XCTAssertThrowsError(try frontier(parent: "newer-parent").validate(host: "host", client: "client", conversation: "conversation", observedParent: "observed-parent"))
        XCTAssertThrowsError(try frontier(host: "foreign").validate(host: "host", client: "client", conversation: "conversation", observedParent: "observed-parent"))
        XCTAssertThrowsError(try frontier(revision: nil).validate(host: "host", client: "client", conversation: "conversation", observedParent: "observed-parent"))
        XCTAssertThrowsError(try frontier(revision: 0).validate(host: "host", client: "client", conversation: "conversation", observedParent: "observed-parent"))
        try frontier(revision: nil, parent: nil).validate(host: "host", client: "client", conversation: "conversation", observedParent: nil)
    }

    private func settings() -> [String: Any] {
        ["host_identity": ["id": "host"],
         "model_profiles": [["profile_id": "p", "revision": 3, "display_name": "Model", "model_name": "upstream"]],
         "default_chat_model_resolution": ["kind": "resolved", "profile_id": "p", "origin": "default"],
         "default_chat_model_revision": 7]
    }
    func testFreezesResolvedHostProfileWithoutProviderSecrets() throws {
        let frozen = try AppleRemoteHostModelCatalog.freezeDefault(settings: settings(), expectedHostID: "host")
        let payload = try frozen.payload(tier: "balanced")
        XCTAssertEqual(Set(payload.keys), ["profile_id", "expected_revision", "tier", "overrides"])
        XCTAssertEqual(payload["profile_id"] as? String, "p")
        XCTAssertEqual(payload["expected_revision"] as? UInt64, 3)
        XCTAssertEqual(frozen.defaultRevision, 7)
        try AppleRemoteHostModelCatalog.validateUnchanged(frozen, settings: settings())
    }
    func testRejectsForeignHostUnavailableOrMissingDefault() {
        XCTAssertThrowsError(try AppleRemoteHostModelCatalog.freezeDefault(settings: settings(), expectedHostID: "foreign"))
        let resolutions: [[String: Any]] = [[:], ["kind": "no_automatic_candidate"],
            ["kind": "unavailable", "profile_id": "p", "origin": "default", "reason": "disabled"],
            ["kind": "resolved", "profile_id": "missing", "origin": "automatic"],
            ["kind": "resolved", "profile_id": "p", "origin": "conversation"]]
        for resolution in resolutions {
            var value = settings(); value["default_chat_model_resolution"] = resolution
            XCTAssertThrowsError(try AppleRemoteHostModelCatalog.freezeDefault(settings: value, expectedHostID: "host"))
        }
    }
    func testDefaultAndProfileRevisionChangesCannotBindStaleRequest() throws {
        let frozen = try AppleRemoteHostModelCatalog.freezeDefault(settings: settings(), expectedHostID: "host")
        var value = settings(); value["default_chat_model_revision"] = 8
        XCTAssertThrowsError(try AppleRemoteHostModelCatalog.validateUnchanged(frozen, settings: value))
        value = settings(); value["model_profiles"] = [["profile_id": "p", "revision": 4, "display_name": "Model", "model_name": "upstream"]]
        XCTAssertThrowsError(try AppleRemoteHostModelCatalog.validateUnchanged(frozen, settings: value))
        XCTAssertEqual(try frozen.payload(tier: "detailed")["expected_revision"] as? UInt64, 3,
                       "frozen payload remains unchanged for original receipt recovery")
    }
    func testMalformedCatalogAndDefaultRevisionAreRejected() {
        for revision in [true, 0, -1, 1.5, "7"] as [Any] {
            var value = settings(); value["default_chat_model_revision"] = revision
            XCTAssertThrowsError(try AppleRemoteHostModelCatalog.freezeDefault(settings: value, expectedHostID: "host"))
        }
        var value = settings(); let profiles = value["model_profiles"] as! [[String: Any]]
        value["model_profiles"] = profiles + profiles
        XCTAssertThrowsError(try AppleRemoteHostModelCatalog.freezeDefault(settings: value, expectedHostID: "host"))
    }
    func testAutomaticDefaultIsHostResolvedAndHasNoInventedRevision() throws {
        var value = settings(); value["default_chat_model_revision"] = NSNull()
        value["default_chat_model_resolution"] = ["kind": "resolved", "profile_id": "p", "origin": "automatic"]
        let frozen = try AppleRemoteHostModelCatalog.freezeDefault(settings: value, expectedHostID: "host")
        XCTAssertNil(frozen.defaultRevision)
        XCTAssertThrowsError(try frozen.payload(tier: "invented"))
    }
}

final class ExecutionBackendPersistenceTests: XCTestCase {
    func testSelectionPolicyAcceptsOnlyHostReportedAvailableExecutionUnits() throws {
        XCTAssertEqual(
            try ExecutionBackendSelectionPolicy.validatedSelection(
                "  codex  ",
                availableBackendIDs: ["nola", "codex"]
            ),
            "codex"
        )
        XCTAssertThrowsError(
            try ExecutionBackendSelectionPolicy.validatedSelection(
                "claude_code",
                availableBackendIDs: ["nola", "codex"]
            )
        )
        XCTAssertThrowsError(
            try ExecutionBackendSelectionPolicy.validatedSelection(
                "",
                availableBackendIDs: ["nola"]
            )
        )
    }

    func testExternalExecutionSessionSurvivesConversationEncoding() throws {
        let conversation = Conversation(
            title: "External executor",
            executionBackend: "claude_code",
            externalExecutionSessions: [
                "claude_code": "session-claude-42",
                "codex": "thread-codex-7",
            ]
        )

        let data = try JSONEncoder().encode(conversation)
        let restored = try JSONDecoder().decode(Conversation.self, from: data)

        XCTAssertEqual(restored.executionBackend, "claude_code")
        XCTAssertEqual(restored.externalExecutionSessions["claude_code"], "session-claude-42")
        XCTAssertEqual(restored.externalExecutionSessions["codex"], "thread-codex-7")
    }

    func testCatalogReconciliationKeepsAvailableConversationSelection() {
        XCTAssertEqual(
            ExecutionBackendSelectionPolicy.resolvedSelection(
                current: "codex",
                hostPreferredBackendID: "claude_code",
                availableBackendIDs: ["nola", "codex", "claude_code"]
            ),
            "codex"
        )
    }

    func testCatalogReconciliationRepairsUnavailablePersistedSelection() {
        XCTAssertEqual(
            ExecutionBackendSelectionPolicy.resolvedSelection(
                current: "removed_backend",
                hostPreferredBackendID: "codex",
                availableBackendIDs: ["nola", "codex"]
            ),
            "codex"
        )
        XCTAssertEqual(
            ExecutionBackendSelectionPolicy.resolvedSelection(
                current: "removed_backend",
                hostPreferredBackendID: "also_removed",
                availableBackendIDs: ["nola", "codex"]
            ),
            "nola"
        )
        XCTAssertNil(
            ExecutionBackendSelectionPolicy.resolvedSelection(
                current: "nola",
                hostPreferredBackendID: nil,
                availableBackendIDs: []
            )
        )
    }

    func testHostPermissionPresentationIsConversationScoped() {
        let owner = UUID()
        XCTAssertTrue(
            HostPermissionPresentationPolicy.shouldPresent(
                pendingConversationID: owner,
                currentConversationID: owner
            )
        )
        XCTAssertFalse(
            HostPermissionPresentationPolicy.shouldPresent(
                pendingConversationID: owner,
                currentConversationID: UUID()
            )
        )
        XCTAssertTrue(
            HostPermissionPresentationPolicy.shouldPresent(
                pendingConversationID: nil,
                currentConversationID: nil
            )
        )
    }

    func testWebHostCredentialResolutionFallsBackToCurrentSessionModel() {
        let model = ModelInfo(
            id: "\(ModelSelectionManager.webBackendIdPrefix)session-fallback",
            name: "Session Host",
            source: ModelSelectionManager.webBackendSource,
            isLocal: false,
            baseURL: "https://session-fallback-\(UUID().uuidString).invalid",
            apiKey: "in-memory-session-token",
            upstreamModelId: "web-host"
        )

        XCTAssertEqual(model.resolvedAPIKey, "in-memory-session-token")
    }

}

final class AppleGoalControlOutboxTests: XCTestCase {
    private func directory() -> URL { FileManager.default.temporaryDirectory.appendingPathComponent("goal-intent-\(UUID().uuidString)") }
    private func intent() -> AppleGoalControlIntent {
        AppleGoalControlIntent(hostID: "host", conversationID: "conversation",
            control: AppleGoalControl(action: .pause, expectedGoalId: "goal", expectedControlToken: "token"))
    }
    private static func response(_ intent: AppleGoalControlIntent, pending: Bool = false,
        host: String = "host", rejection: String? = nil) throws -> AppleGoalControlResponse {
        let operation: [String: Any] = ["command_sha256": String(repeating: "a", count: 64),
            "goal_revision": 2, "rejection": rejection as Any? ?? NSNull()]
        return try JSONDecoder().decode(AppleGoalControlResponse.self, from: JSONSerialization.data(withJSONObject: [
            "source_host_id": host, "conversationId": intent.conversation_id, "operationId": intent.control.operationId,
            "controlPending": pending, "authoritative": !pending,
            "goalProjection": ["applied_operations": pending ? [:] : [intent.control.operationId: operation]]
        ]))
    }
    func testLostResponseSurvivesColdOutboxAndResendsIdenticalOperation() async throws {
        let directory = directory(); defer { try? FileManager.default.removeItem(at: directory) }
        let original = intent()
        let first = AppleGoalControlOutbox(directory: directory)
        do {
            _ = try await first.submit(original) { saved in
                XCTAssertEqual(saved, original)
                throw URLError(.networkConnectionLost)
            }
            XCTFail("transport loss must remain unresolved")
        } catch is URLError {}
        let reopened = AppleGoalControlOutbox(directory: directory)
        let pending = try await reopened.pending(hostID: "host", conversationID: "conversation")
        XCTAssertEqual(pending, original)
        let response = try Self.response(original)
        let confirmed = try await reopened.submit(try XCTUnwrap(pending)) { saved in
            XCTAssertEqual(saved, original)
            return response
        }
        XCTAssertTrue(confirmed)
        let cleared = try await reopened.pending(hostID: "host", conversationID: "conversation")
        XCTAssertNil(cleared)
    }
    func testProvisionalReplyCannotSettleOrReplaceSavedIntent() async throws {
        let directory = directory(); defer { try? FileManager.default.removeItem(at: directory) }
        let outbox = AppleGoalControlOutbox(directory: directory)
        let original = intent(); let provisional = try Self.response(original, pending: true)
        let confirmed = try await outbox.submit(original) { _ in provisional }
        XCTAssertFalse(confirmed)
        do {
            _ = try await outbox.submit(intent()) { _ in XCTFail("replacement cannot reach transport"); return provisional }
            XCTFail("must preserve original operation")
        } catch AppleGoalControlOutbox.Failure.unresolvedPrevious {}
        let pending = try await outbox.pending(hostID: "host", conversationID: "conversation")
        XCTAssertEqual(pending, original)
    }
    func testForeignHostReplyPreservesIntentAndExactProgressLedgerSettlesIt() async throws {
        let directory = directory(); defer { try? FileManager.default.removeItem(at: directory) }
        let outbox = AppleGoalControlOutbox(directory: directory)
        let original = intent(); let foreign = try Self.response(original, host: "foreign")
        do { _ = try await outbox.submit(original) { _ in foreign }; XCTFail() }
        catch AppleGoalControlOutbox.Failure.invalidResponse {}
        let response = try Self.response(original)
        let settled = try await outbox.reconcile(hostID: "host", conversationID: "conversation",
            appliedOperations: try XCTUnwrap(response.goalProjection).applied_operations)
        XCTAssertTrue(settled)
    }
    func testCanonicalRejectionSettlesOriginalIntentWithoutClaimingSuccess() async throws {
        let directory = directory(); defer { try? FileManager.default.removeItem(at: directory) }
        let outbox = AppleGoalControlOutbox(directory: directory)
        let original = intent(); let rejected = try Self.response(original, rejection: "Goal state changed")
        do { _ = try await outbox.submit(original) { _ in rejected }; XCTFail() }
        catch AppleGoalControlOutbox.Failure.rejected(let reason) { XCTAssertEqual(reason, "Goal state changed") }
        let pending = try await outbox.pending(hostID: "host", conversationID: "conversation")
        XCTAssertNil(pending)
    }
    func testGoalWireContainsFixedIntentOnlyAndValidatesActionShape() throws {
        let start = AppleGoalControlIntent(hostID: "host", conversationID: "conversation",
            control: AppleGoalControl(action: .start, objective: "Finish review", constraints: []))
        try start.control.validate()
        let body = try XCTUnwrap(JSONSerialization.jsonObject(with: JSONEncoder().encode(start)) as? [String: Any])
        XCTAssertEqual(Set(body.keys), ["expected_host_id", "conversation_id", "control"])
        let control = try XCTUnwrap(body["control"] as? [String: Any])
        XCTAssertEqual(Set(control.keys), ["operationId", "action", "objective", "constraints"])
        XCTAssertNil(control["nowEpochMs"])
        XCTAssertThrowsError(try AppleGoalControl(action: .resume).validate())
        XCTAssertThrowsError(try AppleGoalControl(action: .schedule, expectedGoalId: "goal", expectedControlToken: "token").validate())
    }
    @MainActor func testPairedGoalUsesEncryptedOwnerAndTreatsHTTP202AsPending() async throws {
        let directory = directory(); defer { try? FileManager.default.removeItem(at: directory) }
        let outbox = AppleGoalControlOutbox(directory: directory)
        let control = AppleGoalControl(action: .pause, expectedGoalId: "goal", expectedControlToken: "token")
        let provider = WebAgentProvider(config: .init(baseURL: URL(string: "http://127.0.0.1:9")!,
            token: "fixture-token", expectedHostId: "host", clientId: "client")) { request in
            XCTAssertEqual(request.url?.path, "/api/goals")
            XCTAssertEqual(request.value(forHTTPHeaderField: "X-Nola-Mobile-Crypto"), "aes-gcm-v1")
            let encrypted = try JSONSerialization.jsonObject(with: XCTUnwrap(request.httpBody)) as! [String: Any]
            let body = try WebAgentProvider.decryptMobileJSON(encrypted, token: "fixture-token")
            let requestControl = try XCTUnwrap(body["control"] as? [String: Any])
            XCTAssertEqual(body["expected_host_id"] as? String, "host")
            XCTAssertEqual(body["conversation_id"] as? String, "conversation")
            XCTAssertEqual(requestControl["operationId"] as? String, control.operationId)
            var envelope = try WebAgentProvider.encryptMobileJSON([
                "source_host_id": "host", "conversationId": "conversation", "operationId": control.operationId,
                "authoritative": true, "goalProjection": ["applied_operations": [control.operationId: [
                    "command_sha256": String(repeating: "a", count: 64), "goal_revision": 2, "rejection": NSNull()
                ]]]
            ], token: "fixture-token")
            envelope["type"] = "encrypted"
            let bytes = try JSONSerialization.data(withJSONObject: envelope)
            return WebAgentTransportResponse(bytes: AsyncThrowingStream {
                for byte in bytes { $0.yield(byte) }; $0.finish()
            }, response: HTTPURLResponse(url: request.url!, statusCode: 202, httpVersion: nil, headerFields: nil)!)
        }
        let confirmed = try await provider.controlConversationGoal(conversationID: "conversation", control: control, outbox: outbox)
        XCTAssertFalse(confirmed)
        let pending = try await outbox.pending(hostID: "host", conversationID: "conversation")
        XCTAssertEqual(pending?.control, control)
    }

    func testErrorWithoutExactOperationLedgerRemainsUnconfirmed() async throws {
        let directory = directory(); defer { try? FileManager.default.removeItem(at: directory) }
        let outbox = AppleGoalControlOutbox(directory: directory)
        let original = intent()
        let response = try JSONDecoder().decode(AppleGoalControlResponse.self,
            from: Data(#"{"source_host_id":"host","error":"Configuration unavailable"}"#.utf8))
        do { _ = try await outbox.submit(original) { _ in response }; XCTFail() }
        catch AppleGoalControlOutbox.Failure.unconfirmed(let reason) { XCTAssertEqual(reason, "Configuration unavailable") }
        let pending = try await outbox.pending(hostID: "host", conversationID: "conversation")
        XCTAssertEqual(pending, original)
    }

    func testStartDraftSurvivesPendingAndRejectionAndClearsOnlyAfterApplied() async throws {
        let directory = directory(); defer { try? FileManager.default.removeItem(at: directory) }
        let original = AppleGoalControlIntent(hostID: "host", conversationID: "conversation",
            control: AppleGoalControl(action: .start, objective: "Retain this objective", constraints: ["Keep files"]))
        let outbox = AppleGoalControlOutbox(directory: directory)
        let provisional = try Self.response(original, pending: true)
        _ = try await outbox.submit(original) { _ in provisional }
        let cold = AppleGoalControlOutbox(directory: directory)
        let pendingDraft = try await cold.draft(hostID: "host", conversationID: "conversation")
        XCTAssertEqual(pendingDraft?.objective, "Retain this objective")
        let rejection = try Self.response(original, rejection: "Model unavailable")
        do { _ = try await cold.submit(original) { _ in rejection }; XCTFail() }
        catch AppleGoalControlOutbox.Failure.rejected {}
        let rejectedDraft = try await cold.draft(hostID: "host", conversationID: "conversation")
        XCTAssertEqual(rejectedDraft, pendingDraft)
        let retry = AppleGoalControlIntent(hostID: "host", conversationID: "conversation",
            control: AppleGoalControl(action: .start, objective: "Retain this objective", constraints: ["Keep files"]))
        let applied = try Self.response(retry)
        _ = try await cold.submit(retry) { _ in applied }
        let clearedDraft = try await cold.draft(hostID: "host", conversationID: "conversation")
        XCTAssertNil(clearedDraft)
    }

    func testDiscardClearsWedgedPendingIntentWhilePreservingStartDraft() async throws {
        let directory = directory(); defer { try? FileManager.default.removeItem(at: directory) }
        let start = AppleGoalControlIntent(hostID: "host", conversationID: "conversation",
            control: AppleGoalControl(action: .start, objective: "Preserve draft text", constraints: ["Rule 1"]))
        let outbox = AppleGoalControlOutbox(directory: directory)
        do {
            _ = try await outbox.submit(start) { _ in throw URLError(.timedOut) }
            XCTFail("Expected transport timeout to leave pending intent")
        } catch {}
        let beforeDiscard = try await outbox.pending(hostID: "host", conversationID: "conversation")
        XCTAssertNotNil(beforeDiscard)
        try await outbox.discard(hostID: "host", conversationID: "conversation")
        let afterDiscard = try await outbox.pending(hostID: "host", conversationID: "conversation")
        XCTAssertNil(afterDiscard)
        let savedDraft = try await outbox.draft(hostID: "host", conversationID: "conversation")
        XCTAssertEqual(savedDraft?.objective, "Preserve draft text")
        XCTAssertEqual(savedDraft?.constraints, ["Rule 1"])
    }

}

struct RemoteModelEndpointURLTests {
    @Test
    func endpointURLValidationAcceptsHTTPSAndLANAndRejectsAmbiguousAddresses() {
        #expect(RemoteModelEndpointURL.isValid("https://api.example.com/v1"))
        #expect(RemoteModelEndpointURL.isValid(" http://127.0.0.1:11434/v1 "))
        for value in [
            "", "api.example.com/v1", "file:///tmp/model", "ftp://api.example.com",
            "https://user@api.example.com/v1", "https://user:secret@api.example.com/v1",
            "https://api.example.com/v1#fragment", "https:///v1",
        ] {
            #expect(!RemoteModelEndpointURL.isValid(value), "Unexpectedly accepted: \(value)")
        }
    }
}

final class IOSHostProcessTransportToolPlanAndOwnershipTests: XCTestCase {
    func testMobileTransportPolicyRemoteToolPlanAndConversationOwnershipContracts() throws {
        for allowed in ["https://host.example", "http://127.0.0.1:5001", "http://10.0.2.2:5001",
                        "http://172.31.4.2:5001", "http://192.168.1.8:5001", "http://nola.local:5001", "http://[fd00::1]:5001"] {
            XCTAssertTrue(MobileHostTransportPolicy.allows(try XCTUnwrap(URL(string: allowed))))
        }
        for rejected in ["http://example.com:5001", "http://8.8.8.8:5001", "http://10.0.0.1.example.com:5001", "https://user:secret@example.com"] {
            XCTAssertFalse(MobileHostTransportPolicy.allows(try XCTUnwrap(URL(string: rejected))))
        }

        let planObj: [String: Any] = [
            "schema_version": 1,
            "reference": ["manifest_id": "issued", "host_id": "linux-host", "client_id": "ios",
                          "session_id": "conversation", "catalog_revision": String(repeating: "a", count: 64)],
            "issued_at_ms": 1000, "expires_at_ms": 3000,
            "tools": [["tool_id": "custom_mcp", "route": "agent_host",
                       "schema": ["type": "function", "function": ["name": "custom_mcp", "parameters": ["type": "object"]]]]]
        ]
        let plan = try JSONDecoder().decode(IOSRemoteHostToolPlan.self, from: JSONSerialization.data(withJSONObject: planObj))
        XCTAssertEqual(try plan.validatedReference(hostID: "linux-host", clientID: "ios", sessionID: "conversation")["manifest_id"] as? String, "issued")
        XCTAssertThrowsError(try plan.validatedReference(hostID: "ios", clientID: "ios", sessionID: "conversation"))
        XCTAssertThrowsError(try plan.validatedReference(hostID: "linux-host", clientID: "other", sessionID: "conversation"))
        XCTAssertThrowsError(try plan.validatedReference(hostID: "linux-host", clientID: "ios", sessionID: "other"))
        for (key, value) in [("legacy", true as Any), ("schema_version", "1" as Any), ("schema_version", true as Any),
                             ("issued_at_ms", "1000" as Any), ("expires_at_ms", "3000" as Any),
                             ("issued_at_ms", 0 as Any), ("issued_at_ms", 3000 as Any),
                             ("expires_at_ms", 301001 as Any), ("expires_at_ms", 9_007_199_254_740_992 as Any)] {
            var changed = planObj; changed[key] = value
            XCTAssertThrowsError(try JSONDecoder().decode(IOSRemoteHostToolPlan.self, from: JSONSerialization.data(withJSONObject: changed))
                .validatedReference(hostID: "linux-host", clientID: "ios", sessionID: "conversation"), key)
        }
        for issued: Int64 in [1_000, 8_000_000_000_000_000] {
            var timed = planObj; timed["issued_at_ms"] = issued; timed["expires_at_ms"] = issued + 300_000
            _ = try JSONDecoder().decode(IOSRemoteHostToolPlan.self, from: JSONSerialization.data(withJSONObject: timed))
                .validatedReference(hostID: "linux-host", clientID: "ios", sessionID: "conversation")
        }
        let origTools = try XCTUnwrap(planObj["tools"] as? [[String: Any]])
        var dupPlan = planObj; dupPlan["tools"] = origTools + origTools
        var foreignPlan = planObj; var origRef = try XCTUnwrap(planObj["reference"] as? [String: Any]); origRef["legacy"] = true; foreignPlan["reference"] = origRef
        var badTool = origTools[0]; badTool["legacy"] = true; var unknownToolPlan = planObj; unknownToolPlan["tools"] = [badTool]
        for changed in [dupPlan, foreignPlan, unknownToolPlan] {
            XCTAssertThrowsError(try JSONDecoder().decode(IOSRemoteHostToolPlan.self, from: JSONSerialization.data(withJSONObject: changed))
                .validatedReference(hostID: "linux-host", clientID: "ios", sessionID: "conversation"))
        }
        for invalid in [" \n", "id\u{0000}suffix", "id\u{0085}suffix", String(repeating: "x", count: 513)] {
            var changed = planObj; var ref = try XCTUnwrap(planObj["reference"] as? [String: Any]); ref["manifest_id"] = invalid; changed["reference"] = ref
            XCTAssertThrowsError(try JSONDecoder().decode(IOSRemoteHostToolPlan.self, from: JSONSerialization.data(withJSONObject: changed))
                .validatedReference(hostID: "linux-host", clientID: "ios", sessionID: "conversation"))
        }

        // AppleConversationExecutionOwnership contracts
        func decodeOwner(_ value: String) throws -> AppleConversationSourceOwner {
            try JSONDecoder().decode(AppleConversationSourceOwner.self, from: Data(value.utf8))
        }
        XCTAssertEqual(try decodeOwner(#"{"kind":"remote","source_host_id":"host","source_store_epoch":"epoch"}"#), .remote(hostID: "host", storeEpoch: "epoch"))
        XCTAssertNotEqual(AppleConversationSourceOwner.remote(hostID: "host", storeEpoch: "old"), .remote(hostID: "host", storeEpoch: "new"))
        XCTAssertEqual(try decodeOwner(#"{"kind":"local"}"#), .local)
        XCTAssertEqual(try decodeOwner(#"{"kind":"unavailable"}"#), .unavailable)
        for invalid in [#"{"kind":"remote","source_host_id":"host"}"#, #"{"kind":"remote","source_host_id":"host","source_store_epoch":" "}"#,
                        #"{"kind":"local","source_host_id":"host"}"#, #"{"kind":"remote","source_host_id":"host","source_store_epoch":"epoch","legacy":true}"#] {
            XCTAssertThrowsError(try decodeOwner(invalid))
        }

        let expectedDest = AppleTaskChildDestination(parentConversationID: "parent", parentRunID: "run", taskID: "task", childConversationID: "child")
        let rawOwnership = Data(#"{"kind":"task_child","parent_conversation_id":"parent","parent_run_id":"run","task_id":"task","child_conversation_id":"child"}"#.utf8)
        XCTAssertEqual(try JSONDecoder().decode(AppleConversationExecutionOwnership.self, from: rawOwnership), .taskChild(expectedDest))
        XCTAssertNotEqual(AppleConversationExecutionOwnership.taskChild(expectedDest),
                          .taskChild(.init(parentConversationID: "parent", parentRunID: "other", taskID: "task", childConversationID: "child")))
        for invalid in [#"{}"#, #"{"kind":"legacy"}"#, #"{"kind":"task_child","parent_conversation_id":"parent","parent_run_id":"run","task_id":"task"}"#] {
            XCTAssertThrowsError(try JSONDecoder().decode(AppleConversationExecutionOwnership.self, from: Data(invalid.utf8)))
        }
        XCTAssertEqual(try JSONDecoder().decode(AppleConversationExecutionOwnership.self, from: Data(#"{"kind":"unavailable"}"#.utf8)), .unavailable)
        let childWinReq = AppleTaskChildWindowRequest(destination: expectedDest, beforeOrdinal: nil, limit: 30, contentByteLimit: 65536)
        let childFields = try XCTUnwrap(JSONSerialization.jsonObject(with: JSONEncoder().encode(childWinReq)) as? [String: Any])
        XCTAssertEqual(childFields["parent_run_id"] as? String, "run")
        XCTAssertEqual(childFields["task_id"] as? String, "task")
        XCTAssertEqual(childFields["child_conversation_id"] as? String, "child")
        XCTAssertTrue(childFields["before_ordinal"] is NSNull)
        XCTAssertNil(childFields["result_ref"])
        XCTAssertFalse(AppleTaskChildDestination(parentConversationID: "same", parentRunID: "run", taskID: "task", childConversationID: "same").hasValidIdentity)
    }

    func testHostProcessWorkerIdentityObservationAndCoreEncoding() throws {
        let unknown: AgentCoreJSONValue = .object(["state": .string("unknown"), "pid": .signedInteger(42)])
        XCTAssertEqual(IOSHostProcessObservation.workerIdentity(observation: unknown, nonce: "worker"),
            .object(["kind": .string("unobservable"), "nonce": .string("worker")]))
        let observed: AgentCoreJSONValue = .object(["state": .string("running"), "pid": .signedInteger(42), "boot_id": .string("boot"), "start_id": .string("123:456")])
        XCTAssertEqual(IOSHostProcessObservation.workerIdentity(observation: observed, nonce: "worker"),
            .object(["kind": .string("process"), "pid": .signedInteger(42), "boot_id": .string("boot"), "start_id": .string("123:456"), "nonce": .string("worker")]))

        guard IOSApplicationRuntime.isAvailable else { throw XCTSkip("Application ABI unavailable") }
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: directory) }
        let runtime = try IOSApplicationRuntime(databaseURL: directory.appendingPathComponent("process.db"))
        let identity = IOSHostProcessObservation.currentWorkerIdentity()
        let encoded = try runtime.executeAgentCommand(.object(["type": .string("encode_host_worker_identity"), "identity": identity]))
        guard case .object(let fields) = encoded, case .string(let generation)? = fields["worker_generation"],
              case .string(let owner)? = fields["lease_owner"] else { return XCTFail("Missing Core worker encoding") }
        XCTAssertFalse(generation.isEmpty || owner.isEmpty)
        guard case .object(let facts) = identity, case .string(let kind)? = facts["kind"] else { return XCTFail() }
        XCTAssertTrue(["process", "unobservable"].contains(kind))
        if kind == "unobservable" { XCTAssertEqual(Set(facts.keys), Set(["kind", "nonce"])) }
    }
}
