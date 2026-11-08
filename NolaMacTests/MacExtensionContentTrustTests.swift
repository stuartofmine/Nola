// SPDX-License-Identifier: Apache-2.0

import XCTest
@testable import Nola

final class MacExtensionContentTrustTests: XCTestCase {
    private var root: URL!
    private var applicationPort: MacApplicationPort!

    override func setUpWithError() throws {
        root = FileManager.default.temporaryDirectory
            .appendingPathComponent("nola-extension-trust-\(UUID().uuidString)", isDirectory: true)
        try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
        applicationPort = try MacApplicationPort(
            databaseURL: root.appendingPathComponent("nola-product.db")
        )
    }

    override func tearDownWithError() throws {
        if let root { try? FileManager.default.removeItem(at: root) }
        applicationPort = nil
        root = nil
    }

    func testChangingSkillContentInvalidatesPreviousActivation() throws {
        let skill = root.appendingPathComponent("SKILL.md")
        try "# Skill: Review\nRead only.".write(to: skill, atomically: true, encoding: .utf8)
        let originalID = MacLocalSkillActivationStore.itemID(
            source: .workspace,
            instructionURL: skill
        )
        try MacLocalSkillActivationStore.setEnabled(
            true,
            itemID: originalID,
            store: applicationPort
        )
        XCTAssertTrue(MacLocalSkillActivationStore.isEnabled(
            itemID: originalID,
            store: applicationPort
        ))

        try "# Skill: Review\nRun an unreviewed command.".write(
            to: skill,
            atomically: true,
            encoding: .utf8
        )
        let modifiedID = MacLocalSkillActivationStore.itemID(
            source: .workspace,
            instructionURL: skill
        )
        XCTAssertNotEqual(originalID, modifiedID)
        XCTAssertFalse(MacLocalSkillActivationStore.isEnabled(
            itemID: modifiedID,
            store: applicationPort
        ))
    }

    func testChangingMCPServerConfigurationInvalidatesPreviousActivation() throws {
        let configuration = root.appendingPathComponent(".mcp.json")
        try #"{"mcpServers":{"files":{"command":"safe-server","args":["--readonly"]}}}"#
            .write(to: configuration, atomically: true, encoding: .utf8)
        let originalID = MacLocalMCPActivationStore.itemID(
            source: .workspace,
            configurationURL: configuration,
            serverName: "files"
        )
        try MacLocalMCPActivationStore.setEnabled(
            true,
            itemID: originalID,
            store: applicationPort
        )
        XCTAssertTrue(MacLocalMCPActivationStore.isEnabled(
            itemID: originalID,
            store: applicationPort
        ))

        try #"{"mcpServers":{"files":{"command":"different-server","args":[]}}}"#
            .write(to: configuration, atomically: true, encoding: .utf8)
        let modifiedID = MacLocalMCPActivationStore.itemID(
            source: .workspace,
            configurationURL: configuration,
            serverName: "files"
        )
        XCTAssertNotEqual(originalID, modifiedID)
        XCTAssertFalse(MacLocalMCPActivationStore.isEnabled(
            itemID: modifiedID,
            store: applicationPort
        ))
    }

    func testSkillActivationUsesRevisionAndCannotResurrectDeletedIdentity() throws {
        let skill = root.appendingPathComponent("SKILL.md")
        try "# Review\nKeep the exact workflow.\n".write(to: skill, atomically: true, encoding: .utf8)
        let id = MacLocalSkillActivationStore.itemID(source: .workspace, instructionURL: skill)
        try MacLocalSkillActivationStore.setEnabled(true, itemID: id, store: applicationPort)
        let opened = try XCTUnwrap(applicationPort.skillEditor(id))
        try MacLocalSkillActivationStore.setEnabled(false, itemID: id, store: applicationPort)
        let disabled = try XCTUnwrap(applicationPort.skillEditor(id))
        XCTAssertFalse(disabled.record.enabled)
        XCTAssertGreaterThan(disabled.revision, opened.revision)
        XCTAssertEqual(disabled.record.instructionsText, opened.record.instructionsText)
        XCTAssertEqual(disabled.record.version, opened.record.version)
        XCTAssertEqual(disabled.record.createdAtMs, opened.record.createdAtMs)
        XCTAssertThrowsError(try applicationPort.setSkillEnabled(opened, enabled: true, updatedAtMs: 3))
        XCTAssertEqual(try applicationPort.skillEditor(id), disabled)
        XCTAssertThrowsError(try applicationPort.deleteUserSkill(opened, deletedAtMs: 4))
        XCTAssertEqual(try applicationPort.skillEditor(id), disabled)
        XCTAssertTrue(try applicationPort.deleteUserSkill(disabled, deletedAtMs: 4))
        XCTAssertThrowsError(try MacLocalSkillActivationStore.setEnabled(true, itemID: id, store: applicationPort))
        XCTAssertTrue(try applicationPort.listSkills().isEmpty)
        XCTAssertThrowsError(try MacLocalSkillActivationStore.setEnabled(true, itemID: "invalid", store: applicationPort))
        XCTAssertThrowsError(try MacLocalSkillActivationStore.setEnabled(true, itemID: id, store: nil))
    }

    func testHiddenBuiltInSkillCannotRemainAvailableDuringPartialRestore() throws {
        try MacBuiltInSkillRegistry.setEnabled(true, for: .word, store: applicationPort)
        let baseline = try XCTUnwrap(applicationPort.skillEditor(MacBuiltInSkillID.word.rawValue))
        try MacBuiltInSkillRegistry.setEnabled(false, for: .word, store: applicationPort)
        XCTAssertThrowsError(try applicationPort.setSkillEnabled(baseline, enabled: true, updatedAtMs: 3))
        try MacBuiltInSkillRegistry.setEnabled(true, for: .word, store: applicationPort)
        let data = try JSONSerialization.data(withJSONObject: [
            "schemaVersion": 1, "ids": [MacBuiltInSkillID.word.rawValue]
        ])
        _ = try applicationPort.putProductSetting(key: MacProductSettingKeys.hiddenBuiltInSkills,
            valueJSON: String(decoding: data, as: UTF8.self), updatedAtMs: 4)
        XCTAssertFalse(try MacBuiltInSkillRegistry.activeDescriptors(store: applicationPort).contains { $0.id == .word })
        XCTAssertFalse(try MacBuiltInSkillRegistry.extensionItems(store: applicationPort).contains {
            MacBuiltInSkillRegistry.skillID(forExtensionItemID: $0.id) == .word
        })
        try MacBuiltInSkillRegistry.restoreBuiltInSkills(store: applicationPort)
        XCTAssertFalse(try MacBuiltInSkillRegistry.hasRemovedSkills(store: applicationPort))
        XCTAssertTrue(try MacBuiltInSkillRegistry.activeDescriptors(store: applicationPort).contains { $0.id == .word })
        try MacBuiltInSkillRegistry.remove(.word, store: applicationPort)
        XCTAssertFalse(try XCTUnwrap(applicationPort.skillEditor(MacBuiltInSkillID.word.rawValue)).record.enabled)
    }

    func testBatchSkillActivationRollsBackOnConflictAndPersistsAfterReopen() throws {
        let snapshots = try ["first", "second"].map { name -> MacCanonicalSkillEditor in
            let file = root.appendingPathComponent("\(name).md")
            try "# \(name)\nRead only.\n".write(to: file, atomically: true, encoding: .utf8)
            let id = MacLocalSkillActivationStore.itemID(source: .workspace, instructionURL: file)
            try MacLocalSkillActivationStore.setEnabled(false, itemID: id, store: applicationPort)
            return try XCTUnwrap(applicationPort.skillEditor(id))
        }
        let changed = try applicationPort.setSkillEnabled(
            snapshots[1], enabled: true, updatedAtMs: 50
        )
        XCTAssertThrowsError(try applicationPort.setSkillsEnabled(
            snapshots, enabled: true, updatedAtMs: 60
        ))
        XCTAssertEqual(try applicationPort.skillEditor(snapshots[0].record.id), snapshots[0])
        XCTAssertEqual(try applicationPort.skillEditor(snapshots[1].record.id), changed)
        let committed = try applicationPort.setSkillsEnabled(
            [snapshots[0], changed], enabled: true, updatedAtMs: 70
        )
        XCTAssertEqual(committed.count, 2)
        XCTAssertTrue(committed.allSatisfy { $0.record.enabled })
        for (before, after) in zip([snapshots[0], changed], committed) {
            if before.record.enabled {
                XCTAssertEqual(after, before)
            } else {
                XCTAssertEqual(after.revision, before.revision + 1)
                XCTAssertEqual(after.record.updatedAtMs, 70)
            }
            XCTAssertEqual(after.record.instructionsText, before.record.instructionsText)
            XCTAssertEqual(after.record.createdAtMs, before.record.createdAtMs)
        }
        applicationPort = nil
        applicationPort = try MacApplicationPort(databaseURL: root.appendingPathComponent("nola-product.db"))
        for snapshot in committed {
            XCTAssertEqual(try applicationPort.skillEditor(snapshot.record.id), snapshot)
        }
    }

    func testRestoreRejectsDuplicateIdentityAndStaleVisibilityWithoutActivation() throws {
        try MacBuiltInSkillRegistry.setEnabled(false, for: .word, store: applicationPort)
        let baseline = try XCTUnwrap(applicationPort.skillEditor(MacBuiltInSkillID.word.rawValue))
        XCTAssertThrowsError(try applicationPort.restoreBuiltInSkills(
            [baseline, baseline], visibilityRevision: nil, updatedAtMs: 100
        ))
        XCTAssertEqual(try applicationPort.skillEditor(baseline.record.id), baseline)
        let snapshots = try applicationPort.listSkills().filter { $0.source == .builtIn }.map {
            try XCTUnwrap(applicationPort.skillEditor($0.id))
        }
        let data = try JSONSerialization.data(withJSONObject: [
            "schemaVersion": 1, "ids": [MacBuiltInSkillID.word.rawValue]
        ])
        let hidden = try applicationPort.putProductSetting(
            key: MacProductSettingKeys.hiddenBuiltInSkills,
            valueJSON: String(decoding: data, as: UTF8.self), updatedAtMs: 101
        )
        XCTAssertThrowsError(try applicationPort.restoreBuiltInSkills(
            snapshots, visibilityRevision: nil, updatedAtMs: 102
        ))
        XCTAssertEqual(try applicationPort.skillEditor(baseline.record.id), baseline)
        XCTAssertEqual(try applicationPort.productSetting(key: MacProductSettingKeys.hiddenBuiltInSkills)?.revision,
                       hidden.revision)
        _ = try applicationPort.restoreBuiltInSkills(
            snapshots, visibilityRevision: hidden.revision, updatedAtMs: 103
        )
        XCTAssertTrue(try XCTUnwrap(applicationPort.skillEditor(baseline.record.id)).record.enabled)
        XCTAssertFalse(try MacBuiltInSkillRegistry.hasRemovedSkills(store: applicationPort))
    }
}
