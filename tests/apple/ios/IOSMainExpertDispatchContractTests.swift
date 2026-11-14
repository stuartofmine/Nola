// SPDX-License-Identifier: Apache-2.0
import XCTest
@testable import Nola

final class IOSMainExpertDispatchContractTests: XCTestCase {
    func testSparseDeclaredCapabilitiesRetainKnownToolsAndUnknownFieldsStayConservative() throws {
        let capabilities = try JSONDecoder().decode(ModelExecutionCapabilities.self,
            from: Data(#"{"toolCalls":true,"imageInput":false,"reasoning":true,"samplingParameters":true}"#.utf8))
        XCTAssertTrue(capabilities.toolCalls)
        XCTAssertTrue(capabilities.reasoning)
        XCTAssertFalse(capabilities.imageInput)
        XCTAssertFalse(capabilities.samplingRequiresNoReasoning)
        XCTAssertEqual(try JSONDecoder().decode(ModelExecutionCapabilities.self, from: JSONEncoder().encode(capabilities)), capabilities)
        XCTAssertThrowsError(try JSONDecoder().decode(ModelExecutionCapabilities.self,
            from: Data(#"{"toolCalls":"yes","imageInput":false,"reasoning":true,"samplingParameters":true}"#.utf8)))
        XCTAssertThrowsError(try JSONDecoder().decode(ModelExecutionCapabilities.self, from: Data(#"{"reasoning":true}"#.utf8)))
        XCTAssertThrowsError(try JSONDecoder().decode(ModelExecutionCapabilities.self,
            from: Data(#"{"toolCalls":true,"imageInput":false,"reasoning":true,"samplingParameters":true,"samplingRequiresNoReasoning":null}"#.utf8)))
    }

    func testCanonicalBlockedBudgetNeverLooksLikeAnsweredAndDoesNotMatchArbitraryDetail() throws {
        let blocked = try JSONDecoder().decode(AppleRunResult<AgentCoreJSONValue>.Value.self, from: Data(#"{"kind":"blocked","reason":{"code":"context.input_budget_exceeded","detail":null},"resolution_condition":"opaque budget condition","blocking_fact_refs":["budget"]}"#.utf8))
        XCTAssertNotNil(blocked.blockedFeedback)
        XCTAssertNotEqual(blocked.blockedFeedback, "opaque budget condition")
        let unrelated = try JSONDecoder().decode(AppleRunResult<AgentCoreJSONValue>.Value.self, from: Data(#"{"kind":"blocked","reason":{"code":"permission.denied","detail":"context.input_budget_exceeded"},"resolution_condition":"Grant the requested permission","blocking_fact_refs":["permission"]}"#.utf8))
        XCTAssertEqual(unrelated.blockedFeedback, "Grant the requested permission")
        let answered = try JSONDecoder().decode(AppleRunResult<AgentCoreJSONValue>.Value.self, from: Data(#"{"kind":"answered","answer_part_refs":["answer"]}"#.utf8))
        XCTAssertNil(answered.blockedFeedback)
    }

    func testOnlyClosedProtocolRejectionsOfferGuidanceAndUnknownOutcomesStayPending() {
        let unsupported = IOSApplicationRuntimeError.rejected("main_expert_selection_requires_tool_capable_main_model")
        XCTAssertEqual(ChatManager.mainExpertRejection(unsupported), .unsupportedModel)
        XCTAssertTrue(AppleMainExpertDispatchRejection.unsupportedModel.permitsDraftRestore)
        XCTAssertFalse(AppleMainExpertDispatchRejection.sourceChanged.permitsDraftRestore)
        XCTAssertNil(ChatManager.mainExpertRejection(IOSApplicationRuntimeError.rejected("unknown outcome")))
        XCTAssertNil(ChatManager.mainExpertRejection(IOSApplicationRuntimeError.rejected("user says main_expert_selection_requires_tool_capable_main_model")))
    }

    func testSelectionUsesOneOriginalUserTurnAndSeparateTypedContext() throws {
        let entry = AppleMainExpertSelectionEntry(agentID: "expert", agentRevision: 7, participantID: "member", contextConversationID: "child")
        let selection = AppleMainExpertSelectionContext(selectedExperts: [entry])
        try selection.validate()
        let context = AgentCoreContextComponent(kind: .mainExpertSelection, id: "nola.main-expert-selection",
            content: try JSONDecoder().decode(AgentCoreJSONValue.self, from: JSONEncoder().encode(selection)), source: nil)
        let original = "当前 search / current，请继续检查"
        let input = try AgentCoreInputSubmission.make(input: original, contextComponents: [context], messageId: "original-user", submissionId: "original-run")
        let object = try XCTUnwrap(try JSONSerialization.jsonObject(with: JSONEncoder().encode(input)) as? [String: Any])
        let items = try XCTUnwrap(object["items"] as? [[String: Any]])
        XCTAssertEqual(items.count, 1)
        XCTAssertEqual(items.first?["content"] as? String, original)
        XCTAssertEqual(items.first?["id"] as? String, "original-user")
        let components = try XCTUnwrap(object["turn_context_components"] as? [[String: Any]])
        XCTAssertEqual(components.first?["kind"] as? String, "main_expert_selection")
        XCTAssertEqual((components.first?["content"] as? [String: Any])?["version"] as? Int, 1)
    }

    func testRemoteSelectionRemainsMetadataAndRetainsObservedEpoch() throws {
        let text = "请继续，不改变我的输入"
        let entry = AppleMainExpertSelectionEntry(agentID: "expert", agentRevision: 2)
        let input = ModelInput(dateRange: .init(start: "", end: ""), candidates: [], userPrefs: ["message": text],
            selectedExperts: [entry], expertSelectionSourceEpoch: "observed-store")
        let payload = WebAgentProvider.makeHostAgentPayload(input: input)
        let messages = try XCTUnwrap(payload["messages"] as? [[String: Any]])
        XCTAssertEqual(messages.count, 1); XCTAssertEqual(messages.first?["content"] as? String, text)
        XCTAssertEqual(payload["expected_source_store_epoch"] as? String, "observed-store")
        let selected = try XCTUnwrap(payload["selected_experts"] as? [[String: Any]])
        XCTAssertEqual(selected.first?["agent_id"] as? String, entry.agentID)
        XCTAssertEqual(selected.first?["agent_revision"] as? Int64, entry.agentRevision)
        XCTAssertTrue(selected.first?["participant_id"] is NSNull)
        XCTAssertNil(payload["model"], "An expert choice cannot replace the Main model")
        let restored = try JSONDecoder().decode(ModelInput.self, from: JSONEncoder().encode(input))
        XCTAssertEqual(restored, input)
    }

    func testInvalidSelectionAndUnknownSourceRejectWhileLegacyHistoryStillDecodes() throws {
        XCTAssertThrowsError(try AppleMainExpertSelectionContext(selectedExperts: [
            .init(agentID: "expert", agentRevision: 1, participantID: "unpaired")]).validate())
        XCTAssertThrowsError(try AppleMainExpertSelectionContext(selectedExperts: [.init(agentID: "expert", agentRevision: 1)], expectedSourceStoreEpoch: "").validate())
        let main = Data(#"{"kind":"main_agent","context_conversation_id":"main"}"#.utf8)
        XCTAssertEqual(try JSONDecoder().decode(AppleExpertTaskSource.self, from: main), .mainAgent(contextConversationID: "main"))
        let legacy = AppleExpertParticipantReference(participant_id: "member", agent_id: "expert", agent_revision: 1, context_conversation_id: "legacy-context")
        XCTAssertEqual(try JSONDecoder().decode(AppleExpertTaskSource.self, from: JSONEncoder().encode(legacy)), .legacyExpert(legacy))
        XCTAssertThrowsError(try JSONDecoder().decode(AppleExpertTaskSource.self, from: Data(#"{"kind":"other","context_conversation_id":"main"}"#.utf8)))
        XCTAssertThrowsError(try IOSExpertPeerHostRequest(request: .object(["session_id": .string("main"), "turn_id": .string("run"),
            "payload": .object(["call_id": .string("call"), "capability_id": .string("delegate_expert"), "input": .object(["action": .string("clarify")])])]),
            expectedContextID: "main", expectedRunID: "run"))
    }

    func testOldHostDoesNotAdvertiseMainDelegationAndUnknownCapabilitiesStayUnsupported() throws {
        let legacy = try JSONDecoder().decode(AppleExpertRoleTemplateCatalog.self, from: Data(#"{"version":1,"templates":[],"features":{"reasoning_effort":true}}"#.utf8))
        XCTAssertFalse(legacy.features.mainAgentDelegation)
        let current = try JSONDecoder().decode(AppleExpertRoleTemplateCatalog.self, from: Data(#"{"version":1,"templates":[],"features":{"main_agent_delegation":true}}"#.utf8))
        XCTAssertTrue(current.features.mainAgentDelegation)
    }

    @MainActor func testEncryptedHostDirectoryChecksSourceAndNeverRebasesAnObservedEpoch() async throws {
        for (host, epoch, matches, succeeds) in [("host", "old-epoch", true, true), ("foreign", "old-epoch", true, false), ("host", "new-epoch", true, false), ("host", "old-epoch", false, false)] {
            let provider = WebAgentProvider(config: .init(baseURL: URL(string: "http://127.0.0.1:9")!, token: "fixture-token", expectedHostId: "host", clientId: "client")) { request in
                XCTAssertTrue(request.url?.path.hasSuffix("/agent-rooms") == true)
                let encrypted = try XCTUnwrap(JSONSerialization.jsonObject(with: XCTUnwrap(request.httpBody)) as? [String: Any])
                let sent = try WebAgentProvider.decryptMobileJSON(encrypted, token: "fixture-token")
                XCTAssertEqual(sent["expected_source_store_epoch"] as? String, "old-epoch")
                XCTAssertEqual((sent["command"] as? [String: Any])?["type"] as? String, "directory")
                var reply = try WebAgentProvider.encryptMobileJSON(["host_id": host, "source_store_epoch": epoch,
                    "source_matches": matches, "result": ["revision": 1, "agents": []]], token: "fixture-token")
                reply["type"] = "encrypted"
                let bytes = try JSONSerialization.data(withJSONObject: reply)
                return WebAgentTransportResponse(bytes: AsyncThrowingStream { stream in bytes.forEach { stream.yield($0) }; stream.finish() },
                    response: HTTPURLResponse(url: request.url!, statusCode: 200, httpVersion: nil, headerFields: nil)!)
            }
            do {
                let value = try await provider.expertRooms(command: .object(["type": .string("directory")]), expectedEpoch: "old-epoch")
                XCTAssertTrue(succeeds); XCTAssertEqual(value.sourceStoreEpoch, "old-epoch")
            } catch { XCTAssertFalse(succeeds, error.localizedDescription) }
        }
    }
}
