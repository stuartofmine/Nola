// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Implements ScheduledActionRunText logic in the iOS UI layer.
// - Interfaces:
//   - ScheduledActionRunText
// - APIs:
//   - none
//
import Foundation
import SwiftUI

/// Shared text extraction/formatting for `ScheduledActionRunRecord`.
///
/// Keep this logic consistent across Home cards, Scheduled Actions overview,
/// and the Runs detail screens.
enum ScheduledActionRunText {
    enum StatusCategory: Equatable {
        case success, issues, skipped, cancelled, delegated
    }

    static func statusCategory(_ status: LocalJSONStorage.ScheduledActionRunRecord.Status) -> StatusCategory {
        switch status {
        case .answered, .actionCompleted: return .success
        case .delegationAccepted: return .delegated
        case .incomplete, .blocked, .failed: return .issues
        case .cancelled: return .cancelled
        case .success: return .success
        case .partial: return .issues
        case .skipped: return .skipped
        }
    }

    enum StatusTone: Equatable {
        case neutral, success, attention, failure

        var color: Color {
            switch self {
            case .neutral: return .secondary
            case .success: return DesignTokens.successColor
            case .attention: return DesignTokens.warningColor
            case .failure: return DesignTokens.dangerColor
            }
        }
    }

    struct StatusPresentation {
        let title: String
        let symbol: String
        let tone: StatusTone
    }

    static func statusPresentation(_ status: LocalJSONStorage.ScheduledActionRunRecord.Status) -> StatusPresentation {
        switch status {
        // These are active built-in scheduled-source outcomes, not aliases for
        // the canonical agent Run result variants below.
        case .success:
            return .init(title: L10n.scheduledActionsOverviewStatusOk, symbol: "checkmark.circle.fill", tone: .success)
        case .partial:
            return .init(title: L10n.scheduledActionsOverviewStatusPartial, symbol: "exclamationmark.triangle.fill", tone: .attention)
        case .skipped:
            return .init(title: L10n.scheduledActionsOverviewStatusSkipped, symbol: "pause.circle.fill", tone: .neutral)
        case .delegationAccepted:
            return .init(title: String(localized: "Delegated"), symbol: "arrow.triangle.branch", tone: .neutral)
        case .answered:
            return .init(title: NSLocalizedString("scheduled_actions.run_status.answered", value: "Answered", comment: "Canonical answer-only result"), symbol: "text.bubble", tone: .neutral)
        case .actionCompleted:
            return .init(title: NSLocalizedString("scheduled_actions.run_status.action_completed", value: "Action completed", comment: "Canonical action completed result"), symbol: "checkmark.circle.fill", tone: .success)
        case .incomplete:
            return .init(title: NSLocalizedString("scheduled_actions.run_status.incomplete", value: "Incomplete", comment: "Canonical incomplete result"), symbol: "exclamationmark.circle", tone: .attention)
        case .blocked:
            return .init(title: NSLocalizedString("scheduled_actions.run_status.blocked", value: "Blocked", comment: "Canonical blocked result; not execution failure"), symbol: "hand.raised", tone: .attention)
        case .cancelled:
            return .init(title: NSLocalizedString("scheduled_actions.run_status.cancelled", value: "Cancelled", comment: "Canonical cancelled result"), symbol: "stop.circle", tone: .neutral)
        case .failed:
            return .init(title: L10n.scheduledActionsOverviewStatusFailed, symbol: "xmark.circle.fill", tone: .failure)
        }
    }

    static func reasonSummaryText(for run: LocalJSONStorage.ScheduledActionRunRecord) -> String {
        reasonText(for: run, includeRawFallback: true)
    }

    static func reasonDetailText(for run: LocalJSONStorage.ScheduledActionRunRecord) -> String {
        reasonText(for: run, includeRawFallback: true)
    }

    static func reasonText(for run: LocalJSONStorage.ScheduledActionRunRecord, includeRawFallback: Bool) -> String {
        let code = normalizeReasonCode(run.reason)
        guard !code.isEmpty else { return "" }

        switch code {
        case "quiet_hours":
            return L10n.scheduledActionRunReasonQuietHours
        case "not_eligible_yet":
            return L10n.scheduledActionRunReasonQuietHours
        case "no_events":
            return L10n.scheduledActionRunReasonNoEvents
        case "deduped":
            return L10n.scheduledActionRunReasonDeduped
        case "fetch_failed":
            return L10n.scheduledActionRunReasonFetchFailed
        case "fetch_too_large":
            return L10n.scheduledActionRunReasonFetchTooLarge
        case "fetch_blocked":
            return L10n.scheduledActionRunReasonFetchBlocked
        case "notification_failed":
            return L10n.scheduledActionRunReasonExecutionFailed
        case "llm_failed":
            return L10n.scheduledActionRunReasonExecutionFailed
        case "execution_failed":
            return L10n.scheduledActionRunReasonExecutionFailed
        case "server_processing_loading_source":
            return NSLocalizedString("scheduled_actions.run_reason.server_processing_loading_source", value: "Server processing: loading task configuration", comment: "Server processing loading source reason")
        case "server_processing_fetching_url":
            return NSLocalizedString("scheduled_actions.run_reason.server_processing_fetching_url", value: "Server processing: reading data source content", comment: "Server processing fetching url reason")
        case "server_processing_llm_generating":
            return NSLocalizedString("scheduled_actions.run_reason.server_processing_llm_generating", value: "Server processing: generating summary", comment: "Server processing llm generating reason")
        case "server_processing_persisting_result":
            return NSLocalizedString("scheduled_actions.run_reason.server_processing_persisting_result", value: "Server processing: writing result", comment: "Server processing persisting result reason")
        case "server_processing":
            return NSLocalizedString("scheduled_actions.run_reason.server_processing", value: "Server processing: task has started", comment: "Server processing reason")
        default:
            if let failureReason = run.failureReason?.trimmingCharacters(in: .whitespacesAndNewlines), !failureReason.isEmpty {
                return failureReason
            }
            return includeRawFallback ? code : ""
        }
    }

    static func detailsText(for run: LocalJSONStorage.ScheduledActionRunRecord) -> String {
        if let failureReason = run.failureReason?.trimmingCharacters(in: .whitespacesAndNewlines), !failureReason.isEmpty {
            return failureReason
        }
        guard let raw = run.details, !raw.isEmpty else { return "" }
        let kv = parseKeyValuePairs(raw)

        if let msg = kv["reasonMessage"], !msg.isEmpty {
            return msg
        }
        if let fetchError = kv["fetchError"], !fetchError.isEmpty {
            return fetchError
        }
        if let llmError = kv["llmError"], !llmError.isEmpty {
            return llmError
        }
        if let httpStatus = kv["httpStatus"], !httpStatus.isEmpty {
            if let contentType = kv["contentType"], !contentType.isEmpty {
                return "HTTP \(httpStatus), \(contentType)"
            }
            return "HTTP \(httpStatus)"
        }

        return raw
    }

    private static func parseKeyValuePairs(_ s: String) -> [String: String] {
        var out: [String: String] = [:]
        for part in s.split(separator: ",") {
            let kv = part.split(separator: "=", maxSplits: 1)
            guard kv.count == 2 else { continue }
            let k = String(kv[0]).trimmingCharacters(in: .whitespacesAndNewlines)
            let v = String(kv[1]).trimmingCharacters(in: .whitespacesAndNewlines)
            guard !k.isEmpty, !v.isEmpty else { continue }
            out[k] = v
        }
        return out
    }

    private static func normalizeReasonCode(_ raw: String?) -> String {
        guard let raw else { return "" }
        return raw
            .trimmingCharacters(in: .whitespacesAndNewlines)
            .lowercased()
            .replacingOccurrences(of: "-", with: "_")
            .replacingOccurrences(of: " ", with: "_")
    }
}
