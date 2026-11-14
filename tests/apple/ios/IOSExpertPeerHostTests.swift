// SPDX-License-Identifier: Apache-2.0
import XCTest
@testable import Nola

final class IOSExpertPeerHostTests: XCTestCase {
    private func request(session: String = "source-context", run: String = "source-run", call: String = "source-call", input: [String: AgentCoreJSONValue] = ["action": .string("directory")]) -> AgentCoreJSONValue {
        .object(["session_id": .string(session), "turn_id": .string(run), "payload": .object([
            "call_id": .string(call), "capability_id": .string("delegate_expert"), "input": .object(input)])])
    }
    func testPeerSourceIdentityComesFromExactHostEnvelopeAndRejectsArgumentInjection() throws {
        let bound = try IOSExpertPeerHostRequest(request: request(), expectedContextID: "source-context", expectedRunID: "source-run")
        XCTAssertEqual(bound.contextID, "source-context"); XCTAssertEqual(bound.runID, "source-run"); XCTAssertEqual(bound.callID, "source-call")
        XCTAssertThrowsError(try IOSExpertPeerHostRequest(request: request(session: "foreign"), expectedContextID: "source-context", expectedRunID: "source-run"))
        XCTAssertThrowsError(try IOSExpertPeerHostRequest(request: request(run: "foreign"), expectedContextID: "source-context", expectedRunID: "source-run"))
        XCTAssertThrowsError(try IOSExpertPeerHostRequest(request: request(call: ""), expectedContextID: "source-context", expectedRunID: "source-run"))
        for key in ["source_context_id", "source_run_id", "call_id", "api_key", "model"] {
            XCTAssertThrowsError(try IOSExpertPeerHostRequest(request: request(input: ["action": .string("directory"), key: .string("spoofed")]),
                expectedContextID: "source-context", expectedRunID: "source-run"))
        }
    }
    func testTimeoutIsOnlyAdmittedForWaitActions() throws {
        for action in ["read", "cancel"] {
            let arguments: [String: AgentCoreJSONValue] = ["action": .string(action), "exchange_id": .string("exchange")]
            XCTAssertNoThrow(try IOSExpertPeerHostRequest(request: request(input: arguments),
                expectedContextID: "source-context", expectedRunID: "source-run"))
            var invalid = arguments; invalid["timeout_ms"] = .number(30_000)
            XCTAssertThrowsError(try IOSExpertPeerHostRequest(request: request(input: invalid),
                expectedContextID: "source-context", expectedRunID: "source-run"))
        }
        XCTAssertNoThrow(try IOSExpertPeerHostRequest(request: request(input: ["action": .string("wait"),
            "exchange_id": .string("exchange"), "timeout_ms": .number(30_000)]),
            expectedContextID: "source-context", expectedRunID: "source-run"))
    }

    func testPeerHostOperationsKeepPrivateMutationAndExactClarificationIdentity() throws {
        let first = AppleExpertPeerClarificationInput(sourceContextID: "context", sourceRunID: "run", callID: "call", exchangeID: "exchange", text: "First")
        let second = AppleExpertPeerClarificationInput(sourceContextID: "context", sourceRunID: "run", callID: "other-call", exchangeID: "exchange", text: "Second")
        func object(_ input: AppleExpertPeerClarificationInput) throws -> [String: Any] {
            try XCTUnwrap(JSONSerialization.jsonObject(with: JSONEncoder().encode(AppleExpertHostOperation<AgentCoreJSONValue>.clarifyPeer(input: input, authority: .local()))) as? [String: Any])
        }
        XCTAssertNotEqual(try AppleExpertHostIntentScope.stableIdentity(object(first)), try AppleExpertHostIntentScope.stableIdentity(object(second)))
        XCTAssertFalse(AppleExpertHostOperation<AgentCoreJSONValue>.clarifyPeer(input: first, authority: .local()).isReadOnly)
        XCTAssertTrue(AppleExpertHostOperation<AgentCoreJSONValue>.loadPeerForSubmission(roomID: "room", exchangeID: "exchange", authority: .local()).isReadOnly)
        XCTAssertTrue(AppleExpertHostOperation<AgentCoreJSONValue>.peerDirectory(sourceContextID: "context", sourceRunID: "run", authority: .local()).isReadOnly)
    }
}

extension IOSExpertPeerHostTests {
    @MainActor func testOnlyMainPlanAdvertisesCoreDelegationSchema() throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent("ios-expert-capability-\(UUID())")
        defer { try? FileManager.default.removeItem(at: root) }
        let runtime = try IOSApplicationRuntime(databaseURL: root.appendingPathComponent("state.db"))
        let identity = AgentCoreExpectedIdentity(hostId: "ios-local", clientId: "fixture", sessionId: "context")
        let ordinary = try AgentCoreToolCatalog.prepareBoundPlan(for: .deviceLocal, expectedIdentity: identity,
            isForeground: true, applicationRuntime: runtime)
        XCTAssertFalse((ordinary.output.enabledTools + ordinary.output.disabledTools).contains { $0.toolId == "delegate_expert" })
        let main = try AgentCoreToolCatalog.prepareBoundPlan(for: .deviceLocal, expectedIdentity: identity,
            isForeground: true, applicationRuntime: runtime, includeExpertTasks: true)
        let tool = try XCTUnwrap(main.output.enabledTools.first { $0.toolId == "delegate_expert" })
        let canonical = try runtime.executeAgentCommand(.object(["type": .string("read_expert_exchange_capability")]))
        let canonicalTool = try XCTUnwrap(canonical.objectFields?["tool"]?.objectFields)
        var expectedBoundSchema = canonicalTool
        expectedBoundSchema["x_nola"] = .object(["permission_contract": .object(["risk": .string("ordinary_read")])])
        XCTAssertEqual(tool.schema, .object(expectedBoundSchema), "Only the registered internal dispatcher receives this contract")
        XCTAssertEqual(tool.schema.objectFields?["function"], canonicalTool["function"], "The canonical model tool schema is unchanged")
        let properties = tool.schema.objectFields?["function"]?.objectFields?["parameters"]?.objectFields?["properties"]?.objectFields
        XCTAssertNotNil(properties?["timeout_ms"]); XCTAssertNil(properties?["timeout_sec"])
        XCTAssertNil(properties?["source_context_id"]); XCTAssertNil(properties?["source_run_id"])
    }

    func testPreparePeerTaskRetryIdentityRetainsFirstTargetConfiguration() throws {
        let request = request()
        func operation(_ marker: String) throws -> [String: Any] {
            try XCTUnwrap(JSONSerialization.jsonObject(with: JSONEncoder().encode(AppleExpertHostOperation<AgentCoreJSONValue>.preparePeerTask(
                request: request, exchangeID: "exchange", targetConfiguration: .object(["manifest_nonce": .string(marker)]),
                targetInvocation: nil, projectRoot: "fixture-root", authority: .local()))) as? [String: Any])
        }
        XCTAssertEqual(try AppleExpertHostIntentScope.stableIdentity(operation("first")), try AppleExpertHostIntentScope.stableIdentity(operation("second")))
    }

    func testCoreTaskTargetProofBindsOwnContextWithoutGrantingDelegation() throws {
        let task = UUID().uuidString.lowercased()
        let raw: [String: Any] = ["version": 1, "task_id": task, "exchange_id": task,
            "parent_conversation_id": "main", "room_id": UUID().uuidString,
            "source": ["participant_id": "source", "agent_id": "source-agent", "agent_revision": 1, "context_conversation_id": "source-context"],
            "target": ["participant_id": "target", "agent_id": "target-agent", "agent_revision": 2, "context_conversation_id": "target-context"],
            "source_run_id": UUID().uuidString, "source_call_id": "call"]
        let proof = try JSONDecoder().decode(AppleExpertTaskTargetV1.self, from: JSONSerialization.data(withJSONObject: raw))
        XCTAssertNoThrow(try proof.validate(taskID: task, contextID: "target-context", runID: task))
        XCTAssertThrowsError(try proof.validate(taskID: "other", contextID: "target-context", runID: task))
        XCTAssertThrowsError(try proof.validate(taskID: task, contextID: "source-context", runID: task))
        XCTAssertThrowsError(try proof.validate(taskID: task, contextID: "target-context", runID: "another-run"))
    }
}

extension IOSExpertPeerHostTests {
    @MainActor func testTargetFactoryFreezesOwnModelEffortAndSkillsWithoutStartingRun() async throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent("ios-expert-target-factory-\(UUID())")
        defer { try? FileManager.default.removeItem(at: root) }
        let runtime = try IOSApplicationRuntime(databaseURL: root.appendingPathComponent("state.db"))
        let anonymous = try JSONSerialization.jsonObject(with: JSONEncoder().encode(IOSCloudCredentialBinding.anonymous))
        let source: [String: Any] = ["baseURL": "https://fixture.invalid/v1", "wireProtocol": "responses", "cloudCredential": anonymous, "capabilities": ["chat"],
            "executionCapabilities": ["toolCalls": true, "imageInput": false, "reasoning": true, "samplingParameters": true],
            "reasoning": ["control": "effort", "efforts": ["low", "high"]]]
        for (id, model) in [("source-profile", "source-model"), ("target-profile", "target-model")] {
            _ = try runtime.upsertModelProfileCAS(.init(id: id, displayName: id, provider: "openai-compatible", modelName: model,
                runtime: "remote", sourceJSON: String(decoding: JSONSerialization.data(withJSONObject: source), as: UTF8.self),
                parametersJSON: #"{"maxContextLength":16384,"maxOutputTokens":1024}"#, selected: id == "source-profile", enabled: true,
                revision: 0, createdAtMs: 1, updatedAtMs: 1, deletedAtMs: nil), expectedRevision: 0)
        }
        let service = IOSAgentRoomService(runtime: runtime)
        let directory = try service.createAgent(name: "Target", description: "", defaults: .object([
            "modelSelectionID": .string("configured:target-profile"), "expertReasoningEffort": .string("high")]), directory: service.directory())
        let agent = try XCTUnwrap(directory.agents.first)
        let parent = try LocalConversationStore(applicationRuntime: runtime).createConversation(title: "Target factory", language: nil)
        let association = try service.joinMention(agentID: agent.id, expectedAgentRevision: agent.revision, conversationID: parent.id.uuidString)
        // Factory input is a typed fixture built from the actual joined target;
        // the separate native Main pump test validates real capture/admission.
        let member = try XCTUnwrap(association.room.room.participants.first)
        let participant = AppleExpertParticipantReference(participant_id: member.id, agent_id: member.agentID,
            agent_revision: member.agentRevision, context_conversation_id: member.contextConversationID)
        let parentRecord = try XCTUnwrap(runtime.loadConversation(id: parent.id.uuidString))
        let runID = UUID().uuidString
        let previewInput = AppleAdmitExpertConversationMessage(conversation_id: parent.id.uuidString,
            expected_revision: parentRecord.revision, room_id: association.roomID, expected_room_revision: association.room.revision,
            submission_id: runID, user_message_id: runID + ":user", text: "Preview own target configuration",
            selected_participants: [participant], created_at_ms: 1)
        let preview: AppleExpertCapturePreview<AgentCoreJSONValue> = try runtime.expertHostOperation(
            .preview(input: previewInput, authority: .local()))
        let resolved = try XCTUnwrap(preview.participants.first)
        let entry = AppleExpertMessageEntry<AgentCoreJSONValue>(participant: participant, display_name: "Target", description: "", defaults: resolved.defaults,
            model_declaration: resolved.model_declaration,
            context_configuration_revision: resolved.context_configuration_revision, child_submission_id: runID,
            child_user_message_id: runID + ":user", child_assistant_message_id: runID + ":assistant", expected_prompt_definition: nil,
            model_witness: nil, instruction_witness: nil)
        let view = AppleExpertPeerView(version: 1, exchangeID: entry.child_submission_id, roomID: association.roomID,
            parentConversationID: parent.id.uuidString, source: .mainAgent(contextConversationID: parent.id.uuidString), target: entry.participant, sourceRunID: UUID().uuidString,
            sourceCallID: "call", sourceDisplayName: "Source", targetDisplayName: "Target", kind: "request", text: "Capture target",
            inReplyTo: nil, rootExchangeID: entry.child_submission_id, hop: 1, createdAtMs: 1, clarifications: [])
        let skill = IOSAgentSkillSnapshot(id: "target-skill", name: "Target Skill", summary: "", instructions: "Frozen target instructions",
            version: "1", requiredTools: [], validationSteps: [])
        let config = try await IOSExpertTaskTargetFactory.configuration(material: .init(view: view, entry: entry, promptDefinitions: []),
            runtime: runtime, skills: [skill], hostExecutor: IOSAgentHostExecutor())
        let identity = try XCTUnwrap(config.objectFields?["turn_identity"]?.objectFields)
        let invocation = try XCTUnwrap(identity["host_invocation"]?.objectFields)
        XCTAssertEqual(invocation["primary"]?.objectFields?["model"], .string("target-model"))
        XCTAssertEqual(invocation["generation"]?.objectFields?["reasoning_selection"]?.objectFields?["value"], .string("high"))
        XCTAssertEqual(identity["host_dependencies"], IOSPreparedHostInvocation.skillIdentity([skill]))
        XCTAssertNotNil(identity["delegation"]?.objectFields?["project_root"])
        let tools: [AgentCoreJSONValue]
        if case .array(let values)? = config.objectFields?["tools"] { tools = values } else { throw IOSApplicationRuntimeError.invalidResponse }
        let toolNames = tools.compactMap { tool in
            tool.objectFields?["function"]?.objectFields?["name"]?.stringScalar ?? tool.objectFields?["name"]?.stringScalar
        }
        for control in ["delegate_expert", "expert_exchange", "spawn_agent", "request_user_input"] {
            XCTAssertFalse(toolNames.contains(control), "Expert capture must leave human interaction and dispatch to Main: \(control)")
        }
        XCTAssertNil(try runtime.loadConversationProjectionV1(id: entry.participant.context_conversation_id).activeRun)
        XCTAssertTrue(try runtime.executeAgentCommand(.object(["type": .string("runtime_task"), "command": .object(["type": .string("list_all")])])) == .array([]),
            "Producing target configuration must not create a worker or Task")
    }
}

extension IOSExpertPeerHostTests {
    func testUncertainPreparationNeverCompletesOriginalSourceToolAsFailed() throws {
        for error: Error in [IOSApplicationRuntimeError.outcomePending("original-command-receipt"),
            IOSExpertTaskPreparationError(exchangeID: "existing-task", reason: "The promotion ACK is not confirmed", taskMayExist: true), CancellationError()] {
            XCTAssertThrowsError(try DeviceAgentCoordinator.expertPreparationFailureResponse(callID: "original-source-call", error: error))
        }
        let rejected = try DeviceAgentCoordinator.expertPreparationFailureResponse(callID: "original-source-call",
            error: IOSExpertTaskPreparationError(exchangeID: "aborted-capture", reason: "Target configuration unsupported", preparationAborted: true))
        XCTAssertEqual(rejected.objectFields?["call_id"], .string("original-source-call"))
        XCTAssertEqual(rejected.objectFields?["outcome"], .string("failed"))
        XCTAssertEqual(rejected.objectFields?["output"]?.objectFields?["execution"]?.objectFields?["state"], .string("not_started"))
        XCTAssertEqual(rejected.objectFields?["output"]?.objectFields?["data"]?.objectFields?["execution_started"], .bool(false))
        XCTAssertEqual(rejected.objectFields?["output"]?.objectFields?["data"]?.objectFields?["preparation_aborted"], .bool(true))
    }
}
