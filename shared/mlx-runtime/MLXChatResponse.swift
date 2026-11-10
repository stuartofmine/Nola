// SPDX-License-Identifier: MIT
// Wire response ownership is separate from inference and socket ownership.
import Foundation

struct MLXChatResponse {
    enum Finish { case stop, length, cancelled }
    enum Failure: Error { case alreadyFinished, cancelled, missingCompletionInfo }

    let model: String
    let id: String
    let created: Int
    private(set) var toolCount = 0
    private var finished = false

    init(model: String, id: String = "chatcmpl-local-\(UUID().uuidString)", created: Int = Int(Date().timeIntervalSince1970)) {
        self.model = model
        self.id = id
        self.created = created
    }

    func chunk(delta: [String: Any], finishReason: Any = NSNull()) throws -> [String: Any] {
        guard !finished else { throw Failure.alreadyFinished }
        return envelope(object: "chat.completion.chunk", choices: [
            ["index": 0, "delta": delta, "finish_reason": finishReason]
        ])
    }

    mutating func tool(name: String, arguments: String, callID: String = "call_\(UUID().uuidString)") throws -> [String: Any] {
        guard !finished else { throw Failure.alreadyFinished }
        let index = toolCount
        toolCount += 1
        return ["index": index, "id": callID, "type": "function",
                "function": ["name": name, "arguments": arguments]]
    }

    mutating func finishStream(reason: Finish?, promptTokens: Int?, completionTokens: Int?, includeUsage: Bool) throws -> [[String: Any]] {
        let finish = try finishReason(reason)
        var result = [try chunk(delta: [:], finishReason: finish)]
        if includeUsage, let promptTokens, let completionTokens {
            var usage = envelope(object: "chat.completion.chunk", choices: [])
            usage["usage"] = usageObject(promptTokens: promptTokens, completionTokens: completionTokens)
            result.append(usage)
        }
        finished = true
        return result
    }

    mutating func completion(text: String, calls: [[String: Any]], reason: Finish?, promptTokens: Int?, completionTokens: Int?) throws -> [String: Any] {
        let finish = try finishReason(reason)
        var message: [String: Any] = ["role": "assistant", "content": text]
        if !calls.isEmpty {
            message["tool_calls"] = calls.map { call in
                var result = call
                result.removeValue(forKey: "index")
                return result
            }
        }
        var result = envelope(object: "chat.completion", choices: [
            ["index": 0, "message": message, "finish_reason": finish]
        ])
        if let promptTokens, let completionTokens {
            result["usage"] = usageObject(promptTokens: promptTokens, completionTokens: completionTokens)
        }
        finished = true
        return result
    }

    private mutating func finishReason(_ reason: Finish?) throws -> String {
        guard !finished else { throw Failure.alreadyFinished }
        guard let reason else {
            finished = true
            throw Failure.missingCompletionInfo
        }
        switch reason {
        case .cancelled:
            finished = true
            throw Failure.cancelled
        case .length: return "length"
        case .stop: return toolCount == 0 ? "stop" : "tool_calls"
        }
    }

    private func envelope(object: String, choices: [[String: Any]]) -> [String: Any] {
        ["id": id, "object": object, "created": created, "model": model, "choices": choices]
    }

    private func usageObject(promptTokens: Int, completionTokens: Int) -> [String: Int] {
        ["prompt_tokens": promptTokens, "completion_tokens": completionTokens,
         "total_tokens": promptTokens + completionTokens]
    }
}
