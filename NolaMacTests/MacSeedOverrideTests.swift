// SPDX-License-Identifier: Apache-2.0
import XCTest
import SQLite3
@testable import Nola

@MainActor
final class MacSeedOverrideTests: XCTestCase {
    func testMalformedPersistedSeedCannotFallBackToDefaults() throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: root) }
        let database = root.appendingPathComponent("invalid-seed.db")
        let repository = MacCanonicalConversationRepository(store: try MacApplicationPort(databaseURL: database))
        try repository.restoreBackupSnapshot([MacStoredConversation(conversation:
            MacConversation(id: "invalid", title: "Fixture", preview: "", updatedAt: .now,
                isPinned: false, isPersisted: true), messages: [])])
        // Simulate an old/corrupt database without using the validated write API.
        var connection: OpaquePointer?
        XCTAssertEqual(sqlite3_open(database.path, &connection), SQLITE_OK)
        defer { sqlite3_close(connection) }
        XCTAssertEqual(sqlite3_exec(connection,
            "UPDATE conversation_settings SET settings_json='{\"modelOverrides\":{\"configured:model\":{\"seed\":0}}}' WHERE conversation_id='invalid'",
            nil, nil, nil), SQLITE_OK)
        XCTAssertThrowsError(try repository.load(conversationID: "invalid"))
        XCTAssertEqual(sqlite3_exec(connection,
            "UPDATE conversation_settings SET settings_json='{\"modelOverrides\":{\"configured:model\":{\"seed\":{\"kind\":\"clear\"}}}}' WHERE conversation_id='invalid'",
            nil, nil, nil), SQLITE_OK)
        XCTAssertEqual(try repository.load(conversationID: "invalid")?.conversation
            .modelOverrides?["configured:model"]?.seed, .clear)
    }

    func testCanonicalCodecAndResolvedValues() throws {
        var defaults = ModelParameters.webAlignedCloudDefaults
        defaults.seed = 42
        for intent: MacSeedOverride? in [nil, .clear, .value(0), .value(-7), .value(Int64.min), .value(Int64.max)] {
            let override = MacConversationModelOverride(seed: intent)
            let data = try JSONEncoder().encode(override)
            let decoded = try JSONDecoder().decode(MacConversationModelOverride.self, from: data)
            XCTAssertEqual(decoded, override)
            switch intent {
            case .none: XCTAssertEqual(decoded.applying(to: defaults).seed, 42)
            case .clear: XCTAssertNil(decoded.applying(to: defaults).seed)
            case .value(let value): XCTAssertEqual(decoded.applying(to: defaults).seed, Int(value))
            }
            if case .value(let value) = intent {
                let object = try XCTUnwrap(JSONSerialization.jsonObject(with: data) as? [String: Any])
                XCTAssertEqual((object["seed"] as? [String: Any])?["value"] as? String, String(value))
            }
        }
    }

    func testInvalidAndLegacyShapesAreRejected() throws {
        for seed in ["null", "0", "{\"kind\":\"clear\",\"value\":null}",
                     "{\"kind\":\"value\",\"value\":0}",
                     "{\"kind\":\"clear\",\"other\":true}", "{}"] {
            XCTAssertThrowsError(try JSONDecoder().decode(MacConversationModelOverride.self,
                from: Data("{\"seed\":\(seed)}".utf8)))
        }
        for invalid in ["+1", "-0", "01", "-01", " 1", "1 ", "1.0", "1e3", "", "9223372036854775808", "-9223372036854775809"] {
            let object: [String: Any] = ["seed": ["kind": "value", "value": invalid]]
            XCTAssertThrowsError(try JSONDecoder().decode(MacConversationModelOverride.self,
                from: JSONSerialization.data(withJSONObject: object)))
        }
    }

    func testRemoteIntentDistinguishesOmissionClearAndExactInteger() throws {
        let profile = MacRemoteModelProfile(hostID: "fixture", profileID: "model", revision: 1,
            displayName: "Fixture", modelName: "model")
        for intent: MacSeedOverride? in [nil, .clear, .value(0), .value(Int64.min), .value(Int64.max)] {
            let payload = try MacRemoteModelIntent(profile: profile, tier: "balanced",
                overrides: .init(seed: intent)).payload()
            let overrides = try XCTUnwrap(payload["overrides"] as? [String: Any])
            // Exercise the actual Foundation HTTP serialization too, not just
            // an intermediate Swift integer or a floating-point comparison.
            let bytes = try JSONSerialization.data(withJSONObject: overrides)
            let object = try XCTUnwrap(JSONSerialization.jsonObject(with: bytes) as? [String: Any])
            let options = object["generation_options"] as? [String: Any]
            switch intent {
            case .none: XCTAssertNil(options)
            case .clear: XCTAssertTrue(options?["seed"] is NSNull)
            case .value(let value):
                XCTAssertEqual((options?["seed"] as? NSNumber)?.stringValue, String(value))
            }
        }
    }

    func testIsolatedCanonicalPersistenceReopensEveryIntent() throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: root) }
        let database = root.appendingPathComponent("seed.db")
        let intents: [MacSeedOverride?] = [nil, .clear, .value(0), .value(-7), .value(Int64.min), .value(Int64.max)]
        let repository = MacCanonicalConversationRepository(store: try MacApplicationPort(databaseURL: database))
        try repository.restoreBackupSnapshot(intents.enumerated().map { index, intent in
            var conversation = MacConversation(id: "seed-\(index)", title: "Seed", preview: "",
                updatedAt: .now, isPinned: false, isPersisted: true)
            conversation.modelOverrides = ["configured:fixture": MacConversationModelOverride(seed: intent)]
            return MacStoredConversation(conversation: conversation, messages: [])
        })
        let reopened = MacCanonicalConversationRepository(store: try MacApplicationPort(databaseURL: database))
        for (index, intent) in intents.enumerated() {
            let conversation = try XCTUnwrap(reopened.load(conversationID: "seed-\(index)")?.conversation)
            XCTAssertEqual(conversation.modelOverrides?["configured:fixture"]?.seed, intent)
        }
    }

    func testLocalRequestConsumesOnlyResolvedSeed() async throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: root) }
        let environment = MacAppEnvironment(applicationSupportRoot: root, cachesRoot: root)
        let port = try MacApplicationPort(databaseURL: environment.databaseURL)
        var defaults = ModelParameters.webAlignedCloudDefaults
        defaults.seed = 42
        for intent: MacSeedOverride? in [nil, .clear, .value(0), .value(Int64.max)] {
            let parameters = MacConversationModelOverride(seed: intent).applying(to: defaults)
            let request = try MacLocalAgentRunRequest(conversationID: "seed-local", prompt: "fixture",
                projectURL: root, attachmentURLs: [], modelID: "fixture", modelTestMode: true,
                modelParameters: parameters, wireProtocol: .chatCompletions,
                appEnvironment: environment, applicationPort: port)
            let data = try await MacAgentHostRuntime().initialRequestData(for: request)
            let payload = try XCTUnwrap(JSONSerialization.jsonObject(with: data) as? [String: Any])
            let options = try XCTUnwrap(payload["generation_options"] as? [String: Any])
            if let expected = parameters.seed {
                XCTAssertEqual((options["seed"] as? NSNumber)?.stringValue, String(expected))
            } else {
                XCTAssertNil(options["seed"])
            }
        }
    }
}
