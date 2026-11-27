// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Consolidated macOS extensions catalog, content trust, editor exit, MCP activation, built-in skill registry, and open-source model skill/installer suite.

import AppKit
import Foundation
import Testing
import XCTest
@testable import Nola

struct MacMCPActivationTests {
    @Test func mcpEditorCASAndDiscoveredActivationLifecycle() throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString, isDirectory: true)
        try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: root) }
        let databaseURL = root.appendingPathComponent("mcp.db")
        let store = try MacApplicationPort(databaseURL: databaseURL)
        let input = MacCanonicalMCPServerInput(
            id: "test-server", displayName: "Original", enabled: true,
            configurationJSON: "{\"transport\":\"stdio\"}", revision: 0,
            createdAtMs: 10, updatedAtMs: 10, deletedAtMs: nil
        )
        let opened = try store.createMCPServer(input)
        #expect(throws: (any Error).self) { try store.createMCPServer(input) }
        let committed = try store.configureMCPServer(
            opened, displayName: "Edited", enabled: false,
            configurationJSON: opened.configurationJSON, updatedAtMs: 20
        )
        #expect(committed.revision > opened.revision)
        #expect(committed.createdAtMs == opened.createdAtMs)
        #expect(throws: (any Error).self) {
            try store.configureMCPServer(opened, displayName: "Stale", enabled: true,
                configurationJSON: opened.configurationJSON, updatedAtMs: 30)
        }
        #expect(throws: (any Error).self) { try store.deleteMCPServer(opened, deletedAtMs: 30) }
        #expect(try store.listMCPServers() == [committed])
        #expect(try store.deleteMCPServer(committed, deletedAtMs: 40))
        #expect(throws: (any Error).self) { try store.createMCPServer(input) }
        #expect(try store.listMCPServers().isEmpty)

        let configURL = root.appendingPathComponent(".mcp.json")
        try """
        {
          // JSONC comments and trailing commas are accepted.
          "mcpServers": {
            "docs": {"type":"http","url":"https://example.test/mcp",},
            "disabled": {"type":"http","url":"https://disabled.test/mcp"}
          },
        }
        """.write(to: configURL, atomically: true, encoding: .utf8)
        #expect(MacLocalMCPActivationStore.serverNames(in: configURL) == ["disabled", "docs"])
        #expect(MacLocalMCPActivationStore.activeServerConfigurationData(workspaceURL: root, store: store) == nil)
        let itemID = MacLocalMCPActivationStore.itemID(source: .workspace, configurationURL: configURL, serverName: "docs")
        try MacLocalMCPActivationStore.setEnabled(true, itemID: itemID, store: store)
        let reopened = try MacApplicationPort(databaseURL: databaseURL)
        let data = try #require(MacLocalMCPActivationStore.activeServerConfigurationData(workspaceURL: root, store: reopened))
        let servers = try #require(JSONSerialization.jsonObject(with: data) as? [[String: Any]])
        #expect(servers.count == 1)
        #expect(servers[0]["server_id"] as? String == MacLocalMCPActivationStore.serverID(for: itemID))
        #expect(MacLocalMCPActivationStore.serverID(for: itemID) != MacLocalMCPActivationStore.serverID(for: "another-source:docs"))
        #expect(servers[0]["endpoint"] as? String == "https://example.test/mcp")
        #expect(servers[0]["enabled"] as? Bool == true)
        let persisted = try #require(reopened.listMCPServers().first(where: { $0.id == itemID }))
        #expect(persisted.enabled)
        #expect(persisted.displayName == "docs")
    }
}

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

    func testSkillIdentityRemainsStableAcrossContentEdits() throws {
        let skill = root.appendingPathComponent("SKILL.md")
        try "# Skill: Review\nRead only.".write(to: skill, atomically: true, encoding: .utf8)
        let originalID = MacLocalSkillActivationStore.itemID(
            source: .workspace,
            instructionURL: skill
        )
        XCTAssertFalse(originalID.contains("#sha256="))
        try MacLocalSkillActivationStore.setEnabled(
            true,
            itemID: originalID,
            store: applicationPort
        )
        XCTAssertTrue(MacLocalSkillActivationStore.isEnabled(
            itemID: originalID,
            store: applicationPort
        ))

        try "# Skill: Review\nUpdated content.".write(
            to: skill,
            atomically: true,
            encoding: .utf8
        )
        let modifiedID = MacLocalSkillActivationStore.itemID(
            source: .workspace,
            instructionURL: skill
        )
        XCTAssertEqual(originalID, modifiedID)
        XCTAssertTrue(MacLocalSkillActivationStore.isEnabled(
            itemID: modifiedID,
            store: applicationPort
        ))
    }

    func testMCPIdentityRemainsStableAcrossConfigurationEdits() throws {
        let configuration = root.appendingPathComponent(".mcp.json")
        try #"{"mcpServers":{"files":{"command":"safe-server","args":["--readonly"]}}}"#
            .write(to: configuration, atomically: true, encoding: .utf8)
        let originalID = MacLocalMCPActivationStore.itemID(
            source: .workspace,
            configurationURL: configuration,
            serverName: "files"
        )
        XCTAssertFalse(originalID.contains("#sha256="))
        try MacLocalMCPActivationStore.setEnabled(
            true,
            itemID: originalID,
            store: applicationPort
        )
        XCTAssertTrue(MacLocalMCPActivationStore.isEnabled(
            itemID: originalID,
            store: applicationPort
        ))

        try #"{"mcpServers":{"files":{"command":"updated-server","args":["--strict"]}}}"#
            .write(to: configuration, atomically: true, encoding: .utf8)
        let modifiedID = MacLocalMCPActivationStore.itemID(
            source: .workspace,
            configurationURL: configuration,
            serverName: "files"
        )
        XCTAssertEqual(originalID, modifiedID)
        XCTAssertTrue(MacLocalMCPActivationStore.isEnabled(
            itemID: modifiedID,
            store: applicationPort
        ))
    }

    func testManagedSQLiteMCPServerCRUDProjectsIntoLocalCatalogAndActiveSnapshot() async throws {
        let draft = MacManagedMCPServerDraft(
            displayName: "Local Docs",
            transport: .http,
            commandOrURL: "https://docs.example.test/mcp",
            headersText: "Authorization: Bearer secret",
            enabled: true
        )
        let created = try await MacExtensionsService.shared.createManagedMCPServer(draft, store: applicationPort)
        let itemID = "managed-mcp:\(created.id)"
        let item = try await MacExtensionsService.shared
            .localExtensions(workspaceURL: root, store: applicationPort)
            .first { $0.id == itemID }
        let unwrappedItem = try XCTUnwrap(item)
        XCTAssertEqual(unwrappedItem.title, "Local Docs")
        XCTAssertTrue(unwrappedItem.isEnabled)

        let activeData = try XCTUnwrap(MacLocalMCPActivationStore.activeServerConfigurationData(
            workspaceURL: root,
            store: applicationPort
        ))
        let activeServers = try XCTUnwrap(JSONSerialization.jsonObject(with: activeData) as? [[String: Any]])
        XCTAssertEqual(activeServers.count, 1)
        XCTAssertEqual(activeServers[0]["name"] as? String, "Local Docs")
        XCTAssertEqual(activeServers[0]["endpoint"] as? String, "https://docs.example.test/mcp")

        try MacLocalMCPActivationStore.setEnabled(false, itemID: itemID, store: applicationPort)
        XCTAssertFalse(MacLocalMCPActivationStore.isEnabled(itemID: itemID, store: applicationPort))
        XCTAssertNil(MacLocalMCPActivationStore.activeServerConfigurationData(
            workspaceURL: root,
            store: applicationPort
        ))

        let deleted = try await MacExtensionsService.shared.deleteManagedMCPServer(itemID: itemID, store: applicationPort)
        XCTAssertTrue(deleted)
        let remaining = try await MacExtensionsService.shared.localExtensions(workspaceURL: root, store: applicationPort)
        XCTAssertFalse(remaining.contains { $0.id == itemID })
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

@MainActor
final class MacExtensionEditorExitTests: XCTestCase {
    func testConfirmedConversationNavigationIgnoresDeletedTarget() throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent("stale-navigation-\(UUID().uuidString)")
        try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: root) }
        let repository = MacCanonicalConversationRepository(
            store: try MacApplicationPort(databaseURL: root.appendingPathComponent("nola.db")))
        let model = MacWorkspaceModel(canonicalRepository: repository,
            localAgentRuntime: PersistenceTestLocalRuntime(), remoteHost: PersistenceTestRemoteHost())
        let oldID = try XCTUnwrap(model.selectedConversationID)
        model.draft = "keep first"
        model.newConversation()
        let selectedID = model.selectedConversationID
        model.draft = "keep current"
        let navigation = MacExtensionsNavigationState()
        let exit = MacExtensionEditorExitState()
        navigation.leaveHandler = { action in
            exit.request(hasChanges: true, isSaving: false, action: action)
        }
        var section = MacWorkspaceSection.extensions
        let actions = MacWorkspaceNavigationActions(model: model, extensions: navigation,
            currentSection: { section }, setSection: { section = $0 })
        actions.selectConversation(oldID)
        // Simulate another window's deletion being reflected by a list reload.
        model.conversations.removeAll { $0.id == oldID }
        exit.discard(isSaving: false) {}
        XCTAssertEqual(section, .extensions)
        XCTAssertEqual(model.selectedConversationID, selectedID)
        XCTAssertEqual(model.draft, "keep current")
    }

    func testWorkspaceConversationIntentsKeepModelUntouchedUntilDiscard() throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent("navigation-intent-\(UUID().uuidString)")
        try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: root) }
        let repository = MacCanonicalConversationRepository(
            store: try MacApplicationPort(databaseURL: root.appendingPathComponent("nola.db")))
        let model = MacWorkspaceModel(canonicalRepository: repository,
            localAgentRuntime: PersistenceTestLocalRuntime(), remoteHost: PersistenceTestRemoteHost())
        let firstID = try XCTUnwrap(model.selectedConversationID)
        model.draft = "first conversation draft"
        model.newConversation()
        let secondID = try XCTUnwrap(model.selectedConversationID)
        model.draft = "second conversation draft"
        let count = model.conversations.count
        let navigation = MacExtensionsNavigationState()
        let exit = MacExtensionEditorExitState()
        navigation.leaveHandler = { action in
            exit.request(hasChanges: true, isSaving: false, action: action)
        }
        var section = MacWorkspaceSection.extensions
        let actions = MacWorkspaceNavigationActions(model: model, extensions: navigation,
            currentSection: { section }, setSection: { section = $0 })

        // These are the exact actions shared by both sidebar sizes, the menu,
        // and the command palette, using the production workspace model.
        actions.selectConversation(firstID)
        XCTAssertEqual(section, .extensions)
        XCTAssertEqual(model.selectedConversationID, secondID)
        XCTAssertEqual(model.draft, "second conversation draft")
        exit.cancel()
        XCTAssertEqual(model.selectedConversationID, secondID)
        actions.newConversation()
        XCTAssertEqual(model.conversations.count, count)
        exit.cancel()
        XCTAssertEqual(model.conversations.count, count)
        XCTAssertEqual(model.draft, "second conversation draft")

        actions.selectConversation(firstID)
        exit.discard(isSaving: false) {}
        XCTAssertEqual(section, .chat)
        XCTAssertEqual(model.selectedConversationID, firstID)
        XCTAssertEqual(model.draft, "first conversation draft")
        section = .extensions
        actions.newConversation()
        XCTAssertEqual(model.conversations.count, count)
        exit.discard(isSaving: false) {}
        XCTAssertEqual(section, .chat)
        XCTAssertEqual(model.conversations.count, count + 1)
        XCTAssertNotEqual(model.selectedConversationID, firstID)
        XCTAssertNotEqual(model.selectedConversationID, secondID)
    }

    func testCleanEditorLeavesWithoutConfirmation() {
        let exit = MacExtensionEditorExitState()
        var leaves = 0
        exit.request(hasChanges: false, isSaving: false) { leaves += 1 }
        XCTAssertEqual(leaves, 1)
        XCTAssertFalse(exit.needsConfirmation)
    }

    func testNavigationUsesEditorDecisionAndCancelRetainsDraft() {
        let navigation = MacExtensionsNavigationState()
        let exit = MacExtensionEditorExitState()
        var draft = "unsaved skill"
        var destination = "extensions"
        navigation.leaveHandler = { action in
            exit.request(hasChanges: !draft.isEmpty, isSaving: false, action: action)
        }
        navigation.requestLeave { destination = "chat" }
        XCTAssertEqual(destination, "extensions")
        XCTAssertTrue(exit.needsConfirmation)
        exit.cancel()
        XCTAssertEqual(draft, "unsaved skill")
        XCTAssertEqual(destination, "extensions")
        navigation.requestLeave { destination = "models" }
        exit.discard(isSaving: false) { draft = "" }
        XCTAssertEqual(destination, "models")
        XCTAssertTrue(draft.isEmpty)
        XCTAssertFalse(exit.needsConfirmation)
    }

    func testSavingBlocksExitAndDoesNotQueueUnexpectedNavigation() {
        let exit = MacExtensionEditorExitState()
        var leaves = 0
        exit.request(hasChanges: true, isSaving: true) { leaves += 1 }
        exit.discard(isSaving: false) { XCTFail("No discard was requested") }
        XCTAssertEqual(leaves, 0)
        XCTAssertFalse(exit.needsConfirmation)
        exit.request(hasChanges: true, isSaving: false) { leaves += 1 }
        exit.discard(isSaving: true) { XCTFail("Saving must retain the draft") }
        XCTAssertEqual(leaves, 0)
        exit.cancel()
    }

    func testSecondNavigationDoesNotReplaceTheActionBeingConfirmed() {
        let exit = MacExtensionEditorExitState()
        var destination = "extensions"
        exit.request(hasChanges: true, isSaving: false) { destination = "chat" }
        exit.request(hasChanges: true, isSaving: false) { destination = "models" }
        exit.discard(isSaving: false) {}
        XCTAssertEqual(destination, "chat")
    }

    func testNativeCloseReceivesTheActualWindowAndUsesSameExitDecision() {
        let window = NSWindow(contentRect: .zero, styleMask: [.titled, .closable], backing: .buffered, defer: false)
        let guardDelegate = MacEditorWindowCloseGuard.Coordinator()
        let exit = MacExtensionEditorExitState()
        var dirty = true
        var confirmedWindow: NSWindow?
        guardDelegate.shouldPreventClose = { dirty }
        guardDelegate.onCloseAttemptWithWindow = { requestedWindow in
            exit.request(hasChanges: dirty, isSaving: false) { confirmedWindow = requestedWindow }
        }
        guardDelegate.attach(to: window)
        defer { guardDelegate.detach() }
        XCTAssertFalse(guardDelegate.windowShouldClose(window))
        XCTAssertNil(confirmedWindow)
        exit.discard(isSaving: false) { dirty = false }
        XCTAssertTrue(confirmedWindow === window)
        XCTAssertTrue(guardDelegate.windowShouldClose(window))
    }
}

@MainActor
final class MacExtensionsCatalogLoaderTests: XCTestCase {
    private func item(_ id: String, source: MacExtensionItem.Source = .workspace) -> MacExtensionItem {
        .init(id: id, kind: .skill, source: source, title: id, detail: "Fixture", path: nil, isEnabled: true)
    }

    func testAllRefreshIncludesHostAndFailedRefreshPreservesLastSuccessfulCatalog() async {
        var hostCalls = 0
        var failHost = false
        let local = item("local")
        let host = item("remote", source: .connectedHost)
        let loader = MacExtensionsCatalogLoader(local: { _ in [local] }, host: {
            hostCalls += 1
            if failHost { throw NSError(domain: "fixture", code: 1) }
            return [host]
        }, removedSkills: { false })
        await loader.load(includeHost: true, workspaceURL: nil)
        XCTAssertEqual(loader.items, [local, host])
        await loader.load(includeHost: true, workspaceURL: nil)
        XCTAssertEqual(hostCalls, 2)
        XCTAssertEqual(loader.items, [local, host])
        failHost = true
        await loader.load(includeHost: true, workspaceURL: nil)
        XCTAssertEqual(loader.items, [local, host])
        XCTAssertNotNil(loader.errorMessage)
        XCTAssertFalse(loader.isLoading)
        failHost = false
        await loader.load(includeHost: true, workspaceURL: nil)
        XCTAssertNil(loader.errorMessage)
        await loader.load(includeHost: false, workspaceURL: nil)
        XCTAssertEqual(hostCalls, 4)
        XCTAssertEqual(loader.items, [local])
    }

    func testLatestSourceWinsWhenEarlierLocalDiscoveryFinishesLast() async {
        let entered = expectation(description: "Earlier local discovery suspended")
        var pending: CheckedContinuation<[MacExtensionItem], Never>?
        var calls = 0
        var hostCalls = 0
        let local = item("new-local")
        let remote = item("remote", source: .connectedHost)
        let loader = MacExtensionsCatalogLoader(local: { _ in
            calls += 1
            if calls == 1 {
                return await withCheckedContinuation { continuation in
                    pending = continuation
                    entered.fulfill()
                }
            }
            return [local]
        }, host: { hostCalls += 1; return [remote] }, removedSkills: { false })
        let stale = Task { await loader.load(includeHost: false, workspaceURL: nil) }
        await fulfillment(of: [entered], timeout: 2)
        XCTAssertTrue(loader.isLoading)
        await loader.load(includeHost: true, workspaceURL: nil)
        XCTAssertEqual(loader.items, [local, remote])
        pending?.resume(returning: [item("stale-local")])
        await stale.value
        XCTAssertEqual(loader.items, [local, remote])
        XCTAssertEqual(hostCalls, 1)
        XCTAssertFalse(loader.isLoading)
    }

    func testDisappearanceInvalidatesInFlightHostPublication() async {
        let entered = expectation(description: "Host query suspended")
        var pending: CheckedContinuation<[MacExtensionItem], Never>?
        let loader = MacExtensionsCatalogLoader(local: { _ in [] }, host: {
            await withCheckedContinuation { continuation in
                pending = continuation
                entered.fulfill()
            }
        }, removedSkills: { false })
        let request = Task { await loader.load(includeHost: true, workspaceURL: nil) }
        await fulfillment(of: [entered], timeout: 2)
        loader.invalidate()
        pending?.resume(returning: [item("stale-host", source: .connectedHost)])
        await request.value
        XCTAssertTrue(loader.items.isEmpty)
        XCTAssertFalse(loader.isLoading)
    }
}

@MainActor
final class MacBuiltInSkillRegistryTests: XCTestCase {
    func testMacCorePolicyExposesOnlyFormalSkillCapability() throws {
        let plan = try MacCoreToolPolicy.localPlan(
            executableURL: URL(fileURLWithPath: "/bin/echo"),
            projectURL: URL(fileURLWithPath: NSTemporaryDirectory(), isDirectory: true),
            sessionId: "skill-policy-contract"
        )

        let skill = try XCTUnwrap(plan.output.enabledTools.first { $0.toolId == "skill" })
        XCTAssertEqual(skill.toolId, "skill")
        XCTAssertEqual(skill.name, "skill")
        XCTAssertEqual(skill.authorization, "granted")
        XCTAssertNil(plan.output.enabledTools.first { $0.toolId == "skill_context" })
        for id in ["github_skill_find", "github_skill_install"] {
            let tool = try XCTUnwrap(plan.output.enabledTools.first { $0.toolId == id && $0.enabled })
            XCTAssertEqual(tool.route, "agent_host")
            XCTAssertEqual(tool.authorization, "granted")
        }
    }

    private var defaults: UserDefaults!
    private var defaultsSuiteName: String!

    override func setUp() {
        super.setUp()
        defaultsSuiteName = "MacBuiltInSkillRegistryTests.\(UUID().uuidString)"
        defaults = UserDefaults(suiteName: defaultsSuiteName)
        defaults.removePersistentDomain(forName: defaultsSuiteName)
        try? MacBuiltInSkillRegistry.restoreBuiltInSkills()
    }

    override func tearDown() {
        try? MacBuiltInSkillRegistry.restoreBuiltInSkills()
        defaults.removePersistentDomain(forName: defaultsSuiteName)
        defaults = nil
        defaultsSuiteName = nil
        super.tearDown()
    }

    func testRegistersEveryBuiltInSkill() {
        let descriptors = MacBuiltInSkillRegistry.descriptors

        XCTAssertEqual(descriptors.map(\.id), MacBuiltInSkillID.allCases)
        XCTAssertEqual(Set(descriptors.map(\.instructionURL.lastPathComponent)), Set(["SKILL.md"]))
        XCTAssertTrue(descriptors.allSatisfy { FileManager.default.fileExists(atPath: $0.instructionURL.path) })
    }

    func testBuiltInEditorReadsCommittedContentAndRevisionInsteadOfMutableFile() async throws {
        let root = FileManager.default.temporaryDirectory
            .appendingPathComponent("nola-skill-editor-snapshot-\(UUID().uuidString)", isDirectory: true)
        defer { try? FileManager.default.removeItem(at: root) }
        let overrides = root.appendingPathComponent("overrides", isDirectory: true)
        let store = try MacApplicationPort(databaseURL: root.appendingPathComponent("state/nola.db"))
        let url = try MacBuiltInSkillRegistry.saveOverride(
            "Committed instructions", for: .word, overridesRootURL: overrides
        )
        try MacBuiltInSkillRegistry.synchronizeCatalog(overridesRootURL: overrides, store: store)
        let item = try XCTUnwrap(MacBuiltInSkillRegistry.extensionItems(store: store).first {
            MacBuiltInSkillRegistry.skillID(forExtensionItemID: $0.id) == .word
        })
        let editor = try XCTUnwrap(store.skillEditor(MacBuiltInSkillID.word.rawValue))
        try Data("Uncommitted external changes".utf8).write(to: url)
        let document = try await MacExtensionsService.shared.document(for: item, store: store)
        XCTAssertEqual(document.content, "Committed instructions")
        XCTAssertEqual(document.canonicalSkillRevision, editor.revision)
        XCTAssertEqual(document.url, url)
        XCTAssertEqual(try store.skillEditor(editor.record.id)?.revision, editor.revision)
    }

    func testMaterializedNolaSkillDoesNotFalsifyOverride() throws {
        let root = FileManager.default.temporaryDirectory
            .appendingPathComponent("nola-skill-cache-\(UUID().uuidString)", isDirectory: true)
        defer { try? FileManager.default.removeItem(at: root) }
        let cacheRoot = root.appendingPathComponent("BuiltInSkillsCache", isDirectory: true)
        let overridesRoot = root.appendingPathComponent("BuiltInSkills", isDirectory: true)
        let skillJSON = """
        {
          "id": "word-document-authoring",
          "source": "built_in",
          "version": "1",
          "display_name": "Create Word Document",
          "summary": "Word docs",
          "category": "documents",
          "instructions_ref": "nola://skills/built-in/word-document-authoring",
          "instructions_text": "# Word\\nCached content.",
          "required_tools": [],
          "default_mode": "inline",
          "enabled": true,
          "created_at_ms": 1,
          "updated_at_ms": 1,
          "deleted_at_ms": null
        }
        """
        let record = try JSONDecoder().decode(MacCanonicalSkill.self, from: Data(skillJSON.utf8))
        let url = try MacBuiltInSkillRegistry.materializedInstructionURL(
            for: .word,
            record: record,
            cacheRootURL: cacheRoot
        )
        XCTAssertTrue(url.isFileURL)
        XCTAssertEqual(try String(contentsOf: url, encoding: .utf8), "# Word\nCached content.")
        XCTAssertFalse(MacBuiltInSkillRegistry.hasOverride(for: .word, overridesRootURL: overridesRoot))
    }

    func testMissingBundleFailsStartupWithoutReplacingInstalledCatalog() throws {
        let root = FileManager.default.temporaryDirectory
            .appendingPathComponent("nola-incomplete-bundle-\(UUID().uuidString)", isDirectory: true)
        defer { try? FileManager.default.removeItem(at: root) }
        let store = try MacApplicationPort(databaseURL: root.appendingPathComponent("state/nola.db"))
        XCTAssertThrowsError(try MacBuiltInSkillRegistry.synchronizeCatalog(
            store: store, resourceRootURL: nil
        ))
        XCTAssertTrue(try store.skillEditors().isEmpty)
        try MacBuiltInSkillRegistry.synchronizeCatalog(store: store)
        let baseline = try store.skillEditors()
        let partialBundle = root.appendingPathComponent("partial-bundle", isDirectory: true)
        let word = partialBundle.appendingPathComponent(MacBuiltInSkillID.word.resourceDirectory, isDirectory: true)
        try FileManager.default.createDirectory(at: word, withIntermediateDirectories: true)
        try Data("Changed instructions in incomplete package".utf8).write(to: word.appendingPathComponent("SKILL.md"))
        XCTAssertThrowsError(try MacBuiltInSkillRegistry.synchronizeCatalog(
            overridesRootURL: root.appendingPathComponent("overrides"),
            store: store, resourceRootURL: partialBundle
        ))
        let afterFailure = try store.skillEditors()
        XCTAssertEqual(afterFailure.map { $0.record.id }, baseline.map { $0.record.id })
        XCTAssertEqual(afterFailure.map(\.revision), baseline.map(\.revision))
        XCTAssertEqual(afterFailure.map { $0.record.instructionsText }, baseline.map { $0.record.instructionsText })
    }

    func testRelocatedBundleRegistersANewImmutableVersionWithTheSameContent() throws {
        let root = FileManager.default.temporaryDirectory
            .appendingPathComponent("nola-relocated-skills-\(UUID().uuidString)", isDirectory: true)
        defer { try? FileManager.default.removeItem(at: root) }
        let source = try XCTUnwrap(MacBuiltInSkillRegistry.resourceRootURL)
        let relocated = root.appendingPathComponent("relocated-bundle", isDirectory: true)
        try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
        try FileManager.default.copyItem(at: source, to: relocated)
        let overrides = root.appendingPathComponent("overrides", isDirectory: true)
        let store = try MacApplicationPort(databaseURL: root.appendingPathComponent("state/nola.db"))
        try MacBuiltInSkillRegistry.synchronizeCatalog(overridesRootURL: overrides, store: store)
        let before = try XCTUnwrap(store.skillEditor(MacBuiltInSkillID.word.rawValue)).record
        try MacBuiltInSkillRegistry.synchronizeCatalog(
            overridesRootURL: overrides, store: store, resourceRootURL: relocated
        )
        let after = try XCTUnwrap(store.skillEditor(before.id)).record
        XCTAssertEqual(after.instructionsText, before.instructionsText)
        XCTAssertNotEqual(after.instructionsRef, before.instructionsRef)
        XCTAssertNotEqual(after.version, before.version)
        let revision = try store.skillEditor(before.id)?.revision
        try MacBuiltInSkillRegistry.synchronizeCatalog(
            overridesRootURL: overrides, store: store, resourceRootURL: relocated
        )
        XCTAssertEqual(try store.skillEditor(before.id)?.revision, revision)
    }

    func testCatalogMetadataRefreshIsDecidedByApplicationRatherThanTextDigest() throws {
        let root = FileManager.default.temporaryDirectory
            .appendingPathComponent("nola-catalog-metadata-\(UUID().uuidString)", isDirectory: true)
        defer { try? FileManager.default.removeItem(at: root) }
        let store = try MacApplicationPort(databaseURL: root.appendingPathComponent("state/nola.db"))
        try MacBuiltInSkillRegistry.synchronizeCatalog(store: store)
        let original = try XCTUnwrap(store.skillEditor(MacBuiltInSkillID.word.rawValue)).record
        let stale = try XCTUnwrap(store.synchronizeBuiltInSkills([MacCanonicalSkillInput(
            id: original.id, source: .builtIn, version: original.version,
            displayName: original.displayName, summary: "Outdated package description",
            category: "outdated-category", instructionsRef: original.instructionsRef,
            instructionsText: original.instructionsText,
            requiredTools: original.requiredTools, defaultMode: original.defaultMode,
            enabled: original.enabled, createdAtMs: original.createdAtMs, updatedAtMs: original.updatedAtMs
        )]).first)
        try MacBuiltInSkillRegistry.synchronizeCatalog(store: store)
        let refreshed = try XCTUnwrap(store.skillEditor(original.id))
        XCTAssertEqual(refreshed.record.summary, original.summary)
        XCTAssertEqual(refreshed.record.category, original.category)
        XCTAssertGreaterThan(refreshed.revision, stale.revision)
        let revisions = try store.skillEditors().map(\.revision)
        try MacBuiltInSkillRegistry.synchronizeCatalog(store: store)
        XCTAssertEqual(try store.skillEditors().map(\.revision), revisions)
    }

    func testUnchangedBuiltInCatalogDoesNotWriteAnotherDurableCatalogReceipt() throws {
        let root = FileManager.default.temporaryDirectory
            .appendingPathComponent("nola-stable-skill-catalog-\(UUID().uuidString)", isDirectory: true)
        defer { try? FileManager.default.removeItem(at: root) }
        let database = root.appendingPathComponent("state/nola.db")
        let store = try MacApplicationPort(databaseURL: database)

        try MacBuiltInSkillRegistry.synchronizeCatalog(store: store)
        let initialReceipts = try builtInSynchronizationReceiptCount(database: database)
        XCTAssertEqual(initialReceipts, 1)

        try MacBuiltInSkillRegistry.synchronizeCatalog(store: store)
        XCTAssertEqual(
            try builtInSynchronizationReceiptCount(database: database),
            initialReceipts
        )
    }

    private func builtInSynchronizationReceiptCount(database: URL) throws -> Int {
        let sqlite = Process()
        let output = Pipe()
        sqlite.executableURL = URL(fileURLWithPath: "/usr/bin/sqlite3")
        sqlite.arguments = [
            database.path,
            "SELECT count(*) FROM kv "
                + "WHERE key LIKE 'cmd_receipt:%' "
                + "AND json_type(value_json, '$.receipt.result') = 'array' "
                + "AND json_extract(value_json, '$.receipt.result[0].record.source') = 'built_in';",
        ]
        sqlite.standardOutput = output
        sqlite.standardError = output
        try sqlite.run()
        sqlite.waitUntilExit()
        let text = String(decoding: output.fileHandleForReading.readDataToEndOfFile(), as: UTF8.self)
            .trimmingCharacters(in: .whitespacesAndNewlines)
        guard sqlite.terminationStatus == 0, let count = Int(text) else {
            throw NSError(
                domain: "MacBuiltInSkillRegistryTests",
                code: Int(sqlite.terminationStatus),
                userInfo: [NSLocalizedDescriptionKey: text]
            )
        }
        return count
    }

    func testFrozenSkillSnapshotKeepsDirectoryAuthorityConsistentAfterToggle() throws {
        let root = FileManager.default.temporaryDirectory
            .appendingPathComponent("nola-frozen-skills-\(UUID().uuidString)", isDirectory: true)
        defer { try? FileManager.default.removeItem(at: root) }
        let store = try MacApplicationPort(databaseURL: root.appendingPathComponent("state/nola.db"))
        try MacBuiltInSkillRegistry.synchronizeCatalog(store: store)
        let frozen = try MacBuiltInSkillRegistry.activeDescriptors(store: store)
        let word = try XCTUnwrap(frozen.first { $0.id == .word })
        let directory = word.instructionURL.deletingLastPathComponent().standardizedFileURL.path
        try MacBuiltInSkillRegistry.setEnabled(false, for: .word, store: store)
        let revisions = try store.skillEditors().map(\.revision)
        XCTAssertTrue(MacBuiltInSkillRegistry.allowedDirectoryArguments(
            projectURL: root, activeSkills: frozen
        ).contains(directory))
        XCTAssertTrue(MacBuiltInSkillRegistry.skillDirectoryArguments(activeSkills: frozen).contains(directory))
        XCTAssertEqual(try store.skillEditors().map(\.revision), revisions)
        let next = try MacBuiltInSkillRegistry.activeDescriptors(store: store)
        XCTAssertFalse(MacBuiltInSkillRegistry.skillDirectoryArguments(activeSkills: next).contains(directory))
        XCTAssertFalse(MacBuiltInSkillRegistry.allowedDirectoryArguments(
            projectURL: root, activeSkills: next
        ).contains(directory))
    }

    func testInvalidLastPackageCannotPartiallyUpdateTheInstalledCatalog() throws {
        let root = FileManager.default.temporaryDirectory
            .appendingPathComponent("nola-catalog-preparation-\(UUID().uuidString)", isDirectory: true)
        defer { try? FileManager.default.removeItem(at: root) }
        let overrides = root.appendingPathComponent("overrides", isDirectory: true)
        let store = try MacApplicationPort(databaseURL: root.appendingPathComponent("state/nola.db"))
        XCTAssertTrue(try MacBuiltInSkillRegistry.extensionItems(store: store).isEmpty)
        XCTAssertTrue(try store.skillEditors().isEmpty)
        try MacBuiltInSkillRegistry.synchronizeCatalog(overridesRootURL: overrides, store: store)
        let installedGallery = try MacBuiltInSkillRegistry.extensionItems(store: store)
        let installedActive = try MacBuiltInSkillRegistry.activeDescriptors(store: store)
        let baseline = try store.skillEditors()
        let ids = MacBuiltInSkillID.allCases
        let first = try XCTUnwrap(ids.first)
        let last = try XCTUnwrap(ids.last)
        XCTAssertNotEqual(first, last)
        for (id, bytes) in [(first, Data("Changed package instructions".utf8)), (last, Data([0xFF]))] {
            let directory = overrides.appendingPathComponent(id.resourceDirectory, isDirectory: true)
            try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
            try bytes.write(to: directory.appendingPathComponent("SKILL.md"))
        }
        XCTAssertThrowsError(try MacBuiltInSkillRegistry.synchronizeCatalog(
            overridesRootURL: overrides, store: store
        ))
        XCTAssertEqual(try MacBuiltInSkillRegistry.extensionItems(store: store), installedGallery)
        XCTAssertEqual(try MacBuiltInSkillRegistry.activeDescriptors(store: store), installedActive)
        let afterFailure = try store.skillEditors()
        XCTAssertEqual(afterFailure.map { $0.record.id }, baseline.map { $0.record.id })
        XCTAssertEqual(afterFailure.map(\.revision), baseline.map(\.revision))
        XCTAssertEqual(afterFailure.map { $0.record.instructionsText }, baseline.map { $0.record.instructionsText })
    }

    func testExtensionsDiscoveryIncludesEveryBuiltInSkillWithoutAWorkspace() async throws {
        let items = try await MacExtensionsService.shared.localExtensions(workspaceURL: nil)
        let builtInSkillIDs = items
            .filter { $0.kind == .skill && $0.source == .builtIn }
            .compactMap { MacBuiltInSkillRegistry.skillID(forExtensionItemID: $0.id) }

        XCTAssertEqual(Set(builtInSkillIDs), Set(MacBuiltInSkillID.allCases))
    }

    func testExtensionCatalogStartsAsGalleryAndGroupsSkillsBeforeOpeningDetail() {
        let skill = MacExtensionItem(
            id: "builtin-skill:\(MacBuiltInSkillID.word.rawValue)",
            kind: .skill,
            source: .builtIn,
            title: "Word",
            detail: "Create documents",
            path: "/tmp/word/SKILL.md",
            isEnabled: true
        )
        let mcp = MacExtensionItem(
            id: "mcp:example",
            kind: .mcp,
            source: .workspace,
            title: "Example MCP",
            detail: "Workspace server",
            path: "/tmp/mcp.json",
            isEnabled: true
        )

        let sections = MacExtensionCatalog.sections(for: [mcp, skill])

        XCTAssertEqual(sections.map(\.kind), [.skill, .mcp])
        XCTAssertEqual(sections[0].functionalCategory, .documents)
        XCTAssertEqual(sections[0].title, String(localized: "Documents"))
        XCTAssertEqual(sections[0].items, [skill])
        XCTAssertEqual(sections[1].items, [mcp])
    }

    func testBuiltInSkillsAreGroupedByFunctionRatherThanPublisherOrSuite() throws {
        let sections = MacExtensionCatalog.sections(
            for: try MacBuiltInSkillRegistry.extensionItems()
        )
        let skillSections = sections.filter { $0.kind == .skill }

        XCTAssertEqual(
            skillSections.compactMap(\.functionalCategory),
            MacSkillFunctionalCategory.allCases.filter { $0 != .custom }
        )
        XCTAssertTrue(skillSections.allSatisfy { !$0.items.isEmpty })
        XCTAssertFalse(skillSections.map(\.title).contains("Nola Productivity"))
        XCTAssertFalse(skillSections.map(\.title).contains("Research Workbench"))
    }

    func testOfficeSkillResourcesUseExplicitDesignAndValidationRules() throws {
        let officeIDs: Set<MacBuiltInSkillID> = [.word, .excel, .powerpoint]
        for descriptor in MacBuiltInSkillRegistry.descriptors where officeIDs.contains(descriptor.id) {
            let content = try String(contentsOf: descriptor.instructionURL, encoding: .utf8)
            let expectedExtension = descriptor.id == .word ? ".docx" : descriptor.id == .excel ? ".xlsx" : ".pptx"
            XCTAssertTrue(content.hasPrefix("---\nname:"), descriptor.id.rawValue)
            XCTAssertTrue(content.contains("\ndescription:"), descriptor.id.rawValue)
            XCTAssertTrue(content.contains(expectedExtension), descriptor.id.rawValue)
            for requiredSection in [
                "## Activation boundary",
                "## Default design system",
                "## Workflow",
                "## Validation gate",
                "## Completion contract",
                "## Safety and quality rules",
            ] {
                XCTAssertTrue(content.contains(requiredSection), "\(descriptor.id.rawValue): \(requiredSection)")
            }
            XCTAssertTrue(content.localizedCaseInsensitiveContains("validate"), descriptor.id.rawValue)
            XCTAssertTrue(content.localizedCaseInsensitiveContains("never overwrite"), descriptor.id.rawValue)
            XCTAssertTrue(content.localizedCaseInsensitiveContains("absolute file path"), descriptor.id.rawValue)
            for sharedColor in ["#18212B", "#1769E0", "#EEF4FA"] {
                XCTAssertTrue(content.contains(sharedColor), "\(descriptor.id.rawValue): \(sharedColor)")
            }

            let orderedSections = [
                "## Activation boundary",
                "## Default design system",
                "## Workflow",
                "## Validation gate",
                "## Completion contract",
                "## Safety and quality rules",
            ]
            let sectionOffsets = try orderedSections.map { section in
                try XCTUnwrap(content.range(of: section)?.lowerBound, "\(descriptor.id.rawValue): \(section)")
            }
            XCTAssertEqual(sectionOffsets, sectionOffsets.sorted(), descriptor.id.rawValue)
        }
    }

    func testWordSkillRequiresNativeOOXMLAndRejectsLossyTextutilConversion() throws {
        let descriptor = try XCTUnwrap(
            MacBuiltInSkillRegistry.descriptors.first { $0.id == .word }
        )
        let content = try String(contentsOf: descriptor.instructionURL, encoding: .utf8)

        XCTAssertTrue(content.contains("scripts/create_document.py"))
        XCTAssertTrue(content.contains("native OOXML"))
        XCTAssertTrue(content.contains("Never create a DOCX with `textutil`"))
        XCTAssertFalse(content.contains("textutil -convert docx"))
    }

    func testEveryGeneralSkillUsesTheSharedSafetyContract() throws {
        let officeIDs: Set<MacBuiltInSkillID> = [.word, .excel, .powerpoint]
        let generalSkills = MacBuiltInSkillRegistry.descriptors.filter { !officeIDs.contains($0.id) }
        let sideEffectingSkillIDs = Set(generalSkills.map(\.id)).subtracting([.deepResearch])

        XCTAssertEqual(generalSkills.count, 25)
        for descriptor in generalSkills {
            let content = try String(contentsOf: descriptor.instructionURL, encoding: .utf8)
            XCTAssertTrue(content.hasPrefix("---\nname: \(descriptor.id.rawValue)\n"), descriptor.id.rawValue)
            XCTAssertTrue(content.contains("\ndescription:"), descriptor.id.rawValue)
            for section in ["## Readiness", "## Workflow", "## User control", "## Validation", "## Completion"] {
                XCTAssertTrue(content.contains(section), "\(descriptor.id.rawValue): \(section)")
            }
            if sideEffectingSkillIDs.contains(descriptor.id) {
                XCTAssertTrue(content.localizedCaseInsensitiveContains("confirm"), descriptor.id.rawValue)
            }
            XCTAssertFalse(content.contains("TODO"), descriptor.id.rawValue)
        }
    }

    func testEveryBuiltInSkillRetainsDomainSpecificQualityGates() throws {
        let requiredTerms: [MacBuiltInSkillID: [String]] = [
            .word: ["tracked changes", "accessibility", "render"],
            .excel: ["recalculation", "external links", "row counts"],
            .powerpoint: ["speaker notes", "overflow", "render every slide"],
            .pdf: ["underlying content", "page manifest", "encryption state"],
            .figma: ["advisory mode", "Auto Layout", "before snapshot"],
            .dataAnalysis: ["analysis contract", "causal inference", "row counts"],
            .deepResearch: ["source ledger", "up to 20 ranked results", "search snippets"],
            .documentReview: ["finding categories", "tracked changes", "page or section anchor"],
            .imageOCR: ["reading order", "OCR confidence", "uncertain"],
            .meetings: ["explicit decisions", "action item", "speaker attribution"],
            .scheduleTasks: ["preview table", "persistent identifier", "timezone"],
            .knowledgeMemory: ["explicit consent", "provenance", "expiry"],
            .fileOrganization: ["dry-run manifest", "hash equality", "symlinks"],
            .automations: ["idempotency", "retry cap", "dry-run"],
            .email: ["reply all", "exact draft", "provider message identifier"],
            .academicReader: ["source format", "source anchors", "figure"],
            .scientificWriting: ["paper_type", "evidence", "output format"],
            .academicPolishing: ["paper_type", "section", "journal"],
            .manuscriptReviewer: ["3 reviewer reports", "claim_pointer", "evidence_pointer"],
            .reviewerResponse: ["point-by-point", "revision", "evidence"],
            .statisticsAudit: ["experimental unit", "effect sizes", "AUTHOR_INPUT_NEEDED"],
            .referenceVerifier: ["多源", "DOI", "Zotero"],
            .paperToPresentation: ["paper_type", "audit_pptx_quality.py", "source"],
            .implementCode: ["task_worktree_selection", "current callers", "read_task"],
            .reviewCode: ["severity", "reproducible evidence", "plausible failure paths"],
            .testSoftware: ["isolated fixtures", "explicit oracles", "untested scope"],
            .createVisualArt: ["composition", "transparency", "text legibility"],
            .openSourceModel: ["hardware profile", "NVIDIA CUDA", "AMD ROCm", "execution_mode: background", "bash_background_status", "Content-Range", "ETag", "cross-process lock", "task.json", "catalog.json", "64K", "peak memory", "SHA-256", "smoke inference"],
        ]

        XCTAssertEqual(Set(requiredTerms.keys), Set(MacBuiltInSkillID.allCases))
        for descriptor in MacBuiltInSkillRegistry.descriptors {
            let content = try String(contentsOf: descriptor.instructionURL, encoding: .utf8)
            XCTAssertFalse(
                content.localizedCaseInsensitiveContains("skill_update"),
                "\(descriptor.id.rawValue): unexpected skill_update instruction"
            )
            for term in try XCTUnwrap(requiredTerms[descriptor.id]) {
                XCTAssertTrue(
                    content.localizedCaseInsensitiveContains(term),
                    "\(descriptor.id.rawValue): \(term)"
                )
            }
        }
    }

    func testResearchSkillsAreBundledWithRuntimeContractsAndSharedReferences() throws {
        let root = try XCTUnwrap(MacBuiltInSkillRegistry.resourceRootURL)
        let researchSkills = MacBuiltInSkillID.allCases.filter(\.usesSharedResearchReferences)

        XCTAssertEqual(researchSkills.count, 8)
        XCTAssertTrue(FileManager.default.fileExists(
            atPath: root.appendingPathComponent("nature-shared/SKILL.md").path
        ))
        for id in researchSkills {
            let directory = root.appendingPathComponent(id.resourceDirectory, isDirectory: true)
            XCTAssertTrue(FileManager.default.fileExists(
                atPath: directory.appendingPathComponent("NOLA_MANIFEST.json").path
            ), id.rawValue)
        }
    }

    func testEveryUserFacingBuiltInSkillShipsAMachineReadableRuntimeContract() throws {
        let root = try XCTUnwrap(MacBuiltInSkillRegistry.resourceRootURL)

        for id in MacBuiltInSkillID.allCases {
            let manifestURL = root
                .appendingPathComponent(id.resourceDirectory, isDirectory: true)
                .appendingPathComponent("NOLA_MANIFEST.json")
            let data = try Data(contentsOf: manifestURL)
            let manifest = try XCTUnwrap(
                JSONSerialization.jsonObject(with: data) as? [String: Any],
                id.rawValue
            )
            XCTAssertEqual(manifest["schema_version"] as? Int, 1, id.rawValue)
            XCTAssertEqual(manifest["id"] as? String, id.rawValue, id.rawValue)
            XCTAssertEqual(manifest["delivery"] as? String, "built-in", id.rawValue)
            XCTAssertNotNil(manifest["category"] as? [String: Any], id.rawValue)
            XCTAssertFalse((manifest["required_tools"] as? [String] ?? []).isEmpty, id.rawValue)
            XCTAssertFalse((manifest["validation"] as? [String] ?? []).isEmpty, id.rawValue)
        }
    }

    func testAgentHostSkillSnapshotRemovesHistoricalGeneratedRouter() throws {
        let directory = URL(fileURLWithPath: NSTemporaryDirectory(), isDirectory: true)
            .appendingPathComponent("nola-skill-router-\(UUID().uuidString)", isDirectory: true)
        defer { try? FileManager.default.removeItem(at: directory) }
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        let agentsURL = directory.appendingPathComponent("AGENTS.md")
        try """
        # User-owned guidance

        <!-- NOLA_MAC_BUILTIN_SKILLS_START -->
        stale generated Skill routes
        <!-- NOLA_MAC_BUILTIN_SKILLS_END -->
        """.write(to: agentsURL, atomically: true, encoding: .utf8)

        try MacBuiltInSkillRegistry.installTrustedRouter(
            configurationDirectory: directory,
            activeSkills: MacBuiltInSkillRegistry.descriptors
        )
        let remaining = try String(contentsOf: agentsURL, encoding: .utf8)
        XCTAssertTrue(remaining.contains("User-owned guidance"))
        XCTAssertFalse(remaining.contains("NOLA_MAC_BUILTIN_SKILLS"))
        XCTAssertFalse(remaining.contains("stale generated Skill routes"))
    }

    func testAllowedDirectoriesKeepProjectAndGrantOnlyEnabledSkillDirectories() throws {
        let project = URL(fileURLWithPath: "/tmp/nola-project", isDirectory: true)
        try MacBuiltInSkillRegistry.setEnabled(false, for: .word)

        let arguments = try MacBuiltInSkillRegistry.allowedDirectoryArguments(
            projectURL: project
        )
        let allowedPaths = stride(from: 1, to: arguments.count, by: 2).map { arguments[$0] }
        let root = try XCTUnwrap(MacBuiltInSkillRegistry.resourceRootURL)

        XCTAssertEqual(arguments.first, "--allow-dir")
        XCTAssertTrue(allowedPaths.contains(project.path))
        XCTAssertFalse(allowedPaths.contains(root.path))
        let descriptorPaths = Dictionary(
            uniqueKeysWithValues: MacBuiltInSkillRegistry.descriptors.map {
                ($0.id, $0.instructionURL.deletingLastPathComponent().standardizedFileURL.path)
            }
        )
        XCTAssertFalse(allowedPaths.contains(try XCTUnwrap(descriptorPaths[.word])))
        XCTAssertTrue(allowedPaths.contains(try XCTUnwrap(descriptorPaths[.excel])))
        XCTAssertTrue(allowedPaths.contains(try XCTUnwrap(descriptorPaths[.powerpoint])))
    }

    func testEnableStatePersistsAndUpdatesExtensionItems() throws {
        try MacBuiltInSkillRegistry.setEnabled(false, for: .excel)

        let items = try MacBuiltInSkillRegistry.extensionItems()
        let excel = items.first {
            MacBuiltInSkillRegistry.skillID(forExtensionItemID: $0.id) == .excel
        }

        XCTAssertEqual(items.count, MacBuiltInSkillID.allCases.count)
        XCTAssertEqual(excel?.isEnabled, false)
        XCTAssertFalse(try MacBuiltInSkillRegistry.activeDescriptors().contains {
            $0.id == .excel
        })
    }

    func testEnableStateUsesTheExplicitApplicationPort() throws {
        let directory = FileManager.default.temporaryDirectory
            .appendingPathComponent("nola-skill-store-\(UUID().uuidString)", isDirectory: true)
        defer { try? FileManager.default.removeItem(at: directory) }
        let isolatedStore = try MacApplicationPort(
            databaseURL: directory.appendingPathComponent("state/nola.db")
        )

        try MacBuiltInSkillRegistry.setEnabled(
            false,
            for: .fileOrganization,
            store: isolatedStore
        )

        XCTAssertFalse(try MacBuiltInSkillRegistry.activeDescriptors(store: isolatedStore).contains {
            $0.id == .fileOrganization
        })
        XCTAssertTrue(try MacBuiltInSkillRegistry.activeDescriptors().contains {
            $0.id == .fileOrganization
        })
    }

    func testInstalledGitHubSkillSettingsAndNextNativeLaunchShareCanonicalRegistry() async throws {
        let workspace = FileManager.default.temporaryDirectory.appendingPathComponent("mac-installed-\(UUID().uuidString)")
        defer { try? FileManager.default.removeItem(at: workspace) }
        let app = MacAppEnvironment(applicationSupportRoot: workspace.appendingPathComponent("support"),
                                    cachesRoot: workspace.appendingPathComponent("cache"))
        try app.prepare()
        let store = try MacApplicationPort(databaseURL: app.databaseURL)
        let id = "example-card"
        let origin = "owner/repo/tree/" + String(repeating: "a", count: 40) + "/skills/example-card"
        let created = try store.createUserSkill(MacCanonicalSkillInput(id: id, source: .user,
            version: "v1", displayName: id, summary: "Research card", category: "custom",
            instructionsRef: "nola://github-skills/" + origin, instructionsText: "# Original",
            requiredTools: [], defaultMode: nil, enabled: true, createdAtMs: 1, updatedAtMs: 1))
        func launch() throws -> MacLocalAgentRunRequest {
            try MacLocalAgentRunRequest(conversationID: "github-selection", prompt: "hello", projectURL: workspace,
                attachmentURLs: [], appEnvironment: app, applicationPort: store)
        }
        let config = app.processEnvironment(base: ["NOLA_HOME": "/unrelated"], projectURL: workspace)
        XCTAssertEqual(config["NOLA_HOME"], app.runtimeHome.path)
        XCTAssertEqual(config["NOLA_DB_PATH"], app.databaseURL.path)
        XCTAssertTrue(try launch().activeSkillIDs.contains(id))
        XCTAssertTrue(try launch().skillDirectoryArguments.contains(id))
        let items = try await MacExtensionsService.shared.localExtensions(workspaceURL: nil, store: store)
        let item = try XCTUnwrap(items.first { $0.installedSkillID == id })
        let document = try await MacExtensionsService.shared.document(for: item, store: store)
        XCTAssertEqual(document.url.absoluteString, "https://github.com/" + origin)
        XCTAssertEqual(document.canonicalSkillRevision, created.revision)
        let edited = try await MacExtensionsService.shared.save("# New", for: item,
            expectedRevision: document.canonicalSkillRevision, store: store)
        XCTAssertEqual(edited.content, "# New")
        do {
            _ = try await MacExtensionsService.shared.save("# Stale", for: item,
                expectedRevision: document.canonicalSkillRevision, store: store)
            XCTFail("A stale draft replaced the new instructions")
        } catch { }
        let current = try XCTUnwrap(store.skillEditor(id))
        XCTAssertEqual(current.record.instructionsRef, created.record.instructionsRef)
        XCTAssertEqual(current.record.instructionsText, "# New")
        XCTAssertTrue(try launch().activeSkillIDs.contains(id))
        XCTAssertTrue(try launch().skillDirectoryArguments.contains(id))
        _ = try store.setSkillEnabled(current, enabled: false, updatedAtMs: 3)
        XCTAssertFalse(try launch().activeSkillIDs.contains(id))
        XCTAssertFalse(try launch().skillDirectoryArguments.contains(id))
    }

    func testDiscoveredWorkspaceSkillRequiresExplicitActivationAndEntersHostSnapshot() async throws {
        let workspace = FileManager.default.temporaryDirectory
            .appendingPathComponent("nola-workspace-skill-\(UUID().uuidString)", isDirectory: true)
        defer { try? FileManager.default.removeItem(at: workspace) }
        let skillDirectory = workspace.appendingPathComponent(".nola/skills/example", isDirectory: true)
        try FileManager.default.createDirectory(at: skillDirectory, withIntermediateDirectories: true)
        try """
        ---
        name: example-workspace-skill
        description: Test workspace activation
        ---
        # Example
        """.write(
            to: skillDirectory.appendingPathComponent("SKILL.md"),
            atomically: true,
            encoding: .utf8
        )

        let discovered = try await MacExtensionsService.shared.localExtensions(workspaceURL: workspace)
        let item = try XCTUnwrap(discovered.first {
            $0.source == .workspace && $0.title == "example-workspace-skill"
        })
        XCTAssertFalse(item.isEnabled)

        let applicationPort = try MacApplicationPort(
            databaseURL: workspace.appendingPathComponent("nola-product.db")
        )
        try MacLocalSkillActivationStore.setEnabled(true, itemID: item.id, store: applicationPort)
        let applicationSupport = workspace.appendingPathComponent("ApplicationSupport", isDirectory: true)
        let request = try MacLocalAgentRunRequest(
            conversationID: "workspace-skill-contract",
            prompt: "Use the enabled workspace Skill.",
            projectURL: workspace,
            attachmentURLs: [],
            appEnvironment: MacAppEnvironment(
                applicationSupportRoot: applicationSupport,
                cachesRoot: workspace.appendingPathComponent("Caches", isDirectory: true)
            ),
            applicationPort: applicationPort,
        )

        XCTAssertTrue(request.activeExternalSkillDirectories.contains(skillDirectory.standardizedFileURL))
        XCTAssertTrue(request.skillDirectoryArguments.contains(skillDirectory.path))
        XCTAssertTrue(request.allowedDirectoryArguments.contains(skillDirectory.path))
        XCTAssertFalse(request.skillDirectoryArguments.contains("--no-skills"))
    }

    func testRemoveHidesSkillAndRestoreReturnsAllSkillsEnabled() throws {
        try MacBuiltInSkillRegistry.setEnabled(false, for: .powerpoint)
        try MacBuiltInSkillRegistry.remove(.word)

        XCTAssertTrue(try MacBuiltInSkillRegistry.hasRemovedSkills())
        let visibleIDs = try MacBuiltInSkillRegistry.extensionItems().compactMap {
            MacBuiltInSkillRegistry.skillID(forExtensionItemID: $0.id)
        }
        XCTAssertEqual(visibleIDs, MacBuiltInSkillID.allCases.filter { $0 != .word })

        try MacBuiltInSkillRegistry.restoreBuiltInSkills()

        XCTAssertFalse(try MacBuiltInSkillRegistry.hasRemovedSkills())
        XCTAssertEqual(
            try MacBuiltInSkillRegistry.extensionItems().count,
            MacBuiltInSkillID.allCases.count
        )
        XCTAssertTrue(try MacBuiltInSkillRegistry.extensionItems().allSatisfy(\.isEnabled))
    }

    func testInvalidCanonicalHiddenSkillSettingsFailClosed() throws {
        let store = try XCTUnwrap(MacApplicationPort.shared)
        let key = MacProductSettingKeys.hiddenBuiltInSkills
        let previous = try store.productSetting(key: key)
        defer {
            if let previous {
                _ = try? store.putProductSetting(
                    key: key,
                    valueJSON: previous.valueJSON,
                    updatedAtMs: previous.updatedAtMs
                )
            } else {
                _ = try? store.deleteProductSetting(key: key)
            }
        }
        let invalidPayloads = [
            #"{"schemaVersion":0,"ids":[]}"#,
            #"{"schemaVersion":1,"ids":[],"legacyIDs":[]}"#,
            #"{"schemaVersion":1,"ids":["removed-skill-that-does-not-exist"]}"#,
        ]
        for (index, payload) in invalidPayloads.enumerated() {
            _ = try store.putProductSetting(
                key: key,
                valueJSON: payload,
                updatedAtMs: Int64(Date().timeIntervalSince1970 * 1_000) + Int64(index)
            )
            XCTAssertThrowsError(try MacBuiltInSkillRegistry.extensionItems())
        }
    }

    func testBuiltInSkillOverrideIsEditableAndUsedByDescriptorUntilReset() throws {
        let root = FileManager.default.temporaryDirectory
            .appendingPathComponent("nola-skill-overrides-\(UUID().uuidString)", isDirectory: true)
        defer { try? FileManager.default.removeItem(at: root) }
        let custom = """
        ---
        name: word
        description: Customized Word workflow
        ---
        # Custom Word workflow
        """

        let savedURL = try MacBuiltInSkillRegistry.saveOverride(
            custom,
            for: .word,
            overridesRootURL: root
        )

        XCTAssertTrue(MacBuiltInSkillRegistry.hasOverride(for: .word, overridesRootURL: root))
        XCTAssertEqual(
            MacBuiltInSkillRegistry.instructionURL(for: .word, overridesRootURL: root),
            savedURL
        )
        XCTAssertEqual(try String(contentsOf: savedURL, encoding: .utf8), custom)

        try MacBuiltInSkillRegistry.resetOverride(for: .word, overridesRootURL: root)

        XCTAssertFalse(MacBuiltInSkillRegistry.hasOverride(for: .word, overridesRootURL: root))
        let restoredURL = try XCTUnwrap(
            MacBuiltInSkillRegistry.instructionURL(for: .word, overridesRootURL: root)
        )
        XCTAssertNotEqual(restoredURL, savedURL)
        XCTAssertTrue(restoredURL.path.contains("NolaBuiltInSkills.bundle"))
    }
}

final class MacOpenSourceModelInstallerTransactionTests: XCTestCase {
    private var repositoryRoot: URL {
        URL(fileURLWithPath: #filePath).deletingLastPathComponent().deletingLastPathComponent().deletingLastPathComponent().deletingLastPathComponent()
    }

    private var installer: URL {
        repositoryRoot
            .appendingPathComponent("apps/macos/Resources/NolaBuiltInSkills.bundle/install-open-source-model/scripts/install_model_variant.py")
    }

    private func writeJSON(_ value: Any, to url: URL) throws {
        let data = try JSONSerialization.data(withJSONObject: value, options: [.prettyPrinted, .sortedKeys])
        try data.write(to: url, options: .atomic)
    }

    @discardableResult
    private func run(_ arguments: [String]) throws -> (status: Int32, output: String) {
        let process = Process()
        let pipe = Pipe()
        process.executableURL = URL(fileURLWithPath: "/usr/bin/python3")
        process.arguments = [installer.path] + arguments
        process.standardOutput = pipe
        process.standardError = pipe
        try process.run()
        process.waitUntilExit()
        let output = String(decoding: pipe.fileHandleForReading.readDataToEndOfFile(), as: UTF8.self)
        return (process.terminationStatus, output)
    }

    func testMultiArtifactSnapshotActivatesOnlyAfterEveryArtifactIsVerified() throws {
        let temporary = FileManager.default.temporaryDirectory
            .appendingPathComponent("nola-model-transaction-\(UUID().uuidString)", isDirectory: true)
        defer { try? FileManager.default.removeItem(at: temporary) }
        try FileManager.default.createDirectory(at: temporary, withIntermediateDirectories: true)

        let first = temporary.appendingPathComponent("config.json")
        let second = temporary.appendingPathComponent("weights.bin")
        try Data("{\"model_type\":\"qwen\"}".utf8).write(to: first)
        try Data(repeating: 0x5A, count: 8_192).write(to: second)
        let total = try first.resourceValues(forKeys: [.fileSizeKey]).fileSize!
            + second.resourceValues(forKeys: [.fileSizeKey]).fileSize!
        let plan = temporary.appendingPathComponent("plan.json")
        try writeJSON([
            "status": "recommended",
            "model": "example/Qwen-Test",
            "selected": [
                "repo_id": "example/Qwen-Test",
                "name": "Qwen Test",
                "revision": "commit-123",
                "runtime": "mlx",
                "format": "mlx",
                "size_bytes": total,
                "context_length": 65_536,
            ],
        ], to: plan)
        let manifest = temporary.appendingPathComponent("artifacts.json")
        try writeJSON([
            "revision": "commit-123",
            "artifacts": [
                ["path": "config.json", "url": first.absoluteString, "size_bytes": Data("{\"model_type\":\"qwen\"}".utf8).count],
                ["path": "model/weights.bin", "url": second.absoluteString, "size_bytes": 8_192],
            ],
        ], to: manifest)
        let models = temporary.appendingPathComponent("models", isDirectory: true)

        let result = try run([
            "--plan", plan.path,
            "--models-root", models.path,
            "--artifacts-json", manifest.path,
            "--task-id", "transaction-test",
            "--max-retries", "0",
        ])
        XCTAssertEqual(result.status, 0, result.output)

        let catalog = try JSONSerialization.jsonObject(with: Data(contentsOf: models.appendingPathComponent("catalog.json"))) as! [String: Any]
        let record = (catalog["models"] as! [[String: Any]]).first!
        let snapshot = URL(fileURLWithPath: record["snapshotPath"] as! String)
        XCTAssertTrue(FileManager.default.fileExists(atPath: snapshot.appendingPathComponent("config.json").path))
        XCTAssertTrue(FileManager.default.fileExists(atPath: snapshot.appendingPathComponent("model/weights.bin").path))
        XCTAssertEqual(record["artifactBytes"] as? Int, total)
        XCTAssertEqual(record["state"] as? String, "configured")
    }

    func testDeadOwnerLockIsRecovered() throws {
        let temporary = FileManager.default.temporaryDirectory
            .appendingPathComponent("nola-model-stale-lock-\(UUID().uuidString)", isDirectory: true)
        defer { try? FileManager.default.removeItem(at: temporary) }
        let source = temporary.appendingPathComponent("model.gguf")
        try FileManager.default.createDirectory(at: temporary, withIntermediateDirectories: true)
        try Data(repeating: 0x41, count: 32).write(to: source)
        let plan = temporary.appendingPathComponent("plan.json")
        try writeJSON([
            "status": "recommended",
            "model": "example/stale",
            "selected": ["repo_id": "example/stale", "revision": "r1", "runtime": "llama.cpp", "format": "gguf", "size_bytes": 32, "context_length": 65_536],
        ], to: plan)
        let models = temporary.appendingPathComponent("models", isDirectory: true)
        let lock = models.appendingPathComponent("example-stale/.install.lock", isDirectory: true)
        try FileManager.default.createDirectory(at: lock, withIntermediateDirectories: true)
        try writeJSON(["pid": 999_999_999, "heartbeatEpoch": Int(Date().timeIntervalSince1970)], to: lock.appendingPathComponent("owner.json"))

        let result = try run([
            "--plan", plan.path,
            "--models-root", models.path,
            "--artifact-url", source.absoluteString,
            "--filename", "model.gguf",
            "--task-id", "stale-lock-test",
            "--max-retries", "0",
        ])
        XCTAssertEqual(result.status, 0, result.output)
        XCTAssertFalse(FileManager.default.fileExists(atPath: lock.path))
    }
}

final class MacOpenSourceModelSkillTests: XCTestCase {
    func testPlannerSelectsNativeVariantAndExplainsRejections() throws {
        let descriptor = try XCTUnwrap(MacBuiltInSkillRegistry.descriptors.first { $0.id == .openSourceModel })
        let script = descriptor.instructionURL.deletingLastPathComponent()
            .appendingPathComponent("scripts/plan_model_variant.py")
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: root) }

        let hardware = root.appendingPathComponent("hardware.json")
        let candidates = root.appendingPathComponent("candidates.json")
        let output = root.appendingPathComponent("plan.json")
        try Data(#"{"platform":"macos","architecture":"arm64","memory_bytes":17179869184,"available_disk_bytes":107374182400,"runtimes":["mlx","llama.cpp"]}"#.utf8).write(to: hardware)
        try Data(#"{"candidates":[{"repo_id":"publisher/model-mlx","revision":"abc","format":"mlx","runtime":"mlx","quantization":"4bit","parameter_billions":4,"size_bytes":3000000000,"context_length":131072,"architectures":["arm64"]},{"repo_id":"publisher/model-f16","revision":"def","format":"gguf","runtime":"llama.cpp","quantization":"f16","parameter_billions":14,"size_bytes":28000000000,"context_length":131072,"architectures":["arm64"]},{"repo_id":"publisher/model-short","revision":"ghi","format":"gguf","runtime":"llama.cpp","quantization":"q4_k_m","parameter_billions":4,"size_bytes":2500000000,"context_length":32768,"architectures":["arm64"]}]}"#.utf8).write(to: candidates)

        let process = Process()
        process.executableURL = URL(fileURLWithPath: "/usr/bin/python3")
        process.arguments = [script.path, "--model", "publisher/model", "--hardware-json", hardware.path, "--candidates-json", candidates.path, "--minimum-context", "65536", "--output", output.path]
        try process.run()
        process.waitUntilExit()
        XCTAssertEqual(process.terminationStatus, 0)

        let plan = try XCTUnwrap(JSONSerialization.jsonObject(with: Data(contentsOf: output)) as? [String: Any])
        XCTAssertEqual(plan["status"] as? String, "recommended")
        XCTAssertEqual((plan["selected"] as? [String: Any])?["repo_id"] as? String, "publisher/model-mlx")
        XCTAssertEqual(plan["side_effects_performed"] as? Bool, false)
        let rejected = try XCTUnwrap(plan["rejected"] as? [[String: Any]])
        XCTAssertEqual(rejected.count, 2)
        XCTAssertTrue(rejected.flatMap { $0["rejection_reasons"] as? [String] ?? [] }.contains { $0.contains("context") })
        XCTAssertTrue(rejected.flatMap { $0["rejection_reasons"] as? [String] ?? [] }.contains { $0.contains("memory") || $0.contains("disk") })
    }

    func testPlannerReportsNoCompatibleVariantWithoutSideEffects() throws {
        let descriptor = try XCTUnwrap(MacBuiltInSkillRegistry.descriptors.first { $0.id == .openSourceModel })
        let script = descriptor.instructionURL.deletingLastPathComponent()
            .appendingPathComponent("scripts/plan_model_variant.py")
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: root) }

        let hardware = root.appendingPathComponent("hardware.json")
        let candidates = root.appendingPathComponent("candidates.json")
        let output = root.appendingPathComponent("plan.json")
        try Data(#"{"platform":"linux","architecture":"x86_64","memory_bytes":4294967296,"available_disk_bytes":10737418240,"runtimes":["llama.cpp"]}"#.utf8).write(to: hardware)
        try Data(#"{"candidates":[{"repo_id":"publisher/model-mlx","revision":"abc","format":"mlx","runtime":"mlx","quantization":"4bit","parameter_billions":4,"size_bytes":3000000000,"context_length":131072,"architectures":["arm64"]}]}"#.utf8).write(to: candidates)

        let process = Process()
        process.executableURL = URL(fileURLWithPath: "/usr/bin/python3")
        process.arguments = [script.path, "--model", "publisher/model", "--hardware-json", hardware.path, "--candidates-json", candidates.path, "--minimum-context", "65536", "--output", output.path]
        try process.run()
        process.waitUntilExit()
        XCTAssertEqual(process.terminationStatus, 2)

        let plan = try XCTUnwrap(JSONSerialization.jsonObject(with: Data(contentsOf: output)) as? [String: Any])
        XCTAssertEqual(plan["status"] as? String, "no_compatible_variant")
        XCTAssertNil(plan["selected"] as? [String: Any])
        XCTAssertEqual(plan["side_effects_performed"] as? Bool, false)
        XCTAssertFalse((plan["rejected"] as? [[String: Any]] ?? []).isEmpty)
    }

    func testPlannerSupportsNvidiaCUDAAndAMDROCmOrVulkan() throws {
        let nvidia = try runPlanner(
            hardware: #"{"platform":"linux","architecture":"x86_64","gpu_vendor":"nvidia","gpu_memory_bytes":25769803776,"memory_bytes":68719476736,"available_disk_bytes":214748364800,"accelerators":["cuda"],"runtimes":["llama.cpp"]}"#,
            candidateAccelerators: ["cuda", "cpu"]
        )
        XCTAssertEqual((nvidia["selected"] as? [String: Any])?["accelerator"] as? String, "cuda")

        let amd = try runPlanner(
            hardware: #"{"platform":"linux","architecture":"x86_64","gpu_vendor":"amd","gpu_memory_bytes":25769803776,"memory_bytes":68719476736,"available_disk_bytes":214748364800,"rocm_available":true,"vulkan_available":true,"runtimes":["llama.cpp"]}"#,
            candidateAccelerators: ["rocm", "vulkan", "cpu"]
        )
        XCTAssertEqual((amd["selected"] as? [String: Any])?["accelerator"] as? String, "rocm")
    }

    func testInstallerPlacesArtifactAndRegistrationInNolaModelsRoot() throws {
        let descriptor = try XCTUnwrap(MacBuiltInSkillRegistry.descriptors.first { $0.id == .openSourceModel })
        let scripts = descriptor.instructionURL.deletingLastPathComponent().appendingPathComponent("scripts")
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: root) }
        let artifact = root.appendingPathComponent("fixture.gguf")
        try Data("verified model fixture".utf8).write(to: artifact)
        let plan = root.appendingPathComponent("plan.json")
        try Data(#"{"status":"recommended","model":"publisher/model","selected":{"repo_id":"publisher/model-gguf","revision":"immutable","runtime":"llama.cpp","accelerator":"cuda","format":"gguf","quantization":"q4_k_m","context_length":131072,"size_bytes":22}}"#.utf8).write(to: plan)
        let models = root.appendingPathComponent("NolaModels")
        let process = Process()
        process.executableURL = URL(fileURLWithPath: "/usr/bin/python3")
        process.arguments = [scripts.appendingPathComponent("install_model_variant.py").path, "--plan", plan.path, "--models-root", models.path, "--artifact-url", artifact.absoluteURL.absoluteString, "--filename", "model.gguf", "--task-id", "task-qwen-test"]
        try process.run()
        process.waitUntilExit()
        XCTAssertEqual(process.terminationStatus, 0)
        let catalog = try XCTUnwrap(JSONSerialization.jsonObject(with: Data(contentsOf: models.appendingPathComponent("catalog.json"))) as? [String: Any])
        let installed = try XCTUnwrap((catalog["models"] as? [[String: Any]])?.first)
        XCTAssertEqual(installed["accelerator"] as? String, "cuda")
        XCTAssertTrue(FileManager.default.fileExists(atPath: try XCTUnwrap(installed["artifactPath"] as? String)))
        XCTAssertTrue(FileManager.default.fileExists(atPath: models.appendingPathComponent("publisher-model-gguf/model.json").path))
        let task = try XCTUnwrap(JSONSerialization.jsonObject(with: Data(contentsOf: models.appendingPathComponent("publisher-model-gguf/task.json"))) as? [String: Any])
        XCTAssertEqual(task["status"] as? String, "configured")
        XCTAssertEqual(task["taskID"] as? String, "task-qwen-test")
        XCTAssertEqual(task["progress"] as? Double, 1.0)
    }

    func testInstallerResumesExistingPartialArtifact() throws {
        let descriptor = try XCTUnwrap(MacBuiltInSkillRegistry.descriptors.first { $0.id == .openSourceModel })
        let installer = descriptor.instructionURL.deletingLastPathComponent().appendingPathComponent("scripts/install_model_variant.py")
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: root) }
        let bytes = Data((0..<4096).map { UInt8($0 % 251) })
        let source = root.appendingPathComponent("source.gguf")
        try bytes.write(to: source)
        let plan = root.appendingPathComponent("plan.json")
        try Data(#"{"status":"recommended","model":"publisher/resume","selected":{"repo_id":"publisher/resume","revision":"fixed","runtime":"llama.cpp","accelerator":"vulkan","format":"gguf","quantization":"q4_k_m","context_length":65536,"size_bytes":4096}}"#.utf8).write(to: plan)
        let models = root.appendingPathComponent("models")
        let directory = models.appendingPathComponent("publisher-resume")
        let staging = directory.appendingPathComponent(".staging/fixed-model-install-publisher-resume")
        try FileManager.default.createDirectory(at: staging, withIntermediateDirectories: true)
        let partial = staging.appendingPathComponent("model.gguf.part")
        try bytes.prefix(1536).write(to: partial)
        let process = Process()
        process.executableURL = URL(fileURLWithPath: "/usr/bin/python3")
        process.arguments = [installer.path, "--plan", plan.path, "--models-root", models.path, "--artifact-url", source.absoluteURL.absoluteString, "--filename", "model.gguf"]
        try process.run()
        process.waitUntilExit()
        XCTAssertEqual(process.terminationStatus, 0)
        let recordData = try Data(contentsOf: directory.appendingPathComponent("model.json"))
        let record = try XCTUnwrap(JSONSerialization.jsonObject(with: recordData) as? [String: Any])
        let artifactPath = try XCTUnwrap(record["artifactPath"] as? String)
        XCTAssertEqual(try Data(contentsOf: URL(fileURLWithPath: artifactPath)), bytes)
        XCTAssertFalse(FileManager.default.fileExists(atPath: partial.path))
    }

    private func runPlanner(hardware: String, candidateAccelerators: [String]) throws -> [String: Any] {
        let descriptor = try XCTUnwrap(MacBuiltInSkillRegistry.descriptors.first { $0.id == .openSourceModel })
        let script = descriptor.instructionURL.deletingLastPathComponent().appendingPathComponent("scripts/plan_model_variant.py")
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: root) }
        let hardwareURL = root.appendingPathComponent("hardware.json")
        let candidatesURL = root.appendingPathComponent("candidates.json")
        let output = root.appendingPathComponent("plan.json")
        try Data(hardware.utf8).write(to: hardwareURL)
        let candidate: [String: Any] = ["repo_id": "publisher/model-gguf", "revision": "abc", "format": "gguf", "runtime": "llama.cpp", "quantization": "q4_k_m", "parameter_billions": 4, "size_bytes": 3_000_000_000, "context_length": 131_072, "architectures": ["x86_64"], "accelerators": candidateAccelerators]
        try JSONSerialization.data(withJSONObject: ["candidates": [candidate]]).write(to: candidatesURL)
        let process = Process()
        process.executableURL = URL(fileURLWithPath: "/usr/bin/python3")
        process.arguments = [script.path, "--model", "publisher/model", "--hardware-json", hardwareURL.path, "--candidates-json", candidatesURL.path, "--minimum-context", "65536", "--output", output.path]
        try process.run()
        process.waitUntilExit()
        XCTAssertEqual(process.terminationStatus, 0)
        return try XCTUnwrap(JSONSerialization.jsonObject(with: Data(contentsOf: output)) as? [String: Any])
    }
}
