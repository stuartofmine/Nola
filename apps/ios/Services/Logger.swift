// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Implements Logger logic in the iOS service layer.
// - Interfaces:
//   - Logger
// - APIs:
//   - none
//
//
//  Logger.swift
//  Nola
//
//  Structured hierarchical logger with in-memory ring buffer.
//

import Foundation
import Combine
import os.log

public final class Logger: ObservableObject {
    // No actor-bound teardown: queued work holds only weak references and
    // ARC releases the stored values. Avoid synthesized isolated destruction
    // on older Swift runtimes (swiftlang/swift#88036).
    nonisolated deinit {}

    public enum Level: Int, CaseIterable, Comparable, Sendable {
        case error = 0
        case warn = 1
        case info = 2
        case debug = 3
        case trace = 4
        
        public static func < (lhs: Level, rhs: Level) -> Bool { lhs.rawValue < rhs.rawValue }
    }

    public struct Entry: Identifiable, Hashable {
        public let id = UUID()
        public let timestamp: Date
        public let level: Level
        public let category: String
        public let message: String
        public let metadata: [String: String]?
    }

    public struct RuntimeLoggingState: Equatable, Sendable {
        public let isDetailedLoggingEnabled: Bool
        public let minimumLevel: Level
    }

    @Published public private(set) var entries: [Entry] = []
    @Published public private(set) var minLevel: Level
    private let capacity: Int
    private let queue = DispatchQueue(label: "nola.logger", qos: .utility)
    private let policyLock = NSLock()
    private var detailedLoggingEnabled: Bool
    private var configuredMinimumLevel: Level

    public init(capacity: Int = 300) {
        self.capacity = capacity
        // Ensure debug builds default to debug verbosity for easier troubleshooting.
        #if DEBUG
        self.minLevel = .debug
        self.configuredMinimumLevel = .debug
        self.detailedLoggingEnabled = true
        #else
        self.minLevel = .info
        self.configuredMinimumLevel = .info
        self.detailedLoggingEnabled = false
        #endif
    }

    // Shared singleton for convenience in modules that don't have DI
    public static let shared = Logger()

    public var runtimeLoggingState: RuntimeLoggingState {
        policyLock.lock()
        defer { policyLock.unlock() }
        return RuntimeLoggingState(
            isDetailedLoggingEnabled: detailedLoggingEnabled,
            minimumLevel: configuredMinimumLevel
        )
    }

    public func configureRuntimeLogging(detailedLoggingEnabled: Bool, minimumLevel: Level) {
        policyLock.lock()
        self.detailedLoggingEnabled = detailedLoggingEnabled
        configuredMinimumLevel = minimumLevel
        policyLock.unlock()

        if Thread.isMainThread {
            minLevel = minimumLevel
        } else {
            DispatchQueue.main.async { [weak self] in
                self?.minLevel = minimumLevel
            }
        }
    }

    func shouldRecord(_ level: Level) -> Bool {
        let state = runtimeLoggingState
        if !state.isDetailedLoggingEnabled {
            return level == .error || level == .warn
        }
        return level.rawValue <= state.minimumLevel.rawValue
    }

    public func log(_ level: Level, _ category: String, _ message: String, metadata: [String: String]? = nil) {
        guard shouldRecord(level) else { return }
        let entry = Entry(
            timestamp: Date(),
            level: level,
            category: Self.sanitizeCategory(category),
            message: Self.sanitizeForDiagnostics(message),
            metadata: Self.sanitizeMetadata(metadata)
        )
        let capacity = capacity
        queue.async { [weak self] in
            let line = Self.formatLine(entry)
            LogFileStore.shared.appendLine(line)

            #if DEBUG
            DispatchQueue.main.async { [weak self] in
                guard let self else { return }
                var copy = self.entries
                copy.append(entry)
                if copy.count > capacity { copy.removeFirst(copy.count - capacity) }
                self.entries = copy
            }
            print(line)
            #else
            let subsystem = "com.nola.app"
            let oslog = OSLog(subsystem: subsystem, category: entry.category)
            switch level {
            case .error:
                os_log("%{private}@", log: oslog, type: .error, entry.message)
            case .warn:
                os_log("%{private}@", log: oslog, type: .default, entry.message)
            case .info:
                os_log("%{private}@", log: oslog, type: .info, entry.message)
            case .debug, .trace:
                os_log("%{private}@", log: oslog, type: .debug, entry.message)
            }
            #endif
        }
    }

    // Convenience overload to match previous usage patterns: Logger.shared.log("msg", level: .info)
    public func log(_ message: String, level: Level, category: String = "app", metadata: [String: String]? = nil) {
        self.log(level, category, message, metadata: metadata)
    }

    public func clear() { entries.removeAll() }

    public static func sanitizeForDiagnostics(_ text: String) -> String {
        var sanitized = replacingMatches(
            in: text,
            pattern: #"(?i)\bBearer\s+[A-Za-z0-9._~+/=-]+"#
        ) { _ in "Bearer <redacted>" }
        sanitized = replacingMatches(
            in: sanitized,
            pattern: #"(?i)\b(api[_-]?key|access[_-]?token|refresh[_-]?token|token|authorization|cookie|prompt|response|thinking|content|preview|delta|payload|body)\b\s*(?:\([^)]*\))?\s*[:=]\s*[^\r\n,}]*"#
        ) { match in
            guard let separator = match.firstIndex(where: { $0 == ":" || $0 == "=" }) else {
                return "<redacted>"
            }
            return String(match[..<separator]).trimmingCharacters(in: .whitespaces) + "=<redacted>"
        }
        sanitized = replacingMatches(
            in: sanitized,
            pattern: #"https?://[^\s,}\]\)]+"#
        ) { sanitizedOrigin(from: $0) }
        sanitized = replacingMatches(
            in: sanitized,
            pattern: #"(?<![A-Za-z0-9])/(?:Users|private|var|tmp|Volumes|Applications|opt|usr|home)/[^\s,}\]]+"#
        ) { _ in "<path>" }
        return sanitized
    }

    private static let tsFormatter: DateFormatter = {
        let f = DateFormatter()
        f.dateFormat = "yyyy-MM-dd HH:mm:ss.SSS"
        return f
    }()

    private static func formatLine(_ entry: Entry) -> String {
        let ts = tsFormatter.string(from: entry.timestamp)
        var line = "[\(ts)][\(entry.level)][\(entry.category)] \(entry.message)"
        if let meta = entry.metadata, !meta.isEmpty {
            let metaStr = meta
                .sorted(by: { $0.key < $1.key })
                .map { "\($0.key)=\($0.value)" }
                .joined(separator: ", ")
            line += " {\(metaStr)}"
        }
        return line
    }

    private static func sanitizeCategory(_ category: String) -> String {
        let allowed = category.unicodeScalars.filter {
            CharacterSet.alphanumerics.contains($0) || $0 == "." || $0 == "_" || $0 == "-"
        }
        let value = String(String.UnicodeScalarView(allowed))
        return value.isEmpty ? "app" : String(value.prefix(64))
    }

    private static func sanitizeMetadata(_ metadata: [String: String]?) -> [String: String]? {
        guard let metadata, !metadata.isEmpty else { return nil }
        return metadata.reduce(into: [String: String]()) { result, item in
            let key = sanitizeCategory(item.key)
            let normalizedKey = key.lowercased()
            if ["key", "token", "authorization", "cookie", "prompt", "response", "thinking", "content", "input", "output", "preview", "delta", "payload", "body", "text"]
                .contains(where: normalizedKey.contains) {
                result[key] = "<redacted>"
            } else if normalizedKey.contains("url") || normalizedKey.contains("endpoint") {
                result[key] = sanitizedOrigin(from: item.value)
            } else if normalizedKey.contains("path") || normalizedKey.contains("file") {
                result[key] = "<path>"
            } else {
                result[key] = sanitizeForDiagnostics(item.value)
            }
        }
    }

    private static func sanitizedOrigin(from rawValue: String) -> String {
        guard let components = URLComponents(string: rawValue),
              let scheme = components.scheme,
              let host = components.host else {
            return rawValue.isEmpty ? "" : "<url>"
        }
        var origin = "\(scheme)://\(host)"
        if let port = components.port { origin += ":\(port)" }
        return origin
    }

    private static func replacingMatches(
        in text: String,
        pattern: String,
        transform: (String) -> String
    ) -> String {
        guard let expression = try? NSRegularExpression(pattern: pattern) else { return text }
        var result = text
        let matches = expression.matches(
            in: result,
            range: NSRange(result.startIndex..<result.endIndex, in: result)
        )
        for match in matches.reversed() {
            guard let range = Range(match.range, in: result) else { continue }
            result.replaceSubrange(range, with: transform(String(result[range])))
        }
        return result
    }
}

// Extension to provide CustomStringConvertible conformance outside of main actor context
extension Logger.Level: CustomStringConvertible {
    public nonisolated var description: String {
        switch self {
        case .error: return "ERROR"
        case .warn: return "WARN"
        case .info: return "INFO"
        case .debug: return "DEBUG"
        case .trace: return "TRACE"
        }
    }
}
