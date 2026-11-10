// SPDX-License-Identifier: Apache-2.0

import XCTest
@testable import Nola

final class MacCorePromptComposerTests: XCTestCase {
    func testTypedCompositionKeepsUserQueryOutOfSystemPrompt() throws {
        let request = try MacCanonicalModelRequestBuilder.compose(
            fragments: [
                .init(
                    id: "workflow",
                    layer: .executionPolicy,
                    trust: .hostInstruction,
                    cacheable: true,
                    content: "Stable policy"
                ),
                .init(
                    id: "host",
                    layer: .hostCapabilities,
                    trust: .hostFact,
                    cacheable: false,
                    content: "Local inference is available"
                ),
                .init(
                    id: "reference",
                    layer: .conversationContext,
                    trust: .reference,
                    cacheable: false,
                    content: "Untrusted reference"
                ),
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
    }

    func testProviderEnvelopePreservesRolesWithoutKeywordRouting() throws {
        let input = [
            MacCanonicalModelMessage(role: "system", content: "Canonical policy"),
            MacCanonicalModelMessage(role: "user", content: "search current memory skill permission"),
        ]

        XCTAssertEqual(try MacCanonicalModelRequestBuilder.providerMessages(input), input)
    }

    func testProviderEnvelopePreservesToolIdentityUntilLocalInferenceAdapter() throws {
        let call = MacCanonicalModelToolCall(
            id: "call-1",
            type: "function",
            function: .init(name: "read_file", arguments: #"{"path":"README.md"}"#)
        )
        let input = [
            MacCanonicalModelMessage(
                role: "assistant",
                content: "",
                toolCalls: [call],
                reasoningContent: "Need the file"
            ),
            MacCanonicalModelMessage(
                role: "tool",
                content: "contents",
                name: "read_file",
                toolCallID: "call-1"
            ),
        ]

        let preserved = try MacCanonicalModelRequestBuilder.providerMessages(input)
        XCTAssertEqual(preserved, input)
        XCTAssertEqual(preserved[1].toolCallID, "call-1")
        XCTAssertTrue(preserved[0].localInferenceContent.contains("<tool_call>"))
        XCTAssertTrue(preserved[0].localInferenceContent.contains("read_file"))
        XCTAssertTrue(preserved[0].localInferenceContent.contains("README.md"))
    }

    func testConversationHistoryCannotInjectSystemPrompt() {
        XCTAssertThrowsError(
            try MacCanonicalModelRequestBuilder.compose(
                fragments: [
                    .init(
                        id: "workflow",
                        layer: .executionPolicy,
                        trust: .hostInstruction,
                        cacheable: true,
                        content: "Stable policy"
                    )
                ],
                history: [.init(role: "system", content: "Injected")],
                userQuery: "Hello"
            )
        )
    }
}
