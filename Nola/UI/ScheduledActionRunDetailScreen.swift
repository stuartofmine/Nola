// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Implements ScheduledActionRunDetailScreen logic in the iOS UI layer.
// - Interfaces:
//   - ScheduledActionRunDetailScreen
//   - JournalDetailSheet
// - APIs:
//   - none
//
import SwiftUI

struct ScheduledActionRunDetailScreen: View {
    let run: LocalJSONStorage.ScheduledActionRunRecord

    @EnvironmentObject var container: ServiceContainer

    @State private var journal: LocalJSONStorage.JournalRecord? = nil
    @State private var journalLoadError: String? = nil
    @State private var selectedJournalId: String? = nil
    @State private var showJournalSheet: Bool = false
    @State private var showsTechnicalDetails = false

    var body: some View {
        List {
            overviewSection
            resultSection
            errorSection
            sourceSection
            timeSection
            technicalSection
        }
        .listStyle(.insetGrouped)
        .scrollContentBackground(.hidden)
        .background(DesignTokens.backgroundColor)
        .navigationTitle(L10n.scheduledActionRunDetailTitle)
        .navigationBarTitleDisplayMode(.inline)
        .task(id: run.resultJournalId) {
            await loadJournalIfNeeded()
        }
        .sheet(isPresented: $showJournalSheet, onDismiss: { selectedJournalId = nil }) {
            if let journalId = selectedJournalId {
                JournalDetailSheet(journalId: journalId)
                    .environmentObject(container)
            } else {
                EmptyView()
            }
        }
    }

    private var overviewSection: some View {
        Section {
            NolaCard {
                ViewThatFits(in: .horizontal) {
                    HStack(alignment: .top, spacing: 12) {
                        overviewIcon
                        overviewCopy
                        Spacer(minLength: 8)
                        overviewStatusBadge
                    }
                    VStack(alignment: .leading, spacing: 10) {
                        HStack(alignment: .center) {
                            overviewIcon
                            Spacer(minLength: 8)
                            overviewStatusBadge
                        }
                        overviewCopy
                    }
                }

                ViewThatFits(in: .horizontal) {
                    HStack(spacing: 12) { overviewMetadata }
                    VStack(alignment: .leading, spacing: 6) { overviewMetadata }
                }
                .font(.caption)
                .foregroundStyle(.secondary)
            }
            .listRowInsets(EdgeInsets(top: 8, leading: 16, bottom: 8, trailing: 16))
            .listRowBackground(Color.clear)
        }
    }

    private var overviewIcon: some View {
        Image(systemName: statusSymbol)
            .font(.system(size: 17, weight: .semibold))
            .foregroundStyle(statusColor)
            .frame(width: 40, height: 40)
            .background(statusColor.opacity(0.11), in: RoundedRectangle(cornerRadius: 13, style: .continuous))
    }

    private var overviewCopy: some View {
        VStack(alignment: .leading, spacing: 4) {
            Text(runTitle)
                .font(.headline)
                .fixedSize(horizontal: false, vertical: true)
            Text(NSLocalizedString("scheduled.run_detail.overview.title", value: "Run overview", comment: "Run overview subtitle"))
                .font(.caption)
                .foregroundStyle(.secondary)
        }
        .frame(maxWidth: .infinity, alignment: .leading)
    }

    private var overviewStatusBadge: some View {
        Text(Self.statusText(run.status))
            .font(.caption2.weight(.semibold))
            .foregroundStyle(statusColor)
            .padding(.horizontal, 8)
            .padding(.vertical, 5)
            .background(statusColor.opacity(0.1), in: Capsule())
    }

    @ViewBuilder
    private var overviewMetadata: some View {
        Label(triggerText, systemImage: "bolt")
        Label(executionPathText, systemImage: "point.3.connected.trianglepath.dotted")
        Label(Self.durationText(run.startedAt, run.finishedAt), systemImage: "timer")
    }

    private var runTitle: String {
        run.actionName?.takeUnlessEmpty
            ?? run.actionUrl?.takeUnlessEmpty
            ?? run.actionType
    }

    private var statusColor: Color {
        switch run.status {
        case .success: return DesignTokens.successColor
        case .partial: return DesignTokens.warningColor
        case .skipped: return .secondary
        case .failed: return DesignTokens.dangerColor
        case .actionCompleted: return DesignTokens.successColor
        case .answered, .delegationAccepted, .cancelled: return .secondary
        case .incomplete, .blocked: return DesignTokens.warningColor
        }
    }

    private var statusSymbol: String {
        switch run.status {
        case .success: return "checkmark.circle.fill"
        case .partial: return "exclamationmark.circle.fill"
        case .skipped: return "forward.circle.fill"
        case .failed: return "xmark.circle.fill"
        case .actionCompleted: return "checkmark.circle.fill"
        case .answered: return "text.bubble"
        case .delegationAccepted: return "arrow.triangle.branch"
        case .cancelled: return "stop.circle"
        case .incomplete, .blocked: return "exclamationmark.circle.fill"
        }
    }

    private var statusSection: some View {
        Section(L10n.scheduledActionRunDetailSectionStatus) {
            LabeledContent(L10n.scheduledActionRunDetailLabelResult, value: Self.statusText(run.status))
            LabeledContent(
                NSLocalizedString("scheduled.run_detail.status.trigger", value: "触发方式", comment: "Run detail trigger label"),
                value: triggerText
            )
            LabeledContent(
                NSLocalizedString("scheduled.run_detail.status.path", value: "执行路径", comment: "Run detail execution path label"),
                value: executionPathText
            )
        }
    }

    private var triggerText: String {
        switch run.trigger ?? (run.isBackground ? "push" : "manual") {
        case "manual", "manualRunNow":
            return NSLocalizedString("scheduled.run_detail.trigger.manual", value: "手动立即执行", comment: "Manual trigger text")
        case "push":
            return NSLocalizedString("scheduled.run_detail.trigger.push", value: "服务器推送", comment: "Push trigger text")
        case "background":
            return NSLocalizedString("scheduled.run_detail.trigger.background", value: "定时后台", comment: "Background trigger text")
        default:
            return run.trigger ?? (run.isBackground
                ? NSLocalizedString("scheduled.run_detail.trigger.fallback.background", value: "后台", comment: "Background fallback text")
                : NSLocalizedString("scheduled.run_detail.trigger.fallback.manual", value: "手动", comment: "Manual fallback text"))
        }
    }

    private var executionPathText: String {
        switch run.executionPath {
        case "server":
            return NSLocalizedString("scheduled.run_detail.path.server", value: "服务器", comment: "Run detail server path")
        case "local":
            return NSLocalizedString("scheduled.run_detail.path.local", value: "本地", comment: "Run detail local path")
        default:
            return run.executionPath ?? (run.isBackground
                ? NSLocalizedString("scheduled.run_detail.path.server", value: "服务器", comment: "Run detail server path")
                : NSLocalizedString("scheduled.run_detail.path.local", value: "本地", comment: "Run detail local path"))
        }
    }

    // Comment in English.

    private var timeSection: some View {
        Section(L10n.scheduledActionRunDetailSectionTime) {
            LabeledContent(L10n.scheduledActionRunDetailLabelStart, value: Self.formatDate(run.startedAt))
            LabeledContent(L10n.scheduledActionRunDetailLabelEnd, value: Self.formatDate(run.finishedAt))
            LabeledContent(L10n.scheduledActionRunDetailLabelDuration, value: Self.durationText(run.startedAt, run.finishedAt))
            if let nextEligible = run.nextEligibleAt {
                LabeledContent(L10n.scheduledActionRunDetailLabelNextEligible, value: Self.formatDate(nextEligible))
            }
        }
    }

    // Comment in English.

    private var sourceSection: some View {
        Section(NSLocalizedString("scheduled.run_detail.source.section", value: "数据源", comment: "Run detail source section")) {
            if let name = run.actionName, !name.isEmpty {
                LabeledContent(NSLocalizedString("scheduled.run_detail.source.name", value: "名称", comment: "Run detail source name"), value: name)
            }
            if let url = run.actionUrl, !url.isEmpty {
                VStack(alignment: .leading, spacing: 2) {
                    Text(NSLocalizedString("scheduled.run_detail.source.url", value: "地址", comment: "Run detail source URL"))
                        .font(.caption)
                        .foregroundColor(.secondary)
                    if let sourceURL {
                        Link(destination: sourceURL) {
                            HStack(alignment: .firstTextBaseline, spacing: 5) {
                                Text(url)
                                    .lineLimit(3)
                                Image(systemName: "arrow.up.right")
                                    .font(.caption2.weight(.semibold))
                            }
                        }
                        .font(.callout)
                    } else {
                        Text(url)
                            .font(.callout)
                            .textSelection(.enabled)
                    }
                }
                .padding(.vertical, 2)
            }
        }
    }

    // Comment in English.

    @ViewBuilder
    private var resultSection: some View {
        let hasContent = (run.eventsCount ?? 0) > 0 || run.resultJournalId != nil
        if hasContent {
            Section(L10n.scheduledActionRunDetailSectionResult) {
                if let count = run.eventsCount, count >= 0 {
                    LabeledContent(
                        L10n.scheduledActionRunDetailLabelEventsCount,
                        value: String(
                            format: NSLocalizedString("scheduled.run_detail.events_count", value: "%d 条", comment: "Run detail events count"),
                            count
                        )
                    )
                }
                journalResultContent
            }
        }
    }

    @ViewBuilder
    private var journalResultContent: some View {
        if let journalId = run.resultJournalId, !journalId.isEmpty {
            if let err = journalLoadError {
                VStack(alignment: .leading, spacing: 8) {
                    Text(L10n.format("scheduled_actions.journal.load_failed_format", err))
                        .font(.caption)
                        .foregroundColor(DesignTokens.dangerColor)
                    Button(NSLocalizedString("common.retry", value: "Retry", comment: "Retry action")) {
                        Task { await loadJournalIfNeeded() }
                    }
                    .buttonStyle(.bordered)
                    .controlSize(.small)
                }
            } else if let journal {
                Text(journal.content.nolaSanitizedSummary())
                    .font(.callout)
                    .foregroundColor(.primary)
                    .textSelection(.enabled)
                    .lineLimit(6)

                Button {
                    selectedJournalId = journalId
                    showJournalSheet = true
                } label: {
                    HStack {
                        Text(L10n.scheduledActionRunDetailViewAnalysis)
                        Spacer()
                        Image(systemName: "book.pages")
                        Image(systemName: "chevron.forward")
                            .font(.caption)
                            .foregroundColor(.secondary)
                    }
                }
                .foregroundColor(DesignTokens.primaryColor)
            } else {
                ProgressView()
            }
        }
    }

    private var sourceURL: URL? {
        guard let raw = run.actionUrl,
              let url = URL(string: raw),
              let scheme = url.scheme?.lowercased(),
              scheme == "http" || scheme == "https" else {
            return nil
        }
        return url
    }

    @ViewBuilder
    private var technicalSection: some View {
        let hasProcessing = run.tokensUsed != nil
            || run.processingDurationSec != nil
            || run.fetchedContentBytes != nil
            || run.serverResultId?.isEmpty == false
            || run.modelUsed?.isEmpty == false
        let hasNotifications = run.notificationSent != nil
            || run.notificationError?.isEmpty == false
            || run.resultEventId?.isEmpty == false
        let hasRetry = run.shouldRetry != nil
        if hasProcessing || hasNotifications || hasRetry {
            Section {
                DisclosureGroup(isExpanded: $showsTechnicalDetails) {
                    if let model = run.modelUsed?.takeUnlessEmpty {
                        LabeledContent(L10n.scheduledActionRunDetailLabelModel, value: model)
                    }
                    if let tokens = run.tokensUsed {
                        LabeledContent(
                            NSLocalizedString("scheduled.run_detail.processing.tokens", value: "Token usage", comment: "Run detail token usage label"),
                            value: String(format: NSLocalizedString("scheduled.run_detail.processing.tokens_value", value: "%d tokens", comment: "Run detail token usage value"), tokens)
                        )
                    }
                    if let sec = run.processingDurationSec {
                        LabeledContent(
                            NSLocalizedString("scheduled.run_detail.processing.network_duration", value: "Network duration", comment: "Run detail network duration label"),
                            value: String(format: NSLocalizedString("scheduled.run_detail.processing.network_duration_value", value: "%.1f seconds", comment: "Run detail network duration value"), sec)
                        )
                    }
                    if let bytes = run.fetchedContentBytes {
                        LabeledContent(
                            NSLocalizedString("scheduled.run_detail.processing.content_size", value: "Fetched content size", comment: "Run detail fetched content size label"),
                            value: Self.formatBytes(bytes)
                        )
                    }
                    if let resultID = run.serverResultId?.takeUnlessEmpty {
                        LabeledContent(NSLocalizedString("scheduled.run_detail.processing.result_id", value: "Result ID", comment: "Run detail result ID label"), value: resultID)
                            .textSelection(.enabled)
                    }
                    if let sent = run.notificationSent {
                        LabeledContent(L10n.scheduledActionRunDetailLabelNotificationSent, value: sent ? L10n.yes : L10n.no)
                    }
                    if let notificationError = run.notificationError?.takeUnlessEmpty {
                        Text(notificationError)
                            .font(.callout)
                            .foregroundStyle(DesignTokens.dangerColor)
                            .textSelection(.enabled)
                    }
                    if let eventID = run.resultEventId?.takeUnlessEmpty {
                        LabeledContent(L10n.scheduledActionRunDetailLabelEventID, value: eventID)
                            .textSelection(.enabled)
                    }
                    if let shouldRetry = run.shouldRetry {
                        LabeledContent(L10n.scheduledActionRunDetailLabelRetryAllowed, value: shouldRetry ? L10n.yes : L10n.no)
                    }
                    if let retryCount = run.retryCount, retryCount > 0 {
                        LabeledContent(L10n.scheduledActionRunDetailLabelRetryCount, value: L10n.format("scheduled_actions.run_detail.retry_count", retryCount))
                    }
                } label: {
                    Label(
                        NSLocalizedString("scheduled.run_detail.technical.title", value: "Technical details", comment: "Run technical details disclosure"),
                        systemImage: "wrench.and.screwdriver"
                    )
                }
                .disclosureGroupStyle(NolaRowDisclosureStyle())
            }
        }
    }

    // Comment in English.

    @ViewBuilder
    private var errorSection: some View {
        let hasReason = run.reason?.isEmpty == false
        let hasFailure = run.failureReason?.isEmpty == false || run.errorMessage?.isEmpty == false
        let hasDetailsExtra = run.details.flatMap { ScheduledActionRunText.detailsText(for: run).isEmpty ? nil : $0 } != nil
        if hasReason || hasFailure || hasDetailsExtra {
            Section(NSLocalizedString("scheduled.run_detail.error.section", value: "错误 / 说明", comment: "Run detail error section")) {
                // Translated reason
                if let reason = run.reason, !reason.isEmpty {
                    let text = ScheduledActionRunText.reasonDetailText(for: run)
                    if !text.isEmpty {
                        Text(text)
                            .font(.callout)
                            .textSelection(.enabled)
                    }
                }
                // Primary user-facing error
                if let err = run.errorMessage, !err.isEmpty {
                    Text(err)
                        .font(.callout)
                        .foregroundColor(DesignTokens.dangerColor)
                        .textSelection(.enabled)
                } else if let fr = run.failureReason, !fr.isEmpty {
                    Text(fr)
                        .font(.callout)
                        .foregroundColor(DesignTokens.dangerColor)
                        .textSelection(.enabled)
                }
                // Supplemental parsed details from server diagnostics.
                let detailsText = ScheduledActionRunText.detailsText(for: run)
                if !detailsText.isEmpty,
                   detailsText != run.errorMessage,
                   detailsText != run.failureReason {
                    Text(detailsText)
                        .font(.caption)
                        .foregroundColor(.secondary)
                        .textSelection(.enabled)
                }
            }
        }
    }

    // Comment in English.

    @ViewBuilder
    private var processingSection: some View {
        let hasTokens = run.tokensUsed != nil
        let hasDuration = run.processingDurationSec != nil
        let hasBytes = run.fetchedContentBytes != nil
        let hasResultId = run.serverResultId?.isEmpty == false
        let hasModel = run.modelUsed?.isEmpty == false
        if hasTokens || hasDuration || hasBytes || hasResultId || hasModel {
            Section(NSLocalizedString("scheduled.run_detail.processing.section", value: "处理信息", comment: "Run detail processing section")) {
                if let model = run.modelUsed, !model.isEmpty {
                    LabeledContent(L10n.scheduledActionRunDetailLabelModel, value: model)
                }
                if let tokens = run.tokensUsed {
                    LabeledContent(
                        NSLocalizedString("scheduled.run_detail.processing.tokens", value: "Token 用量", comment: "Run detail token usage label"),
                        value: String(
                            format: NSLocalizedString("scheduled.run_detail.processing.tokens_value", value: "%d tokens", comment: "Run detail token usage value"),
                            tokens
                        )
                    )
                }
                if let sec = run.processingDurationSec {
                    LabeledContent(
                        NSLocalizedString("scheduled.run_detail.processing.network_duration", value: "网络耗时", comment: "Run detail network duration label"),
                        value: String(
                            format: NSLocalizedString("scheduled.run_detail.processing.network_duration_value", value: "%.1f 秒", comment: "Run detail network duration value"),
                            sec
                        )
                    )
                }
                if let bytes = run.fetchedContentBytes {
                    LabeledContent(
                        NSLocalizedString("scheduled.run_detail.processing.content_size", value: "抓取内容大小", comment: "Run detail fetched content size label"),
                        value: Self.formatBytes(bytes)
                    )
                }
                if let rid = run.serverResultId, !rid.isEmpty {
                    LabeledContent(NSLocalizedString("scheduled.run_detail.processing.result_id", value: "结果 ID", comment: "Run detail result ID label"), value: rid)
                        .textSelection(.enabled)
                }
            }
        }
    }

    // Comment in English.

    @ViewBuilder
    private var notificationsSection: some View {
        let hasSent = run.notificationSent != nil
        let hasError = run.notificationError?.isEmpty == false
        let hasEventId = run.resultEventId?.isEmpty == false
        if hasSent || hasError || hasEventId {
            Section(L10n.scheduledActionRunDetailSectionNotifications) {
                if let sent = run.notificationSent {
                    LabeledContent(L10n.scheduledActionRunDetailLabelNotificationSent, value: sent ? L10n.yes : L10n.no)
                }
                if let e = run.notificationError, !e.isEmpty {
                    Text(e)
                        .font(.callout)
                        .foregroundColor(DesignTokens.dangerColor)
                        .textSelection(.enabled)
                }
                if let id = run.resultEventId, !id.isEmpty {
                    LabeledContent(L10n.scheduledActionRunDetailLabelEventID, value: id)
                }
            }
        }
    }

    // Comment in English.

    @ViewBuilder
    private var retrySection: some View {
        if let shouldRetry = run.shouldRetry {
            Section(L10n.scheduledActionRunDetailSectionRetry) {
                LabeledContent(L10n.scheduledActionRunDetailLabelRetryAllowed, value: shouldRetry ? L10n.yes : L10n.no)
                if let retryCount = run.retryCount, retryCount > 0 {
                    LabeledContent(L10n.scheduledActionRunDetailLabelRetryCount, value: L10n.format("scheduled_actions.run_detail.retry_count", retryCount))
                }
            }
        }
    }

    // MARK: - Helpers

    @MainActor
    private func loadJournalIfNeeded() async {
        journal = nil
        journalLoadError = nil

        guard let journalId = run.resultJournalId, !journalId.isEmpty else { return }
        guard let storage = container.localJSONStorage else {
            journalLoadError = NSLocalizedString("scheduled_actions.journal.storage_unavailable", value: "Local result storage is unavailable.", comment: "Scheduled result journal storage unavailable")
            return
        }

        do {
            if let loaded = try storage.getJournal(id: journalId) {
                journal = loaded
            } else {
                journalLoadError = NSLocalizedString("scheduled_actions.journal.not_found", value: "This result is no longer available.", comment: "Scheduled result journal missing")
            }
        } catch {
            journalLoadError = error.localizedDescription
        }
    }

    private static func formatDate(_ date: Date) -> String {
        let formatter = DateFormatter()
        formatter.dateStyle = .medium
        formatter.timeStyle = .medium
        return formatter.string(from: date)
    }

    private static func durationText(_ start: Date, _ end: Date) -> String {
        let seconds = max(0, end.timeIntervalSince(start))
        if seconds < 1 { return "<1s" }
        if seconds < 60 { return "\(Int(seconds))s" }
        return String(format: "%.1fm", seconds / 60.0)
    }

    private static func formatBytes(_ bytes: Int) -> String {
        if bytes < 1024 { return "\(bytes) B" }
        if bytes < 1024 * 1024 { return String(format: "%.1f KB", Double(bytes) / 1024.0) }
        return String(format: "%.1f MB", Double(bytes) / (1024.0 * 1024.0))
    }

    private static func statusText(_ status: LocalJSONStorage.ScheduledActionRunRecord.Status) -> String {
        switch status {
        case .success: return L10n.scheduledActionsOverviewStatusOk
        case .partial: return L10n.scheduledActionsOverviewStatusPartial
        case .skipped: return L10n.scheduledActionsOverviewStatusSkipped
        case .failed: return L10n.scheduledActionsOverviewStatusFailed
        case .answered: return String(localized: "Answered")
        case .delegationAccepted: return String(localized: "Delegated")
        case .actionCompleted: return String(localized: "Action completed")
        case .incomplete: return String(localized: "Incomplete")
        case .blocked: return String(localized: "Blocked")
        case .cancelled: return String(localized: "Cancelled")
        }
    }
}

private extension String {
    var takeUnlessEmpty: String? { isEmpty ? nil : self }
}

private struct JournalDetailSheet: View {
    let journalId: String
    @EnvironmentObject var container: ServiceContainer
    @Environment(\.dismiss) var dismiss
    @State private var journal: LocalJSONStorage.JournalRecord? = nil
    @State private var loadError: String? = nil

    var body: some View {
        NavigationStack {
            ScrollView {
                VStack(alignment: .leading, spacing: 16) {
                    if let error = loadError {
                        VStack(alignment: .leading, spacing: 10) {
                            Text(L10n.format("scheduled_actions.journal.load_failed_format", error))
                                .foregroundColor(DesignTokens.dangerColor)
                                .font(.caption)
                            Button(NSLocalizedString("common.retry", value: "Retry", comment: "Retry action")) {
                                Task { await load() }
                            }
                            .buttonStyle(.bordered)
                        }
                    } else if let journal = journal {
                        VStack(alignment: .leading, spacing: 8) {
                            Text(journal.title)
                                .font(.title2)
                                .fontWeight(.bold)

                            HStack {
                                Text(Self.formatDate(journal.createdAt))
                                    .font(.caption)
                                    .foregroundColor(.secondary)
                                Spacer()
                            }

                            ChatSmoothStreamingMarkdownView(content: journal.content, isStreaming: false)
                                .font(.body)
                                .textSelection(.enabled)
                        }
                    } else {
                        ProgressView()
                            .frame(maxWidth: .infinity)
                            .padding(.vertical, 30)
                    }
                }
                .padding(16)
            }
            .navigationTitle(L10n.scheduledActionRunDetailViewAnalysis)
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .navigationBarTrailing) {
                    Button(L10n.close) { dismiss() }
                }
            }
            .task {
                await load()
            }
        }
    }

    @MainActor
    private func load() async {
        guard let storage = container.localJSONStorage else {
            journal = nil
            loadError = NSLocalizedString("scheduled_actions.journal.storage_unavailable", value: "Local result storage is unavailable.", comment: "Scheduled result journal storage unavailable")
            return
        }
        do {
            if let loaded = try storage.getJournal(id: journalId) {
                journal = loaded
                loadError = nil
            } else {
                journal = nil
                loadError = NSLocalizedString("scheduled_actions.journal.not_found", value: "This result is no longer available.", comment: "Scheduled result journal missing")
            }
        } catch {
            journal = nil
            loadError = error.localizedDescription
        }
    }

    private static func formatDate(_ date: Date) -> String {
        let formatter = DateFormatter()
        formatter.dateStyle = .medium
        formatter.timeStyle = .medium
        return formatter.string(from: date)
    }
}
