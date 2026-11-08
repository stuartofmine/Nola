// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Verifies the complete macOS data-source lifecycle across a real Store reopen.

import Foundation
import XCTest
@testable import Nola

final class MacCanonicalDataSourceEndToEndTests: XCTestCase {
    func testValidatedDefinitionOverrideAndUsageSurviveColdReopen() throws {
        let root = FileManager.default.temporaryDirectory
            .appendingPathComponent("nola-data-source-e2e-\(UUID().uuidString)", isDirectory: true)
        defer { try? FileManager.default.removeItem(at: root) }
        let databaseURL = root.appendingPathComponent("nola.db")
        let sourceID = "web:fixture"
        let now = Int64(Date().timeIntervalSince1970 * 1_000)

        do {
            let store = try MacApplicationPort(databaseURL: databaseURL)
            XCTAssertTrue(try store.createConversation(MacCanonicalCreateConversationInput(
                id: "conversation-a", title: "Data source consumer", settingsJSON: nil,
                createdAtMs: now, updatedAtMs: now
            )).created)
            var contract = MacDataSourceContract(
                sourceID: sourceID,
                connectorID: "mac.test.http",
                connectorVersion: 1,
                readerDescription: "Bounded fixture read",
                readRecipe: MacDataSourceReadRecipe(
                    method: .httpGet,
                    operationID: "mac.test.http.read",
                    configurationKeys: ["url"],
                    outputSchemaJSON: #"{"type":"object","properties":{"title":{"type":"string"}}}"#
                ),
                schemaFields: ["title"],
                redactedSample: "fixture",
                recordCount: 1,
                maximumAgeMinutes: 60,
                validationState: .verified,
                validatedAt: .now,
                observedAt: .now,
                errorMessage: nil,
                validationDigest: nil
            )
            contract.sealValidation()
            let draft = try store.createDataSourceDraft(MacCanonicalDataSourceDraft(
                id: sourceID,
                name: "Fixture",
                kind: .web,
                configuration: MacCanonicalDataSourceConfiguration(
                    connectorID: "mac.test.http",
                    locationKind: .httpURL,
                    location: "https://example.test/data.json",
                    parameters: ["metadataJSON": .string(#"{"version":1}"#)],
                    secretReferences: []
                ),
                accessPolicy: MacDataSourceAccessPolicy.conversationOptIn.rawValue,
                authorization: MacCanonicalDataSourceAuthorization(
                    state: .notRequired,
                    capability: nil,
                    observedAtMs: now
                ),
                freshnessPolicy: MacCanonicalDataSourceFreshnessPolicy(
                    maximumAgeMinutes: 60,
                    allowStaleFallback: false
                ),
                capabilities: ["read", "context"],
                privacy: "cloud_reference",
                createdAtMs: now,
                updatedAtMs: now
            ))
            let pending = try store.requestDataSourceValidation(
                id: sourceID,
                expectedRevision: draft.revision,
                requestedAtMs: now + 1
            )
            let validated = try store.recordDataSourceValidationSuccess(
                id: sourceID,
                validationToken: try XCTUnwrap(pending.validation.validationToken),
                contract: contract,
                recordedAtMs: now + 2
            )
            _ = try store.setDataSourceDefaultEnabled(
                id: sourceID,
                expectedRevision: validated.revision,
                enabled: true,
                updatedAtMs: now + 3
            )
            XCTAssertThrowsError(try store.setDataSourceOverride(
                conversationID: "missing-conversation", sourceID: sourceID,
                value: .enabled, updatedAtMs: now + 4
            ))
            XCTAssertNil(try store.loadConversation("missing-conversation"),
                "An override cannot implicitly create its conversation owner")
            XCTAssertTrue(try store.setDataSourceOverride(
                conversationID: "conversation-a",
                sourceID: sourceID,
                value: .enabled,
                updatedAtMs: now + 4
            ))
            XCTAssertTrue(try store.appendDataSourceUsage(MacCanonicalDataSourceUsageInput(
                id: UUID().uuidString,
                sourceID: sourceID,
                sourceTitle: "Fixture",
                consumer: "chat",
                purpose: "answer",
                conversationID: "conversation-a",
                disclosedToCloud: true,
                metadata: [:],
                usedAtMs: now + 5
            )))
        }

        do {
            let reopened = try MacApplicationPort(databaseURL: databaseURL)
            let record = try XCTUnwrap(reopened.listDataSources().first { $0.id == sourceID })
            XCTAssertEqual(record.name, "Fixture")
            XCTAssertEqual(record.configuration.location, "https://example.test/data.json")
            XCTAssertTrue(record.defaultEnabled)
            XCTAssertEqual(
                try reopened.dataSourceOverride(
                    conversationID: "conversation-a",
                    sourceID: sourceID
                ),
                .enabled
            )
            let usage = try XCTUnwrap(
                reopened.listDataSourceUsage().first { $0.sourceID == sourceID }
            )
            XCTAssertEqual(usage.conversationID, "conversation-a")
            XCTAssertTrue(usage.disclosedToCloud)
        }
    }
}
