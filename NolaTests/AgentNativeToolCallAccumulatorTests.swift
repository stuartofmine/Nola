import XCTest
@testable import Nola

final class AgentNativeToolCallAccumulatorTests: XCTestCase {
    func testParallelDeltasBecomeOrderedCompleteCalls() throws {
        var accumulator = AgentNativeToolCallAccumulator()
        accumulator.append(
            callId: "call-a",
            capabilityId: "read_file",
            argumentsDelta: #"{"path":""#
        )
        accumulator.append(
            callId: "call-b",
            capabilityId: "web_search",
            argumentsDelta: #"{"query":"current"#
        )
        accumulator.append(
            callId: "call-a",
            capabilityId: "read_file",
            argumentsDelta: #"/tmp/a"}"#
        )
        accumulator.append(
            callId: "call-b",
            capabilityId: "web_search",
            argumentsDelta: #" status"}"#
        )

        let calls = try accumulator.completedCalls()

        XCTAssertEqual(calls.map(\.id), ["call-a", "call-b"])
        XCTAssertEqual(calls.map(\.name), ["read_file", "web_search"])
        XCTAssertEqual(
            calls[0].arguments,
            .object(["path": .string("/tmp/a")])
        )
        XCTAssertEqual(
            calls[1].arguments,
            .object(["query": .string("current status")])
        )
    }

    func testInvalidFinalArgumentsAreRejectedInsteadOfSilentlyExecuting() {
        var accumulator = AgentNativeToolCallAccumulator()
        accumulator.append(
            callId: "call-invalid",
            capabilityId: "read_file",
            argumentsDelta: #"{"path":"unterminated""#
        )

        XCTAssertThrowsError(try accumulator.completedCalls())
    }

    func testProviderIndexKeepsLateCallIdentityAndArgumentsInOneLogicalCall() throws {
        var accumulator = AgentNativeToolCallAccumulator()

        XCTAssertNil(accumulator.append(
            index: 3,
            callId: "",
            capabilityId: "",
            argumentsDelta: "{\"path\":\""
        ))
        let activated = accumulator.append(
            index: 3,
            callId: "provider-id-arrived-late",
            capabilityId: "read_file",
            argumentsDelta: "notes.md\"}"
        )

        XCTAssertEqual(activated?.callId, "tool-call-3")
        XCTAssertEqual(activated?.capabilityId, "read_file")
        XCTAssertEqual(activated?.argumentsDelta, #"{"path":"notes.md"}"#)
        let calls = try accumulator.completedCalls()
        XCTAssertEqual(calls.count, 1)
        XCTAssertEqual(calls[0].id, "tool-call-3")
        XCTAssertEqual(calls[0].name, "read_file")
        XCTAssertEqual(calls[0].arguments, .object(["path": .string("notes.md")]))
    }

    func testFallbackIdentityIsScopedToTheModelIteration() throws {
        var first = AgentNativeToolCallAccumulator(fallbackCallIdPrefix: "turn-a-1-tool-call")
        var second = AgentNativeToolCallAccumulator(fallbackCallIdPrefix: "turn-a-2-tool-call")

        _ = first.append(index: 0, callId: "", capabilityId: "read_file", argumentsDelta: "{}")
        _ = second.append(index: 0, callId: "", capabilityId: "read_file", argumentsDelta: "{}")

        XCTAssertEqual(try first.completedCalls().first?.id, "turn-a-1-tool-call-0")
        XCTAssertEqual(try second.completedCalls().first?.id, "turn-a-2-tool-call-0")
    }

    @MainActor
    func testMemoryWorkerExecutorRequiresStructuredSubmissionTool() async throws {
        let expected: AgentCoreJSONValue = .object([
            "candidates": .array([]),
        ])
        let client = CapturingAgentModelClient(completion: .init(
            content: "",
            toolCalls: [AgentCoreToolCall(
                id: "memory-call",
                name: AgentMemoryWorkerModelExecutor.submissionToolName,
                arguments: expected
            )],
            finishReason: "tool_calls"
        ))
        let plan: AgentCoreJSONValue = .object([
            "job_id": .string("job-1"),
            "system_instructions": .string("Extract durable memory."),
            "model_input": .object(["transcript": .array([])]),
            "response_schema": .object([
                "type": .string("object"),
                "properties": .object([:]),
            ]),
        ])

        let output = try await AgentMemoryWorkerModelExecutor.execute(
            plan: plan,
            client: client
        )

        XCTAssertEqual(output, expected)
        XCTAssertEqual(
            client.request?.requiredToolName,
            AgentMemoryWorkerModelExecutor.submissionToolName
        )
        XCTAssertEqual(client.request?.tools.count, 1)
        XCTAssertEqual(client.request?.invocationPlan.nativeToolCalls, true)
    }

    @MainActor
    func testStructuredOutputExecutorRejectsDuplicateRequiredCalls() async {
        let arguments: AgentCoreJSONValue = .object(["value": .string("first")])
        let toolName = "submit_fixture"
        let client = CapturingAgentModelClient(completion: .init(
            content: "",
            toolCalls: [
                AgentCoreToolCall(id: "call-1", name: toolName, arguments: arguments),
                AgentCoreToolCall(id: "call-2", name: toolName, arguments: arguments),
            ],
            finishReason: "tool_calls"
        ))

        do {
            _ = try await AgentStructuredOutputExecutor.execute(
                sessionId: "fixture-session",
                turnId: "fixture-turn",
                systemInstructions: "Return structured output.",
                modelInput: .object(["input": .string("fixture")]),
                responseSchema: .object(["type": .string("object")]),
                submissionToolName: toolName,
                submissionToolDescription: "Submit the fixture.",
                client: client
            )
            XCTFail("duplicate required tool calls must be rejected")
        } catch {
            XCTAssertEqual(client.request?.requiredToolName, toolName)
        }
    }
}

@MainActor
private final class CapturingAgentModelClient: AgentModelClient {
    let completion: AgentModelCompletion
    var request: AgentModelRequest?

    init(completion: AgentModelCompletion) {
        self.completion = completion
    }

    func generateAgentTurn(
        request: AgentModelRequest
    ) -> AsyncThrowingStream<AgentModelEvent, Error> {
        self.request = request
        return AsyncThrowingStream { continuation in
            continuation.yield(.completed(completion))
            continuation.finish()
        }
    }
}
