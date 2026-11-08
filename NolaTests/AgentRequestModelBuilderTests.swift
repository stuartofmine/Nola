import XCTest
@testable import Nola

final class AgentRequestModelBuilderTests: XCTestCase {
    func testMainTurnPreservesCoreSystemWithoutHostRecomposition() throws {
        let builder = AgentRequestModelBuilder { _ in
            XCTFail("Main Agent turns must not be recomposed by the iOS Host")
            throw AgentCorePromptComposerError.invalidResponse
        }
        let query = "search is an ambiguous word, do not force a tool"
        let request = AgentModelRequest(
            sessionId: "session-1",
            turnId: "turn-1",
            iteration: 2,
            messages: [
                AgentCoreMessage(role: "system", content: "CORE REFERENCE"),
                AgentCoreMessage(role: "user", content: query),
            ],
            tools: [
                .object([
                    "type": .string("function"),
                    "function": .object(["name": .string("device_read_file")]),
                ]),
            ],
            reservedOutputTokens: 4_096,
            stablePrefixSHA256: "core-digest"
        )

        let prompt = try builder.build(request)

        XCTAssertEqual(prompt.systemPrompt, "CORE REFERENCE")
        XCTAssertEqual(prompt.allowedToolNames, ["device_read_file"])
        XCTAssertTrue(prompt.prompt.contains(query))
        XCTAssertEqual(prompt.stablePrefixSHA256, "core-digest")
    }

    func testBuilderDoesNotInferToolsFromNaturalLanguage() throws {
        let builder = AgentRequestModelBuilder { _ in
            XCTFail("Natural language must not trigger prompt recomposition")
            throw AgentCorePromptComposerError.invalidResponse
        }
        let request = AgentModelRequest(
            sessionId: "session-1",
            turnId: "turn-1",
            iteration: 1,
            messages: [AgentCoreMessage(role: "user", content: "搜索当前文件")],
            tools: [],
            reservedOutputTokens: 1_024
        )

        let prompt = try builder.build(request)

        XCTAssertTrue(prompt.allowedToolNames.isEmpty)
        XCTAssertTrue(prompt.prompt.contains("搜索当前文件"))
        XCTAssertFalse(prompt.systemPrompt.contains("device_read_file"))
    }

    func testAuxiliaryRequestComposesOnlyExplicitPolicyFragments() throws {
        var captured: [AgentCorePromptFragment] = []
        let builder = AgentRequestModelBuilder { fragments in
            captured = fragments
            return AgentCorePromptComposition(
                stablePrefix: "",
                dynamicSuffix: "AUXILIARY POLICY",
                rendered: "AUXILIARY POLICY",
                fragmentIds: fragments.map(\.id),
                stablePrefixSha256: "auxiliary-digest",
                stableFragmentSha256: nil
            )
        }
        let policy = AgentCorePromptFragment(
            id: "core-context-compaction",
            layer: .executionPolicy,
            trust: .hostInstruction,
            cacheable: false,
            content: "Summarize faithfully."
        )
        let request = AgentModelRequest(
            sessionId: "session-1",
            turnId: "compaction",
            iteration: 1,
            messages: [AgentCoreMessage(role: "user", content: "history")],
            tools: [],
            reservedOutputTokens: 1_024,
            policyFragments: [policy]
        )

        let prompt = try builder.build(request)

        XCTAssertEqual(captured.map(\.id), ["core-context-compaction"])
        XCTAssertEqual(prompt.systemPrompt, "AUXILIARY POLICY")
        XCTAssertNil(prompt.stablePrefixSHA256)
    }
}
