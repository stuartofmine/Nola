// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Presents committed context usage and optional cache diagnostics.

import Foundation

struct IOSContextUsageEvidence {
    let usedTokens: Int
    let limitTokens: Int
    let cachedInputTokens: Int?
    let cacheWriteInputTokens: Int?
    let promptCacheStatus: String?
    var cacheHitRateBps: UInt16? = nil
    var cacheReportedRequests: UInt64? = nil
    var cacheUnreportedRequests: UInt64? = nil
    var hasCommittedTokenTotals = true
    var isEstimated = false
    var measuredRequestID: String? = nil
    var cacheDiagnosticRequestID: String? = nil
    var cacheDiagnosticAttemptID: String? = nil

    static func committed(_ usage: IOSConversationProjectionV1.ContextUsage) -> Self? {
        guard let usedTokens = Int(exactly: usage.usedTokens),
              let limitTokens = Int(exactly: usage.promptTokenLimit) else { return nil }
        let totals = usage.conversationCacheUsage
        return Self(usedTokens: usedTokens, limitTokens: limitTokens,
            cachedInputTokens: totals?.read_tokens.map { Int(clamping: $0) },
            cacheWriteInputTokens: totals?.write_tokens.map { Int(clamping: $0) },
            promptCacheStatus: usage.promptCacheMeasurement == nil ? nil : usage.promptCacheDiagnostic?.status,
            cacheHitRateBps: totals?.hit_rate_bps,
            cacheReportedRequests: totals?.reported_requests,
            cacheUnreportedRequests: totals?.unreported_requests,
            hasCommittedTokenTotals: limitTokens > 0, isEstimated: usage.estimated,
            measuredRequestID: usage.measurement?.request_id,
            cacheDiagnosticRequestID: usage.promptCacheMeasurement?.request_id,
            cacheDiagnosticAttemptID: usage.promptCacheMeasurement?.attempt_id)
    }
}
