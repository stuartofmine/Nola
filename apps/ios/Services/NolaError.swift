// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Implements NolaError logic in the iOS service layer.
// - Interfaces:
//   - NolaError
//   - NetworkingError
//   - LLMError
//   - StorageError
//   - DataSourceError
// - APIs:
//   - none
//
//
//  NolaError.swift
//  Nola
//
//  Unified error domain for the application.
//

import Foundation

/// Top-level error types for Nola.
public enum NolaError: Error {
    case networking(NetworkingError)
    case llm(LLMError)
    case storage(StorageError)
    case dataSource(DataSourceError)
    case unknown(Error)
    
    public var localizedDescription: String {
        switch self {
        case .networking(let e): return e.localizedDescription
        case .llm(let e): return e.localizedDescription
        case .storage(let e): return e.localizedDescription
        case .dataSource(let e): return e.localizedDescription
        case .unknown(let e): return e.localizedDescription
        }
    }
}

extension NolaError: LocalizedError {
    public var errorDescription: String? {
        localizedDescription
    }
}

/// Networking-related errors.
public enum NetworkingError: Error, LocalizedError {
    case notConnected
    case timeout
    case badRequest
    case unauthorized
    case rateLimited
    case serverError(Int)
    case invalidResponse
    case unknown(Error)
    
    public var errorDescription: String? {
        switch self {
        case .notConnected: return NSLocalizedString("autogen.NolaError.be2dc7fc", comment: "AUTOGEN")
        case .timeout: return NSLocalizedString("autogen.NolaError.e41ce24c", comment: "AUTOGEN")
        case .badRequest: return NSLocalizedString("autogen.NolaError.9e9ffd48", comment: "AUTOGEN")
        case .unauthorized: return NSLocalizedString("autogen.NolaError.05017035", comment: "AUTOGEN")
        case .rateLimited: return NSLocalizedString("autogen.NolaError.a465c89c", comment: "AUTOGEN")
        case .serverError(let code): return "服务器错误 (\(code))"
        case .invalidResponse: return NSLocalizedString("autogen.NolaError.ad08ccdc", comment: "AUTOGEN")
        case .unknown(let e): return "网络错误：\(e.localizedDescription)"
        }
    }
}

/// LLM-related errors.
public enum LLMError: Error, LocalizedError {
    case notConfigured
    case timeout
    case rateLimited
    case decoding
    case network(Error)
    case modelLoadFailed(reason: String)
    case modelNotLoaded
    case inferenceError(String)
    case invalidInput(reason: String)
    case invalidResponse(reason: String)
    case providerFailure(code: String, message: String, retryable: Bool)
    case unknown
    
    public var errorDescription: String? {
        switch self {
        case .notConfigured: return NSLocalizedString("autogen.NolaError.1b17319a", comment: "AUTOGEN")
        case .timeout: return NSLocalizedString("autogen.NolaError.4fe90130", comment: "AUTOGEN")
        case .rateLimited: return NSLocalizedString("autogen.NolaError.4dbe8495", comment: "AUTOGEN")
        case .decoding: return NSLocalizedString("autogen.NolaError.f03719c5", comment: "AUTOGEN")
        case .network(let e): return "LLM 网络错误：\(e.localizedDescription)"
        case .modelLoadFailed(let reason): return "本地模型加载失败：\(reason)"
        case .modelNotLoaded: return NSLocalizedString("autogen.NolaError.3d1e8609", comment: "AUTOGEN")
        case .inferenceError(let msg): return "推理错误：\(msg)"
        case .invalidInput(let reason): return "输入无效：\(reason)"
        case .invalidResponse(let reason): return "响应无效：\(reason)"
        case .providerFailure(_, let message, _): return message
        case .unknown: return NSLocalizedString("autogen.NolaError.239f2c07", comment: "AUTOGEN")
        }
    }
}

/// Storage-related errors.
public enum StorageError: Error, LocalizedError {
    case saveFailed(String)
    case loadFailed(String)
    case deleteFailed(String)
    case corruptedData
    
    public var errorDescription: String? {
        switch self {
        case .saveFailed(let msg): return "保存失败：\(msg)"
        case .loadFailed(let msg): return "加载失败：\(msg)"
        case .deleteFailed(let msg): return "删除失败：\(msg)"
        case .corruptedData: return NSLocalizedString("autogen.NolaError.4a919919", comment: "AUTOGEN")
        }
    }
}

/// Data source access errors.
public enum DataSourceError: Error, LocalizedError {
    case permissionDenied(source: String)
    case noData
    case parsingFailed
    case unknown(Error)
    
    public var errorDescription: String? {
        switch self {
        case .permissionDenied(let src): return "\(src) 权限被拒绝"
        case .noData: return NSLocalizedString("autogen.NolaError.3a1e2e0f", comment: "AUTOGEN")
        case .parsingFailed: return NSLocalizedString("autogen.NolaError.7799860c", comment: "AUTOGEN")
        case .unknown(let e): return "数据源错误：\(e.localizedDescription)"
        }
    }
}

// MARK: - Convenience Constructors

extension NolaError {
    static func modelLoadFailed(reason: String) -> NolaError {
        return .llm(.modelLoadFailed(reason: reason))
    }
    
    static var modelNotLoaded: NolaError {
        return .llm(.modelNotLoaded)
    }
    
    static var timeout: NolaError {
        return .llm(.timeout)
    }
    
    static func invalidInput(reason: String) -> NolaError {
        return .llm(.invalidInput(reason: reason))
    }
    
    static func invalidResponse(reason: String) -> NolaError {
        return .llm(.invalidResponse(reason: reason))
    }
    
    static func inferenceError(_ message: String) -> NolaError {
        return .llm(.inferenceError(message))
    }
}
