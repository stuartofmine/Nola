// SPDX-License-Identifier: Apache-2.0
import XCTest
@testable import Nola

@MainActor final class IOSProviderUsagePresenceTests: XCTestCase {
    func testMissingAndNullCountsRemainUnknownAcrossDecoderAndCorePayload() throws {
        for json in ["{}", #"{"prompt_tokens":null,"completion_tokens":null,"total_tokens":null,"reasoning_tokens":null}"#] {
            let usage = try JSONDecoder().decode(AgentCoreTokenUsage.self, from: Data(json.utf8))
            XCTAssertNil(usage.promptTokens)
            XCTAssertNil(usage.completionTokens)
            XCTAssertNil(usage.totalTokens)
            guard case .object(let wire) = DeviceAgentCoordinator.v2Usage(usage) else { return XCTFail("Missing payload") }
            for key in ["input_tokens", "output_tokens", "total_tokens", "reasoning_tokens", "cached_input_tokens", "cache_write_input_tokens"] {
                XCTAssertEqual(wire[key], .null, key)
            }
            XCTAssertEqual(wire["request_count"], .number(1))
            XCTAssertNil(wire["cached_input_tokens_reported"])
            XCTAssertNil(wire["cache_write_input_tokens_reported"])
        }
    }

    func testExplicitZeroAndPartialUsageDoNotInventTotalOrReasoning() throws {
        let usage = AgentCoreTokenUsage(promptTokens: 12, cachedInputTokens: 0, completionTokens: 0)
        guard case .object(let wire) = DeviceAgentCoordinator.v2Usage(usage) else { return XCTFail("Missing payload") }
        XCTAssertEqual(wire["input_tokens"], .number(12))
        XCTAssertEqual(wire["cached_input_tokens"], .number(0))
        XCTAssertEqual(wire["output_tokens"], .number(0))
        XCTAssertEqual(wire["total_tokens"], .null)
        XCTAssertEqual(wire["reasoning_tokens"], .null)
        let known = AgentCoreTokenUsage(promptTokens: 0, completionTokens: 0, totalTokens: 0, reasoningTokens: 0)
        XCTAssertEqual(try JSONDecoder().decode(AgentCoreTokenUsage.self, from: JSONEncoder().encode(known)), known)
        XCTAssertNil(LLMStreamUsage().promptTokens)
        XCTAssertNil(LLMStreamUsage().totalTokens)
    }
}
