// SPDX-License-Identifier: Apache-2.0
import XCTest
@testable import Nola

@MainActor final class IOSModelAttemptPresentationTests: XCTestCase {
    private func part(state: String, successor: String? = nil, nextModel: String? = "qwen") throws -> IOSConversationProjectionV1.ModelAttemptPart {
        var value: [String: Any] = ["part_id":"attempt-part", "revision":1, "state":state, "attempt_id":"original",
            "provider":"device_local", "model":"qwen", "failure":["code":"model_resampled", "message":"Request was resumed", "retryable":true]]
        if let successor { value["fallback_attempt_id"] = successor }
        if let nextModel { value["fallback_model"] = nextModel }
        value["fallback_provider"] = "device_local"
        return try JSONDecoder().decode(IOSConversationProjectionV1.ModelAttemptPart.self,
            from: JSONSerialization.data(withJSONObject: value))
    }
    func testActualBubbleProjectionDoesNotRenderColdRecoveryAsRedModelFallback() throws {
        let disclosure = IOSModelAttemptDisclosure(try part(state: "failed", successor: "recovered"))
        XCTAssertEqual(disclosure.id, "attempt-part")
        XCTAssertEqual(disclosure.presentation.kind, .restarted)
        XCTAssertFalse(disclosure.presentation.isFailure)
        XCTAssertEqual(disclosure.presentation.modelName, "qwen")
        XCTAssertEqual(disclosure.failure, "Request was resumed")
        let noSuccessor = IOSModelAttemptDisclosure(try part(state: "failed", nextModel: "other-model"))
        XCTAssertEqual(noSuccessor.presentation.kind, .failed)
        XCTAssertTrue(noSuccessor.presentation.isFailure)
        XCTAssertFalse(noSuccessor.presentation.hasSuccessor)
        let changed = IOSModelAttemptDisclosure(try part(state: "failed", successor: "next", nextModel: "other-model"))
        XCTAssertEqual(changed.presentation.kind, .routeChanged)
        XCTAssertFalse(changed.presentation.isFailure)
    }
    func testCancelledAndCompletedActualPartsRetainTheirOwnStateWithoutASuccessor() throws {
        XCTAssertEqual(IOSModelAttemptDisclosure(try part(state: "cancelled")).presentation.kind, .cancelled)
        XCTAssertEqual(IOSModelAttemptDisclosure(try part(state: "completed")).presentation.kind, .completed)
        XCTAssertFalse(IOSModelAttemptDisclosure(try part(state: "running")).presentation.isFailure)
    }
}
