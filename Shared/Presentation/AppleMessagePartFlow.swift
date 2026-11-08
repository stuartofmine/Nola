// SPDX-License-Identifier: Apache-2.0
//! Presentation-only contiguous segments. The canonical sequence is never rewritten.
nonisolated enum AppleMessagePartFlowKind { case reasoning, tool, content }
nonisolated enum AppleMessagePartSegmentKind { case process, content }

nonisolated struct AppleMessagePartFlowSegment<Part>: Identifiable {
    let id: String
    let kind: AppleMessagePartSegmentKind
    var parts: [Part]
}

nonisolated enum AppleMessagePartFlow {
    /// Resolve message-owned Part choices after a segment regroup. An explicit
    /// collapse wins; native transient state never becomes a Core Run fact.
    static func isExpanded<Choices: Sequence>(choices: Choices, defaultExpanded: Bool) -> Bool where Choices.Element == Bool {
        var expanded = defaultExpanded
        for choice in choices {
            if !choice { return false }
            expanded = true
        }
        return expanded
    }

    static func segments<Part>(_ parts: [Part], id: (Part) -> String,
                               kind: (Part) -> AppleMessagePartFlowKind) -> [AppleMessagePartFlowSegment<Part>] {
        var result: [AppleMessagePartFlowSegment<Part>] = []
        for part in parts {
            let process = kind(part) != .content
            if process, result.last?.kind == .process {
                result[result.count - 1].parts.append(part)
            } else {
                result.append(.init(id: (process ? "process:" : "part:") + id(part),
                                    kind: process ? .process : .content, parts: [part]))
            }
        }
        return result
    }
}
