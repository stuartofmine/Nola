// SPDX-License-Identifier: Apache-2.0
import XCTest
@testable import Nola

final class AppleModelAttemptPresentationTests: XCTestCase {
    private func presentation(_ state: String, successor: String? = nil,
        provider: String? = "provider-a", model: String? = "model-a",
        nextProvider: String? = "provider-a", nextModel: String? = "model-a") -> AppleModelAttemptPresentation {
        .init(state: state, provider: provider, model: model, successorAttemptID: successor,
            successorProvider: nextProvider, successorModel: nextModel)
    }
    func testCommittedSuccessorMeansRestartWithoutInventingFallbackOrCurrentFailure() {
        for state in ["failed", "cancelled", "completed", "running"] {
            let item = presentation(state, successor: "next-attempt")
            XCTAssertEqual(item.kind, .restarted)
            XCTAssertFalse(item.isFailure)
            XCTAssertEqual(item.modelName, "model-a")
            XCTAssertNil(item.successorModelName)
        }
        XCTAssertEqual(presentation("failed", successor: "next", provider: nil, nextProvider: "other").kind, .restarted,
            "Missing identity does not prove either the same route or a route change")
        XCTAssertEqual(presentation("failed", successor: "next", model: nil, nextModel: "other").kind, .restarted)
        XCTAssertEqual(presentation("failed", successor: "next", nextProvider: "provider-b").kind, .routeChanged)
        XCTAssertEqual(presentation("failed", successor: "next", nextModel: "model-b").successorModelName, "model-b")
        XCTAssertEqual(presentation("failed", successor: "next", nextModel: "model-b").kind, .routeChanged)
    }
    func testOnlyActualFailedStateWithoutSuccessorIsFailure() {
        for (state, kind) in [("pending", AppleModelAttemptPresentation.Kind.pending), ("running", .running),
            ("waiting_for_permission", .waiting), ("waiting_for_input", .waiting), ("completed", .completed),
            ("sealed", .completed), ("cancelled", .cancelled), ("failed", .failed), ("unknown", .recorded)] {
            let item = presentation(state, nextProvider: "provider-b", nextModel: "model-b")
            XCTAssertEqual(item.kind, kind, "A suggested next model without a committed successor cannot invent succession")
            XCTAssertEqual(item.isFailure, state == "failed")
            XCTAssertFalse(item.hasSuccessor)
            XCTAssertNil(item.successorModelName)
        }
    }
    func testModelDisplayNeverUsesExecutionModeOrOpaqueProviderAsModelName() {
        let item = presentation("failed", successor: "recovered", provider: "device_local", model: "qwen",
            nextProvider: "device_local", nextModel: "qwen")
        XCTAssertEqual(item.modelName, "qwen")
        XCTAssertEqual(item.kind, .restarted)
        XCTAssertFalse(item.modelName.contains("device_local"))
        XCTAssertEqual(presentation("running", provider: "opaque-provider", model: nil).modelName, "—")
    }
}
