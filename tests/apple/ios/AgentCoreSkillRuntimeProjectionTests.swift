import XCTest
@testable import Nola

final class AgentCoreSkillRuntimeProjectionTests: XCTestCase {
    func testSharedCapabilityActivationProjectionContract() throws {
        struct Fixture: Decodable {
            let iosProjection: AgentCoreJSONValue

            enum CodingKeys: String, CodingKey {
                case iosProjection = "ios_projection"
            }
        }

        let fixtureURL = try XCTUnwrap(
            Bundle(for: Self.self).url(
                forResource: "capability_activation_v1",
                withExtension: "json"
            )
        )
        let data = try Data(contentsOf: fixtureURL)
        let fixture = try JSONDecoder().decode(Fixture.self, from: data)
        let activation = try XCTUnwrap(
            AgentCoreCapabilityActivationEnvelope.decode(from: fixture.iosProjection)
        )

        XCTAssertEqual(activation.sessionID, "conversation-a")
        XCTAssertEqual(activation.turnID, "turn-a")
        XCTAssertEqual(activation.cursor, 7)
        XCTAssertEqual(activation.revision, 11)
        XCTAssertEqual(activation.source, "tool_search")
        XCTAssertEqual(activation.capabilityIDs, ["web_search", "web_fetch"])
        XCTAssertEqual(activation.hostToolEvent.id, "core-capability:turn-a:tool_search")
    }

    func testTypedSkillEventsPreserveIdentityAndFreezeTerminalRun() throws {
        var reducer = AgentCoreSkillRuntimeProjectionReducer()
        let events = try [
            Self.started(cursor: 1, eventRevision: 1, runtimeRevision: 1),
            Self.usage(cursor: 2, eventRevision: 2, runtimeRevision: 1),
            Self.completed(cursor: 3, eventRevision: 3, runtimeRevision: 2),
        ].map(Self.decode)

        for event in events {
            XCTAssertEqual(reducer.apply(event, expectedSessionID: "session-1", expectedTurnID: "turn-1"), .applied)
        }
        XCTAssertEqual(reducer.apply(events[2], expectedSessionID: "session-1", expectedTurnID: "turn-1"), .duplicate)
        XCTAssertEqual(
            reducer.apply(try Self.decode(Self.resumed(cursor: 4, eventRevision: 4, runtimeRevision: 3)), expectedSessionID: "session-1", expectedTurnID: "turn-1"),
            .rejected
        )
        XCTAssertEqual(reducer.projection.order, ["run-1"])
        XCTAssertEqual(reducer.projection.orderedRuns.first?.status, .completed)
        XCTAssertEqual(reducer.projection.orderedRuns.first?.resultRef, "artifact://report")
    }

    func testRejectsWrongOwnerTurnAndNonMonotonicUsage() throws {
        var reducer = AgentCoreSkillRuntimeProjectionReducer()
        let started = try Self.decode(Self.started(cursor: 1, eventRevision: 1, runtimeRevision: 1))
        XCTAssertEqual(reducer.apply(started, expectedSessionID: "other", expectedTurnID: "turn-1"), .rejected)
        XCTAssertEqual(reducer.apply(started, expectedSessionID: "session-1", expectedTurnID: "other"), .rejected)
        XCTAssertEqual(reducer.apply(started, expectedSessionID: "session-1", expectedTurnID: "turn-1"), .applied)
        let regressed = try Self.decode(Self.usage(cursor: 2, eventRevision: 2, runtimeRevision: 2, turns: 0, calls: 0, elapsed: 0))
        XCTAssertEqual(reducer.apply(regressed, expectedSessionID: "session-1", expectedTurnID: "turn-1"), .applied)
        let valid = try Self.decode(Self.usage(cursor: 3, eventRevision: 3, runtimeRevision: 3, turns: 1, calls: 1, elapsed: 10))
        XCTAssertEqual(reducer.apply(valid, expectedSessionID: "session-1", expectedTurnID: "turn-1"), .applied)
        let backwards = try Self.decode(Self.usage(cursor: 4, eventRevision: 4, runtimeRevision: 4, turns: 0, calls: 1, elapsed: 10))
        XCTAssertEqual(reducer.apply(backwards, expectedSessionID: "session-1", expectedTurnID: "turn-1"), .rejected)
    }

    func testMetadataRoundTripUsesTypedProjectionOnly() throws {
        var reducer = AgentCoreSkillRuntimeProjectionReducer()
        let event = try Self.decode(Self.started(cursor: 1, eventRevision: 1, runtimeRevision: 1))
        XCTAssertEqual(reducer.apply(event, expectedSessionID: "session-1", expectedTurnID: "turn-1"), .applied)
        let metadata = AgentCoreSkillProjectionMetadata.encode(reducer.projection, into: ["body": "search memory current"])
        let restored = try XCTUnwrap(AgentCoreSkillProjectionMetadata.decode(from: metadata))
        XCTAssertEqual(restored, reducer.projection)
        XCTAssertNil(AgentCoreSkillProjectionMetadata.decode(from: ["body": "skill completed"] ))
        let toolEvent = try XCTUnwrap(restored.hostToolEvents.first)
        XCTAssertEqual(toolEvent.id, "core-skill:run-1")
        XCTAssertEqual(toolEvent.name, "Documents")
        XCTAssertEqual(restored.mergingHostToolEvents(restored.hostToolEvents).count, 1)
    }

    func testCurrentSkillProgressArtifactsAndEvidenceSurviveProjectionRoundTrip() throws {
        var reducer = AgentCoreSkillRuntimeProjectionReducer()
        XCTAssertEqual(reducer.apply(try Self.decode(Self.started(cursor: 1, eventRevision: 1, runtimeRevision: 1)),
            expectedSessionID: "session-1", expectedTurnID: "turn-1"), .applied)
        let updates = [
            #"{"type":"progress_updated","run_id":"run-1","step_id":"draft","completed":true}"#,
            #"{"type":"artifact_registered","run_id":"run-1","artifact_id":"report","artifact_ref":"artifact://report"}"#,
            #"{"type":"verification_recorded","run_id":"run-1","result":{"verifierId":"check","passed":true,"evidence":{"kind":"host_check","source":{"kind":"tool_call","call_id":"write-1"},"resource_uri":"file:///fixture/report","version":"sha256:original","check_kind":"postcondition","predicate":"content_matches"},"replacesEvidence":null,"summary":"Host receipt"}}"#,
            #"{"type":"resume_point_updated","run_id":"run-1","resume_point":"review"}"#,
        ]
        for (index, update) in updates.enumerated() {
            XCTAssertEqual(reducer.apply(try Self.decode(Self.envelope(update, cursor: index + 2)),
                expectedSessionID: "session-1", expectedTurnID: "turn-1"), .applied)
        }
        let run = try XCTUnwrap(reducer.projection.runs["run-1"])
        XCTAssertEqual(run.completedProgressSteps, ["draft"])
        XCTAssertNil(run.currentProgressStep)
        XCTAssertEqual(run.artifacts["report"], "artifact://report")
        XCTAssertEqual(run.verifications["check"]?.evidence.kind, .hostCheck)
        XCTAssertEqual(run.verifications["check"]?.evidence.source?.callID, "write-1")
        XCTAssertEqual(run.verifications["check"]?.evidence.version, "sha256:original")
        XCTAssertEqual(run.resumePoint, "review")
        let encoded = try JSONEncoder().encode(reducer.projection)
        XCTAssertEqual(try JSONDecoder().decode(AgentCoreSkillRuntimeProjection.self, from: encoded), reducer.projection)

        let selfClaim = #"{"type":"verification_recorded","run_id":"run-1","result":{"verifierId":"check","passed":true,"evidence":{"kind":"model_judgment","sources":[]},"replacesEvidence":null,"summary":"I checked"}}"#
        XCTAssertEqual(reducer.apply(try Self.decode(Self.envelope(selfClaim, cursor: 6)), expectedSessionID: "session-1"), .rejected)
        let changedSource = updates[2].replacingOccurrences(of: "write-1", with: "write-2")
        XCTAssertEqual(reducer.apply(try Self.decode(Self.envelope(changedSource, cursor: 6)), expectedSessionID: "session-1"), .rejected)
        let previousEvidence = String(decoding: try JSONEncoder().encode(try XCTUnwrap(run.verifications["check"]?.evidence)), as: UTF8.self)
        let replacement = changedSource.replacingOccurrences(of: #""replacesEvidence":null"#, with: #""replacesEvidence":\#(previousEvidence)"#)
        XCTAssertEqual(reducer.apply(try Self.decode(Self.envelope(replacement, cursor: 6)), expectedSessionID: "session-1"), .applied)
        XCTAssertEqual(reducer.projection.runs["run-1"]?.verifications["check"]?.evidence.source?.callID, "write-2")
        let missingIdentity = Self.started(cursor: 1, eventRevision: 1, runtimeRevision: 1)
            .replacingOccurrences(of: #""instructions_digest":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","#, with: "")
        XCTAssertThrowsError(try Self.decode(missingIdentity))
    }

    private static func envelope(_ update: String, cursor: Int) -> String {
        #"{"protocol_version":1,"type":"core_skill_runtime_updated","session_id":"session-1","turn_id":"turn-1","cursor":\#(cursor),"revision":\#(cursor),"skill_runtime_revision":\#(cursor),"update":\#(update)}"#
    }

    private static func decode(_ json: String) throws -> AgentCoreSkillRuntimeUpdateEnvelope {
        try JSONDecoder().decode(AgentCoreSkillRuntimeUpdateEnvelope.self, from: Data(json.utf8))
    }

    private static func started(cursor: Int, eventRevision: Int, runtimeRevision: Int) -> String {
        #"{"protocol_version":1,"type":"core_skill_runtime_updated","session_id":"session-1","turn_id":"turn-1","cursor":\#(cursor),"revision":\#(eventRevision),"skill_runtime_revision":\#(runtimeRevision),"update":{"type":"started","run":{"run_id":"run-1","owner":{"session_id":"session-1","parent_turn_id":"turn-1","child_turn_id":null},"descriptor":{"skill_id":"documents","version":"1","display_name":"Documents","instructions_ref":"builtin://documents#sha256=aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","instructions_digest":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","execution_contract":{"schemaVersion":1,"progressSteps":[{"id":"draft","title":"Draft","required":true}],"artifacts":[{"id":"report","mediaType":"text/plain","required":true,"description":"Report"}],"verifiers":[{"id":"check","description":"Check bytes","method":"host_check","required":true}],"resumePoints":["review"]},"required_tools":[],"default_mode":"inline"},"mode":"inline","status":"running","budget":{"max_model_turns":8,"max_tool_calls":32,"max_elapsed_millis":300000},"usage":{"model_turns":0,"tool_calls":0,"elapsed_millis":0},"missing_tools":[],"catalog_revision":1,"result_ref":null,"failure":null,"current_progress_step":"draft","completed_progress_steps":[],"artifacts":{},"verifications":{},"resume_point":null}}}"#
    }

    private static func usage(cursor: Int, eventRevision: Int, runtimeRevision: Int, turns: Int = 1, calls: Int = 1, elapsed: Int = 10) -> String {
        #"{"protocol_version":1,"type":"core_skill_runtime_updated","session_id":"session-1","turn_id":"turn-1","cursor":\#(cursor),"revision":\#(eventRevision),"skill_runtime_revision":\#(runtimeRevision),"update":{"type":"usage_updated","run_id":"run-1","usage":{"model_turns":\#(turns),"tool_calls":\#(calls),"elapsed_millis":\#(elapsed)}}}"#
    }

    private static func completed(cursor: Int, eventRevision: Int, runtimeRevision: Int) -> String {
        #"{"protocol_version":1,"type":"core_skill_runtime_updated","session_id":"session-1","turn_id":"turn-1","cursor":\#(cursor),"revision":\#(eventRevision),"skill_runtime_revision":\#(runtimeRevision),"update":{"type":"completed","run_id":"run-1","result_ref":"artifact://report"}}"#
    }

    private static func resumed(cursor: Int, eventRevision: Int, runtimeRevision: Int) -> String {
        #"{"protocol_version":1,"type":"core_skill_runtime_updated","session_id":"session-1","turn_id":"turn-1","cursor":\#(cursor),"revision":\#(eventRevision),"skill_runtime_revision":\#(runtimeRevision),"update":{"type":"resumed","run_id":"run-1","catalog_revision":2}}"#
    }
}
