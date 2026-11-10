// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Implements AgentModelsTests logic in the project module.
// - Interfaces:
//   - AgentModelsTests
// - APIs:
//   - none
//
import XCTest
@testable import Nola

final class AgentModelsTests: XCTestCase {

    func testLLMActionResponseEncodeDecode() throws {
        let action1 = Action(type: LLMActionType.notify,
                     priority: LLMActionPriority.medium,
                             dedupeKey: "url:example.com:post-123:2025-11-16",
                             scheduledAt: ISO8601DateFormatter().date(from: "2025-11-16T08:00:00Z"),
                             contentSummary: "摘要示例",
                             reason: "包含工作相关建议",
                             confidence: 0.85)
        let action2 = Action(type: LLMActionType.store,
                     priority: LLMActionPriority.low,
                             dedupeKey: "store:url:example.com:post-123",
                             contentSummary: "保存文章 URL",
                             reason: "用户可能需要稍后查阅",
                             confidence: 0.6)

        let resp = LLMActionResponse(actions: [action1, action2], summary: "建议发送一条中午提醒并将文章存档。")

        let encoder = JSONEncoder.nolaEncoder()
        let data = try encoder.encode(resp)

        let decoder = JSONDecoder.nolaDecoder()
        let decoded = try decoder.decode(LLMActionResponse.self, from: data)

        XCTAssertEqual(decoded.actions.count, 2)
        XCTAssertEqual(decoded.summary, resp.summary)
        XCTAssertEqual(decoded.actions[0].dedupeKey, action1.dedupeKey)
    }
}
