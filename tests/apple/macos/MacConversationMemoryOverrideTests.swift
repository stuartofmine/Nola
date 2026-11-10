import XCTest
@testable import Nola

@MainActor
final class MacConversationMemoryOverrideTests: XCTestCase {
    func testMemoryInheritanceUsesOnlyTheOwningStoreForEveryOverride() throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: root) }
        let first = try MacApplicationPort(databaseURL: root.appendingPathComponent("first.db"))
        let second = try MacApplicationPort(databaseURL: root.appendingPathComponent("second.db"))
        MacAssistantPreferences.setUserPreferences("first owner preference", in: first)
        MacAssistantPreferences.setUserPreferences("second owner preference", in: second)
        XCTAssertEqual(MacCanonicalConversationRepository(store: first).userPreferences, "first owner preference")
        XCTAssertEqual(MacCanonicalConversationRepository(store: second).userPreferences, "second owner preference")
        let firstModel = MacWorkspaceModel(canonicalRepository: MacCanonicalConversationRepository(store: first),
            localAgentRuntime: PersistenceTestLocalRuntime(), remoteHost: PermissionEnvelopeRemoteHost())
        let secondModel = MacWorkspaceModel(canonicalRepository: MacCanonicalConversationRepository(store: second),
            localAgentRuntime: PersistenceTestLocalRuntime(), remoteHost: PermissionEnvelopeRemoteHost())
        defer {
            firstModel.presentationDidDetach()
            secondModel.presentationDidDetach()
        }
        for global in [false, true] {
            MacAssistantPreferences.setMemoryEnabled(global, in: first)
            MacAssistantPreferences.setMemoryEnabled(!global, in: second)
            for override: Bool? in [nil, true, false] {
                firstModel.memoryEnabledOverride = override
                secondModel.memoryEnabledOverride = override
                XCTAssertEqual(firstModel.memoryEnabled, override ?? global)
                XCTAssertEqual(secondModel.memoryEnabled, override ?? !global)
            }
        }
        firstModel.memoryEnabledOverride = nil
        MacAssistantPreferences.setMemoryEnabled(false, in: first)
        XCTAssertFalse(firstModel.memoryEnabled)
        MacAssistantPreferences.setMemoryEnabled(true, in: first)
        XCTAssertTrue(firstModel.memoryEnabled)
    }

    func testThreeStatesPersistWithoutChangingAnotherConversation() async throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: root) }
        let port = try MacApplicationPort(databaseURL: root.appendingPathComponent("memory.db"))
        let repository = MacCanonicalConversationRepository(store: port)
        try repository.restoreBackupSnapshot(["one", "two"].map { id in
            MacStoredConversation(conversation: MacConversation(id: id, title: id, preview: "",
                updatedAt: .now, isPinned: false, isPersisted: true), messages: [])
        })
        let model = MacWorkspaceModel(canonicalRepository: repository,
            localAgentRuntime: PersistenceTestLocalRuntime(), remoteHost: PermissionEnvelopeRemoteHost())
        defer { model.presentationDidDetach() }
        model.selectedConversationID = "one"
        XCTAssertNil(model.memoryEnabledOverride)
        for value: Bool? in [true, false, nil] {
            model.memoryEnabledOverride = value
            let deadline = Date().addingTimeInterval(3)
            while try repository.load(conversationID: "one")?.conversation.memoryEnabledOverride != value,
                  Date() < deadline {
                try await Task.sleep(for: .milliseconds(10))
            }
            XCTAssertEqual(try repository.load(conversationID: "one")?.conversation.memoryEnabledOverride, value)
            XCTAssertNil(try repository.load(conversationID: "two")?.conversation.memoryEnabledOverride)
            model.selectedConversationID = "two"
            XCTAssertNil(model.memoryEnabledOverride)
            model.selectedConversationID = "one"
            XCTAssertEqual(model.memoryEnabledOverride, value)
        }
        let reopened = try MacApplicationPort(databaseURL: root.appendingPathComponent("memory.db"))
        XCTAssertNil(try MacCanonicalConversationRepository(store: reopened).load(conversationID: "one")?.conversation.memoryEnabledOverride)
    }

    func testChangingOverrideDoesNotMutateAlreadyConstructedRunRequest() throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: root) }
        let environment = MacAppEnvironment(applicationSupportRoot: root, cachesRoot: root)
        let port = try MacApplicationPort(databaseURL: environment.databaseURL)
        let repository = MacCanonicalConversationRepository(store: port)
        let model = MacWorkspaceModel(canonicalRepository: repository,
            localAgentRuntime: PersistenceTestLocalRuntime(), remoteHost: PermissionEnvelopeRemoteHost())
        defer { model.presentationDidDetach() }
        model.memoryEnabledOverride = true
        let request = try MacLocalAgentRunRequest(conversationID: "frozen", prompt: "fixture",
            projectURL: root, attachmentURLs: [], memoryEnabled: model.memoryEnabled,
            appEnvironment: environment, applicationPort: port)
        model.memoryEnabledOverride = false
        XCTAssertFalse(model.memoryEnabled)
        XCTAssertTrue(request.memoryEnabled)
        model.memoryEnabledOverride = nil
        XCTAssertNil(model.memoryEnabledOverride)
        XCTAssertEqual(model.memoryEnabled, MacAssistantPreferences.memoryEnabled(in: port))
        XCTAssertTrue(request.memoryEnabled)
    }
}
