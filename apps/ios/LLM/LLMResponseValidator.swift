// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Implements LLMResponseValidator logic in the iOS LLM integration layer.
// - Interfaces:
//   - LLMValidationError
//   - LLMResponseValidator
// - APIs:
//   - none
//
import Foundation

public enum LLMValidationError: Error, LocalizedError {
    case noJSONFound
    case decodeError(Error)
    case emptyActions
    case invalidDedupeKey(index: Int)
    case contentTooLong(index: Int, length: Int)

    public var errorDescription: String? {
        switch self {
        case .noJSONFound: return "No JSON object could be extracted from the response."
        case .decodeError(let e): return "JSON decode error: \(e)"
        case .emptyActions: return "LLM response contained no actions."
        case .invalidDedupeKey(let idx): return "Action at index \(idx) has empty dedupeKey."
        case .contentTooLong(let idx, let length): return "Action at index \(idx) contentSummary too long (\(length) chars)."
        }
    }
}

public final class LLMResponseValidator {
    /// Maximum characters allowed for contentSummary
    public static var maxContentSummaryLength: Int = 1000

    /// Try to extract a JSON substring from a possibly noisy LLM response.
    /// Strategy:
    /// 1. Look for a ```json ... ``` fenced block via regex.
    /// 2. Otherwise, find the first '{' and extract a balanced-brace substring (handles nested braces).
    public static func extractJSONSubstring(from response: String) -> String? {
        // 1. regex for ```json ... ```
        if let fenced = extractFencedJSON(response: response) {
            return fenced.trimmingCharacters(in: .whitespacesAndNewlines)
        }

        // 2. balanced braces extraction
        guard let brace = extractBalancedBraces(response: response) else {
            return nil
        }
        return brace.trimmingCharacters(in: .whitespacesAndNewlines)
    }

    private static func extractFencedJSON(response: String) -> String? {
        do {
            let pattern = "```json([\\s\\S]*?)```"
            let re = try NSRegularExpression(pattern: pattern, options: [.caseInsensitive])
            let ns = response as NSString
            if let m = re.firstMatch(in: response, options: [], range: NSRange(location: 0, length: ns.length)) {
                let range = m.range(at: 1)
                return ns.substring(with: range)
            }
        } catch {
            // ignore regex errors
        }
        return nil
    }

    private static func extractBalancedBraces(response: String) -> String? {
        // find first '{'
        guard let startIdx = response.firstIndex(of: "{") else { return nil }
        var depth = 0
        var idx = startIdx
        var endIdx: String.Index? = nil
        while idx < response.endIndex {
            let ch = response[idx]
            if ch == "{" {
                depth += 1
            } else if ch == "}" {
                depth -= 1
                if depth == 0 {
                    endIdx = idx
                    break
                }
            }
            idx = response.index(after: idx)
        }
        if let end = endIdx {
            return String(response[startIdx...end])
        }
        return nil
    }

    /// Validate LLM response string and decode into LLMActionResponse
    /// Throws LLMValidationError on failure
    public static func validateAndDecode(_ response: String) throws -> LLMActionResponse {
        guard let jsonStr = extractJSONSubstring(from: response) else {
            throw LLMValidationError.noJSONFound
        }

        let data = Data(jsonStr.utf8)
        do {
            let decoder = JSONDecoder.nolaDecoder()
            let decoded = try decoder.decode(LLMActionResponse.self, from: data)

            // basic semantic checks
            if decoded.actions.isEmpty { throw LLMValidationError.emptyActions }
            for (i, action) in decoded.actions.enumerated() {
                if action.dedupeKey.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
                    throw LLMValidationError.invalidDedupeKey(index: i)
                }
                if let cs = action.contentSummary {
                    let len = cs.count
                    if len > maxContentSummaryLength {
                        throw LLMValidationError.contentTooLong(index: i, length: len)
                    }
                }
            }

            return decoded
        } catch let err as LLMValidationError {
            throw err
        } catch {
            throw LLMValidationError.decodeError(error)
        }
    }

    /// Map validation error to a user-friendly (translatable) message.
    /// In a real app, these strings should be localized via NSLocalizedString.
    public static func userFriendlyMessage(for error: Error) -> String {
        if let ve = error as? LLMValidationError {
            switch ve {
            case .noJSONFound:
                return NSLocalizedString("autogen.LLMResponseValidator.e0bf4688", comment: "AUTOGEN")
            case .decodeError:
                return NSLocalizedString("autogen.LLMResponseValidator.b352d746", comment: "AUTOGEN")
            case .emptyActions:
                return NSLocalizedString("autogen.LLMResponseValidator.a5a04700", comment: "AUTOGEN")
            case .invalidDedupeKey:
                return NSLocalizedString("autogen.LLMResponseValidator.1cf6e0d8", comment: "AUTOGEN")
            case .contentTooLong:
                return NSLocalizedString("autogen.LLMResponseValidator.89154a16", comment: "AUTOGEN")
            }
        }
        return NSLocalizedString("autogen.LLMResponseValidator.211ee58e", comment: "AUTOGEN")
    }

    /// Append a simple audit entry to the app support directory for debugging/audit.
    /// This writes a timestamped line with brief metadata. Avoid writing sensitive raw inputs in production.
    public static func recordAudit(candidateId: String?, modelId: String?, inputPreview: String?, extractedJSON: String?, decodedSummary: String?, error: Error?) {
        let fm = FileManager.default
        let appSupport: URL
        if let dir = fm.urls(for: .applicationSupportDirectory, in: .userDomainMask).first {
            appSupport = dir
        } else {
            appSupport = fm.temporaryDirectory
        }
        let folder = appSupport.appendingPathComponent("Nola", isDirectory: true)
        do {
            try fm.createDirectory(at: folder, withIntermediateDirectories: true)
            let logFile = folder.appendingPathComponent("llm_audit.log")
            var line = "\(ISO8601DateFormatter().string(from: Date())) | candidate:\(candidateId ?? "-") | model:\(modelId ?? "-")"
            if let ip = inputPreview { line += " | inputPreview:\(ip.prefix(200))" }
            if let ej = extractedJSON { line += " | extractedJSON:\(ej.prefix(200))" }
            if let ds = decodedSummary { line += " | decodedSummary:\(ds.prefix(200))" }
            if let e = error { line += " | error:\(e.localizedDescription)" }
            line += "\n"
            if let data = line.data(using: .utf8) {
                if fm.fileExists(atPath: logFile.path) {
                    let fh = try FileHandle(forWritingTo: logFile)
                    fh.seekToEndOfFile()
                    fh.write(data)
                    try fh.close()
                } else {
                    try data.write(to: logFile, options: .atomic)
                }
            }
        } catch {
            // Best-effort: do not throw
        }
    }
}
