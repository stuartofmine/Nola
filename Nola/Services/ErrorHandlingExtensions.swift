// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Implements ErrorHandlingExtensions logic in the iOS service layer.
// - Interfaces:
//   - SensitiveDataMasker
//   - ErrorContext
//   - logErrorContext
//   - logOperationError
//   - NolaErrorEnhanced
// - APIs:
//   - none
//
//
//  ErrorHandlingExtensions.swift
//  Nola
//
//  Improved error handling and sanitization for logging

import Foundation
import os.log

// MARK: - Sensitive Data Masking

public struct SensitiveDataMasker {
    /// Patterns for sensitive data that should be masked
    private static let sensitivePatterns: [String] = [
        "api[_-]?key",
        "password",
        "token",
        "secret",
        "credential",
        "authz",
        "bearer",
        "key",
        "phone",
        "email",
        "ssn"
    ]
    
    /// Mask sensitive values in metadata
    public static func sanitizeMetadata(_ metadata: [String: String]) -> [String: String] {
        var sanitized: [String: String] = [:]
        
        for (key, value) in metadata {
            let lowerKey = key.lowercased()
            let isSensitive = sensitivePatterns.contains { pattern in
                lowerKey.contains(pattern)
            }
            
            if isSensitive {
                // Mask API keys and tokens
                if value.count > 4 {
                    let prefix = String(value.prefix(2))
                    sanitized[key] = "\(prefix)***"
                } else {
                    sanitized[key] = "***"
                }
            } else {
                sanitized[key] = value
            }
        }
        
        return sanitized
    }
    
    /// Mask sensitive strings in messages
    public static func sanitizeMessage(_ message: String) -> String {
        var result = message
        
        // Mask email-like patterns
        result = result.replacingOccurrences(
            of: "[a-zA-Z0-9._%+-]+@[a-zA-Z0-9.-]+\\.[a-zA-Z]{2,}",
            with: "[EMAIL]",
            options: .regularExpression
        )
        
        // Mask phone-like patterns
        result = result.replacingOccurrences(
            of: "\\b\\d{3}[-.]?\\d{3}[-.]?\\d{4}\\b",
            with: "[PHONE]",
            options: .regularExpression
        )
        
        // Mask API key-like patterns
        result = result.replacingOccurrences(
            of: "[A-Za-z0-9]{32,}",
            with: "[KEY]",
            options: .regularExpression
        )
        
        return result
    }
}

// MARK: - Error Context

/// Error with additional context information
public struct ErrorContext {
    public let error: Error
    public let requestId: String?
    public let sessionId: String?
    public let sourceFile: String?
    public let functionName: String?
    public let lineNumber: Int?
    public let timestamp: Date
    public let userInfo: [String: String]?
    
    public init(
        error: Error,
        requestId: String? = nil,
        sessionId: String? = nil,
        sourceFile: String? = #file,
        functionName: String? = #function,
        lineNumber: Int? = #line,
        userInfo: [String: String]? = nil
    ) {
        self.error = error
        self.requestId = requestId
        self.sessionId = sessionId
        self.sourceFile = sourceFile.map { URL(fileURLWithPath: $0).lastPathComponent }
        self.functionName = functionName
        self.lineNumber = lineNumber
        self.timestamp = Date()
        self.userInfo = userInfo
    }
    
    public func toMetadata() -> [String: String] {
        var meta: [String: String] = [:]
        
        if let requestId = requestId {
            meta["requestId"] = requestId
        }
        if let sessionId = sessionId {
            meta["sessionId"] = sessionId
        }
        if let sourceFile = sourceFile {
            meta["source"] = sourceFile
        }
        if let functionName = functionName {
            meta["function"] = functionName
        }
        if let lineNumber = lineNumber {
            meta["line"] = "\(lineNumber)"
        }
        
        meta["error"] = error.localizedDescription
        
        if let userInfo = userInfo {
            for (key, value) in userInfo {
                meta["user_\(key)"] = value
            }
        }
        
        return SensitiveDataMasker.sanitizeMetadata(meta)
    }
}

// MARK: - Error Logging Helper

private let errorLog = OSLog(subsystem: "com.nola.app", category: "error")

/// Helper function for consistent error logging with sanitization
public func logErrorContext(
    _ context: ErrorContext,
    category: String = "Error"
) {
    let message = SensitiveDataMasker.sanitizeMessage(context.error.localizedDescription)
    let metadata = context.toMetadata()
    let sanitized = SensitiveDataMasker.sanitizeMetadata(metadata)
    let metaString = sanitized.map { "\($0.key)=\($0.value)" }.joined(separator: ", ")
    
    os_log(
        "%{public}@: %{private}@",
        log: errorLog,
        type: .error,
        "\(category)",
        "\(message) [\(metaString)]"
    )
}

/// Helper for operation error logging
public func logOperationError(
    operation: String,
    error: Error,
    requestId: String? = nil,
    metadata: [String: String]? = nil
) {
    let context = ErrorContext(
        error: error,
        requestId: requestId,
        userInfo: metadata
    )
    logErrorContext(context, category: "Operation[\(operation)]")
}

// MARK: - NolaError Enhanced

public enum NolaErrorEnhanced: LocalizedError {
    case dataSourceError(source: String, underlying: Error)
    case processingError(step: String, underlying: Error)
    case llmError(provider: String, underlying: Error)
    case storageError(operation: String, underlying: Error)
    case notificationError(reason: String)
    
    public var errorDescription: String? {
        switch self {
        case .dataSourceError(let source, _):
            return "Failed to fetch from \(source)"
        case .processingError(let step, _):
            return "Processing failed at step: \(step)"
        case .llmError(let provider, _):
            return "LLM provider '\(provider)' error"
        case .storageError(let op, _):
            return "Storage operation '\(op)' failed"
        case .notificationError(let reason):
            return "Notification error: \(reason)"
        }
    }
    
    public var underlyingError: Error? {
        switch self {
        case .dataSourceError(_, let error),
             .processingError(_, let error),
             .llmError(_, let error),
             .storageError(_, let error):
            return error
        case .notificationError:
            return nil
        }
    }
}
