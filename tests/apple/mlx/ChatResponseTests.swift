// SPDX-License-Identifier: MIT
import Foundation

@main
struct ChatResponseTests {
    static func main() throws {
        var response = MLXChatResponse(model: "fixture", id: "completion", created: 123)
        let first = try response.tool(name: "one", arguments: "{\"path\":\"你好\"}", callID: "call-1")
        let second = try response.tool(name: "two", arguments: "{}", callID: "call-2")
        precondition(first["index"] as? Int == 0 && second["index"] as? Int == 1)
        for call in [first, second] {
            let chunk = try response.chunk(delta: ["tool_calls": [call]])
            precondition(chunk["id"] as? String == "completion")
            precondition(chunk["created"] as? Int == 123)
            _ = try JSONSerialization.data(withJSONObject: chunk)
        }
        let final = try response.finishStream(reason: .stop, promptTokens: 7, completionTokens: 5, includeUsage: true)
        let choices = final[0]["choices"] as! [[String: Any]]
        precondition(choices[0]["finish_reason"] as? String == "tool_calls")
        precondition((final[1]["choices"] as! [[String: Any]]).isEmpty)
        precondition((final[1]["usage"] as! [String: Int])["total_tokens"] == 12)
        reject { _ = try response.chunk(delta: ["content": "late"]) }
        reject { _ = try response.finishStream(reason: .stop, promptTokens: nil, completionTokens: nil, includeUsage: false) }

        var limited = MLXChatResponse(model: "fixture")
        _ = try limited.tool(name: "one", arguments: "{}")
        let length = try limited.finishStream(reason: .length, promptTokens: nil, completionTokens: nil, includeUsage: false)
        precondition((length[0]["choices"] as! [[String: Any]])[0]["finish_reason"] as? String == "length")
        var interrupted = MLXChatResponse(model: "fixture")
        reject { _ = try interrupted.finishStream(reason: nil, promptTokens: nil, completionTokens: nil, includeUsage: false) }
        reject { _ = try interrupted.chunk(delta: ["content": "late"]) }
        var cancelled = MLXChatResponse(model: "fixture")
        reject { _ = try cancelled.finishStream(reason: .cancelled, promptTokens: nil, completionTokens: nil, includeUsage: false) }
        reject { _ = try cancelled.tool(name: "late", arguments: "{}") }

        var complete = MLXChatResponse(model: "fixture")
        let call = try complete.tool(name: "one", arguments: "{}", callID: "same-call")
        let json = try complete.completion(text: "", calls: [call], reason: .stop, promptTokens: 1, completionTokens: 2)
        let message = (json["choices"] as! [[String: Any]])[0]["message"] as! [String: Any]
        let calls = message["tool_calls"] as! [[String: Any]]
        precondition(calls[0]["index"] == nil && calls[0]["id"] as? String == "same-call")
        _ = try JSONSerialization.data(withJSONObject: json)
        print("MLX response contracts passed: distinct tools, stable envelope, usage, finish reasons, interruption and non-stream parity")
    }

    static func reject(_ body: () throws -> Void) {
        do { try body(); fatalError("Expected response state rejection") }
        catch { }
    }
}
