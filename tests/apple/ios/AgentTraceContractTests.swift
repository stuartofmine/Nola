import XCTest
@testable import Nola

final class AgentTraceContractTests: XCTestCase {
    func testCompactionOwnsStageUntilCompletionAndModelProgressIsNotATool() {
        var reducer = AgentTraceReducer()
        func event(_ sequence: UInt64, _ kind: AgentTraceKind, _ payload: AgentTracePayload) -> AgentTraceEvent {
            AgentTraceEvent(traceId: "trace", runId: "run", conversationId: "conversation", turnId: "turn", sequence: sequence, kind: kind, spanId: "model-effect", payload: payload)
        }
        reducer.apply(event(1, .phaseChanged, .init(phase: "compacting")))
        reducer.apply(event(2, .toolProgress, .init(summary: "model_attempt_started")))
        XCTAssertEqual(reducer.projection.phase, "compacting")
        reducer.apply(event(3, .toolProgress, .init(summary: "model_response_provenance")))
        XCTAssertTrue(reducer.projection.activities.isEmpty)
        reducer.apply(event(4, .contextCompacted, .init(phase: "compacted")))
        XCTAssertEqual(reducer.projection.phase, "processing")
        reducer.apply(event(5, .toolProgress, .init(summary: "model_retry")))
        XCTAssertEqual(reducer.projection.phase, "model_retry")
        reducer.apply(event(6, .toolProgress, .init(summary: "model_attempt_started")))
        XCTAssertEqual(reducer.projection.phase, "processing")
        XCTAssertTrue(reducer.projection.activities.isEmpty)
        reducer.apply(event(7, .phaseChanged, .init(phase: "compacting")))
        reducer.apply(event(8, .runCancelled, .init(reason: "cancelled")))
        XCTAssertNil(reducer.projection.phase)
    }

    func testTraceLifecycleSettlesCanonicalPartToolPresentation() {
        let partEvent = HostAgentToolEvent(
            id: "call-1",
            name: "read_file",
            argsPreview: #"{"path":"Doc/README.md"}"#,
            status: .running,
            display: "Read file"
        )
        let completedTrace = HostAgentToolEvent(
            id: "call-1",
            name: "read_file",
            status: .succeeded,
            text: "read 12 lines"
        )

        let reconciled = partEvent.reconcilingLifecycle(from: completedTrace)

        XCTAssertEqual(reconciled.status, .succeeded)
        XCTAssertEqual(reconciled.argsPreview, partEvent.argsPreview)
        XCTAssertEqual(reconciled.display, "Read file")
        XCTAssertEqual(reconciled.text, "read 12 lines")
    }

    func testSharedCoreLongTaskFixtureReplaysIdenticallyOnIOS() throws {
        let fixtureURL = try XCTUnwrap(
            Bundle(for: Self.self).url(
                forResource: "long_task_trace_v1",
                withExtension: "json"
            )
        )
        let data = try Data(contentsOf: fixtureURL)
        let events = try JSONDecoder().decode([AgentTraceEvent].self, from: data)
        var reducer = AgentTraceReducer()
        var results: [AgentTraceApplyResult] = []

        for event in events {
            results.append(reducer.apply(event))
        }

        XCTAssertEqual(results.filter { $0 == .duplicate }.count, 1)
        XCTAssertFalse(results.contains {
            if case .gap = $0 { return true }
            if case .outOfOrder = $0 { return true }
            return false
        })
        XCTAssertEqual(reducer.projection.traceId, "trace_contract_long_task")
        XCTAssertEqual(reducer.projection.phase, "planning")
        XCTAssertEqual(reducer.projection.activities.map(\.toolName), ["web_search", "read_file"])
        XCTAssertEqual(reducer.projection.activities.map(\.status), [.succeeded, .succeeded])
        XCTAssertEqual(reducer.projection.activities.last?.resources.first?.uri, "Doc/软件架构设计/总体设计/SOFTWARE_DESIGN.md")
        XCTAssertEqual(reducer.projection.usage?.usedTokens, 8_192)
        XCTAssertEqual(reducer.projection.usage?.contextWindowTokens, 131_072)
        XCTAssertEqual(reducer.projection.status, .succeeded)
        XCTAssertEqual(reducer.projection.lastSequence, 9)
    }

    func testDecodesAuthoritativeCoreTraceFixture() throws {
        let object: [String: Any] = [
            "type": "phase",
            "trace": [
                "schema_version": 1,
                "trace_id": "trace_fixture",
                "run_id": "run_fixture",
                "conversation_id": "conversation_fixture",
                "message_id": "assistant_fixture",
                "turn_id": "turn_fixture",
                "event_id": "event_fixture",
                "sequence": 7,
                "emitted_at": "2026-07-19T08:00:00.000Z",
                "kind": "phase.changed",
                "parent_span_id": "turn_fixture",
                "status": "running",
                "visibility": "summary",
                "payload": ["phase": "processing"],
            ],
        ]

        let trace = try XCTUnwrap(AgentTraceAdapter.decodeNestedTrace(from: object))
        XCTAssertEqual(trace.schemaVersion, 1)
        XCTAssertEqual(trace.sequence, 7)
        XCTAssertEqual(trace.kind, .phaseChanged)
        XCTAssertEqual(trace.payload.phase, "processing")
    }

    func testLongTaskProjectionPreservesOrderedIntermediateState() {
        var context = AgentTraceContext(conversationId: "conversation", messageId: "assistant")
        var reducer = AgentTraceReducer()
        let callId = "call_search"
        let events = [
            context.makeEvent(kind: .runStarted, payload: .init(phase: "planning")),
            context.makeEvent(
                kind: .toolStarted,
                spanId: callId,
                payload: .init(
                    summary: "搜索最新资料",
                    toolName: "web_search",
                    callId: callId,
                    arguments: .object(["query": .string("latest statement")])
                )
            ),
            context.makeEvent(
                kind: .toolPermissionRequested,
                status: .waiting,
                spanId: callId,
                payload: .init(toolName: "web_search", callId: callId, permissionRequestId: "permission")
            ),
            context.makeEvent(
                kind: .toolPermissionResolved,
                spanId: callId,
                payload: .init(toolName: "web_search", callId: callId, permissionDecision: "allow_once")
            ),
            context.makeEvent(
                kind: .toolCompleted,
                status: .succeeded,
                spanId: callId,
                payload: .init(
                    summary: "找到 20 个结果",
                    toolName: "web_search",
                    callId: callId,
                    result: .object(["count": .number(20)])
                )
            ),
            context.makeEvent(
                kind: .usageSnapshot,
                visibility: .detail,
                payload: .init(usage: .init(
                    usedTokens: 4_096,
                    contextWindowTokens: 128_000,
                    promptTokenLimit: 120_000
                ))
            ),
            context.makeEvent(kind: .runCompleted, status: .succeeded),
        ]

        for event in events {
            XCTAssertEqual(reducer.apply(event), .applied)
        }

        XCTAssertEqual(reducer.projection.lastSequence, 7)
        XCTAssertEqual(reducer.projection.activities.count, 1)
        XCTAssertEqual(reducer.projection.activities.first?.status, .succeeded)
        XCTAssertEqual(reducer.projection.activities.first?.arguments, .object(["query": .string("latest statement")]))
        XCTAssertEqual(reducer.projection.usage?.usedTokens, 4_096)
        XCTAssertEqual(reducer.projection.status, .succeeded)
    }

    func testPermissionResumePreservesControlPhaseAndResolvesExactActivity() {
        var context = AgentTraceContext(conversationId: "conversation")
        var reducer = AgentTraceReducer()
        let initial = [
            context.makeEvent(kind: .phaseChanged, payload: .init(phase: "processing")),
            context.makeEvent(
                kind: .toolPermissionRequested,
                status: .waiting,
                spanId: "call",
                payload: .init(toolName: "read_file", callId: "call", permissionRequestId: "request")
            ),
        ]
        initial.forEach { XCTAssertEqual(reducer.apply($0), .applied) }

        XCTAssertEqual(reducer.apply(AgentTraceAdapter.permissionResolved(
            requestId: "request",
            toolName: "read_file",
            allowed: true,
            context: &context
        )), .applied)
        XCTAssertEqual(reducer.apply(context.makeEvent(
            kind: .phaseChanged,
            payload: .init(phase: "processing")
        )), .applied)

        XCTAssertEqual(reducer.projection.phase, "processing")
        XCTAssertEqual(reducer.projection.lastSequence, 4)
        XCTAssertEqual(reducer.projection.activities.first?.status, .running)
    }

    func testSequenceAndEventIdAreStableAndIdempotent() {
        var context = AgentTraceContext(conversationId: "conversation")
        var reducer = AgentTraceReducer()
        let first = context.makeEvent(kind: .phaseChanged, payload: .init(phase: "processing"))
        XCTAssertEqual(reducer.apply(first), .applied)
        XCTAssertEqual(reducer.apply(first), .duplicate)

        var gap = context.makeEvent(kind: .phaseChanged, payload: .init(phase: "waiting"))
        gap.sequence = 3
        XCTAssertEqual(reducer.apply(gap), .gap(expected: 2, received: 3))

        var outOfOrder = first
        outOfOrder.eventId = "new-id-old-sequence"
        XCTAssertEqual(reducer.apply(outOfOrder), .outOfOrder(last: 1, received: 1))
        XCTAssertEqual(reducer.projection.phase, "processing")
    }

    func testRemovedBodyTraceKindsCannotEnterControlProjection() throws {
        var mutable = AgentTraceContext(conversationId: "conversation")
        let control = mutable.makeEvent(kind: .phaseChanged, payload: .init(phase: "processing"))
        var object = try XCTUnwrap(JSONSerialization.jsonObject(with: JSONEncoder().encode(control)) as? [String: Any])
        for kind in ["assistant.delta", "assistant.reset", "thinking.delta"] {
            object["kind"] = kind
            object["payload"] = ["delta": "正文只能由 canonical journal 提供"]
            XCTAssertNil(AgentTraceAdapter.decodeNestedTrace(from: ["trace": object]))
        }
        let encoded = try XCTUnwrap(JSONSerialization.jsonObject(with: JSONEncoder().encode(control)) as? [String: Any])
        XCTAssertNil((encoded["payload"] as? [String: Any])?["delta"])
    }
}
