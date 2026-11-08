// SPDX-License-Identifier: Apache-2.0
// Purpose: Verifies host permission requests remain visible when protocol request and tool span IDs differ.

import XCTest
@testable import Nola

final class HostPermissionPresentationTests: XCTestCase {
    @MainActor
    func testImmediateRetryFromFailurePublicationOrCallbackCannotBeLost() async throws {
        for retryFromState in [true, false] {
            let session = IOSHostPermissionSession()
            let request = HostAgentPermissionRequest(id: "retry", allowConversation: true, toolName: "read", match: "fixture", nonce: "same-nonce")
            let submitted = expectation(description: "Explicit retry delivered")
            var sends = 0
            var failures = 0
            session.onStateChange = { state in
                guard let pending = state.request else { return }
                if state.submissionError == nil || retryFromState {
                    session.resolve(requestID: pending.id, decision: .deny)
                }
            }
            let task = session.start(request, conversationID: UUID(), isCurrent: { true }, send: { payload in
                sends += 1
                XCTAssertEqual(payload["nonce"] as? String, "same-nonce")
                if sends == 1 { throw URLError(.notConnectedToInternet) }
            }, onSubmitted: { _ in submitted.fulfill() }, onSubmissionFailure: { request, _ in
                failures += 1
                if !retryFromState { session.resolve(requestID: request.id, decision: .deny) }
            })
            await fulfillment(of: [submitted], timeout: 1)
            // Also bound failure cleanup if the regression loses the retry.
            session.cancel()
            await task.value
            XCTAssertEqual(sends, 2)
            XCTAssertEqual(failures, 1)
        }
    }

    @MainActor
    func testImmediateDecisionWhenCardIsPublishedCannotBeLost() async throws {
        let session = IOSHostPermissionSession()
        let request = HostAgentPermissionRequest(id: "immediate", allowConversation: true, toolName: "read", match: "fixture", nonce: "nonce")
        let submitted = expectation(description: "Immediate visible-card decision delivered")
        session.onStateChange = { state in
            if let pending = state.request {
                session.resolve(requestID: pending.id, decision: .deny)
            }
        }
        let task = session.start(request, conversationID: UUID(), isCurrent: { true }, send: { payload in
            XCTAssertEqual(payload["nonce"] as? String, "nonce")
            submitted.fulfill()
        }, onSubmitted: { _ in }, onSubmissionFailure: { _, error in
            XCTFail("Unexpected permission failure: \(error)")
        })
        defer { task.cancel(); session.cancel() }
        await fulfillment(of: [submitted], timeout: 1)
        session.cancel()
        await task.value
    }
    @MainActor
    func testRenewedNonceReplacesSameRequestWithoutSendingStaleDecision() async throws {
        let session = IOSHostPermissionSession()
        let conversation = UUID()
        let original = HostAgentPermissionRequest(id: "request", allowConversation: true, toolName: "read", nonce: "old", expiresAt: 100)
        let renewed = HostAgentPermissionRequest(id: "request", allowConversation: true, toolName: "read", nonce: "new", expiresAt: 400)
        let ready = expectation(description: "Original is visible")
        var visibleNonces: [String] = []
        var sentNonces: [String] = []
        session.onStateChange = { state in
            guard let request = state.request, !state.isSubmitting else { return }
            visibleNonces.append(request.nonce ?? "")
            if request.nonce == "old" { ready.fulfill() }
            else { session.resolve(requestID: request.id, decision: .allowOnce) }
        }
        let originalTask = session.start(original, conversationID: conversation, isCurrent: { true },
            send: { payload in sentNonces.append(payload["nonce"] as! String) },
            onSubmitted: { _ in XCTFail("Replaced challenge must not submit") },
            onSubmissionFailure: { _, error in XCTFail("Unexpected failure: \(error)") })
        await fulfillment(of: [ready], timeout: 1)
        // Exact replay does not publish a second card or lose its receiver.
        session.start(original, conversationID: conversation, isCurrent: { true }, send: { _ in
            XCTFail("Replay must keep the original interaction")
        }, onSubmitted: { _ in }, onSubmissionFailure: { _, _ in })
        let successor = session.start(renewed, conversationID: conversation, isCurrent: { true },
            send: { payload in sentNonces.append(payload["nonce"] as! String) },
            onSubmitted: { _ in }, onSubmissionFailure: { _, error in XCTFail("Unexpected failure: \(error)") })
        await originalTask.value
        await successor.value
        XCTAssertEqual(visibleNonces, ["old", "new"])
        XCTAssertEqual(sentNonces, ["new"])
    }

    @MainActor
    func testDecisionPublishesBusyAndIgnoresRepeatedTapUntilReply() async throws {
        let session = IOSHostPermissionSession()
        let request = HostAgentPermissionRequest(id: "busy", allowConversation: true, toolName: "read", match: "fixture", nonce: "nonce")
        var states: [Bool] = []
        var sends = 0
        session.onStateChange = { state in
            if state.request != nil {
                states.append(state.isSubmitting)
                // A second tap during publication must not resume twice.
                session.resolve(requestID: request.id, decision: .allowOnce)
            }
        }
        let task = session.start(request, conversationID: UUID(), isCurrent: { true }, send: { _ in
            sends += 1
            XCTAssertEqual(states, [false, true])
            session.resolve(requestID: request.id, decision: .deny)
            try await Task.sleep(nanoseconds: 10_000_000)
        }, onSubmitted: { decision in
            XCTAssertEqual(decision, .allowOnce)
        }, onSubmissionFailure: { _, error in XCTFail("Unexpected failure: \(error)") })
        await task.value
        XCTAssertEqual(sends, 1)
        XCTAssertEqual(states, [false, true])
    }

    func testUsesExplicitPermissionOwnerEvenWhenAnotherToolIsWaiting() {
        XCTAssertEqual(
            resolveActiveHostPermissionEventID(
                requestID: "permission-42",
                waitingEventIDs: ["call-1", "call-2"],
                permissionOwners: [(requestID: "permission-42", toolCallID: "call-1")]
            ),
            "call-1"
        )
    }

    func testDoesNotGuessFromWaitingOrderOrCoincidentIdentity() {
        XCTAssertNil(
            resolveActiveHostPermissionEventID(
                requestID: "call-2",
                waitingEventIDs: ["call-1", "call-2"],
                permissionOwners: []
            )
        )
    }

    func testDoesNotAttachRequestWithoutWaitingTool() {
        XCTAssertNil(
            resolveActiveHostPermissionEventID(
                requestID: "permission-42",
                waitingEventIDs: [],
                permissionOwners: [(requestID: "permission-42", toolCallID: "call-1")]
            )
        )
    }

    func testDoesNotAttachAmbiguousOrForeignPermissionOwner() {
        for owners in [
            [(requestID: "permission", toolCallID: "call-1"), (requestID: "permission", toolCallID: "call-2")],
            [(requestID: "older-permission", toolCallID: "call-1")],
            [(requestID: "permission", toolCallID: "foreign-call")],
        ] {
            XCTAssertNil(resolveActiveHostPermissionEventID(
                requestID: "permission", waitingEventIDs: ["call-1", "call-2"], permissionOwners: owners))
        }
    }
}
