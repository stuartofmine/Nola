import XCTest
@testable import Nola

final class OutboundContextPreparationTests: XCTestCase {
    func testCoreTurnKeepsLatestSubmissionSeparateFromCommittedHistory() {
        let first = "Explain blue sky. Begin SKY_REVIEW"
        let second = "Now explain red sunsets. Begin SUNSET_REVIEW"
        let history = [
            AgentCoreContextPrepareMessage(id: "system", role: "system", content: "typed context"),
            AgentCoreContextPrepareMessage(id: "u1", role: "user", content: first),
            AgentCoreContextPrepareMessage(id: "a1", role: "assistant", content: "SKY_REVIEW: Blue light scatters.")
        ]
        let turn = ConversationTurnInput(currentUserText: second, historySource: .prepared(history.map { .init(role: $0.role, content: $0.content) }))
        XCTAssertEqual(turn.input, second)
        XCTAssertEqual(turn.history.map(\.role), ["user", "assistant"])
        XCTAssertEqual(turn.history.map(\.content), [first, "SKY_REVIEW: Blue light scatters."])
    }

    func testFirstCoreTurnUsesPlainSubmittedTextRatherThanProviderEnvelope() {
        let prompt = "请解释晚霞；从 SUNSET_REVIEW 开始。"
        let turn = ConversationTurnInput(currentUserText: prompt, historySource: .prepared([]))
        XCTAssertEqual(turn.input, prompt)
        XCTAssertTrue(turn.history.isEmpty)
    }

    func testIdenticalOrUnansweredPriorUserTurnIsNotRemovedFromHistory() {
        let prompt = "再解释一下"
        let history = [AgentCoreContextPrepareMessage(id: "prior", role: "user", content: prompt)]
        let turn = ConversationTurnInput(currentUserText: prompt, historySource: .prepared(history.map { .init(role: $0.role, content: $0.content) }))
        XCTAssertEqual(turn.input, prompt)
        XCTAssertEqual(turn.history.map(\.content), [prompt], "Text equality cannot identify a new turn")
    }

    func testSessionMemoryWireKindIsSeparateFromDurableMemoryAndUserInstructions() throws {
        for kind: AgentCoreContextComponentKind in [.sessionMemory, .memory, .userPreference] {
            let component = AgentCoreContextComponent(kind: kind, id: "fixture", content: .string("same text"))
            let encoded = try JSONEncoder().encode(component)
            XCTAssertEqual(try JSONDecoder().decode(AgentCoreContextComponent.self, from: encoded), component)
            let object = try XCTUnwrap(JSONSerialization.jsonObject(with: encoded) as? [String: Any])
            XCTAssertEqual(object["kind"] as? String, kind.rawValue)
        }
        XCTAssertEqual(AgentCoreContextComponentKind.sessionMemory.rawValue, "session_memory")
    }

    @MainActor
    func testDurableLearnedPreferencesRemainMemoryNotUserInstructions() {
        for content in ["Prefer concise answers", "Procedure: verify the result", "普通长期记忆", "session_memory"] {
            let component = DefaultContextManager.durableMemoryComponent(id: "durable-fixture", content: content)
            XCTAssertEqual(component.kind, .memory)
            XCTAssertEqual(component.content, .string(content))
        }
    }

    func testReadyCoreMessagesAreForwardedInAuthoritativeOrderWithoutSwiftTrimming() throws {
        let prepared = AgentCoreContextPrepareResponse(
            action: "ready",
            messages: [
                AgentCoreContextPrepareMessage(id: "system", role: "system", content: "policy"),
                AgentCoreContextPrepareMessage(id: "old-user", role: "user", content: "old question"),
                AgentCoreContextPrepareMessage(id: "old-assistant", role: "assistant", content: "Core-compacted answer"),
                AgentCoreContextPrepareMessage(id: "new-user", role: "user", content: "latest question")
            ],
            metrics: AgentCoreContextPrepareMetrics(
                messageTokens: 20,
                toolSchemaTokens: 0,
                totalPromptTokens: 20,
                promptTokenLimit: 100,
                reservedOutputTokens: 20,
                usagePercent: 20,
                beforeTokens: 80,
                afterTokens: 20
            ),
            archiveRequest: nil
        )

        let segments = try DefaultContextManager.outboundPromptSegments(from: prepared)

        XCTAssertEqual(segments.map(\.role), ["system", "user", "assistant", "user"])
        XCTAssertEqual(segments.map(\.content), ["policy", "old question", "Core-compacted answer", "latest question"])
    }

    func testLatestPersistedUserIsIncludedExactlyOnceInProviderEnvelope() throws {
        let response = AgentCoreContextPrepareResponse(
            action: "ready",
            messages: [
                AgentCoreContextPrepareMessage(id: "assistant", role: "assistant", content: "previous answer"),
                AgentCoreContextPrepareMessage(id: "user", role: "user", content: "latest question")
            ],
            metrics: AgentCoreContextPrepareMetrics(
                messageTokens: 12,
                toolSchemaTokens: 0,
                totalPromptTokens: 12,
                promptTokenLimit: 100,
                reservedOutputTokens: 20,
                usagePercent: 12,
                beforeTokens: 12,
                afterTokens: 12
            ),
            archiveRequest: nil
        )
        let prepared = try CorePreparedOutboundPayload(response: response)
        let data = try XCTUnwrap(
            prepared.providerEnvelopeJSON(
                systemPrompt: "composed policy",
                userQuery: "latest question"
            ).data(using: .utf8)
        )
        let envelope = try XCTUnwrap(JSONSerialization.jsonObject(with: data) as? [String: Any])
        let messages = try XCTUnwrap(envelope["messages"] as? [[String: Any]])
        let contents = messages.compactMap { $0["content"] as? String }

        XCTAssertEqual(envelope["system_prompt"] as? String, "composed policy")
        XCTAssertEqual(contents.filter { $0 == "latest question" }.count, 1)
        XCTAssertEqual(contents, ["previous answer", "latest question"])
        XCTAssertFalse(String(data: data, encoding: .utf8)?.contains("[User]:") == true)
    }

    func testPreparedPayloadReportsOnlyDataSourcesActuallyIncludedByCore() throws {
        let usedID = UUID()
        let truncatedID = UUID()
        let droppedID = UUID()
        let response = AgentCoreContextPrepareResponse(
            action: "ready",
            messages: [AgentCoreContextPrepareMessage(id: "user", role: "user", content: "question")],
            metrics: AgentCoreContextPrepareMetrics(
                messageTokens: 10,
                toolSchemaTokens: 0,
                totalPromptTokens: 10,
                promptTokenLimit: 100,
                reservedOutputTokens: 20,
                usagePercent: 10,
                beforeTokens: 10,
                afterTokens: 10
            ),
            componentMetrics: [
                AgentCoreContextComponentMetric(
                    kind: .platformData,
                    usedTokens: 6,
                    droppedTokens: 4,
                    truncatedTokens: 2,
                    usedIds: ["data_source:\(usedID.uuidString)"],
                    droppedIds: ["data_source:\(droppedID.uuidString)"],
                    truncatedIds: ["data_source:\(truncatedID.uuidString)"]
                ),
                AgentCoreContextComponentMetric(
                    kind: .platformData,
                    usedTokens: 2,
                    droppedTokens: 0,
                    truncatedTokens: 0,
                    usedIds: ["platform_data:health", "data_source:\(usedID.uuidString)"],
                    droppedIds: [],
                    truncatedIds: []
                )
            ]
        )

        let payload = try CorePreparedOutboundPayload(response: response)

        XCTAssertEqual(Set(payload.usedDataSourceIDs), Set([usedID, truncatedID]))
        XCTAssertFalse(payload.usedDataSourceIDs.contains(droppedID))
    }

    func testPreparedPayloadRetainsTypedComponentsForCoreHostedTurn() throws {
        let skill = AgentCoreContextComponent(
            kind: .skill,
            id: "skill:word",
            content: .string(#"{"type":"available_skill","name":"Word","invocation":{"tool":"skill_invoke","arguments":{"skill":"Word"}}}"#),
            source: .init(platform: .ios, title: "Word")
        )
        let response = AgentCoreContextPrepareResponse(
            action: "ready",
            messages: [
                AgentCoreContextPrepareMessage(id: "skill:word", role: "system", content: "prepared skill"),
                AgentCoreContextPrepareMessage(id: "user", role: "user", content: "create a document")
            ],
            metrics: AgentCoreContextPrepareMetrics(
                messageTokens: 12,
                toolSchemaTokens: 8,
                totalPromptTokens: 20,
                promptTokenLimit: 100,
                reservedOutputTokens: 20,
                usagePercent: 20,
                beforeTokens: 20,
                afterTokens: 20
            ),
            archiveRequest: nil
        )

        let payload = try CorePreparedOutboundPayload(
            response: response,
            contextComponents: [skill]
        )

        XCTAssertEqual(payload.contextComponents, [skill])
        XCTAssertEqual(payload.contextComponents.first?.kind, .skill)
    }

    func testCannotFitBlocksOutboundInsteadOfFallingBackToLatestMessage() {
        let prepared = AgentCoreContextPrepareResponse(
            action: "cannot_fit",
            messages: [AgentCoreContextPrepareMessage(id: "latest", role: "user", content: "must not send alone")],
            metrics: AgentCoreContextPrepareMetrics(
                messageTokens: 120,
                toolSchemaTokens: 0,
                totalPromptTokens: 120,
                promptTokenLimit: 100,
                reservedOutputTokens: 20,
                usagePercent: 120,
                beforeTokens: 120,
                afterTokens: 120
            ),
            archiveRequest: nil
        )

        XCTAssertThrowsError(try DefaultContextManager.outboundPromptSegments(from: prepared)) { error in
            XCTAssertEqual(error as? ContextOutboundPreparationError, .cannotFit)
        }
    }

    func testSummaryRequiredCannotEscapePreflight() {
        let prepared = AgentCoreContextPrepareResponse(
            action: "summary_required",
            messages: [],
            metrics: AgentCoreContextPrepareMetrics(
                messageTokens: 90,
                toolSchemaTokens: 0,
                totalPromptTokens: 90,
                promptTokenLimit: 100,
                reservedOutputTokens: 20,
                usagePercent: 90,
                beforeTokens: 90,
                afterTokens: 90
            ),
            archiveRequest: nil
        )

        XCTAssertThrowsError(try DefaultContextManager.outboundPromptSegments(from: prepared)) { error in
            XCTAssertEqual(error as? ContextOutboundPreparationError, .summaryStillRequired)
        }
    }
}
