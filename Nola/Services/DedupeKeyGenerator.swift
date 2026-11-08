// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Implements DedupeKeyGenerator logic in the iOS service layer.
// - Interfaces:
//   - DedupeKeyGenerator
// - APIs:
//   - none
//
//
//  DedupeKeyGenerator.swift
//  Nola
//

import Foundation

public struct DedupeKeyGenerator {
    // MARK: - Stable hashing (fallback only)
    // Swift's hashValue is intentionally not stable across processes.
    // When Rust is unavailable, we still want deterministic dedupe keys.
    private static func stableHexHash(_ s: String) -> String {
        // 64-bit FNV-1a
        var hash: UInt64 = 0xcbf29ce484222325
        let prime: UInt64 = 0x00000100000001B3
        for b in s.utf8 {
            hash ^= UInt64(b)
            hash &*= prime
        }
        return String(format: "%016llx", hash)
    }

    /// - Parameters:
    public static func generate(
        source: String,
        title: String,
        date: Date?,
        sourceId: String? = nil
    ) -> String {
        if let rustKey = RustInterop.generateDedupeKey(source: source, title: title, dateISO: dateISOString(from: date), sourceId: sourceId) {
            return rustKey
        }
        let normalizedTitle = normalizeString(title)
        
        let dateStr: String
        if let date = date {
            let formatter = ISO8601DateFormatter()
            formatter.formatOptions = [.withFullDate]
            dateStr = formatter.string(from: date)
        } else {
            dateStr = "nodate"
        }
        
        let titleHash = stableHexHash(normalizedTitle)
        
        if let sourceId = sourceId, !sourceId.isEmpty {
            return "\(source):\(sourceId):\(titleHash):\(dateStr)"
        } else {
            return "\(source):\(titleHash):\(dateStr)"
        }
    }

    private static func dateISOString(from date: Date?) -> String? {
        guard let date = date else { return nil }
        let f = ISO8601DateFormatter()
        f.formatOptions = [.withFullDate]
        return f.string(from: date)
    }
    
    /// - Parameters:
    public static func generateSummary(
        date: Date = Date(),
        summaryText: String? = nil
    ) -> String {
        let dateFormatter = ISO8601DateFormatter()
        dateFormatter.formatOptions = [.withFullDate]
        let dateStr = dateFormatter.string(from: date)
        
        let textHash = stableHexHash(summaryText ?? "")
        return "summary:\(dateStr):\(textHash)"
    }
    
    /// - Parameters:
    public static func generateTask(
        taskId: UUID,
        title: String? = nil
    ) -> String {
        if let title = title, !title.isEmpty {
            // Delegate to Rust for task key generation to ensure consistency
            if let s = RustInterop.generateDedupeKey(source: "task", title: title, dateISO: nil, sourceId: taskId.uuidString) {
                return s
            }
            return generate(source: "task", title: title, date: nil, sourceId: taskId.uuidString)
        } else {
            return "task:\(taskId.uuidString)"
        }
    }
    
    /// - Parameters:
    public static func generateURLEvent(
        sourceId: UUID,
        title: String,
        date: Date? = nil
    ) -> String {
        return generate(source: "url", title: title, date: date, sourceId: sourceId.uuidString)
    }
    
    /// - Parameters:
    public static func generateTestItem(text: String) -> String {
        let normalizedText = normalizeString(text)
        let textHash = stableHexHash(normalizedText)
        return "test:\(textHash)"
    }
    
    // MARK: - Private Helpers
    
    private static func normalizeString(_ str: String) -> String {
        let lowercased = str.lowercased()
        
        let singleSpaces = lowercased.replacingOccurrences(of: "\\s+", with: " ", options: .regularExpression)
        
        let trimmed = singleSpaces.trimmingCharacters(in: .whitespaces)
        
        let allowedChars = CharacterSet.alphanumerics.union(CharacterSet(charactersIn: " "))
        let filtered = trimmed.unicodeScalars
            .filter { allowedChars.contains($0) }
            .map(String.init)
            .joined()
        
        return filtered
    }
}
