// SPDX-License-Identifier: Apache-2.0
import Foundation
import XCTest
@testable import Nola

/// Reads the source publication identified by the real runtime notifications.
/// Call before disposing the isolated source database, never from trace text.
@MainActor
func readCommittedAssistantText(_ events: [MacLocalAgentEvent], databaseURL: URL, reasoning: Bool = false) async throws -> String {
    var order: [String] = []
    var heads: [String: MacAssistantJournalCursor] = [:]
    for event in events {
        guard let cursor = event.assistantJournalCursor else { continue }
        let id = cursor.identity.assistant_message_id
        if let previous = heads[id] {
            guard previous.identity == cursor.identity else { throw MacRemoteHostError.invalidResponse }
        } else { order.append(id) }
        heads[id] = cursor
    }
    var text = ""
    for id in order {
        let cursor = try XCTUnwrap(heads[id])
        let reference = MacJournalMessageReference(databaseURL: databaseURL, cursor: cursor)
        if !reasoning {
            text += try await MacJournalTextRead.markdown(reference, historicalFallback: "")
            continue
        }
        for part in try await MacJournalTextRead.parts(reference) {
            guard case .reasoning = part.metadata else { continue }
            for body in part.bodies {
                var offset = "0"
                repeat {
                    let range = try await MacJournalTextRead.range(reference, partID: part.part_id, content: body, offset: offset)
                    guard range.offset_bytes == offset, let next = UInt64(range.next_offset_bytes),
                          let start = UInt64(offset), next >= start,
                          next - start == UInt64(range.text.utf8.count),
                          range.end_of_content || next > start else { throw MacRemoteHostError.invalidResponse }
                    text += range.text
                    if range.end_of_content { break }
                    offset = range.next_offset_bytes
                } while true
            }
        }
    }
    return text
}
