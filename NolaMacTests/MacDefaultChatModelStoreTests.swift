// SPDX-License-Identifier: Apache-2.0
import Foundation
import SQLite3
import XCTest
@testable import Nola

final class MacDefaultChatModelStoreTests: XCTestCase {
    func testCorruptPreferenceRetainsCASRevisionForRepair() throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent("default-chat-repair-\(UUID().uuidString)")
        defer { try? FileManager.default.removeItem(at: root) }
        let database = root.appendingPathComponent("store.db")
        let application = try MacApplicationPort(databaseURL: database)
        let store = MacDefaultChatModelStore(application: application)
        let saved = try store.set(.automatic, expectedRevision: nil, candidates: [])
        var connection: OpaquePointer?
        XCTAssertEqual(sqlite3_open(database.path, &connection), SQLITE_OK)
        defer { sqlite3_close(connection) }
        XCTAssertEqual(sqlite3_exec(connection, "UPDATE product_settings SET value_json='{}' WHERE key='chat.default_model'", nil, nil, nil), SQLITE_OK)
        XCTAssertThrowsError(try store.snapshot())
        XCTAssertEqual(try store.resolve(candidates: []), .invalidConfiguration)
        let repair = try store.rawSnapshot()
        XCTAssertEqual(repair.revision, saved.revision)
        XCTAssertEqual(repair.valueJSON, "{}")
        XCTAssertEqual(try store.set(.automatic, expectedRevision: repair.revision, candidates: []).selection, .automatic)
    }

    func testCanonicalDefaultCASAndRustResolutionRoundTrip() throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent("default-chat-\(UUID().uuidString)")
        defer { try? FileManager.default.removeItem(at: root) }
        let application = try MacApplicationPort(databaseURL: root.appendingPathComponent("store.db"))
        let store = MacDefaultChatModelStore(application: application)
        let candidates = [MacChatModelCandidate(profileID: "api:a:same"), MacChatModelCandidate(profileID: "local:b:same")]
        XCTAssertEqual(try store.snapshot().selection, .automatic)
        XCTAssertEqual(try store.resolve(candidates: candidates), .resolved(profileID: "api:a:same", origin: .automatic))
        let saved = try store.set(.profile("local:b:same"), expectedRevision: nil, candidates: candidates)
        XCTAssertEqual(saved.revision, 1)
        XCTAssertEqual(try store.resolve(candidates: candidates), .resolved(profileID: "local:b:same", origin: .default))
        XCTAssertEqual(try store.resolve(intent: .explicit("api:a:same"), candidates: candidates), .resolved(profileID: "api:a:same", origin: .conversation))
        XCTAssertThrowsError(try store.set(.profile("api:a:same"), expectedRevision: nil, candidates: candidates))
        XCTAssertEqual(try store.snapshot(), saved)
        XCTAssertEqual(try store.resolve(candidates: [candidates[0]]), .unavailable(profileID: "local:b:same", origin: .default, reason: .missing))
        XCTAssertEqual(try store.resolve(intent: .explicit("absent:same"), candidates: candidates), .unavailable(profileID: "absent:same", origin: .conversation, reason: .missing))
        let reopened = MacDefaultChatModelStore(application: try MacApplicationPort(databaseURL: root.appendingPathComponent("store.db")))
        XCTAssertEqual(try reopened.snapshot(), saved)
        let restored = try store.set(.automatic, expectedRevision: saved.revision, candidates: [])
        XCTAssertEqual(restored.selection, .automatic)
        XCTAssertEqual(try store.resolve(candidates: []), .noAutomaticCandidate)
    }

    func testUnavailableFactsAndMalformedConfigurationFailThroughRustBridge() throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent("default-chat-invalid-\(UUID().uuidString)")
        defer { try? FileManager.default.removeItem(at: root) }
        let application = try MacApplicationPort(databaseURL: root.appendingPathComponent("store.db"))
        let store = MacDefaultChatModelStore(application: application)
        let facts: [(MacChatModelAvailability, MacChatModelUnavailableReason)] = [
            (.disabled, .disabled), (.notInstalled, .notInstalled), (.unsupportedRuntime, .unsupportedRuntime)
        ]
        for (availability, reason) in facts {
            let candidates = [MacChatModelCandidate(profileID: "local:model", availability: availability), MacChatModelCandidate(profileID: "api:other")]
            XCTAssertEqual(try store.resolve(intent: .explicit("local:model"), candidates: candidates), .unavailable(profileID: "local:model", origin: .conversation, reason: reason))
            XCTAssertThrowsError(try store.set(.profile("local:model"), expectedRevision: nil, candidates: candidates))
        }
        XCTAssertEqual(try application.resolveChatModel(intent: .inherit, defaultValueJSON: "{}", candidates: []), .invalidConfiguration)
        XCTAssertEqual(try application.resolveChatModel(intent: .explicit("api:valid"), defaultValueJSON: "{}", candidates: [.init(profileID: "api:valid")]), .resolved(profileID: "api:valid", origin: .conversation))
        XCTAssertThrowsError(try application.putProductSettingChecked(key: MacDefaultChatModelStore.key, valueJSON: "{}", expectedRevision: nil, updatedAtMs: 1))
        XCTAssertNil(try store.snapshot().revision)
        XCTAssertEqual(try store.resolve(intent: .explicit("api:restricted"), candidates: [.init(profileID: "api:restricted", allowedForChat: false)]), .unavailable(profileID: "api:restricted", origin: .conversation, reason: .notAllowedForChat))
    }
}
