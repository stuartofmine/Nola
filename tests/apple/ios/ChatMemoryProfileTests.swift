import XCTest
@testable import Nola

final class ChatMemoryProfileTests: XCTestCase {
    func testCanonicalSensitivitySurvivesProfileRoundTripAndDefaultPolicyRejectsIt() throws {
        let canonical = IOSCanonicalMemoryRecord(
            record: IOSCanonicalSemanticMemoryRecord(
                id: "sensitive-memory",
                owner: .localProfile,
                namespace: IOSCanonicalMemoryNamespace(kind: "custom", name: "long_term"),
                scope: "global",
                kind: "reference",
                content: "Private health detail",
                summary: nil,
                tags: [],
                confidence: 0.95,
                importance: 0.8,
                sensitivity: "health.private",
                retention: IOSCanonicalMemoryRetention(mode: "permanent", expiresAt: nil),
                supersedes: [],
                provenance: IOSCanonicalMemoryProvenance(
                    kind: "explicit_user",
                    source: "nola://memory/sensitive-memory",
                    sessionID: nil,
                    turnID: nil,
                    itemIDs: [],
                    observedAt: "2026-09-10T00:00:00Z"
                )
            ),
            createdAtMs: 1,
            updatedAtMs: 1,
            lastUsedAtMs: nil,
            deletedAtMs: nil
        )

        var unbound = try XCTUnwrap(JSONSerialization.jsonObject(with: JSONEncoder().encode(canonical.record)) as? [String: Any])
        unbound.removeValue(forKey: "owner")
        XCTAssertThrowsError(try JSONDecoder().decode(IOSCanonicalSemanticMemoryRecord.self,
            from: JSONSerialization.data(withJSONObject: unbound)))
        let profileRecord = try XCTUnwrap(canonical.chatMemoryRecord)
        XCTAssertEqual(profileRecord.canonicalRecord?.record.owner, .localProfile)
        XCTAssertEqual(profileRecord.sensitivity, "health.private")
        XCTAssertEqual(profileRecord.canonicalRecord?.record.sensitivity, "health.private")

        let selectionRecord = CoreMemoryRecord(
            id: profileRecord.id,
            owner: .localProfile,
            namespace: .personal,
            scope: .global,
            kind: .reference,
            content: profileRecord.text,
            confidence: profileRecord.confidence ?? 0.6,
            importance: 0.8,
            sensitivity: profileRecord.sensitivity,
            retention: .permanent,
            provenance: CoreMemoryProvenance(
                kind: .explicitUser,
                source: "nola://memory/sensitive-memory"
            )
        )
        let policy = CoreMemoryPolicy(
            longTermEnabled: true,
            automaticExtractionEnabled: true,
            searchLimit: 8,
            minimumConfidence: 0.5
        )
        let encodedPolicy = try JSONSerialization.jsonObject(
            with: JSONEncoder().encode(policy)
        ) as? [String: Any]
        XCTAssertEqual(encodedPolicy?["allowed_sensitivities"] as? [String], [])
        let unlabeledPolicy = try JSONDecoder().decode(
            CoreMemoryPolicy.self,
            from: Data(
                """
                {"long_term_enabled":true,"automatic_extraction_enabled":true,"search_limit":8,"minimum_confidence":0.5,"maximum_age_days":null}
                """.utf8
            )
        )
        XCTAssertEqual(unlabeledPolicy.allowedSensitivities, [])

        let plan = try XCTUnwrap(PersonalIntelligenceCore.selectMemories(
            CoreMemorySelectionInput(
                policy: policy,
                records: [selectionRecord],
                allowedNamespaces: [.personal],
                now: "2026-09-10T00:00:01Z",
                maximumInputTokens: 1_024,
                tokenAccounting: .coreEstimate
            )
        ))
        XCTAssertTrue(plan.selected.isEmpty)
        XCTAssertEqual(plan.rejected.first?.memoryID, "sensitive-memory")
        XCTAssertEqual(plan.rejected.first?.reason, .sensitivityNotAllowed)
    }

    func testStructuredWriteIsIdempotentAndPreservesRealProvenance() throws {
        let conversationID = UUID()
        let observedAt = Date(timeIntervalSince1970: 1_700_000_000)
        let provenance = ChatMemoryProvenance(
            source: "assistant_memory_tool",
            sourceID: "event-1",
            conversationID: conversationID,
            model: "test-model",
            route: "cloud"
        )
        let existing = ChatMemoryRecord(
            id: "memory:existing",
            namespace: .longTerm,
            kind: .semantic,
            text: "Existing fact",
            provenance: provenance,
            observedAt: observedAt
        )
        let profile = ChatMemoryProfile(
            longTermMemory: "Existing fact",
            memoryRecords: [existing]
        )
        let first = profile.addingMemory(
            longTerm: ["Existing fact", "New fact"],
            preferences: ["Prefer tables"],
            provenance: provenance,
            observedAt: observedAt
        )
        let second = first.addingMemory(
            longTerm: ["New fact"],
            preferences: ["Prefer tables"],
            provenance: provenance,
            observedAt: observedAt
        )

        XCTAssertEqual(second.memoryRecords, first.memoryRecords)
        XCTAssertEqual(second.memoryRecords.count, 3)
        let newFact = try XCTUnwrap(second.memoryRecords.first { $0.text == "New fact" })
        XCTAssertEqual(newFact.namespace, .longTerm)
        XCTAssertEqual(newFact.kind, .semantic)
        XCTAssertEqual(newFact.confidence, 0.76)
        XCTAssertEqual(newFact.provenance, provenance)
        XCTAssertEqual(newFact.observedAt, observedAt)
        XCTAssertEqual(newFact.retention, .durable)
        XCTAssertTrue(newFact.supersedes.isEmpty)
        XCTAssertEqual(second.memoryRecords.first { $0.text == "Existing fact" }?.observedAt, observedAt)
    }
}

extension ChatMemoryProfileTests {
    func testCoreRejectedMemoryNeverReturnsThroughClientFallback() async throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent("memory-policy-\(UUID().uuidString)")
        defer { try? FileManager.default.removeItem(at: root) }
        let runtime = try IOSApplicationRuntime(databaseURL: root.appendingPathComponent("product.db"))
        let store = try LocalConversationStore(applicationRuntime: runtime)
        let conversation = try store.createConversation(title: "Policy isolation", language: "en")
        try store.importMessages(conversationId: conversation.id, messages: [
            ConversationMessage(conversationId: conversation.id, role: .user, text: "Fixture history")
        ], source: .externalArchive)
        let profileStore = CanonicalChatMemoryProfileStore(applicationRuntime: runtime)
        let manager = DefaultContextManager(store: store, profileStore: profileStore)
        let sensitive = ChatMemoryRecord(
            id: "private-policy-marker", namespace: .longTerm, kind: .semantic,
            text: "private marker that the default policy excludes", confidence: 0.95,
            provenance: ChatMemoryProvenance(source: "manual_profile_edit"),
            observedAt: Date(), sensitivity: "health.private"
        )
        try runtime.upsertMemory(XCTUnwrap(sensitive.canonicalRecord))
        let emptySelection = try await manager.prepareOutboundPayload(for: conversation.id,
            maxTokens: 8_000, additionalComponents: [], tools: [])
        XCTAssertFalse(emptySelection.contextComponents.contains { $0.kind == .memory })

        let ordinary = ChatMemoryRecord(
            id: "ordinary-policy-marker", namespace: .longTerm, kind: .semantic,
            text: "A normal durable project preference", confidence: 0.95,
            provenance: ChatMemoryProvenance(source: "manual_profile_edit"), observedAt: Date()
        )
        try runtime.upsertMemory(XCTUnwrap(ordinary.canonicalRecord))
        let selected = try await manager.prepareOutboundPayload(for: conversation.id,
            maxTokens: 8_000, additionalComponents: [], tools: [])
        let ids = Set(selected.contextComponents.filter { $0.kind == .memory }.map(\.id))
        XCTAssertEqual(ids, ["profile:memory:ordinary-policy-marker"])
    }
}
