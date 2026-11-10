// SPDX-License-Identifier: Apache-2.0
// Pure contract for proving that real UI artifacts were actually observed.

import Foundation

nonisolated enum MacVisualAcceptanceContract {
    static let schemaVersion = 1
    static let conversationCheckpoints: Set<String> = [
        "assistant_shell",
        "active_reasoning",
        "active_tool",
        "partial_markdown",
        "composer_follow",
        "terminal",
        "reasoning_toggle",
        "conversation_restore",
        "application_relaunch",
    ]

    struct Observation: Decodable, Equatable, Sendable {
        let checkpoint: String
        let verdict: String
        let notes: String
        let evidencePath: String
        let evidenceKind: String
        enum CodingKeys: String, CodingKey { case checkpoint, verdict, notes; case evidencePath = "evidence_path"; case evidenceKind = "evidence_kind" }
    }

    struct Receipt: Decodable, Equatable, Sendable {
        let schemaVersion: Int
        let artifactPath: String
        let artifactKind: String
        let observer: String
        let observationMethod: String
        let verdict: String
        let observations: [Observation]
        let motionReviewed: Bool
        let notes: String

        enum CodingKeys: String, CodingKey {
            case observer, verdict, observations, notes
            case schemaVersion = "schema_version"
            case artifactPath = "artifact_path"
            case artifactKind = "artifact_kind"
            case motionReviewed = "motion_reviewed"
            case observationMethod = "observation_method"
        }
    }

    struct Validation: Equatable, Sendable {
        let passed: Bool
        let observed: Bool
        let artifactPath: String
        let artifactKind: String
        let requiredScope: String
        let observer: String
        let verdict: String
        let observedCheckpoints: [String]
        let missingCheckpoints: [String]
        let motionReviewed: Bool
        let notes: String
        let reason: String?

        var dictionary: [String: Any] {
            var result: [String: Any] = [
                "passed": passed,
                "observed": observed,
                "artifact_path": artifactPath,
                "artifact_kind": artifactKind,
                "required_scope": requiredScope,
                "observer": observer,
                "verdict": verdict,
                "observed_checkpoints": observedCheckpoints,
                "missing_checkpoints": missingCheckpoints,
                "motion_reviewed": motionReviewed,
                "notes": notes,
            ]
            if let reason { result["reason"] = reason }
            return result
        }
    }

    static func validate(
        receiptURL: URL,
        artifactPath: String,
        artifactKind: String,
        requiredScope: String
    ) -> Validation {
        let expectedArtifact = URL(fileURLWithPath: artifactPath).standardizedFileURL.path
        guard let data = try? Data(contentsOf: receiptURL),
              let receipt = try? JSONDecoder().decode(Receipt.self, from: data) else {
            return failure(
                reason: "Visual artifact requires a valid observation receipt",
                artifactPath: expectedArtifact,
                artifactKind: artifactKind,
                requiredScope: requiredScope
            )
        }

        let artifactRoot = URL(fileURLWithPath: expectedArtifact, isDirectory: true).standardizedFileURL.path
        let acceptedObservationMethods: Set<String> = ["human", "image_capable_reviewer"]
        let acceptedEvidenceKinds: Set<String> = ["screenshot", "recording_frame", "recording"]
        let passedCheckpoints = Set(receipt.observations.compactMap { observation -> String? in
            let evidence = URL(fileURLWithPath: observation.evidencePath).standardizedFileURL.path
            guard observation.verdict == "pass",
                  !observation.notes.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty,
                  acceptedEvidenceKinds.contains(observation.evidenceKind),
                  evidence.hasPrefix(artifactRoot + "/"),
                  FileManager.default.fileExists(atPath: evidence) else {
                return nil
            }
            return observation.checkpoint
        })
        let missing = conversationCheckpoints.subtracting(passedCheckpoints).sorted()
        let normalizedReceiptArtifact = URL(fileURLWithPath: receipt.artifactPath)
            .standardizedFileURL.path
        let observer = receipt.observer.trimmingCharacters(in: .whitespacesAndNewlines)
        let valid = receipt.schemaVersion == schemaVersion
            && normalizedReceiptArtifact == expectedArtifact
            && receipt.artifactKind == artifactKind
            && artifactKind == requiredScope
            && !observer.isEmpty
            && acceptedObservationMethods.contains(receipt.observationMethod)
            && receipt.verdict == "pass"
            && missing.isEmpty
            && receipt.motionReviewed

        return Validation(
            passed: valid,
            observed: true,
            artifactPath: expectedArtifact,
            artifactKind: artifactKind,
            requiredScope: requiredScope,
            observer: observer,
            verdict: receipt.verdict,
            observedCheckpoints: passedCheckpoints.sorted(),
            missingCheckpoints: missing,
            motionReviewed: receipt.motionReviewed,
            notes: receipt.notes,
            reason: valid ? nil : "Observation receipt does not satisfy the visual contract"
        )
    }

    static func failure(
        reason: String,
        artifactPath: String = "",
        artifactKind: String = "",
        requiredScope: String = ""
    ) -> Validation {
        Validation(
            passed: false,
            observed: false,
            artifactPath: artifactPath,
            artifactKind: artifactKind,
            requiredScope: requiredScope,
            observer: "",
            verdict: "invalid",
            observedCheckpoints: [],
            missingCheckpoints: conversationCheckpoints.sorted(),
            motionReviewed: false,
            notes: "",
            reason: reason
        )
    }
}
