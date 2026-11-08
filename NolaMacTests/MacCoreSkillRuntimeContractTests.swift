// SPDX-License-Identifier: Apache-2.0
// Current Skill snapshots retain execution facts; presentation never invents completion.
import Foundation
import XCTest
@testable import Nola

@MainActor
final class MacCoreSkillRuntimeContractTests: XCTestCase {
    private func runObject() -> [String: Any] {
        let digest = String(repeating: "a", count: 64)
        return [
            "run_id": "skill-run", "owner": ["session_id": "session", "parent_turn_id": "turn", "child_turn_id": NSNull()],
            "descriptor": ["skill_id": "documents", "version": "1", "display_name": "Documents",
                "instructions_ref": "skill://documents/1/SKILL.md#sha256=\(digest)", "instructions_digest": digest,
                "required_tools": [], "default_mode": "inline", "execution_contract": [
                    "schemaVersion": 1,
                    "progressSteps": [["id": "prepare", "title": "Prepare", "required": true],
                                      ["id": "render", "title": "Render", "required": true]],
                    "artifacts": [["id": "report", "mediaType": "text/plain", "required": true, "description": "Report"]],
                    "verifiers": [["id": "check", "description": "Check report", "method": "host_check", "required": true]],
                    "resumePoints": ["render"]]],
            "mode": "inline", "status": "running",
            "budget": ["max_model_turns": 8, "max_tool_calls": 32, "max_elapsed_millis": 300_000],
            "usage": ["model_turns": 2, "tool_calls": 1, "elapsed_millis": 120],
            "missing_tools": [], "catalog_revision": 4, "result_ref": NSNull(), "failure": NSNull(),
            "current_progress_step": "render", "completed_progress_steps": ["prepare"],
            "artifacts": ["report": "artifact://report"], "verifications": ["check": verificationObject()],
            "resume_point": "render"
        ]
    }

    private func verificationObject() -> [String: Any] {
        ["verifierId": "check", "passed": false, "summary": "Needs another check", "replacesEvidence": NSNull(),
         "evidence": ["kind": "host_check", "source": ["kind": "execution_evidence", "evidence_id": "evidence-1"],
            "resource_uri": "artifact://report", "version": "v1", "check_kind": "postcondition", "predicate": "report is complete"]]
    }

    private func decode(_ update: [String: Any], protocolVersion: Int = 2) throws -> MacCoreSkillRuntimeUpdateEnvelope {
        try JSONDecoder().decode(MacCoreSkillRuntimeUpdateEnvelope.self, from: JSONSerialization.data(withJSONObject: [
            "protocol_version": protocolVersion, "type": "core_skill_runtime_updated", "session_id": "session",
            "turn_id": "turn", "cursor": 1, "revision": 1, "skill_runtime_revision": 1, "update": update]))
    }

    func testStartedAndColdCanonicalSnapshotPreserveAllExecutionFacts() throws {
        let started = try decode(["type": "started", "run": runObject()])
        guard case .started(let run) = started.update else { return XCTFail("Expected started snapshot") }
        XCTAssertEqual(run.currentProgressStep, "render")
        XCTAssertEqual(run.completedProgressSteps, ["prepare"])
        XCTAssertEqual(run.artifacts, ["report": "artifact://report"])
        XCTAssertEqual(run.verifications["check"]?.evidence.source?.evidenceID, "evidence-1")
        XCTAssertEqual(run.verifications["check"]?.passed, false)
        XCTAssertEqual(run.resumePoint, "render")
        XCTAssertEqual(run.status, .running, "Evidence and progress do not themselves establish completion")
        XCTAssertEqual(run.descriptor.executionContract.verifiers.first?.method, .hostCheck)
        let cold = try JSONDecoder().decode(MacCanonicalSkillRuntimeProjection.self,
            from: JSONSerialization.data(withJSONObject: ["session_id": "session", "cursor": 1, "revision": 1,
                "catalog_revision": 4, "order": ["skill-run"], "runs": ["skill-run": runObject()]]))
        XCTAssertEqual(cold.presentationProjection.runs["skill-run"], run)
        let restored = try JSONDecoder().decode(MacCoreSkillRun.self, from: JSONEncoder().encode(run))
        XCTAssertEqual(restored, run)
    }

    func testFourExecutionEventsRoundTripWithoutPromotingVerificationToCompletion() throws {
        let events: [[String: Any]] = [
            ["type": "progress_updated", "run_id": "skill-run", "step_id": "render", "completed": true],
            ["type": "artifact_registered", "run_id": "skill-run", "artifact_id": "report", "artifact_ref": "artifact://report"],
            ["type": "verification_recorded", "run_id": "skill-run", "result": verificationObject()],
            ["type": "resume_point_updated", "run_id": "skill-run", "resume_point": "render"]
        ]
        for event in events {
            let envelope = try decode(event)
            let restored = try JSONDecoder().decode(MacCoreSkillRuntimeUpdateEnvelope.self,
                from: JSONEncoder().encode(envelope))
            XCTAssertEqual(restored, envelope)
            XCTAssertThrowsError(try decode(event, protocolVersion: 3))
        }
    }

    func testCurrentSnapshotRejectsMissingExecutionStateOrUnboundInstructions() throws {
        for key in ["completed_progress_steps", "artifacts", "verifications"] {
            var run = runObject()
            run.removeValue(forKey: key)
            XCTAssertThrowsError(try decode(["type": "started", "run": run]), key)
        }
        for key in ["instructions_digest", "execution_contract"] {
            var run = runObject()
            var descriptor = try XCTUnwrap(run["descriptor"] as? [String: Any])
            descriptor.removeValue(forKey: key)
            run["descriptor"] = descriptor
            XCTAssertThrowsError(try decode(["type": "started", "run": run]), key)
        }
        for digest in [String(repeating: "A", count: 64), String(repeating: "b", count: 64), "short"] {
            var run = runObject()
            var descriptor = try XCTUnwrap(run["descriptor"] as? [String: Any])
            descriptor["instructions_digest"] = digest
            run["descriptor"] = descriptor
            XCTAssertThrowsError(try decode(["type": "started", "run": run]))
        }
        var run = runObject()
        var descriptor = try XCTUnwrap(run["descriptor"] as? [String: Any])
        var contract = try XCTUnwrap(descriptor["execution_contract"] as? [String: Any])
        contract["schemaVersion"] = 2
        descriptor["execution_contract"] = contract
        run["descriptor"] = descriptor
        XCTAssertThrowsError(try decode(["type": "started", "run": run]))
    }
}
