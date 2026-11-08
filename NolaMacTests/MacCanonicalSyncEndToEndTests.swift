import Foundation
import XCTest
@testable import Nola

final class MacCanonicalSyncEndToEndTests: XCTestCase {
    func testRemoteReplicaPageAppliesThroughRealCoreABIAndSurvivesColdReopen() throws {
        let root = FileManager.default.temporaryDirectory
            .appendingPathComponent("nola-mac-remote-sync-e2e-\(UUID().uuidString)", isDirectory: true)
        let sourceURL = root.appendingPathComponent("source.db")
        let targetURL = root.appendingPathComponent("target.db")
        defer { try? FileManager.default.removeItem(at: root) }

        let pending: MacCanonicalPendingSyncChange
        do {
            let source = try MacApplicationPort(databaseURL: sourceURL)
            _ = try source.putProductSetting(
                key: "assistant.memory.enabled",
                valueJSON: "true",
                updatedAtMs: 100
            )
            pending = try XCTUnwrap(source.pendingSyncChanges().first)
        }

        do {
            let target = try MacApplicationPort(databaseURL: targetURL)
            let result = try target.applyRemoteSyncPage(
                replicaID: "account-replica",
                changes: [.init(
                    cursor: 1,
                    operationID: pending.operationID,
                    entityType: pending.entityType,
                    entityID: pending.entityID,
                    revision: pending.revision,
                    deleted: pending.deleted,
                    payloadJSON: pending.payloadJSON,
                    createdAtMs: pending.createdAtMs
                )],
                nextCursor: 1,
                appliedAtMs: 200
            )
            XCTAssertEqual(result.applied, 1)
            XCTAssertEqual(result.skipped, 0)
        }

        let reopened = try MacApplicationPort(databaseURL: targetURL)
        XCTAssertEqual(try reopened.productSetting(key: "assistant.memory.enabled")?.valueJSON, "true")
        XCTAssertEqual(try reopened.syncCursor(replicaID: "account-replica")?.cursor, 1)
        XCTAssertTrue(try reopened.pendingSyncChanges().isEmpty)
    }

    func testProductSettingOutboxAcknowledgementTombstoneAndCursorSurviveColdReopen() throws {
        let root = FileManager.default.temporaryDirectory
            .appendingPathComponent("nola-mac-sync-e2e-\(UUID().uuidString)", isDirectory: true)
        let databaseURL = root.appendingPathComponent("product.db")
        defer { try? FileManager.default.removeItem(at: root) }

        let operationID: String
        do {
            let store = try MacApplicationPort(databaseURL: databaseURL)
            _ = try store.putProductSetting(
                key: "assistant.memory.enabled",
                valueJSON: #"{"enabled":true}"#,
                updatedAtMs: 10
            )
            let pending = try store.pendingSyncChanges()
            XCTAssertEqual(pending.count, 1)
            XCTAssertEqual(pending[0].entityType, "product_setting")
            operationID = pending[0].operationID
        }

        do {
            let store = try MacApplicationPort(databaseURL: databaseURL)
            XCTAssertEqual(try store.pendingSyncChanges()[0].operationID, operationID)
            XCTAssertTrue(try store.resolveSyncChange(
                operationID: operationID,
                status: "acknowledged",
                remoteCursor: 7,
                resolvedAtMs: 20
            ))
            XCTAssertEqual(try store.saveSyncCursor(replicaID: "primary", cursor: 7, updatedAtMs: 20).cursor, 7)
            XCTAssertTrue(try store.deleteProductSetting(key: "assistant.memory.enabled"))
        }

        let reopened = try MacApplicationPort(databaseURL: databaseURL)
        XCTAssertEqual(try reopened.syncCursor(replicaID: "primary")?.cursor, 7)
        let tombstones = try reopened.pendingSyncChanges()
        XCTAssertEqual(tombstones.count, 1)
        XCTAssertTrue(tombstones[0].deleted)
        XCTAssertEqual(tombstones[0].revision, 2)
    }
}
