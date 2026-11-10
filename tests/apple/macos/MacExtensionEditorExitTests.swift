// SPDX-License-Identifier: Apache-2.0
import AppKit
import XCTest
@testable import Nola

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
