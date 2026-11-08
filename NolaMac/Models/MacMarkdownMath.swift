// SPDX-License-Identifier: Apache-2.0
// Presentation-only math tokenization. Code and canonical source stay intact.
import Foundation

nonisolated enum MacMarkdownMath {
    enum Segment: Equatable {
        case text(String)
        case math(source: String, display: Bool, original: String)
    }

    static func segments(_ source: String) -> [Segment] {
        var result: [Segment] = []
        var plain = ""
        var index = source.startIndex
        var codeFence = 0
        while index < source.endIndex {
            let tail = source[index...]
            if tail.first == "`" {
                let run = tail.prefix { $0 == "`" }
                if codeFence == 0 { codeFence = run.count }
                else if codeFence == run.count { codeFence = 0 }
                plain += run
                index = source.index(index, offsetBy: run.count)
                continue
            }
            if codeFence == 0 {
                let delimiters = [("$$", "$$", true), ("\\[", "\\]", true),
                                  ("\\(", "\\)", false), ("$", "$", false)]
                if let (open, close, display) = delimiters.first(where: { tail.hasPrefix($0.0) }) {
                    let start = source.index(index, offsetBy: open.count)
                    if let end = source.range(of: close, range: start..<source.endIndex) {
                        let body = String(source[start..<end.lowerBound])
                        let validSingleDollar = open != "$" ||
                            (!body.contains("\n") && body.first?.isWhitespace == false && body.last?.isWhitespace == false)
                        if !body.isEmpty, body.count <= 8192, validSingleDollar {
                            if !plain.isEmpty { result.append(.text(plain)); plain = "" }
                            result.append(.math(source: body, display: display,
                                                original: String(source[index..<end.upperBound])))
                            index = end.upperBound
                            continue
                        }
                    }
                }
            }
            let character = source[index]
            plain.append(character)
            index = source.index(after: index)
            // Escapes do not open math delimiters (except \( and \[ above).
            if character == "\\", index < source.endIndex {
                plain.append(source[index])
                index = source.index(after: index)
            }
        }
        if !plain.isEmpty { result.append(.text(plain)) }
        return result
    }

    /// Interpret only HTML line breaks, never arbitrary HTML. Backtick spans
    /// remain literal, including examples demonstrating HTML syntax.
    static func lineBreaks(_ source: String) -> String {
        var result = ""
        var index = source.startIndex
        var codeFence = 0
        while index < source.endIndex {
            let tail = source[index...]
            if tail.first == "`" {
                let run = tail.prefix { $0 == "`" }
                if codeFence == 0 { codeFence = run.count }
                else if codeFence == run.count { codeFence = 0 }
                result += run
                index = source.index(index, offsetBy: run.count)
                continue
            }
            if codeFence == 0, let match = tail.range(of: #"^<br\s*/?>"#, options: [.regularExpression, .caseInsensitive]) {
                result += "\n"
                index = match.upperBound
            } else {
                result.append(source[index])
                index = source.index(after: index)
            }
        }
        return result
    }
}
