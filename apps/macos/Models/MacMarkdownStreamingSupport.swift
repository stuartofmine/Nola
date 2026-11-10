// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Provides stable sentence segmentation and safe inline Markdown preparation for streaming UI.

import Foundation

/// Shared table grammar for live tails, committed blocks and restored history.
/// Keep escaped pipes inside cells and require matching header/divider widths.
nonisolated enum MacMarkdownTableSyntax {
    static func cells(_ line: String) -> [String] {
        var cells = [String()]
        var escaped = false
        for character in line.trimmingCharacters(in: .whitespacesAndNewlines) {
            if character == "|", !escaped {
                cells.append("")
            } else {
                cells[cells.count - 1].append(character)
            }
            if character == "\\", !escaped {
                escaped = true
            } else {
                escaped = false
            }
        }
        if cells.count > 1, cells.first == "" { cells.removeFirst() }
        if cells.count > 1, cells.last == "" { cells.removeLast() }
        return cells.map { $0.trimmingCharacters(in: .whitespacesAndNewlines) }
    }

    static func isRow(_ line: String) -> Bool {
        var escaped = false
        for character in line {
            if character == "|", !escaped { return true }
            escaped = character == "\\" && !escaped
        }
        return false
    }

    static func isDivider(_ line: String) -> Bool {
        let values = cells(line)
        return !values.isEmpty && values.allSatisfy { cell in
            var core = cell[...]
            if core.first == ":" { core.removeFirst() }
            if core.last == ":" { core.removeLast() }
            return !core.isEmpty && core.allSatisfy { $0 == "-" }
        }
    }

    static func hasHeader(_ header: String, divider: String) -> Bool {
        isRow(header) && isDivider(divider) && cells(header).count == cells(divider).count
    }
}

struct MacStreamingSentenceFragment: Identifiable, Equatable, Sendable {
    let id: String
    let text: String
}

enum MacMarkdownSentenceSegmenter {
    static func fragments(text: String, identitySeed: String) -> [MacStreamingSentenceFragment] {
        guard !text.isEmpty else { return [] }
        var result: [MacStreamingSentenceFragment] = []
        var buffer = ""
        var ordinal = 0
        var inlineCode = false
        var squareDepth = 0
        var roundDepth = 0
        var previous: Character?
        let characters = Array(text)

        func appendBuffer() {
            guard !buffer.isEmpty else { return }
            result.append(.init(id: "\(identitySeed):sentence:\(ordinal)", text: buffer))
            ordinal += 1
            buffer = ""
        }

        for index in characters.indices {
            let character = characters[index]
            buffer.append(character)
            let escaped = previous == "\\"
            if character == "`", !escaped {
                inlineCode.toggle()
            } else if !inlineCode {
                switch character {
                case "[": squareDepth += 1
                case "]": squareDepth = max(0, squareDepth - 1)
                case "(": roundDepth += 1
                case ")": roundDepth = max(0, roundDepth - 1)
                default: break
                }
            }

            let next = index < characters.index(before: characters.endIndex)
                ? characters[characters.index(after: index)]
                : nil
            if isSentenceBoundary(
                character,
                previous: previous,
                next: next,
                buffer: buffer,
                inlineCode: inlineCode,
                squareDepth: squareDepth,
                roundDepth: roundDepth
            ) {
                appendBuffer()
            }
            previous = character
        }
        appendBuffer()
        return result
    }

    private static func isSentenceBoundary(
        _ character: Character,
        previous: Character?,
        next: Character?,
        buffer: String,
        inlineCode: Bool,
        squareDepth: Int,
        roundDepth: Int
    ) -> Bool {
        guard !inlineCode, squareDepth == 0, roundDepth == 0 else { return false }
        if character == "\n" {
            return buffer.count > 1
        }
        let cjkTerminators: Set<Character> = ["。", "！", "？", "；"]
        if cjkTerminators.contains(character) {
            return inlineMarkersAreBalanced(buffer)
        }
        guard ".!?;".contains(character), buffer.count >= 8 else { return false }
        if character == ".", previous?.isNumber == true, next?.isNumber == true {
            return false
        }
        guard next == nil || next?.isWhitespace == true else { return false }
        return inlineMarkersAreBalanced(buffer)
    }

    private static func inlineMarkersAreBalanced(_ source: String) -> Bool {
        let unescaped = source.replacingOccurrences(of: "\\*", with: "")
            .replacingOccurrences(of: "\\_", with: "")
            .replacingOccurrences(of: "\\~", with: "")
        let strongCount = unescaped.components(separatedBy: "**").count - 1
        let strikeCount = unescaped.components(separatedBy: "~~").count - 1
        return strongCount.isMultiple(of: 2) && strikeCount.isMultiple(of: 2)
    }
}

enum MacMarkdownInlinePreprocessor {
    /// Foundation leaves some CJK-adjacent strong spans literal when their
    /// closing punctuation is followed immediately by a Han character. Repair
    /// only unparsed text runs; code examples and existing attributes survive.
    static func repairUnparsedStrongEmphasis(_ parsed: AttributedString, source: String) -> AttributedString {
        // Parsing removes escape provenance. Leave explicitly escaped stars
        // alone rather than guessing whether their literal glyphs are markup.
        guard !source.contains(#"\*"#) else { return parsed }
        let expression = try! NSRegularExpression(pattern: #"(?<!\\)\*\*(\S(?:[^\n]*?\S)?)\*\*"#)
        var output = AttributedString()
        for run in parsed.runs {
            let text = String(parsed[run.range].characters)
            guard run.inlinePresentationIntent?.contains(.code) != true else {
                output.append(AttributedString(parsed[run.range]))
                continue
            }
            var end = text.startIndex
            for match in expression.matches(in: text, range: NSRange(text.startIndex..., in: text)) {
                guard let whole = Range(match.range, in: text), let body = Range(match.range(at: 1), in: text) else { continue }
                var prefix = AttributedString(text[end..<whole.lowerBound])
                prefix.setAttributes(run.attributes)
                output.append(prefix)
                var strong = AttributedString(text[body])
                strong.setAttributes(run.attributes)
                strong.inlinePresentationIntent = (run.inlinePresentationIntent ?? []).union(.stronglyEmphasized)
                output.append(strong)
                end = whole.upperBound
            }
            var suffix = AttributedString(text[end...])
            suffix.setAttributes(run.attributes)
            output.append(suffix)
        }
        return output
    }

    static func prepare(_ source: String, repairingIncompleteSyntax: Bool) -> String {
        var result = autolink(source)
        guard repairingIncompleteSyntax else { return result }
        result = closeUnbalanced("`", in: result)
        result = closeUnbalanced("~~", in: result)
        result = closeUnbalanced("**", in: result)
        result = closeUnbalancedSingleDelimiter("*", in: result)
        result = closeUnbalancedSingleDelimiter("_", in: result)
        result = repairIncompleteLinkLabel(in: result)
        return result
    }

    private static func closeUnbalanced(_ marker: String, in source: String) -> String {
        let count = unescapedOccurrences(of: marker, in: source)
        return count.isMultiple(of: 2) ? source : source + marker
    }

    /// Repairs emphasis whose closing delimiter has not arrived yet. Runs
    /// after strong emphasis so the two-character delimiter is not counted as
    /// two independent markers. Intraword underscores remain literal.
    private static func closeUnbalancedSingleDelimiter(
        _ marker: Character,
        in source: String
    ) -> String {
        let characters = Array(source)
        var count = 0
        for index in characters.indices where characters[index] == marker {
            let escaped = index > characters.startIndex && characters[index - 1] == "\\"
            let adjacentSame = (index > characters.startIndex && characters[index - 1] == marker)
                || (index < characters.index(before: characters.endIndex)
                    && characters[index + 1] == marker)
            let intrawordUnderscore = marker == "_"
                && index > characters.startIndex
                && index < characters.index(before: characters.endIndex)
                && characters[index - 1].isLetter
                && characters[index + 1].isLetter
            if !escaped && !adjacentSame && !intrawordUnderscore { count += 1 }
        }
        return count.isMultiple(of: 2) ? source : source + String(marker)
    }

    /// An unmatched link label is rendered as its readable label while the
    /// destination is still in flight. This mirrors Streamdown/remend's
    /// text-only recovery without inventing a clickable temporary URL.
    private static func repairIncompleteLinkLabel(in source: String) -> String {
        var squareDepth = 0
        var lastUnmatchedOpening: String.Index?
        var escaping = false
        var inlineCode = false
        var index = source.startIndex
        while index < source.endIndex {
            let character = source[index]
            if escaping {
                escaping = false
            } else if character == "\\" {
                escaping = true
            } else if character == "`" {
                inlineCode.toggle()
            } else if !inlineCode {
                if character == "[" {
                    squareDepth += 1
                    lastUnmatchedOpening = index
                } else if character == "]", squareDepth > 0 {
                    squareDepth -= 1
                    if squareDepth == 0 { lastUnmatchedOpening = nil }
                }
            }
            index = source.index(after: index)
        }
        guard squareDepth > 0, let opening = lastUnmatchedOpening else { return source }
        var repaired = source
        repaired.remove(at: opening)
        return repaired
    }

    private static func unescapedOccurrences(of marker: String, in source: String) -> Int {
        var count = 0
        var searchStart = source.startIndex
        while let range = source.range(of: marker, range: searchStart..<source.endIndex) {
            var slashCount = 0
            var cursor = range.lowerBound
            while cursor > source.startIndex {
                let previous = source.index(before: cursor)
                guard source[previous] == "\\" else { break }
                slashCount += 1
                cursor = previous
            }
            if slashCount.isMultiple(of: 2) { count += 1 }
            searchStart = range.upperBound
        }
        return count
    }

    private static func autolink(_ source: String) -> String {
        // Never rewrite Markdown-owned syntax. In particular a URL inside a
        // code span is literal, not a destination with a trailing backtick.
        let protectedExpression = try! NSRegularExpression(pattern: #"(`+)[\s\S]*?\1|\[[^\]\n]*\]\([^\n]*?\)|<[^>\n]*>"#)
        let protected = protectedExpression.matches(in: source,
            range: NSRange(source.startIndex..., in: source)).map(\.range)
        // CJK punctuation is not whitespace, so a generic URL matcher would
        // swallow the following Chinese clause. Stop at ideographic sentence
        // punctuation, then trim ordinary trailing ASCII punctuation below.
        let pattern = #"(?<!\]\()https?://[^\s<>`\[\]。，；：！？）》」』]+"#
        guard let regex = try? NSRegularExpression(pattern: pattern) else { return source }
        var result = source
        let matches = regex.matches(
            in: source,
            range: NSRange(source.startIndex..<source.endIndex, in: source)
        )
        let trailing = CharacterSet(charactersIn: ".,;:!?。，；：！？）》」』")
        for match in matches.reversed() {
            guard !protected.contains(where: { NSIntersectionRange($0, match.range).length > 0 }) else { continue }
            guard let range = Range(match.range, in: result) else { continue }
            let raw = String(result[range])
            let url = raw.trimmingCharacters(in: trailing)
            guard !url.isEmpty else { continue }
            let suffix = String(raw.dropFirst(url.count))
            result.replaceSubrange(range, with: "[\(url)](\(url))\(suffix)")
        }
        return result
    }
}
