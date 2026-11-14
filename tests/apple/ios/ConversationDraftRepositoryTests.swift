import XCTest
@testable import Nola

final class ConversationDraftRepositoryTests: XCTestCase {
    func testClosedPreAdmissionRejectionRestoresOnlyOriginalOwnerAndRetiresReceipt() throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: root) }
        let repository = FileConversationDraftRepository(root: root)
        var drafts = ConversationComposerDraftStore(repository: repository)
        drafts.current.text = "原始问题 @专家"
        let owner = UUID(), other = UUID()
        let submission = try drafts.prepareSubmission(to: owner,
            identity: IOSCanonicalAgentTurnIdentity(userMessageID: UUID()), configurationRevision: nil)
        drafts.select(other); drafts.current.text = "new work"
        XCTAssertTrue(try drafts.restoreRejectedSubmission(submission, conversationID: owner))
        XCTAssertEqual(drafts.selected, .conversation(other))
        XCTAssertEqual(drafts.current.text, "new work")
        XCTAssertTrue(try repository.pendingSubmissions().isEmpty)
        drafts.select(owner)
        XCTAssertEqual(drafts.current.text, "原始问题 @专家")
        XCTAssertTrue(drafts.current.restored)
    }

    func testRejectedSubmissionNeverOverwritesNewInputOrRetiresCommittedReceipt() throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: root) }
        let repository = FileConversationDraftRepository(root: root)
        var drafts = ConversationComposerDraftStore(repository: repository)
        drafts.current.text = "original"
        let owner = UUID()
        let submission = try drafts.prepareSubmission(to: owner,
            identity: IOSCanonicalAgentTurnIdentity(userMessageID: UUID()), configurationRevision: nil)
        drafts.current.text = "newer draft"
        XCTAssertFalse(try drafts.restoreRejectedSubmission(submission, conversationID: owner))
        XCTAssertEqual(drafts.current.text, "newer draft")
        XCTAssertEqual(try repository.pendingSubmissions().count, 1)
        try submission.committed()
        XCTAssertThrowsError(try repository.retireRejectedSubmission(XCTUnwrap(submission.ticket)))
        XCTAssertEqual(try repository.load(.conversation(owner))?.text, "newer draft")
    }

    func testRemoteSamplingRoundTripsWithoutLocalDefaults() throws {
        var parameters = ModelParameters.webAlignedCloudDefaults
        let saved = try ConversationParameterSnapshot.updating(nil, modelKey: "remote", parameters: parameters)
        let restored = try ConversationParameterSnapshot.read(saved, modelKey: "remote")
        XCTAssertNil(restored.topK)
        XCTAssertNil(restored.repetitionPenalty)
        parameters.topK = 40
        parameters.repetitionPenalty = 1
        let explicit = try ConversationParameterSnapshot.updating(saved, modelKey: "remote", parameters: parameters)
        let explicitRestored = try ConversationParameterSnapshot.read(explicit, modelKey: "remote")
        XCTAssertEqual(explicitRestored.topK, 40)
        XCTAssertEqual(explicitRestored.repetitionPenalty, 1)
    }

    func testSubmissionIntentSurvivesRestartAndCommitPreservesNewInput() throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: root) }
        let repository = FileConversationDraftRepository(root: root)
        let conversation = UUID()
        let owner = ConversationComposerDraftStore.Owner.conversation(conversation)
        try repository.save(.init(text: "captured input"), owner: owner)
        _ = try repository.load(owner)
        let identity = IOSCanonicalAgentTurnIdentity(userMessageID: UUID())
        let ticket = try repository.stageSubmission(owner: owner, conversationID: conversation,
            userMessageID: identity.visibleUserMessageID, configurationRevision: 7,
            draft: .init(text: "captured input"))
        let reopened = FileConversationDraftRepository(root: root)
        XCTAssertEqual(try reopened.pendingSubmissions(), [ticket])
        try repository.save(.init(text: "new input typed after dispatch"), owner: owner)
        XCTAssertEqual(try repository.load(owner)?.text, "new input typed after dispatch")
        try reopened.commitSubmission(ticket)
        try reopened.commitSubmission(ticket)
        XCTAssertTrue(try reopened.pendingSubmissions().isEmpty)
        XCTAssertEqual(try reopened.load(owner)?.text, "new input typed after dispatch")
    }

    func testLostAckReopensOriginalConversationAndRetiresOnlyCapturedUnsentVersion() throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: root) }
        let repository = FileConversationDraftRepository(root: root)
        var drafts = ConversationComposerDraftStore(repository: repository)
        drafts.current.text = "send exactly once"
        let original = drafts.selected
        let conversation = UUID()
        let identity = IOSCanonicalAgentTurnIdentity(userMessageID: UUID())
        let submission = try drafts.prepareSubmission(to: conversation, identity: identity, configurationRevision: 2)
        XCTAssertEqual(try repository.pendingSubmissions().first?.text, "send exactly once")
        let restarted = ConversationComposerDraftStore(repository: FileConversationDraftRepository(root: root))
        XCTAssertEqual(restarted.selected, .conversation(conversation))
        XCTAssertEqual(try repository.load(original)?.text, "send exactly once")
        try submission.committed()
        XCTAssertTrue(try repository.pendingSubmissions().isEmpty)
        XCTAssertThrowsError(try repository.load(original))
    }

    func testPendingIndexPointingAtRetiredDraftRecoversAfterCrash() throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: root) }
        let repo = FileConversationDraftRepository(root: root)
        let old = try repo.pendingOwner()
        try repo.remove(.unsent(old))
        // Simulate the crash boundary: durable tombstone, old pending index.
        try JSONEncoder().encode(old).write(to: root.appendingPathComponent("pending-owner.json"), options: .atomic)
        let reopened = FileConversationDraftRepository(root: root)
        let new = try reopened.pendingOwner()
        XCTAssertNotEqual(new, old)
        try reopened.save(.init(text: "new draft"), owner: .unsent(new))
        XCTAssertEqual(try reopened.load(.unsent(new))?.text, "new draft")
        XCTAssertThrowsError(try reopened.load(.unsent(old)))
    }
    func testSameAttachmentIdentityUsesImmutableVersionsAndFailedCommitKeepsOldSnapshot() throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: root) }
        let owner = ConversationComposerDraftStore.Owner.conversation(UUID())
        let id = UUID()
        let first = ChatAttachmentDraft(id: id, name: "file", typeLabel: "text", sizeBytes: 3, extractedContent: "old")
        let second = ChatAttachmentDraft(id: id, name: "file", typeLabel: "text", sizeBytes: 3, extractedContent: "new")
        let repo = FileConversationDraftRepository(root: root)
        try repo.save(.init(text: "old snapshot", attachments: [first]), owner: owner)
        XCTAssertEqual(try repo.load(owner)?.attachments, [first])
        let failing = FileConversationDraftRepository(root: root, beforeManifestCommit: { throw CocoaError(.fileWriteUnknown) })
        _ = try failing.load(owner)
        let failure = expectation(forNotification: FileConversationDraftRepository.writeFailed, object: nil) {
            $0.userInfo?["repository"] as? UUID == failing.identity
        }
        try failing.save(.init(text: "uncommitted", attachments: [second]), owner: owner)
        wait(for: [failure], timeout: 2)
        XCTAssertEqual(try repo.load(owner)?.text, "old snapshot")
        XCTAssertEqual(try repo.load(owner)?.attachments, [first])
        try repo.save(.init(text: "committed", attachments: [second]), owner: owner)
        XCTAssertEqual(try repo.load(owner)?.attachments, [second])
        let blobs = try FileManager.default.subpathsOfDirectory(atPath: root.path).filter { $0.contains("\(id.uuidString)-") }
        XCTAssertEqual(blobs.count, 2)
        // Hash verification must reject corrupted content while keeping text.
        for path in blobs { try Data("corrupt".utf8).write(to: root.appendingPathComponent(path)) }
        let recovered = try XCTUnwrap(repo.load(owner))
        XCTAssertEqual(recovered.text, "committed")
        XCTAssertTrue(recovered.attachments.isEmpty)
        XCTAssertNotNil(recovered.notice)
    }
    func testUnsentConfigurationPersistsWithoutCredentialsAndMissingModelDoesNotFallback() throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: root) }
        let repo = FileConversationDraftRepository(root: root)
        let owner = ConversationComposerDraftStore.Owner.unsent(try repo.pendingOwner())
        var parameters = ModelParameters.webAlignedCloudDefaults
        parameters.temperature = 0.23
        let configuration = ConversationComposerDraftStore.ModelConfiguration(modelID: "specific-profile", source: .configured, parameters: parameters)
        try repo.save(.init(text: "unsent", configuration: configuration), owner: owner)
        let restored = try XCTUnwrap(FileConversationDraftRepository(root: root).load(owner))
        XCTAssertEqual(restored.configuration, configuration)
        XCTAssertTrue(restored.restored)
        let differentModel = ModelInfo(id: "global-default", name: "Default", source: "api", isLocal: false)
        XCTAssertThrowsError(try IOSDraftModelResolver.resolve(configuration, catalog: [differentModel]))
        let sameIDLocal = ModelInfo(id: "specific-profile", name: "Local", source: "local", isLocal: true)
        XCTAssertThrowsError(try IOSDraftModelResolver.resolve(configuration, catalog: [sameIDLocal]))
        let matchingCloud = ModelInfo(id: "specific-profile", name: "Cloud", source: "api", isLocal: false)
        XCTAssertEqual(try IOSDraftModelResolver.resolve(configuration, catalog: [sameIDLocal, matchingCloud]), matchingCloud)
        let files = try FileManager.default.subpathsOfDirectory(atPath: root.path)
        let manifest = try XCTUnwrap(files.first { $0.hasSuffix("manifest.json") })
        let data = try Data(contentsOf: root.appendingPathComponent(manifest))
        let json = try XCTUnwrap(JSONSerialization.jsonObject(with: data) as? [String: Any])
        let savedConfiguration = try XCTUnwrap(json["configuration"] as? [String: Any])
        XCTAssertEqual(Set(savedConfiguration.keys), ["modelID", "source", "parameters"])
    }

    func testTwoInstancesRejectStaleWriteAndDeletionCannotBeResurrected() throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: root) }
        let a = FileConversationDraftRepository(root: root), b = FileConversationDraftRepository(root: root)
        let owner = ConversationComposerDraftStore.Owner.conversation(UUID())
        try a.save(.init(text: "original"), owner: owner)
        XCTAssertEqual(try b.load(owner)?.text, "original")
        try a.save(.init(text: "A committed"), owner: owner)
        let conflict = expectation(forNotification: FileConversationDraftRepository.writeFailed, object: nil) {
            $0.userInfo?["repository"] as? UUID == b.identity
        }
        try b.save(.init(text: "B must remain local"), owner: owner)
        wait(for: [conflict], timeout: 2)
        XCTAssertEqual(try a.load(owner)?.text, "A committed")
        try a.remove(owner)
        let deleted = expectation(forNotification: FileConversationDraftRepository.writeFailed, object: nil) {
            $0.userInfo?["repository"] as? UUID == b.identity
        }
        try b.save(.init(text: "late"), owner: owner)
        wait(for: [deleted], timeout: 2)
        XCTAssertThrowsError(try FileConversationDraftRepository(root: root).load(owner))
    }

    func testSendingExistingConversationClearsButDoesNotRetireItsComposer() throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: root) }
        let repo = FileConversationDraftRepository(root: root)
        var store = ConversationComposerDraftStore(repository: repo)
        let id = UUID()
        store.select(id)
        store.current.text = "first"
        store.submitted(to: id)
        store.current.text = "second"
        XCTAssertEqual(try FileConversationDraftRepository(root: root).load(.conversation(id))?.text, "second")
    }

    func testSameConversationInDifferentDatabaseScopesDoesNotShareOrDeleteDrafts() throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: root) }
        let first = FileConversationDraftRepository.scoped(to: root.appendingPathComponent("one.sqlite"))
        let second = FileConversationDraftRepository.scoped(to: root.appendingPathComponent("two.sqlite"))
        let owner = ConversationComposerDraftStore.Owner.conversation(UUID())
        try first.save(.init(text: "first"), owner: owner)
        try second.save(.init(text: "second"), owner: owner)
        XCTAssertEqual(try first.load(owner)?.text, "first")
        XCTAssertEqual(try second.load(owner)?.text, "second")
        try first.remove(owner)
        XCTAssertThrowsError(try first.load(owner))
        XCTAssertEqual(try second.load(owner)?.text, "second")
    }

    func testRestartRestoresTextAndAttachmentsAndMissingAttachmentKeepsText() throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: root) }
        let repo = FileConversationDraftRepository(root: root)
        let owner = ConversationComposerDraftStore.Owner.conversation(UUID())
        let attachment = ChatAttachmentDraft(name: "notes.txt", typeLabel: "text", sizeBytes: 4, extractedContent: "body")
        try repo.save(.init(text: "draft", attachments: [attachment]), owner: owner)
        let reopened = FileConversationDraftRepository(root: root)
        XCTAssertEqual(try reopened.load(owner)?.text, "draft")
        XCTAssertEqual(try reopened.load(owner)?.attachments, [attachment])
        let resources = try FileManager.default.subpathsOfDirectory(atPath: root.path)
        let resource = try XCTUnwrap(resources.first { $0.contains("\(attachment.id.uuidString)-") && $0.hasSuffix(".json") })
        try FileManager.default.removeItem(at: root.appendingPathComponent(resource))
        let recovered = try XCTUnwrap(reopened.load(owner))
        XCTAssertEqual(recovered.text, "draft")
        XCTAssertTrue(recovered.attachments.isEmpty)
        XCTAssertNotNil(recovered.notice)
        try repo.remove(owner)
        XCTAssertThrowsError(try reopened.load(owner))
    }

    func testPendingIdentityAndSnapshotAreIndependentOfLiveDefaults() throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: root) }
        let repo = FileConversationDraftRepository(root: root)
        let first = try repo.pendingOwner()
        XCTAssertEqual(try FileConversationDraftRepository(root: root).pendingOwner(), first)
        XCTAssertNotEqual(try repo.newPendingOwner(), first)
        let key = "configured:fixture"
        var parameters = ModelParameters.webAlignedCloudDefaults
        parameters.temperature = 0.25
        let json = try ConversationParameterSnapshot.updating("{\"foreign\":true}", modelKey: key, parameters: parameters)
        parameters.temperature = 1
        XCTAssertEqual(try ConversationParameterSnapshot.read(json, modelKey: key).temperature, 0.25)
        XCTAssertThrowsError(try ConversationParameterSnapshot.read(json, modelKey: "configured:missing"))
        XCTAssertEqual(try IOSConversationSettingsObject.decode(json)["foreign"] as? Bool, true)
    }
}
