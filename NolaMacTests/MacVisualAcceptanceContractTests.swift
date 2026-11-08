// SPDX-License-Identifier: Apache-2.0

import XCTest
@testable import Nola

final class MacVisualAcceptanceContractTests: XCTestCase {
    func testOverallPassWithoutObservedLifecycleFramesIsRejected() throws {
        let root = FileManager.default.temporaryDirectory
            .appendingPathComponent("nola-visual-contract-\(UUID().uuidString)", isDirectory: true)
        try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: root) }
        let artifact = root.appendingPathComponent("frames", isDirectory: true)
        try FileManager.default.createDirectory(at: artifact, withIntermediateDirectories: true)
        let receipt = root.appendingPathComponent("receipt.json")
        let payload: [String: Any] = [
            "schema_version": 2,
            "artifact_path": artifact.path,
            "artifact_kind": "application_window_sequence",
            "observer": "visual-reviewer",
            "observation_method": "image_capable_reviewer",
            "verdict": "pass",
            "observations": [],
            "motion_reviewed": false,
            "notes": "Generated but not inspected",
        ]
        try JSONSerialization.data(withJSONObject: payload).write(to: receipt)

        let result = MacVisualAcceptanceContract.validate(
            receiptURL: receipt,
            artifactPath: artifact.path,
            artifactKind: "application_window_sequence",
            requiredScope: "application_window_sequence"
        )

        XCTAssertFalse(result.passed)
        XCTAssertEqual(Set(result.missingCheckpoints), MacVisualAcceptanceContract.conversationCheckpoints)
        XCTAssertFalse(result.motionReviewed)
    }

    func testPassingNotesWithoutOpenedEvidenceFilesAreRejected() throws {
        let root = FileManager.default.temporaryDirectory
            .appendingPathComponent("nola-visual-contract-\(UUID().uuidString)", isDirectory: true)
        let artifact = root.appendingPathComponent("frames", isDirectory: true)
        try FileManager.default.createDirectory(at: artifact, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: root) }
        let observations = MacVisualAcceptanceContract.conversationCheckpoints.map { checkpoint in
            [
                "checkpoint": checkpoint,
                "verdict": "pass",
                "notes": "Claimed as observed",
                "evidence_path": artifact.appendingPathComponent("missing-\(checkpoint).png").path,
                "evidence_kind": "screenshot",
            ]
        }
        let receipt = root.appendingPathComponent("receipt.json")
        let payload: [String: Any] = [
            "schema_version": 2,
            "artifact_path": artifact.path,
            "artifact_kind": "application_window_sequence",
            "observer": "visual-reviewer",
            "observation_method": "image_capable_reviewer",
            "verdict": "pass",
            "observations": observations,
            "motion_reviewed": true,
            "notes": "Evidence paths were never generated or opened",
        ]
        try JSONSerialization.data(withJSONObject: payload).write(to: receipt)

        let result = MacVisualAcceptanceContract.validate(
            receiptURL: receipt,
            artifactPath: artifact.path,
            artifactKind: "application_window_sequence",
            requiredScope: "application_window_sequence"
        )
        XCTAssertFalse(result.passed)
        XCTAssertEqual(Set(result.missingCheckpoints), MacVisualAcceptanceContract.conversationCheckpoints)
    }
}
