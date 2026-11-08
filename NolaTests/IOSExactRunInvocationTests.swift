import Foundation
import CryptoKit
import XCTest
@testable import Nola

final class IOSExactRunInvocationTests: XCTestCase {
    func testFrozenSkillsUseExactCheckpointInsteadOfChangedCurrentCatalog() throws {
        let fixture = Fixture()
        let original = IOSPreparedHostInvocation.skillIdentity([.init(id: "skill", name: "Original", summary: "summary",
            instructions: "frozen instructions", version: "1", requiredTools: [], validationSteps: [])])
        let current = IOSPreparedHostInvocation.skillIdentity([.init(id: "skill", name: "Changed", summary: "summary",
            instructions: "new instructions", version: "2", requiredTools: ["write_file"], validationSteps: [])])
        let body: AgentCoreJSONValue = .object(["session_id": .string("actual-core-session"),
            "state": .object(["configuration": .object(["turn_identity": .object(["host_dependencies": original])])])])
        let raw = String(decoding: try JSONEncoder().encode(body), as: UTF8.self)
        var fields = fixture.fields
        fields["checkpoint_sha256"] = .string(SHA256.hash(data: Data(raw.utf8)).map { String(format: "%02x", $0) }.joined())
        let frozen = try fixture.application(Store(response: .object(fields))).loadExactRunInvocation()
        let checkpoint: [String: AgentCoreJSONValue] = ["conversation_id": .string("conversation"),
            "revision": .number(12), "event_cursor": .number(34), "checkpoint_json": .string(raw)]
        let store = Store(response: .object(checkpoint))
        let recovered = try fixture.application(store).loadFrozenSkillIdentity(frozen)
        XCTAssertEqual(recovered, original)
        XCTAssertNotEqual(recovered, current)
        XCTAssertEqual(store.types, ["load_checkpoint"])
        for (key, altered): (String, AgentCoreJSONValue) in [
            ("conversation_id", .string("foreign")), ("revision", .number(13)),
            ("event_cursor", .number(35)), ("checkpoint_json", .string(raw + " ")),
        ] {
            var changed = checkpoint; changed[key] = altered
            XCTAssertThrowsError(try fixture.application(Store(response: .object(changed))).loadFrozenSkillIdentity(frozen))
        }
    }

    func testCoreCheckpointAdvancePreservesOnlyExactFrozenInvocation() throws {
        let fixture = Fixture()
        let original = try fixture.application(Store(response: fixture.response)).loadExactRunInvocation()
        var advanced = fixture.fields
        advanced["checkpoint_revision"] = .number(13)
        advanced["checkpoint_event_cursor"] = .number(40)
        advanced["checkpoint_sha256"] = .string(String(repeating: "c", count: 64))
        let resumed = try fixture.application(Store(response: .object(advanced))).loadExactRunInvocation()
        XCTAssertNotEqual(original, resumed)
        XCTAssertTrue(resumed.hasSameFrozenInvocation(as: original))
        advanced["host_invocation"] = .object(["version": .number(3), "tampered": .bool(true)])
        let changed = try fixture.application(Store(response: .object(advanced))).loadExactRunInvocation()
        XCTAssertFalse(changed.hasSameFrozenInvocation(as: original))
    }

    func testFrozenRemoteToolPlanIsRequiredAndParticipatesInAuthorityComparison() throws {
        let fixture = Fixture()
        let original = try fixture.application(Store(response: fixture.response)).loadExactRunInvocation()
        var changed = fixture.fields
        changed["remote_tool_plan"] = .object(["reference": .string("changed-plan")])
        let current = try fixture.application(Store(response: .object(changed))).loadExactRunInvocation()
        XCTAssertFalse(current.hasSameFrozenInvocation(as: original))
        changed.removeValue(forKey: "remote_tool_plan")
        XCTAssertThrowsError(try fixture.application(Store(response: .object(changed))).loadExactRunInvocation())
        changed["remote_tool_plan"] = .string("invalid-plan-shape")
        XCTAssertThrowsError(try fixture.application(Store(response: .object(changed))).loadExactRunInvocation())
    }

    func testRecoveryTraceReplayPinsFrontierAndFeedsNextIncrementWithoutGap() throws {
        let fixture = Fixture()
        func event(_ sequence: UInt64) -> AgentTraceEvent {
            AgentTraceEvent(traceId: "trace", runId: fixture.identity.submissionID,
                conversationId: "conversation", messageId: fixture.identity.assistantMessageID,
                turnId: fixture.identity.submissionID, sequence: sequence, kind: .toolProgress)
        }
        func page(_ sequence: UInt64) throws -> AgentCoreJSONValue {
            .object(["trace_id": .string("trace"), "through_sequence": .number(2),
                "next_sequence": .number(Double(sequence)),
                "events": .array([try JSONDecoder().decode(AgentCoreJSONValue.self, from: JSONEncoder().encode(event(sequence)))])])
        }
        let store = ReplayStore(pages: [try page(1), try page(2)])
        let replay = try fixture.application(store).recoveryTraceEvents(afterSequence: 0)
        var reducer = AgentTraceReducer()
        for trace in replay { XCTAssertEqual(reducer.apply(trace), .applied) }
        XCTAssertEqual(reducer.apply(event(3)), .applied)
        XCTAssertEqual(store.commands.count, 2)
        guard case .object(let second) = store.commands[1] else { return XCTFail("Missing replay request") }
        XCTAssertEqual(second["after_sequence"], .signedInteger(1))
        XCTAssertEqual(second["through_sequence"], .signedInteger(2))
        var foreign = event(1); foreign.runId = "foreign"
        let invalid = AgentCoreJSONValue.object(["trace_id": .string("trace"), "through_sequence": .number(1),
            "next_sequence": .number(1), "events": .array([
                try JSONDecoder().decode(AgentCoreJSONValue.self, from: JSONEncoder().encode(foreign))])])
        XCTAssertThrowsError(try fixture.application(ReplayStore(pages: [invalid])).recoveryTraceEvents(afterSequence: 0))
    }

    @MainActor
    func testRecordedColdCancellationTraceRestoresThroughActualTerminal() throws {
        let env = ProcessInfo.processInfo.environment
        guard let file = env["NOLA_RECORDED_TRACE_COPY"],
              let conversation = env["NOLA_RECORDED_TRACE_CONVERSATION"],
              let run = env["NOLA_RECORDED_TRACE_RUN"], let userID = UUID(uuidString: run) else {
            throw XCTSkip("Requires an isolated copy of a real cold-cancelled iOS Run")
        }
        let runtime = try IOSApplicationRuntime(databaseURL: FileManager.default.temporaryDirectory.appendingPathComponent(file))
        let identity = IOSCanonicalAgentTurnIdentity(userMessageID: userID)
        let application = IOSCanonicalAgentApplication(store: runtime,
            descriptor: .init(conversationID: conversation, conversationTitle: "", identity: identity,
                userText: "", userContentJSON: nil, contextCheckpointID: nil))
        let events = try application.recoveryTraceEvents(afterSequence: 0)
        XCTAssertGreaterThan(events.count, 2)
        XCTAssertEqual(events.last?.kind, .runCancelled)
        var reducer = AgentTraceReducer()
        for event in events { XCTAssertEqual(reducer.apply(event), .applied) }
        XCTAssertEqual(reducer.projection.runId, run)
        XCTAssertEqual(reducer.projection.lastSequence, events.last?.sequence)
        XCTAssertEqual(reducer.apply(try XCTUnwrap(events.last)), .duplicate)
        XCTAssertTrue(try application.recoveryTraceEvents(afterSequence: Int64(reducer.projection.lastSequence)).isEmpty)
    }

    func testExactReadIsReadOnlyAndPreservesOpaqueBinding() throws {
        let fixture = Fixture()
        let store = Store(response: fixture.response)
        let result = try fixture.application(store).loadExactRunInvocation()
        XCTAssertEqual(result.runID, fixture.identity.submissionID)
        XCTAssertEqual(result.hostInvocation, .object(["version": .number(3)]))
        XCTAssertEqual(result.coreSessionID, "actual-core-session")
        XCTAssertEqual(result.checkpointRevision, 12)
        XCTAssertEqual(result.checkpointEventCursor, 34)
        XCTAssertEqual(result.checkpointSHA256, String(repeating: "b", count: 64))
        XCTAssertEqual(store.types, ["load_exact_run_invocation"])
    }

    func testRawReceiptIntegersRemainExactThroughFoundationAndOpaqueBinding() throws {
        let fixture = Fixture()
        // UInt64.max checks the generic JSON container, not valid i64 seed admission.
        let numbers: [(String, AgentCoreJSONValue)] = [
            ("9007199254740993", .signedInteger(9_007_199_254_740_993)),
            ("9223372036854775807", .signedInteger(Int64.max)),
            ("-9223372036854775808", .signedInteger(Int64.min)),
            ("18446744073709551615", .unsignedInteger(UInt64.max)),
        ]
        for (literal, expected) in numbers {
            // Mirrors transmitEnvelope -> receipt.result -> executeMutation decoding.
            // Do not manufacture a Double before decoding the raw Rust JSON integer.
            let json = """
            {"ok":true,"result":{"result":{
              "conversation_id":"conversation","run_id":"\(fixture.identity.submissionID)",
              "assistant_message_id":"\(fixture.identity.assistantMessageID)",
              "invocation_config_digest":"\(String(repeating: "a", count: 64))","status":"running",
              "core_session_id":null,"checkpoint_revision":null,"checkpoint_event_cursor":null,"checkpoint_sha256":null,
              "host_invocation":{"version":3,"generation_options":{"seed":\(literal)}},
              "remote_tool_plan":null,"context_budget":{"context_window_tokens":4096,"reserved_output_tokens":64,"prompt_token_limit":null,
              "auto_compact_tokens":null,"compaction_target_tokens":null,"auto_compact_percent":80,"compaction_target_percent":50}
            }}}
            """
            let envelope = try XCTUnwrap(try JSONSerialization.jsonObject(with: Data(json.utf8)) as? [String: Any])
            let receipt = try XCTUnwrap(envelope["result"] as? [String: Any])
            let rawResult = try XCTUnwrap(receipt["result"])
            let resultData = try JSONSerialization.data(withJSONObject: rawResult, options: [.fragmentsAllowed])
            let response = try JSONDecoder().decode(AgentCoreJSONValue.self, from: resultData)
            let frozen = try fixture.application(Store(response: response)).loadExactRunInvocation()
            guard case .object(let binding) = frozen.hostInvocation,
                  case .object(let options)? = binding["generation_options"] else {
                return XCTFail("Opaque binding structure changed")
            }
            XCTAssertEqual(options["seed"], expected)
            let encoded = try JSONEncoder().encode(frozen.hostInvocation)
            XCTAssertEqual(try JSONDecoder().decode(AgentCoreJSONValue.self, from: encoded), frozen.hostInvocation)
            XCTAssertTrue(String(decoding: encoded, as: UTF8.self).contains(literal))
        }
    }

    func testAdmissionWithoutCheckpointUsesFourExplicitNullWitnessFields() throws {
        let fixture = Fixture()
        var fields = fixture.fields
        for key in ["core_session_id", "checkpoint_revision", "checkpoint_event_cursor", "checkpoint_sha256"] {
            fields[key] = .null
        }
        let result = try fixture.application(Store(response: .object(fields))).loadExactRunInvocation()
        XCTAssertNil(result.coreSessionID)
        XCTAssertNil(result.checkpointRevision)
        XCTAssertNil(result.checkpointEventCursor)
        XCTAssertNil(result.checkpointSHA256)
    }

    func testCheckpointWitnessIsStrictCompleteAndLossless() throws {
        let fixture = Fixture()
        let keys = ["core_session_id", "checkpoint_revision", "checkpoint_event_cursor", "checkpoint_sha256"]
        for key in keys {
            var missing = fixture.fields; missing.removeValue(forKey: key)
            XCTAssertThrowsError(try fixture.application(Store(response: .object(missing))).loadExactRunInvocation())
            var partial = fixture.fields; partial[key] = .null
            XCTAssertThrowsError(try fixture.application(Store(response: .object(partial))).loadExactRunInvocation())
        }
        let invalid: [(String, AgentCoreJSONValue)] = [
            ("core_session_id", .string(" ")), ("checkpoint_sha256", .string(String(repeating: "B", count: 64))),
            ("checkpoint_revision", .number(-1)), ("checkpoint_event_cursor", .number(1.5)),
            ("checkpoint_event_cursor", .unsignedInteger(UInt64.max)), ("checkpoint_revision", .string("12")),
        ]
        for (key, value) in invalid {
            var fields = fixture.fields; fields[key] = value
            XCTAssertThrowsError(try fixture.application(Store(response: .object(fields))).loadExactRunInvocation())
        }
        var fields = fixture.fields
        fields["checkpoint_revision"] = .signedInteger(Int64.max)
        fields["checkpoint_event_cursor"] = .signedInteger(9_007_199_254_740_993)
        let exact = try fixture.application(Store(response: .object(fields))).loadExactRunInvocation()
        XCTAssertEqual(exact.checkpointRevision, Int64.max)
        XCTAssertEqual(exact.checkpointEventCursor, 9_007_199_254_740_993)
        fields["checkpoint_sha256"] = .string(String(repeating: "c", count: 64))
        let changed = try fixture.application(Store(response: .object(fields))).loadExactRunInvocation()
        XCTAssertNotEqual(changed, exact, "Recovery preparation must detect a changed checkpoint witness")
    }

    func testRejectsMissingMalformedAndWrongIdentityResponsesWithoutClaim() throws {
        let fixture = Fixture()
        var samples: [AgentCoreJSONValue] = [.null, .object([:])]
        for key in ["conversation_id", "run_id", "assistant_message_id", "invocation_config_digest", "status", "host_invocation", "context_budget"] {
            var value = fixture.fields
            value[key] = .null
            samples.append(.object(value))
        }
        for (key, replacement) in [("conversation_id", "other"), ("run_id", "other"),
            ("assistant_message_id", "other"), ("invocation_config_digest", String(repeating: "A", count: 64)),
            ("invocation_config_digest", "short")] {
            var value = fixture.fields
            value[key] = .string(replacement)
            samples.append(.object(value))
        }
        for version in [0.0, -1.0, 1.5, Double(UInt32.max) + 1, Double.infinity] {
            var value = fixture.fields
            value["host_invocation"] = .object(["version": .number(version)])
            samples.append(.object(value))
        }
        for response in samples {
            let store = Store(response: response)
            XCTAssertThrowsError(try fixture.application(store).loadExactRunInvocation())
            XCTAssertEqual(store.types, ["load_exact_run_invocation"])
        }
    }

    func testMissingPreparationCannotClaimEvenWithValidBindingEnvelope() async throws {
        let fixture = Fixture()
        let store = Store(response: fixture.response)
        do {
            _ = try await fixture.application(store).recoverPendingPermissionWait()
            XCTFail("Opaque binding alone must not authorize recovery")
        } catch {}
        XCTAssertEqual(store.types, ["load_exact_run_invocation"])
    }

    func testPreparationFailureOccursBeforeAnyClaimOrScheduleMutation() async throws {
        let fixture = Fixture()
        let store = Store(response: fixture.response)
        do {
            _ = try await fixture.application(store).recoverPendingPermissionWait(prepareRecovery: { frozen in
                XCTAssertEqual(frozen.runID, fixture.identity.submissionID)
                XCTAssertEqual(store.types, ["load_exact_run_invocation"])
                throw PreparationError.revoked
            })
            XCTFail("Revoked authorization must block recovery")
        } catch PreparationError.revoked {} catch { XCTFail("Unexpected error: \(error)") }
        XCTAssertEqual(store.types, ["load_exact_run_invocation"])
    }

    func testRecoveryAuthorityRequiresExactCoreProofWithoutClaimingRun() throws {
        let fixture = Fixture()
        let frozen = try fixture.application(Store(response: fixture.response)).loadExactRunInvocation()
        let candidate: AgentCoreJSONValue = .object([
            "host_catalog": .object([:]), "tools": .array([]), "permission_rules": .array([]),
            "context_window": .object([:]), "turn_identity": .object(["host_invocation": frozen.hostInvocation]),
        ])
        let proof = fixture.fields.filter { ["conversation_id", "run_id", "assistant_message_id", "invocation_config_digest"].contains($0.key) }
        let store = Store(response: .object(proof))
        try fixture.application(store).validateRecoveryAuthority(frozen, candidate: candidate)
        XCTAssertEqual(store.types, ["validate_mobile_run_recovery_authority"])
        for key in proof.keys {
            var foreign = proof; foreign[key] = .string("foreign")
            let rejected = Store(response: .object(foreign))
            XCTAssertThrowsError(try fixture.application(rejected).validateRecoveryAuthority(frozen, candidate: candidate))
            XCTAssertEqual(rejected.types, ["validate_mobile_run_recovery_authority"])
        }
        let malformed = Store(response: .object(proof))
        XCTAssertThrowsError(try fixture.application(malformed).validateRecoveryAuthority(frozen, candidate: .object([:])))
        XCTAssertTrue(malformed.types.isEmpty)
    }

    func testTerminalOwnerUsesCommittedProofAndRejectsMissingOrForeignOwner() throws {
        let message = IOSCanonicalConversationMessage(id: "visible", runID: "run", role: "assistant",
            status: "completed", ordinal: 1, createdAtMs: 1, completedAtMs: 2, parts: [])
        let valid: AgentCoreJSONValue = .object(["conversation_id": .string("conversation"),
            "run_id": .string("run"), "assistant_message_id": .string("root-assistant")])
        let store = OwnerStore(valid)
        let owner = try store.confirmedTerminalOwner(conversationID: "conversation", observedMessage: message)
        XCTAssertEqual(owner?.assistant_message_id, "root-assistant")
        XCTAssertEqual(store.command, .object(["type": .string("canonical_run_terminal"),
            "conversation_id": .string("conversation"), "run_id": .string("run")]))
        XCTAssertThrowsError(try OwnerStore(.null).confirmedTerminalOwner(conversationID: "conversation", observedMessage: message))
        guard case .object(var foreign) = valid else { return XCTFail() }
        foreign["conversation_id"] = .string("foreign")
        XCTAssertThrowsError(try OwnerStore(.object(foreign)).confirmedTerminalOwner(conversationID: "conversation", observedMessage: message))
        let empty = OwnerStore(.null)
        XCTAssertNil(try empty.confirmedTerminalOwner(conversationID: "new", observedMessage: nil))
        XCTAssertNil(empty.command)
    }

    private nonisolated final class OwnerStore: ConversationRemoteTurnStore, @unchecked Sendable {
        let result: AgentCoreJSONValue
        var command: AgentCoreJSONValue?
        init(_ result: AgentCoreJSONValue) { self.result = result }
        func prepareRemoteSubmission(_ descriptor: IOSRemoteConversationTurnDescriptor) throws {}
        func executeRemoteJournalCommand(_ command: AgentCoreJSONValue) throws -> AgentCoreJSONValue {
            self.command = command
            return result
        }
    }

    private enum PreparationError: Error { case revoked }

    private struct Fixture: Sendable {
        let identity = IOSCanonicalAgentTurnIdentity(userMessageID: UUID())
        var fields: [String: AgentCoreJSONValue] { [
            "conversation_id": .string("conversation"), "run_id": .string(identity.submissionID),
            "assistant_message_id": .string(identity.assistantMessageID),
            "invocation_config_digest": .string(String(repeating: "a", count: 64)),
            "core_session_id": .string("actual-core-session"), "checkpoint_revision": .number(12),
            "checkpoint_event_cursor": .number(34), "checkpoint_sha256": .string(String(repeating: "b", count: 64)),
            "status": .string("running"), "host_invocation": .object(["version": .number(3)]),
            "remote_tool_plan": .null,
            "context_budget": .object(["context_window_tokens": .number(4096), "reserved_output_tokens": .number(64),
                "prompt_token_limit": .null, "auto_compact_tokens": .null, "compaction_target_tokens": .null,
                "auto_compact_percent": .number(80), "compaction_target_percent": .number(50)]),
        ] }
        var response: AgentCoreJSONValue { .object(fields) }
        func application(_ store: any IOSAgentApplicationCommandStore) -> IOSCanonicalAgentApplication {
            IOSCanonicalAgentApplication(store: store, descriptor: .init(conversationID: "conversation",
                conversationTitle: "Test", identity: identity, userText: "", userContentJSON: nil,
                contextCheckpointID: nil))
        }
    }

    private final class ReplayStore: IOSAgentApplicationCommandStore, @unchecked Sendable {
        private var pages: [AgentCoreJSONValue]
        private(set) var commands: [AgentCoreJSONValue] = []
        init(pages: [AgentCoreJSONValue]) { self.pages = pages }
        func executeAgentCommand(_ command: AgentCoreJSONValue) throws -> AgentCoreJSONValue {
            commands.append(command)
            guard !pages.isEmpty else { throw PreparationError.revoked }
            return pages.removeFirst()
        }
    }

    private final class Store: IOSAgentApplicationCommandStore, @unchecked Sendable {
        let response: AgentCoreJSONValue
        private let lock = NSLock()
        private var recorded: [String] = []
        var types: [String] { lock.lock(); defer { lock.unlock() }; return recorded }
        init(response: AgentCoreJSONValue) { self.response = response }
        func executeAgentCommand(_ command: AgentCoreJSONValue) throws -> AgentCoreJSONValue {
            lock.lock(); defer { lock.unlock() }
            guard case .object(let value) = command, case .string(let type)? = value["type"] else {
                throw PreparationError.revoked
            }
            recorded.append(type)
            return response
        }
    }
}
