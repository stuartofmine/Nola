// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Defines canonical conversation knowledge and recall projections.

import Foundation

nonisolated struct MacConversationKnowledge: Codable, Sendable, Equatable {
    var keyPointsMarkdown: String
    var wikiMarkdown: String
    var updatedAt: Date?

    static let empty = MacConversationKnowledge(
        keyPointsMarkdown: "",
        wikiMarkdown: "",
        updatedAt: nil
    )

    var isEmpty: Bool {
        keyPointsMarkdown.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
            && wikiMarkdown.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
    }
}
nonisolated struct MacContextArchiveRecord: Codable, Sendable, Equatable {
    let conversationID: String
    let archiveID: String
    let archiveDigest: String
    let payloadDigest: String
    let summary: String
    let messageIDs: [String]
    let createdAt: Date
}

nonisolated struct MacConversationRecallResult: Identifiable, Sendable, Equatable {
    enum Source: String, Sendable {
        case originalMessage
        case compactionSummary
    }

    let id: String
    let source: Source
    let role: MacChatMessage.Role?
    let content: String
    let snippet: String
    let messageID: String
    let partID: String
    let messageOrdinal: Int64
    let startCharacter: Int64
}

nonisolated struct MacCanonicalConversationTextMatch: Decodable, Sendable {
    let messageID: String
    let partID: String
    let role: String
    let messageOrdinal: Int64
    let snippet: String
    let startCharacter: Int64

    private enum CodingKeys: String, CodingKey {
        case role, snippet
        case messageID = "message_id"
        case partID = "part_id"
        case messageOrdinal = "message_ordinal"
        case startCharacter = "start_character"
    }
}

nonisolated enum MacConversationRecallBridge {
    static func responseData(
        requestID: String,
        conversationID: String,
        arguments: [String: Any],
        store: MacApplicationPort? = .shared
    ) -> Data {
        guard let store else {
            return response(
                requestID: requestID,
                output: ["error": "conversation is unavailable"],
                display: "Conversation recall is unavailable",
                isError: true
            )
        }
        do {
            let input = try canonicalQuery(conversationID: conversationID, arguments: arguments)
            let output = try store.queryCanonicalHistory(input)
            return response(
                requestID: requestID,
                output: output,
                display: "Loaded canonical conversation originals",
                isError: false
            )
        } catch {
            return response(
                requestID: requestID,
                output: ["error": String(describing: error)],
                display: "Conversation history could not be loaded",
                isError: true
            )
        }
    }

    // Translate the current tool schema into the shared Application query. Core
    // owns source validation and bounded reads; the bridge must not narrow scope
    // silently or substitute a search for an exact original-text read.
    static func canonicalQuery(conversationID: String, arguments: [String: Any]) throws -> [String: Any] {
        let reading = arguments["message_id"] != nil || arguments["part_id"] != nil
        let allowed: Set<String> = reading
            ? ["conversation_id", "message_id", "part_id", "expected_conversation_revision", "offset_bytes", "limit_bytes"]
            : ["query", "scope", "conversation_ids", "after_conversation_id", "limit"]
        guard Set(arguments.keys).isSubset(of: allowed) else {
            throw MacApplicationPortError.rejected("Unsupported or mixed canonical history arguments")
        }
        var input = arguments
        input["requester_conversation_id"] = conversationID
        if reading {
            guard arguments["message_id"] is String, arguments["part_id"] is String else {
                throw MacApplicationPortError.rejected("message_id and part_id must be supplied together")
            }
            input["kind"] = "read"
            input["conversation_id"] = arguments["conversation_id"] ?? conversationID
            input["offset_bytes"] = arguments["offset_bytes"] ?? 0
            input["limit_bytes"] = arguments["limit_bytes"] ?? 8192
        } else {
            let scope = arguments["scope"] ?? "current_conversation"
            guard let scope = scope as? String,
                  ["current_conversation", "local_user_history", "conversations"].contains(scope),
                  (scope == "conversations") == (arguments["conversation_ids"] != nil) else {
                throw MacApplicationPortError.rejected("Invalid canonical history scope")
            }
            var scopeObject: [String: Any] = ["kind": scope]
            scopeObject["conversation_ids"] = arguments["conversation_ids"]
            input.removeValue(forKey: "conversation_ids")
            input["kind"] = "search"
            input["scope"] = scopeObject
            input["limit"] = arguments["limit"] ?? 8
        }
        return input
    }

    private static func response(
        requestID: String,
        output: Any,
        display: String,
        isError: Bool
    ) -> Data {
        let payload: [String: Any] = [
            "type": "device_tool_result",
            "request_id": requestID,
            "output": output,
            "display": display,
            "is_error": isError,
        ]
        return (try? JSONSerialization.data(withJSONObject: payload, options: [.sortedKeys]))
            ?? Data("{\"type\":\"device_tool_result\",\"is_error\":true}".utf8)
    }
}
