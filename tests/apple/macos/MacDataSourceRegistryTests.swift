// SPDX-License-Identifier: Apache-2.0

import Foundation
import XCTest
@testable import Nola

@MainActor
final class MacDataSourceRegistryTests: XCTestCase {
    func testDisplayOverrideSnapshotIsConversationScopedAndDoesNotReplaceRuntimeReads() async throws {
        let directory = FileManager.default.temporaryDirectory
            .appendingPathComponent("nola-data-display-\(UUID().uuidString)", isDirectory: true)
        let storeURL = directory.appendingPathComponent("registry.json")
        defer { try? FileManager.default.removeItem(at: directory) }
        try createConsumerConversation(storeURL: storeURL, id: "a")
        try createConsumerConversation(storeURL: storeURL, id: "b")
        let registry = MacDataSourceRegistry(storeURL: storeURL)
        let source = MacDataSourceRegistry.platformSourceID("calendar")
        registry.recordValidation(sourceID: source, connectorID: "eventkit.events",
            readerDescription: "fixture", schemaFields: ["title"], redactedSample: "{}",
            recordCount: 1, maximumAgeMinutes: 15, observedAt: .now)
        registry.setPolicy(.conversationOptIn, for: source)
        let revision = registry.overrideDisplayRevision
        registry.setConversationOverride(true, sourceID: source, conversationID: "a")
        XCTAssertGreaterThan(registry.overrideDisplayRevision, revision)
        let first = try await registry.loadOverridesForDisplay(conversationID: "a", sourceIDs: [source, source])
        let second = try await registry.loadOverridesForDisplay(conversationID: "b", sourceIDs: [source])
        XCTAssertEqual(first, [source: true])
        XCTAssertTrue(second.isEmpty)
        XCTAssertEqual(registry.displayResolution(sourceID: source, override: first[source])?.enabled, true)
        registry.setConversationOverride(false, sourceID: source, conversationID: "a")
        XCTAssertFalse(registry.isEnabled(sourceID: source, conversationID: "a"),
            "An old display snapshot must not authorize runtime access")
        let refreshed = try await registry.loadOverridesForDisplay(conversationID: "a", sourceIDs: [source])
        XCTAssertEqual(refreshed, [source: false])
        registry.setPolicy(.blocked, for: source)
        XCTAssertEqual(registry.displayResolution(sourceID: source, override: first[source])?.enabled, false)
        registry.setConversationOverride(nil, sourceID: source, conversationID: "a")
        let inherited = try await registry.loadOverridesForDisplay(conversationID: "a", sourceIDs: [source])
        XCTAssertTrue(inherited.isEmpty)
    }

    private func createConsumerConversation(storeURL: URL, id: String) throws {
        let store = try MacApplicationPort(databaseURL: storeURL.deletingLastPathComponent()
            .appendingPathComponent(MacAppEnvironment.canonicalStoreFileName))
        let now = Int64(Date().timeIntervalSince1970 * 1_000)
        XCTAssertTrue(try store.createConversation(MacCanonicalCreateConversationInput(
            id: id, title: "Data source consumer", settingsJSON: nil,
            createdAtMs: now, updatedAtMs: now
        )).created)
    }

    func testValidatedContractPolicyConversationOverrideAndUsageSurviveRestart() throws {
        let directory = FileManager.default.temporaryDirectory
            .appendingPathComponent("nola-data-source-registry-\(UUID().uuidString)", isDirectory: true)
        let storeURL = directory.appendingPathComponent("registry.json")
        defer { try? FileManager.default.removeItem(at: directory) }

        let sourceID = MacDataSourceRegistry.platformSourceID("calendar")
        try createConsumerConversation(storeURL: storeURL, id: "conversation-a")
        try createConsumerConversation(storeURL: storeURL, id: "conversation-b")
        let registry = MacDataSourceRegistry(storeURL: storeURL)
        registry.recordValidation(
            sourceID: sourceID,
            connectorID: "eventkit.events",
            readerDescription: "EKEventStore.events(next: 30 days)",
            schemaFields: ["title", "startDate"],
            redactedSample: #"{"title":"<redacted>"}"#,
            recordCount: 3,
            maximumAgeMinutes: 15,
            observedAt: .now
        )
        registry.setPolicy(.conversationOptIn, for: sourceID)
        registry.setConversationOverride(true, sourceID: sourceID, conversationID: "conversation-a")
        XCTAssertNil(registry.persistenceError)
        registry.recordUsage(
            sourceID: sourceID,
            sourceTitle: "Calendar",
            consumer: "chat",
            purpose: "context",
            conversationID: "conversation-a",
            disclosedToCloud: true
        )

        XCTAssertFalse(registry.isEnabled(sourceID: sourceID, conversationID: "conversation-b"))
        XCTAssertTrue(registry.isEnabled(sourceID: sourceID, conversationID: "conversation-a"))

        let restored = MacDataSourceRegistry(storeURL: storeURL)
        XCTAssertEqual(restored.contract(for: sourceID)?.recordCount, 3)
        XCTAssertEqual(restored.contract(for: sourceID)?.schemaFields, ["title", "startDate"])
        XCTAssertEqual(restored.policy(for: sourceID), .conversationOptIn)
        XCTAssertTrue(restored.isEnabled(sourceID: sourceID, conversationID: "conversation-a"))
        XCTAssertEqual(restored.usageRecords.first?.conversationID, "conversation-a")

        let canonical = try MacApplicationPort(
            databaseURL: storeURL.deletingLastPathComponent().appendingPathComponent(
                MacAppEnvironment.canonicalStoreFileName
            )
        )
        let stored = try XCTUnwrap(canonical.listDataSources().first { $0.id == sourceID })
        XCTAssertEqual(stored.kind, .platform)
        XCTAssertEqual(stored.configuration.location, sourceID)
    }

    func testBlockedPolicyCannotBecomeEnabledWithoutExplicitConversationOverride() throws {
        let directory = FileManager.default.temporaryDirectory
            .appendingPathComponent("nola-data-source-policy-\(UUID().uuidString)", isDirectory: true)
        let storeURL = directory.appendingPathComponent("registry.json")
        defer { try? FileManager.default.removeItem(at: directory) }
        try createConsumerConversation(storeURL: storeURL, id: "conversation")
        let registry = MacDataSourceRegistry(storeURL: storeURL)
        let sourceID = MacDataSourceRegistry.platformSourceID("contacts")

        registry.setPolicy(.blocked, for: sourceID)
        XCTAssertFalse(registry.isEnabled(sourceID: sourceID, conversationID: nil))
        XCTAssertFalse(registry.isEnabled(sourceID: sourceID, conversationID: "conversation"))

        registry.setConversationOverride(true, sourceID: sourceID, conversationID: "conversation")
        XCTAssertNil(registry.persistenceError, "Blocked access must not be confused with a failed override write")
        XCTAssertFalse(registry.isEnabled(sourceID: sourceID, conversationID: "conversation"))
    }

    func testFailedRevalidationPreservesKnownSchemaAndMarksContractUnusable() throws {
        let directory = FileManager.default.temporaryDirectory
            .appendingPathComponent("nola-data-source-failure-\(UUID().uuidString)", isDirectory: true)
        let storeURL = directory.appendingPathComponent("registry.json")
        defer { try? FileManager.default.removeItem(at: directory) }
        let registry = MacDataSourceRegistry(storeURL: storeURL)
        let sourceID = MacDataSourceRegistry.platformSourceID("reminders")

        registry.recordValidation(
            sourceID: sourceID,
            connectorID: "eventkit.reminders",
            readerDescription: "fetch reminders",
            schemaFields: ["title", "dueDate"],
            redactedSample: #"{"title":"<redacted>"}"#,
            recordCount: 0,
            maximumAgeMinutes: 15,
            observedAt: .now
        )
        registry.recordValidationFailure(
            sourceID: sourceID,
            connectorID: "eventkit.reminders",
            readerDescription: "fetch reminders",
            error: "permission revoked"
        )

        let contract = try XCTUnwrap(registry.contract(for: sourceID))
        XCTAssertEqual(contract.validationState, .failed)
        XCTAssertFalse(contract.isUsable)
        XCTAssertEqual(contract.schemaFields, ["title", "dueDate"])
        XCTAssertEqual(contract.errorMessage, "permission revoked")
    }
}
