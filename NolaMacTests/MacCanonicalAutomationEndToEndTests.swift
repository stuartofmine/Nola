// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Verifies macOS automation definitions cross the real Core FFI and survive SQLite reopen.

import XCTest
@testable import Nola

final class MacCanonicalAutomationEndToEndTests: XCTestCase {
    func testAutomationDefinitionSurvivesColdReopenAndReconcilesDeletion() throws {
        let root = FileManager.default.temporaryDirectory
            .appendingPathComponent("nola-mac-automation-e2e-\(UUID().uuidString)", isDirectory: true)
        defer { try? FileManager.default.removeItem(at: root) }
        let database = root.appendingPathComponent("State/nola.db")
        let createdAt: Int64 = 1_800_000_000_000
        let id = UUID().uuidString.lowercased()
        let metadata = #"{"id":"automation-e2e","frequency":"daily","hour":9,"minute":30}"#
        let spec = MacAutomationExecutionSpec(prompt: "Review", executionScope: .localWorkspace(root: root.path), permissionProfile: "cautious", modelSelection: "use_current_default_at_admission", contextRefs: [], skillRefs: [])

        do {
            let store = try MacApplicationPort(databaseURL: database)
            let inserted = try store.upsertAutomation(
                MacCanonicalAutomationInput(
                    id: id,
                    name: "Daily review",
                    scheduleFormat: "nola.mac.automation.v1",
                    enabled: true,
                    revision: 0,
                    executionSpec: .agent(spec),
                    metadataJSON: metadata,
                    createdAtMs: createdAt,
                    updatedAtMs: createdAt,
                    deletedAtMs: nil
                )
            )
            XCTAssertEqual(inserted.revision, 1)

            let unchanged = try store.upsertAutomation(
                MacCanonicalAutomationInput(
                    id: id,
                    name: "Daily review",
                    scheduleFormat: "nola.mac.automation.v1",
                    enabled: true,
                    revision: inserted.revision,
                    executionSpec: .agent(spec),
                    metadataJSON: metadata,
                    createdAtMs: createdAt,
                    updatedAtMs: createdAt + 1,
                    deletedAtMs: nil
                )
            )
            XCTAssertEqual(unchanged.revision, 1)
        }

        do {
            let reopened = try MacApplicationPort(databaseURL: database)
            let restored = try XCTUnwrap(reopened.listAutomations().first)
            XCTAssertEqual(restored.id, id)
            XCTAssertEqual(restored.name, "Daily review")
            XCTAssertEqual(restored.metadataJSON, metadata)

            _ = try reopened.upsertAutomation(MacCanonicalAutomationInput(
                id: "web-fixture", name: "Web task", scheduleFormat: "script",
                enabled: false, revision: 0, executionSpec: .agent(spec), metadataJSON: #"{"id":"web-fixture","format":"script"}"#,
                createdAtMs: createdAt, updatedAtMs: createdAt, deletedAtMs: nil
            ))

            let remaining = try reopened.reconcileAutomations(
                MacCanonicalReconcileAutomationsInput(
                    inputs: [],
                    reconciledAtMs: createdAt + 2
                )
            )
            XCTAssertTrue(remaining.isEmpty)
        }

        let finalReopen = try MacApplicationPort(databaseURL: database)
        XCTAssertEqual(try finalReopen.listAutomations().map(\.id), ["web-fixture"], "Mac reconciliation must not delete Web tasks")
    }
}
