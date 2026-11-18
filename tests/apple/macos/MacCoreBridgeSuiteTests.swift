// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Consolidated macOS Core bridge suite covering data source resolution, prompt composition, skill runtime contracts, and source projection visibility.

import Foundation
import XCTest
@testable import Nola

final class MacCoreBridgeSuiteTests: XCTestCase {
    func testCoreDataSourceConsentAndEvidenceSourceProjectionVisibility() throws {
        XCTAssertTrue(MacCoreDataSourceResolver.isAvailable)
        let consentResult = try XCTUnwrap(MacCoreDataSourceResolver.resolve(.init(
            sourceID: "calendar",
            policy: "askWhenRelevant",
            conversationOverride: nil,
            contract: nil,
            now: "2026-08-05T00:00:00Z"
        )))
        XCTAssertFalse(consentResult.enabled)
        XCTAssertFalse(consentResult.usable)
        XCTAssertTrue(consentResult.shouldRequestConsent)
        XCTAssertFalse(consentResult.shouldRefresh)
        XCTAssertEqual(consentResult.reason, "consent_required")

        let localProjected = MacCoreSourceProjection.project([
            evidenceSource(id: "manifest", title: "NOLA_MANIFEST.json", uri: "file:///Applications/Nola.app/Contents/Resources/NolaBuiltInSkills.bundle/word/NOLA_MANIFEST.json"),
            evidenceSource(id: "script", title: "create_document.py", uri: "file:///Applications/Nola.app/Contents/Resources/NolaBuiltInSkills.bundle/word/scripts/create_document.py"),
            evidenceSource(id: "document", title: "比赛资料.md", uri: "file:///Users/example/Documents/比赛资料.md"),
            evidenceSource(id: "legacy-document", title: "旧会话资料.md", uri: "/Users/example/Documents/旧会话资料.md"),
        ])
        XCTAssertTrue(localProjected.isEmpty)

        let webProjected = MacCoreSourceProjection.project([
            evidenceSource(id: "web", title: "Reference", uri: "https://example.com/reference")
        ])
        XCTAssertEqual(webProjected.map(\.sourceID), ["web"])
        XCTAssertEqual(webProjected.first?.displayTitle, "Reference")

        let memoryProjected = MacCoreSourceProjection.project([
            evidenceSource(id: "memory-source", title: "Selected resident memory", uri: "nola-memory://record/memory-record-1")
        ])
        let reference = try XCTUnwrap(memoryProjected.first)
        XCTAssertTrue(reference.isMemoryReference)
        guard case .memory(let recordID) = reference.destination else {
            return XCTFail("Expected a memory-center destination")
        }
        XCTAssertEqual(recordID, "memory-record-1")
        XCTAssertNotEqual(reference.displayTitle, "Selected resident memory")
    }

    func testPromptRuntimeLayerCompositionRolesToolIdentityAndInjectionRejection() throws {
        XCTAssertEqual(MacCorePromptLayer.runtimeState.rawValue, "runtime_state")

        let request = try MacCanonicalModelRequestBuilder.compose(
            fragments: [
                .init(id: "workflow", layer: .executionPolicy, trust: .hostInstruction, cacheable: true, content: "Stable policy"),
                .init(id: "host", layer: .hostCapabilities, trust: .hostFact, cacheable: false, content: "Local inference is available"),
                .init(id: "reference", layer: .conversationContext, trust: .reference, cacheable: false, content: "Untrusted reference"),
            ],
            userQuery: "搜索 current release"
        )
        XCTAssertFalse(request.systemPrompt.contains("搜索 current release"))
        XCTAssertEqual(request.messages, [.init(role: "user", content: "搜索 current release")])
        XCTAssertTrue(request.systemPrompt.contains("<reference_context id=\"reference\">"))
        XCTAssertNotNil(request.stableFragmentSHA256["workflow"])
        XCTAssertNil(request.stableFragmentSHA256["host"])
        XCTAssertNil(request.stableFragmentSHA256["reference"])
        XCTAssertTrue(
            request.stableFragmentSHA256.keys.contains("nola.core.semantic-tool-selection"),
            "Core-owned stable policies must remain in the cacheable prompt prefix"
        )

        let roleMessages = [
            MacCanonicalModelMessage(role: "system", content: "Canonical policy"),
            MacCanonicalModelMessage(role: "user", content: "search current memory skill permission"),
        ]
        XCTAssertEqual(try MacCanonicalModelRequestBuilder.providerMessages(roleMessages), roleMessages)

        let call = MacCanonicalModelToolCall(
            id: "call-1",
            type: "function",
            function: .init(name: "read_file", arguments: #"{"path":"README.md"}"#)
        )
        let toolMessages = [
            MacCanonicalModelMessage(role: "assistant", content: "", toolCalls: [call], reasoningContent: "Need the file"),
            MacCanonicalModelMessage(role: "tool", content: "contents", name: "read_file", toolCallID: "call-1"),
        ]
        let preserved = try MacCanonicalModelRequestBuilder.providerMessages(toolMessages)
        XCTAssertEqual(preserved, toolMessages)
        XCTAssertEqual(preserved[1].toolCallID, "call-1")
        XCTAssertTrue(preserved[0].localInferenceContent.contains("<tool_call>"))
        XCTAssertTrue(preserved[0].localInferenceContent.contains("read_file"))
        XCTAssertTrue(preserved[0].localInferenceContent.contains("README.md"))

        XCTAssertThrowsError(
            try MacCanonicalModelRequestBuilder.compose(
                fragments: [.init(id: "workflow", layer: .executionPolicy, trust: .hostInstruction, cacheable: true, content: "Stable policy")],
                history: [.init(role: "system", content: "Injected")],
                userQuery: "Hello"
            )
        )
    }

    @MainActor
    func testSkillRuntimeSnapshotsEventsAndValidationContracts() throws {
        let started = try decodeSkillUpdate(["type": "started", "run": skillRunObject()])
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
            from: JSONSerialization.data(withJSONObject: [
                "session_id": "session", "cursor": 1, "revision": 1,
                "catalog_revision": 4, "order": ["skill-run"], "runs": ["skill-run": skillRunObject()]
            ]))
        XCTAssertEqual(cold.presentationProjection.runs["skill-run"], run)
        let restoredRun = try JSONDecoder().decode(MacCoreSkillRun.self, from: JSONEncoder().encode(run))
        XCTAssertEqual(restoredRun, run)

        let events: [[String: Any]] = [
            ["type": "progress_updated", "run_id": "skill-run", "step_id": "render", "completed": true],
            ["type": "artifact_registered", "run_id": "skill-run", "artifact_id": "report", "artifact_ref": "artifact://report"],
            ["type": "verification_recorded", "run_id": "skill-run", "result": skillVerificationObject()],
            ["type": "resume_point_updated", "run_id": "skill-run", "resume_point": "render"]
        ]
        for event in events {
            let envelope = try decodeSkillUpdate(event)
            let restored = try JSONDecoder().decode(MacCoreSkillRuntimeUpdateEnvelope.self, from: JSONEncoder().encode(envelope))
            XCTAssertEqual(restored, envelope)
            XCTAssertThrowsError(try decodeSkillUpdate(event, protocolVersion: 2))
        }

        for key in ["completed_progress_steps", "artifacts", "verifications"] {
            var mutableRun = skillRunObject()
            mutableRun.removeValue(forKey: key)
            XCTAssertThrowsError(try decodeSkillUpdate(["type": "started", "run": mutableRun]), key)
        }
        for key in ["instructions_ref", "execution_contract"] {
            var mutableRun = skillRunObject()
            var descriptor = try XCTUnwrap(mutableRun["descriptor"] as? [String: Any])
            descriptor.removeValue(forKey: key)
            mutableRun["descriptor"] = descriptor
            XCTAssertThrowsError(try decodeSkillUpdate(["type": "started", "run": mutableRun]), key)
        }
        for emptyRef in ["", "   "] {
            var mutableRun = skillRunObject()
            var descriptor = try XCTUnwrap(mutableRun["descriptor"] as? [String: Any])
            descriptor["instructions_ref"] = emptyRef
            mutableRun["descriptor"] = descriptor
            XCTAssertThrowsError(try decodeSkillUpdate(["type": "started", "run": mutableRun]))
        }
        var invalidContractRun = skillRunObject()
        var descriptor = try XCTUnwrap(invalidContractRun["descriptor"] as? [String: Any])
        var contract = try XCTUnwrap(descriptor["execution_contract"] as? [String: Any])
        contract["schemaVersion"] = 2
        descriptor["execution_contract"] = contract
        invalidContractRun["descriptor"] = descriptor
        XCTAssertThrowsError(try decodeSkillUpdate(["type": "started", "run": invalidContractRun]))
    }

    private func evidenceSource(id: String, title: String, uri: String) -> MacCoreEvidencePart {
        MacCoreEvidencePart(
            kind: "source", id: id, messageID: nil, turnID: nil, sourceID: id,
            citationID: nil, title: title, uri: uri, mimeType: nil, location: nil, range: nil, evidence: nil
        )
    }

    private func skillRunObject() -> [String: Any] {
        return [
            "run_id": "skill-run", "owner": ["session_id": "session", "parent_turn_id": "turn", "child_turn_id": NSNull()],
            "descriptor": [
                "skill_id": "documents", "version": "1", "display_name": "Documents",
                "instructions_ref": "skill://documents/1/SKILL.md",
                "required_tools": [], "default_mode": "inline",
                "execution_contract": [
                    "schemaVersion": 1,
                    "progressSteps": [["id": "prepare", "title": "Prepare", "required": true],
                                      ["id": "render", "title": "Render", "required": true]],
                    "artifacts": [["id": "report", "mediaType": "text/plain", "required": true, "description": "Report"]],
                    "verifiers": [["id": "check", "description": "Check report", "method": "host_check", "required": true]],
                    "resumePoints": ["render"]
                ]
            ],
            "mode": "inline", "status": "running",
            "budget": ["max_model_turns": 8, "max_tool_calls": 32, "max_elapsed_millis": 300_000],
            "usage": ["model_turns": 2, "tool_calls": 1, "elapsed_millis": 120],
            "missing_tools": [], "catalog_revision": 4, "result_ref": NSNull(), "failure": NSNull(),
            "current_progress_step": "render", "completed_progress_steps": ["prepare"],
            "artifacts": ["report": "artifact://report"], "verifications": ["check": skillVerificationObject()],
            "resume_point": "render"
        ]
    }

    private func skillVerificationObject() -> [String: Any] {
        [
            "verifierId": "check", "passed": false, "summary": "Needs another check", "replacesEvidence": NSNull(),
            "evidence": [
                "kind": "host_check", "source": ["kind": "execution_evidence", "evidence_id": "evidence-1"],
                "resource_uri": "artifact://report", "version": "v1", "check_kind": "postcondition", "predicate": "report is complete"
            ]
        ]
    }

    private func decodeSkillUpdate(_ update: [String: Any], protocolVersion: Int = 1) throws -> MacCoreSkillRuntimeUpdateEnvelope {
        try JSONDecoder().decode(MacCoreSkillRuntimeUpdateEnvelope.self, from: JSONSerialization.data(withJSONObject: [
            "protocol_version": protocolVersion, "type": "core_skill_runtime_updated", "session_id": "session",
            "turn_id": "turn", "cursor": 1, "revision": 1, "skill_runtime_revision": 1, "update": update
        ]))
    }
}
