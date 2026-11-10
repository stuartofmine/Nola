// SPDX-License-Identifier: Apache-2.0
import XCTest
@testable import Nola

@MainActor final class MacRoomRecoveryBoundaryTests: XCTestCase {
    func testPersistedClaimReplaysOriginalIntentWithoutRecapturing() async throws {
        let fixture = MacRoomBoundaryFixture()
        let workspace = MacAgentRoomWorkspace(service: fixture.service) { receipt, intent in
            XCTAssertEqual(receipt.executionID, fixture.original.execution_id)
            XCTAssertEqual(intent, fixture.original)
            fixture.record("execute")
        }
        workspace.prepareOwner = { _ in XCTFail("A persisted intent must not recapture"); return fixture.original }
        await workspace.startSelected(fixture.receipt(state: "executing"))
        let events = fixture.events
        XCTAssertFalse(events.contains("prepare_execution"))
        XCTAssertTrue(events.contains("claim_execution"))
        XCTAssertTrue(events.contains("execute"), "False claim permission still requires original-ID recovery after absent Run")
    }

    func testExistingRunOnlyRecoversAndNeverAdmitsAgain() async throws {
        let fixture = MacRoomBoundaryFixture(existingRun: true)
        let workspace = MacAgentRoomWorkspace(service: fixture.service) { _, _ in XCTFail("Existing Run must not dispatch") }
        workspace.prepareOwner = { _ in XCTFail("Must use persisted intent"); return fixture.original }
        workspace.recoverOwner = { receipt, intent in
            XCTAssertEqual(receipt.executionID, intent.execution_id)
            fixture.record("recover")
        }
        await workspace.startSelected(fixture.receipt(state: "executing"))
        XCTAssertTrue(fixture.events.contains("recover"))
    }

    func testCancelledClaimCleansPreparedOwnerWithoutDispatch() async throws {
        let fixture = MacRoomBoundaryFixture(claimState: "cancelled")
        let workspace = MacAgentRoomWorkspace(service: fixture.service) { _, _ in XCTFail("Cancelled claim cannot dispatch") }
        workspace.prepareOwner = { _ in fixture.original }
        workspace.cancelPreparedOwner = { receipt in
            XCTAssertEqual(receipt.state, "cancelled")
            fixture.record("cleanup")
        }
        await workspace.startSelected(fixture.receipt(state: "selected"))
        XCTAssertTrue(fixture.events.contains("cleanup"))
        XCTAssertFalse(fixture.events.contains("load_exact_run_invocation"))
    }

    func testCancellationDuringPrepareCASFailureStillCleansFrozenRemoteRequest() async throws {
        let fixture = MacRoomBoundaryFixture(persisted: false, prepareRejected: true)
        let workspace = MacAgentRoomWorkspace(service: fixture.service) { _, _ in XCTFail("Failed prepare cannot dispatch") }
        workspace.prepareOwner = { _ in fixture.record("capture"); return fixture.original }
        workspace.cancelPreparedOwner = { _ in fixture.record("cleanup") }
        await workspace.startSelected(fixture.receipt(state: "selected"))
        XCTAssertEqual(fixture.events.filter { $0 == "capture" }.count, 1)
        XCTAssertTrue(fixture.events.contains("cleanup"))
        XCTAssertFalse(fixture.events.contains("claim_execution"))
    }

    func testLateReceiptCannotRegressCancellationOrOverwriteAnotherRoom() async throws {
        let fixture = MacRoomBoundaryFixture()
        let workspace = MacAgentRoomWorkspace(service: fixture.service)
        let newer = fixture.receipt(state: "cancelled", revision: 9)
        workspace.applyReceipt(newer)
        await workspace.select(.init(revision: 1, room: .init(id: "room-b", title: "B", participants: [])))
        workspace.applyReceipt(fixture.receipt(state: "selected", revision: 3))
        let other = fixture.receipt(state: "accepted", revision: 1, roomID: "room-b")
        workspace.applyReceipt(other)
        XCTAssertEqual(workspace.submissions["room-a"]?.first?.state, "cancelled")
        XCTAssertEqual(workspace.submissions["room-b"]?.first?.state, "accepted")
        let changed = fixture.receipt(state: "executing", revision: 10, text: "foreign input")
        workspace.applyReceipt(changed)
        XCTAssertEqual(workspace.submissions["room-a"]?.first?.input.text, "original")
        XCTAssertEqual(workspace.submissions["room-a"]?.first?.revision, 9)
        XCTAssertEqual(workspace.selectedRoom?.room.id, "room-b", "Late room A callback cannot retarget selection")
    }

    func testRemoteRecoveryUsesSourcePathNotLocalInvocationLookup() async throws {
        let fixture = MacRoomBoundaryFixture(remote: true)
        let workspace = MacAgentRoomWorkspace(service: fixture.service) { _, intent in
            XCTAssertEqual(intent, fixture.original)
            fixture.record("source-observe")
        }
        workspace.prepareOwner = { _ in XCTFail("Must not rebuild remote body"); return fixture.original }
        await workspace.startSelected(fixture.receipt(state: "executing"))
        XCTAssertTrue(fixture.events.contains("source-observe"))
        XCTAssertFalse(fixture.events.contains("load_exact_run_invocation"))
    }
}

private nonisolated final class MacRoomBoundaryFixture: @unchecked Sendable {
    let original: MacAgentRoomService.ExecutionIntent
    let existingRun: Bool
    let claimState: String
    let persisted: Bool
    let prepareRejected: Bool
    private let lock = NSLock()
    private var recorded: [String] = []
    var events: [String] { lock.withLock { recorded } }
    func record(_ event: String) { lock.withLock { recorded.append(event) } }

    init(existingRun: Bool = false, claimState: String = "executing", persisted: Bool = true,
         prepareRejected: Bool = false, remote: Bool = false) {
        self.existingRun = existingRun; self.claimState = claimState
        self.persisted = persisted; self.prepareRejected = prepareRejected
        let id = "00000000-0000-4000-8000-000000000001"
        if remote {
            original = .init(execution_id: id, envelope: .remoteHost(.init(kind: "remote_host",
                identity: .init(source_host_id: "host", client_id: "paired", conversation_id: "context",
                    submission_id: id, user_message_id: id, assistant_message_id: id + ":assistant"),
                request_digest: String(repeating: "a", count: 64))))
        } else {
            original = .init(execution_id: id, envelope: .init(kind: "native", platform: "macos",
                conversation_configuration_revision: 7, model_profile_id: "profile", model_profile_revision: 2,
                expected_parent_message_id: "parent", text: "original"))
        }
    }
    func receipt(state: String, revision: Int64 = 3, roomID: String = "room-a", text: String = "original") -> MacRoomSubmission {
        .init(revision: revision, input: .init(roomID: roomID, submissionID: "submission",
            expectedRoomRevision: 1, text: text, participantID: "member", atMs: 1),
            state: state, selectedParticipantID: "member", failureCode: nil, executionID: original.execution_id,
            terminal: nil, contextConversationID: "context")
    }
    var service: MacAgentRoomService { .init(execute: { [self] root in
        guard case .object(let fields) = root else { throw CocoaError(.coderInvalidValue) }
        let command: [String: MacCanonicalJSONValue]
        if case .object(let nested)? = fields["command"] { command = nested } else { command = fields }
        guard case .string(let type)? = command["type"] else { throw CocoaError(.coderInvalidValue) }
        record(type)
        switch type {
        case "execution_intent": return persisted ? try value(original) : .null
        case "prepare_execution":
            if prepareRejected { throw CocoaError(.coderInvalidValue) }
            return try value(receipt(state: "selected", revision: 4))
        case "claim_execution":
            return .object(["submission": try value(receipt(state: claimState, revision: 5)), "dispatch_permitted": .bool(false)])
        case "load_exact_run_invocation":
            return existingRun ? .object(["conversation_id": .string("context"), "run_id": .string(original.execution_id),
                "assistant_message_id": .string(original.execution_id + ":assistant")]) : .null
        case "reconcile_execution": return try value(receipt(state: "completed", revision: 6))
        case "submissions":
            if command["room_id"] == .string("room-b") { return .array([]) }
            return .array([try value(receipt(state: "cancelled", revision: 8))])
        case "read": return try value(MacAgentRoomSnapshot(revision: 1,
            room: .init(id: "room-b", title: "B", participants: [])))
        case "get_product_setting": return .null
        default: throw CocoaError(.coderInvalidValue)
        }
    }) }
    private func value<T: Encodable>(_ value: T) throws -> MacCanonicalJSONValue {
        try JSONDecoder().decode(MacCanonicalJSONValue.self, from: JSONEncoder().encode(value))
    }
}
