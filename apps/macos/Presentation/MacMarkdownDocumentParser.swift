// SPDX-License-Identifier: Apache-2.0
// CommonMark/GFM owns block structure; native views own presentation and motion.
import Foundation
import Markdown

nonisolated enum MacMarkdownTextParser {
    static func parse(_ source: String) -> [MacMarkdownTextBlock] {
        let normalized = source.replacingOccurrences(of: "\r\n", with: "\n")
            .replacingOccurrences(of: "\r", with: "\n")
        let original = Document(parsing: normalized, options: [.disableSmartOpts])
        let originalSource = Source(normalized)
        var edits: [Source.Edit] = []
        var codeLines: Set<Int> = []
        func inspect(_ node: any Markup) {
            if let code = node as? CodeBlock, let range = code.range {
                codeLines.formUnion(range.lowerBound.line...range.upperBound.line)
                return
            }
            // Retain the standard parser's resolved destinations when a block
            // later enters Foundation's independent inline renderer. Only
            // reference links change representation; their original label,
            // math, emphasis and surrounding source remain untouched.
            if let link = node as? Markdown.Link,
               let range = link.range, let first = Array(link.children).first?.range,
               let last = Array(link.children).last?.range,
               let destination = link.destination,
               originalSource.raw(range).hasPrefix("["),
               !originalSource.raw(last.upperBound..<range.upperBound).hasPrefix("](") {
                let label = originalSource.verbatim(first.lowerBound..<last.upperBound)
                let escapedDestination = destination.replacingOccurrences(of: "\\", with: "\\\\")
                    .replacingOccurrences(of: "<", with: "%3C")
                    .replacingOccurrences(of: ">", with: "%3E")
                    .replacingOccurrences(of: "\n", with: "%0A")
                let title = link.title.map {
                    " \"" + $0.replacingOccurrences(of: "\\", with: "\\\\")
                        .replacingOccurrences(of: "\"", with: "\\\"") + "\""
                } ?? ""
                if let bytes = originalSource.byteRange(range) {
                    edits.append(.init(range: bytes, replacement: "[\(label)](<\(escapedDestination)>\(title))"))
                }
                return
            }
            for child in node.children { inspect(child) }
        }
        inspect(original)
        // Preserve our visible footnote-definition extension, but never edit
        // code. Code ownership comes from the original CommonMark tree, not a
        // second fence/indentation recognizer.
        for (index, line) in normalized.components(separatedBy: "\n").enumerated()
            where !codeLines.contains(index + 1) && line.hasPrefix("[^") && line.contains("]:") {
            let offset = originalSource.lineOffsets[index]
            edits.append(.init(range: offset..<offset, replacement: "\\"))
        }
        guard !edits.isEmpty else {
            return original.children.flatMap { blocks($0, source: originalSource) }
        }
        var bytes = Array(normalized.utf8)
        for edit in edits.sorted(by: { $0.range.lowerBound > $1.range.lowerBound }) {
            bytes.replaceSubrange(edit.range, with: edit.replacement.utf8)
        }
        let prepared = String(decoding: bytes, as: UTF8.self)
        let input = Source(prepared)
        return Document(parsing: prepared, options: [.disableSmartOpts]).children.flatMap { blocks($0, source: input) }
    }

    private static func blocks(_ node: any Markup, source: Source) -> [MacMarkdownTextBlock] {
        switch node {
        case let heading as Heading:
            return [.heading(level: heading.level, text: source.inline(heading))]
        case let paragraph as Paragraph:
            let text = source.inline(paragraph)
            if text.hasPrefix("\\[^"), let end = text.range(of: "]:") {
                let label = String(text.dropFirst(3).prefix { $0 != "]" })
                return [.footnote(label: label, text: String(text[end.upperBound...]).trimmingCharacters(in: .whitespaces))]
            }
            return [.paragraph(text)]
        case let list as OrderedList:
            return list.children.enumerated().map { index, child in
                .listItem(marker: "\(list.startIndex + UInt(index)).", children: child.children.flatMap { blocks($0, source: source) })
            }
        case let list as UnorderedList:
            return list.children.map { child in
                let checkbox = (child as? ListItem)?.checkbox
                let marker = checkbox.map { $0 == .checked ? "☑" : "☐" } ?? "•"
                return .listItem(marker: marker, children: child.children.flatMap { blocks($0, source: source) })
            }
        case let quote as BlockQuote:
            return [.quote(quote.children.flatMap { blocks($0, source: source) })]
        case let code as CodeBlock:
            let language = code.language ?? ""
            // cmark includes the source line terminator, not an extra code line.
            let body = code.code.hasSuffix("\n") ? String(code.code.dropLast()) : code.code
            return [language.lowercased() == "mermaid" ? .mermaid(body) : .code(language: language, source: body)]
        case is ThematicBreak:
            return [.divider]
        case let table as Markdown.Table:
            return [.table(headers: table.head.cells.map { source.inline($0) },
                           rows: table.body.rows.map { $0.cells.map { source.inline($0) } })]
        default:
            // HTML is inert text, never executable content in a native answer.
            return [.paragraph(source.raw(node.range))]
        }
    }

    nonisolated private struct Source {
        nonisolated struct Edit {
            let range: Range<Int>
            let replacement: String
        }
        let lines: [[UInt8]]
        let lineOffsets: [Int]
        let bytes: [UInt8]
        init(_ source: String) {
            bytes = Array(source.utf8)
            lines = source.components(separatedBy: "\n").map { Array($0.utf8) }
            var offset = 0
            lineOffsets = lines.map { line in
                defer { offset += line.count + 1 }
                return offset
            }
        }

        func verbatim(_ range: SourceRange) -> String {
            guard let range = byteRange(range) else { return "" }
            return String(decoding: bytes[range], as: UTF8.self)
        }

        func byteRange(_ range: SourceRange) -> Range<Int>? {
            guard range.lowerBound.line > 0, range.upperBound.line <= lines.count else { return nil }
            let lowerLine = range.lowerBound.line - 1
            let upperLine = range.upperBound.line - 1
            let lower = lineOffsets[lowerLine] + min(lines[lowerLine].count, max(0, range.lowerBound.column - 1))
            let upper = lineOffsets[upperLine] + min(lines[upperLine].count, max(0, range.upperBound.column - 1))
            return lower <= upper ? lower..<upper : nil
        }

        func inline(_ node: any Markup) -> String {
            let children = Array(node.children)
            guard let first = children.first?.range, let last = children.last?.range else {
                return raw(node.range)
            }
            return raw(first.lowerBound..<last.upperBound)
        }

        func raw(_ range: SourceRange?) -> String {
            guard let range, range.lowerBound.line > 0, range.upperBound.line <= lines.count else { return "" }
            let prefixWidth = max(0, range.lowerBound.column - 1)
            return (range.lowerBound.line...range.upperBound.line).map { number in
                let bytes = lines[number - 1]
                let end = number == range.upperBound.line ? min(bytes.count, max(0, range.upperBound.column - 1)) : bytes.count
                var start = number == range.lowerBound.line ? min(prefixWidth, end) : 0
                // Continuation lines retain container prefixes in source ranges.
                // Strip only indentation/quote markers, never lazy prose content.
                if number != range.lowerBound.line {
                    while start < min(prefixWidth, end), [UInt8(32), 9, 62].contains(bytes[start]) { start += 1 }
                }
                return String(decoding: bytes[start..<end], as: UTF8.self)
            }.joined(separator: "\n")
        }
    }

    static func plainText(_ source: String) -> String {
        func inline(_ source: String) -> String {
            guard let parsed = try? AttributedString(markdown: source,
                options: .init(interpretedSyntax: .inlineOnlyPreservingWhitespace)) else { return source }
            return String(parsed.characters)
        }
        func render(_ blocks: [MacMarkdownTextBlock]) -> String {
            blocks.map { block in
                switch block {
                case .heading(_, let text), .paragraph(let text): return inline(text)
                case .listItem(let marker, let children): return marker + " " + render(children).replacingOccurrences(of: "\n", with: "\n  ")
                case .quote(let children): return render(children)
                case .code(_, let text), .mermaid(let text): return text
                case .footnote(let label, let text): return "[\(label)] " + inline(text)
                case .divider: return "———"
                case .table(let headers, let rows): return ([headers] + rows).map { $0.map(inline).joined(separator: "\t") }.joined(separator: "\n")
                }
            }.joined(separator: "\n\n")
        }
        return render(parse(source))
    }
}
