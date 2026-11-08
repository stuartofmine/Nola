// SPDX-License-Identifier: Apache-2.0
import XCTest
@testable import Nola

@MainActor final class MacMessageSurfaceContextTests: XCTestCase {
    private func identity(conversation: String = "child", message: String = "assistant", run: String = "run", generation: UInt64 = 7) -> AppleUISurfaceIdentity {
        .init(owner: .init(authority_id: "real-authority", conversation_id: conversation, participant_id: "expert",
            context_id: "private-context", originating_run_id: run, message_id: message, part_id: "part"),
            surface_id: "form", generation: generation)
    }
    private func context(read: @escaping (AppleUISurfaceIdentity, UInt64?) async throws -> AppleUISurfaceUpdate<MacCanonicalJSONValue>,
                         submit: @escaping (AppleUISubmission<MacCanonicalJSONValue>) async -> AppleUISubmissionOutcome<MacCanonicalJSONValue> = { _ in .rejected("Host rejected") }) -> MacUISurfaceContext {
        .init(readChanges: read, preview: { _ in throw MacApplicationPortError.invalidResponse },
              readRevision: { _ in nil }, pendingSubmission: { _ in nil }, submit: submit)
    }
    func testChildScopeForwardsCompleteIdentityAndRevisionToOriginalHost() async throws {
        let expected = identity()
        var observed: AppleUISurfaceIdentity?
        var observedRevision: UInt64?
        let callbacks = context(read: { identity, revision in
            observed = identity; observedRevision = revision
            return .unchanged(identity, revision ?? 0)
        }).scoped(to: .init(conversationID: "child", messageID: "assistant", runID: "run"), isCurrent: { true })
        _ = try await callbacks.readChanges(expected, 13)
        XCTAssertEqual(observed, expected)
        XCTAssertEqual(observed?.generation, 7)
        XCTAssertEqual(observedRevision, 13)
        for foreign in [identity(conversation: "parent"), identity(message: "other"), identity(run: "next-run")] {
            do { _ = try await callbacks.readChanges(foreign, nil); XCTFail("Foreign owner must not reach child Host") }
            catch { }
            XCTAssertEqual(observed, expected)
        }
    }
    func testParentLifetimeInvalidatesPendingAndLateReadsWithoutChangingTheChildOwner() async throws {
        var current = true
        let expected = identity()
        let gate = MacSurfaceContextReadGate()
        let callbacks = context(read: { identity, revision in
            await gate.suspend()
            return .unchanged(identity, revision ?? 0)
        }).scoped(to: .init(conversationID: "child", messageID: "assistant", runID: "run"), isCurrent: { current })
        let read = Task { try await callbacks.readChanges(expected, 3) }
        try await gate.waitUntilSuspended()
        current = false
        gate.release()
        do { _ = try await read.value; XCTFail("Late result from the detached parent must not render") }
        catch { }
        XCTAssertThrowsError(try callbacks.pendingSubmission(expected))
        XCTAssertEqual(expected.owner.conversation_id, "child")
    }
    func testReadonlyExpertContextCannotReachGenericQueuedRunFactory() async throws {
        var submissions = 0
        var raw = context(read: { identity, revision in .unchanged(identity, revision ?? 0) }, submit: { _ in
            submissions += 1
            return .rejected("Host result")
        })
        raw.permitsFollowupActions = false
        let callbacks = raw.scoped(to: .init(conversationID:"child", messageID:"assistant", runID:"run"), isCurrent:{true})
        XCTAssertFalse(callbacks.permitsFollowupActions)
        let input = AppleUISubmission<MacCanonicalJSONValue>(interaction:.init(identity:identity(),expected_surface_revision:1,edits:[],action:nil),
            submission_id:UUID().uuidString,submitted_at_ms:1)
        guard case .rejected = await callbacks.submit(input) else { return XCTFail("Read-only expert action must remain explicit unavailable") }
        XCTAssertEqual(submissions,0)
    }

    func testReadonlyVersionValidationRequiresWholeIdentityRevisionPhaseAndNoActions() throws {
        let url = try XCTUnwrap(Bundle(for:Self.self).url(forResource:"form_interaction_v0_9_1",withExtension:"json"))
        let fixture = try XCTUnwrap(JSONSerialization.jsonObject(with:Data(contentsOf:url)) as? [String:Any])
        func decode(_ key:String) throws -> AppleUISnapshot<MacCanonicalJSONValue> {
            try JSONDecoder().decode(AppleUISnapshot<MacCanonicalJSONValue>.self,
                from:JSONSerialization.data(withJSONObject:try XCTUnwrap(fixture[key])))
        }
        let historical = try decode("historical_snapshot")
        let reference = AppleUISurfaceReference(identity:historical.identity,revision:historical.revision)
        XCTAssertTrue(AppleUIReadOnlyReferenceValidation.matches(historical,reference:reference))
        XCTAssertFalse(AppleUIReadOnlyReferenceValidation.matches(historical,
            reference:.init(identity:historical.identity,revision:historical.revision+1)))
        let interactive = try decode("edited_snapshot")
        XCTAssertFalse(AppleUIReadOnlyReferenceValidation.matches(interactive,
            reference:.init(identity:interactive.identity,revision:interactive.revision)))
        let injected = AppleUISnapshot(identity:interactive.identity,revision:interactive.revision,
            catalog_id:interactive.catalog_id,catalog_digest:interactive.catalog_digest,phase:.readOnly,
            root_instance_id:interactive.root_instance_id,nodes:interactive.nodes,summary:interactive.summary)
        XCTAssertFalse(AppleUIReadOnlyReferenceValidation.matches(injected,
            reference:.init(identity:interactive.identity,revision:interactive.revision)),
            "A renderer must not make a mutable snapshot readonly by changing only its phase")
    }

    func testActualCanonicalSourceDeterminesLocalAttachmentAccessEvenWithoutJournalReference() throws {
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent("attachment-source-\(UUID())")
        try FileManager.default.createDirectory(at:directory,withIntermediateDirectories:true)
        defer { try? FileManager.default.removeItem(at:directory) }
        let port = try MacApplicationPort(databaseURL:directory.appendingPathComponent("mirror.db"))
        _ = try port.createConversation(.init(id:"local",title:"Local",settingsJSON:nil,createdAtMs:1,updatedAtMs:1))
        let source = try MacAssistantJournalTestFixture(conversationID:"remote",runID:"remote-run",assistantMessageID:"remote-assistant")
        _ = try source.append("Remote source")
        try source.ingest(into:port,through:try XCTUnwrap(source.heads().last))
        let model = MacWorkspaceModel(canonicalRepository:MacCanonicalConversationRepository(store:port),
            localAgentRuntime:PersistenceTestLocalRuntime(),remoteHost:PersistenceTestRemoteHost(),modelApplication:port)
        defer { model.presentationDidDetach() }
        XCTAssertTrue(try model.isLocalConversationSource("local"))
        XCTAssertFalse(try model.isLocalConversationSource("remote"))
        XCTAssertThrowsError(try model.isLocalConversationSource("missing"))
        model.selectedConversationID = "local"
        XCTAssertFalse(try model.isLocalConversationSource("remote"), "The parent's selected local context grants no local file access to a source-owned child")
    }

}

@MainActor private final class MacSurfaceContextReadGate {
    private var continuation: CheckedContinuation<Void, Never>?
    func suspend() async { await withCheckedContinuation { continuation = $0 } }
    func waitUntilSuspended() async throws {
        let deadline = SuspendingClock.now + .seconds(3)
        while continuation == nil, SuspendingClock.now < deadline { await Task.yield() }
        guard continuation != nil else { throw MacApplicationPortError.invalidResponse }
    }
    func release() { continuation?.resume(); continuation = nil }
}
