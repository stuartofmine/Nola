// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Implements URLDataSourcesView logic in the iOS UI layer.
// - Interfaces:
//   - URLDataSourcesView
//   - URLActionRow
//   - AlertState
//   - RunNowFeedback
//   - AddURLDataSourceSheet
// - APIs:
//   - none
//
//
//  URLDataSourcesView.swift
//  Nola
//
//  URL data sources management view.
//

import SwiftUI
import UIKit
import UniformTypeIdentifiers

struct URLDataSourcesView: View {
    @EnvironmentObject var container: ServiceContainer
    @Environment(\.scenePhase) private var scenePhase
    @State private var showingAddSheet = false
    @State private var editingSource: URLDataSource? = nil
    @State private var refreshErrorMessage: String? = nil

    @State private var runningSourceIds: Set<UUID> = []
    @State private var alertState: AlertState? = nil
    @State private var pendingDeleteSource: URLDataSource? = nil
    @State private var refreshNonce: Int = 0
    @State private var runNowFeedback: RunNowFeedback? = nil
    
    var body: some View {
        applyAlerts(to: applySheets(to: baseView))
    }

    private var baseView: some View {
        contentScroll
            .background(Color(.systemGroupedBackground))
            .navigationTitle(NSLocalizedString("url_sources.navigation.title", value: "Data sources", comment: "URL sources navigation title"))
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .primaryAction) {
                    Button {
                        showingAddSheet = true
                    } label: {
                        Image(systemName: "plus")
                    }
                    .disabled(sources.count >= ClientConfig.shared.effectiveURLLimit())
                }
            }
            .onAppear {
                refreshNonce += 1
            }
            .onReceive(NotificationCenter.default.publisher(for: .nolaCustomSourcesUpdated)) { _ in
                refreshNonce += 1
            }
            .onReceive(NotificationCenter.default.publisher(for: .nolaScheduledActionRunsUpdated)) { _ in
                refreshNonce += 1
            }
            .onChange(of: scenePhase) { _, newPhase in
                guard newPhase == .active else { return }
                refreshNonce += 1
            }
    }

    private func applySheets<V: View>(to view: V) -> some View {
        view
            .sheet(isPresented: $showingAddSheet, onDismiss: {
                refreshNonce += 1
                NotificationCenter.default.post(name: .nolaCustomSourcesUpdated, object: nil)
            }) {
                AddURLDataSourceSheet(existingSource: nil)
                    .environmentObject(container)
            }
            .sheet(item: $editingSource, onDismiss: {
                refreshNonce += 1
                NotificationCenter.default.post(name: .nolaCustomSourcesUpdated, object: nil)
            }) { source in
                AddURLDataSourceSheet(existingSource: source)
                    .environmentObject(container)
            }
    }

    private func applyAlerts<V: View>(to view: V) -> some View {
        view
            .alert(L10n.alertTitle, isPresented: Binding(get: { refreshErrorMessage != nil }, set: { if !$0 { refreshErrorMessage = nil } })) {
                Button(L10n.ok, role: .cancel) {}
            } message: {
                Text(refreshErrorMessage ?? "")
            }
            .alert(item: $alertState) { state in
                if state.allowRetry, let sourceId = state.sourceId {
                    return Alert(
                        title: Text(state.title),
                        message: state.message.map(Text.init),
                        primaryButton: .default(Text(NSLocalizedString("common.retry", value: "Retry", comment: "Retry button"))) {
                            Task { await retryRunNow(sourceId: sourceId) }
                        },
                        secondaryButton: .cancel(Text(L10n.cancel))
                    )
                }

                if state.allowOpenSettings {
                    return Alert(
                        title: Text(state.title),
                        message: state.message.map(Text.init),
                        primaryButton: .default(Text(L10n.permissionGoToSettings)) {
                            openSystemSettings()
                        },
                        secondaryButton: .cancel(Text(L10n.cancel))
                    )
                }

                return Alert(
                    title: Text(state.title),
                    message: state.message.map(Text.init),
                    dismissButton: .default(Text(L10n.ok))
                )
            }
            .alert(L10n.urlSourcesDeleteTitle, isPresented: Binding(get: { pendingDeleteSource != nil }, set: { if !$0 { pendingDeleteSource = nil } })) {
                Button(L10n.urlSourcesDeleteCancel, role: .cancel) {}
                Button(L10n.urlSourcesDeleteConfirm, role: .destructive) {
                    guard let source = pendingDeleteSource else {
                        pendingDeleteSource = nil
                        return
                    }
                    pendingDeleteSource = nil
                    Task { @MainActor in
                        do {
                            guard let fetcher = container.urlFetcher else { throw IOSApplicationRuntimeError.applicationUnavailable }
                            try await fetcher.removeSourceConfirmed(id: source.id, expectedRevision: source.canonicalRevision)
                            refreshNonce += 1
                        } catch { refreshErrorMessage = error.localizedDescription }
                    }
                }
            } message: {
                Text(L10n.urlSourcesDeleteMessage.localized(pendingDeleteSource?.name ?? ""))
            }
    }

    private var contentScroll: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 14) {
                readableSourcesSection

                if backgroundRefreshStatus != .available {
                    backgroundRefreshSection
                }

                customSourcesSection

                if let feedback = runNowFeedback {
                    runNowFeedbackSection(feedback)
                }
            }
            .frame(maxWidth: .infinity, alignment: .leading)
            .padding(.horizontal, 16)
            .padding(.top, 12)
            .padding(.bottom, 24)
        }
    }

    private var readableSourcesSection: some View {
        DataSourcesCardSection(title: NSLocalizedString("url_sources.readable.section_title", value: "Readable sources", comment: "Readable data sources section title")) {
            VStack(alignment: .leading, spacing: 10) {
                Text(NSLocalizedString("url_sources.readable.section_subtitle", value: "Manage sources readable via URL/API: websites, search result pages, JSON APIs, RSS, and iCalendar. They can be injected into chat context and used in scheduled actions with notifications.", comment: "Readable data sources section subtitle"))
                    .font(.subheadline)
                    .foregroundColor(.secondary)
                    .fixedSize(horizontal: false, vertical: true)
                HStack {
                    Text(String(format: L10n.urlSourcesUsageFormat, sources.count, ClientConfig.shared.effectiveURLLimit()))
                        .foregroundColor(.secondary)
                    Spacer()
                }
            }
        }
    }

    private var backgroundRefreshSection: some View {
        DataSourcesCardSection(title: L10n.backgroundRefreshSectionTitle) {
            VStack(alignment: .leading, spacing: 10) {
                Text(L10n.backgroundRefreshDisabledMessage)
                    .font(.subheadline)
                    .foregroundColor(.secondary)

                if ProcessInfo.processInfo.isLowPowerModeEnabled {
                    Text(L10n.backgroundRefreshLowPowerModeNote)
                        .font(.caption)
                        .foregroundColor(.secondary)
                }

                Button(L10n.backgroundRefreshOpenSystemSettings) {
                    guard let url = URL(string: UIApplication.openSettingsURLString) else { return }
                    UIApplication.shared.open(url)
                }
                .buttonStyle(.bordered)

                Text(L10n.backgroundRefreshHint)
                    .font(.caption)
                    .foregroundColor(.secondary)
            }
        }
    }

    private var customSourcesSection: some View {
        DataSourcesCardSection(title: NSLocalizedString("url_sources.custom_sources.title", value: "Custom sources", comment: "Custom sources section title")) {
            if sources.isEmpty {
                NolaInlineEmptyState(
                    icon: "tray.full",
                    title: NSLocalizedString("url_sources.custom_sources.empty.title", value: "No custom sources yet", comment: "Custom sources empty title"),
                    message: NSLocalizedString("url_sources.custom_sources.empty.message", value: "Add websites, search-result URLs, APIs, or feeds. Then reference them in chat or use them in scheduled processing.", comment: "Custom sources empty message"),
                    tint: DesignTokens.primaryColor
                )
                .padding(.vertical, 4)
            } else {
                VStack(spacing: 0) {
                    ForEach(Array(sources.enumerated()), id: \.element.id) { idx, source in
                        URLActionRow(
                            source: source,
                            isRunningNow: runningSourceIds.contains(source.id),
                            onToggle: { toggleSource(source) },
                            onEdit: { editingSource = source },
                            onRefresh: { refreshSource(source) },
                            onRunNow: { Task { await runNow(source) } },
                            onDelete: { pendingDeleteSource = source },
                            scheduleText: scheduleText(for: source),
                            lastRunText: lastRunText(for: source)
                        )
                        if idx != sources.count - 1 {
                            NolaDivider(leading: 12)
                        }
                    }
                }
            }
        }
    }

    private func runNowFeedbackSection(_ feedback: RunNowFeedback) -> some View {
        DataSourcesCardSection(title: NSLocalizedString("url_sources.run_now_log.title", value: "Run now log", comment: "Run now log section title")) {
            VStack(alignment: .leading, spacing: 10) {
                HStack(spacing: 8) {
                    Image(systemName: feedback.isFailure ? "xmark.circle.fill" : "checkmark.circle.fill")
                        .foregroundColor(feedback.isFailure ? DesignTokens.dangerColor : DesignTokens.successColor)
                    Text(feedback.title)
                        .font(.subheadline)
                        .fontWeight(.semibold)
                    Spacer()
                    Text(feedback.time.formatted(date: .omitted, time: .standard))
                        .font(.caption2)
                        .foregroundColor(.secondary)
                }

                if !feedback.sourceName.isEmpty {
                    Text("\(NSLocalizedString("url_sources.run_now_log.source_prefix", value: "Source: ", comment: "Run now source prefix"))\(feedback.sourceName)")
                        .font(.caption)
                        .foregroundColor(.secondary)
                }

                if !feedback.message.isEmpty {
                    Text(feedback.message)
                        .font(.caption)
                        .foregroundColor(.secondary)
                        .textSelection(.enabled)
                }

                HStack(spacing: 10) {
                    if feedback.allowRetry, let sourceId = feedback.sourceId {
                        Button(NSLocalizedString("common.retry", value: "Retry", comment: "Retry button")) {
                            Task { await retryRunNow(sourceId: sourceId) }
                        }
                        .buttonStyle(.bordered)
                    }

                    if feedback.allowOpenSettings {
                        Button(L10n.permissionGoToSettings) {
                            openSystemSettings()
                        }
                        .buttonStyle(.bordered)
                    }

                    Button(L10n.close) {
                        runNowFeedback = nil
                    }
                    .buttonStyle(.borderless)
                }
            }
        }
    }

    private var sources: [URLDataSource] {
        _ = refreshNonce
        return container.urlFetcher?.sources ?? []
    }

    private var backgroundRefreshStatus: UIBackgroundRefreshStatus {
        UIApplication.shared.backgroundRefreshStatus
    }

    private func toggleSource(_ source: URLDataSource) {
        container.urlFetcher?.toggleSource(id: source.id)
        refreshNonce += 1
        NotificationCenter.default.post(name: .nolaCustomSourcesUpdated, object: nil)
    }

    private func isScheduled(_ source: URLDataSource) -> Bool {
        guard let freq = source.scheduleFrequency, !freq.isEmpty, freq != "none" else { return false }
        return source.scheduledTimeISO != nil
    }

    private func scheduleText(for source: URLDataSource) -> String? {
        guard isScheduled(source) else { return nil }
        let freqLabel: String = {
            switch source.scheduleFrequency {
            case "daily":
                return NSLocalizedString("common.frequency.daily", value: "Daily", comment: "Daily frequency")
            case "weekly":
                return NSLocalizedString("common.frequency.weekly", value: "Weekly", comment: "Weekly frequency")
            default:
                return source.scheduleFrequency ?? ""
            }
        }()
        let time = source.scheduledTimeISO ?? ""
        return "\(freqLabel) · \(time)"
    }

    private func lastRunText(for source: URLDataSource) -> String? {
        guard isScheduled(source) else { return nil }
        guard let storage = container.localJSONStorage else { return nil }
        let actionId = source.id.uuidString
        let runs = (try? storage.getScheduledActionRuns(limit: 200)) ?? []
        guard let latest = runs.first(where: { $0.actionId == actionId }) else { return nil }
        return ScheduledActionRunText.statusPresentation(latest.status).title
    }

    private func runNow(_ source: URLDataSource) async {
        guard isScheduled(source) else { return }
        guard !runningSourceIds.contains(source.id) else { return }
        runningSourceIds.insert(source.id)
        defer {
            runningSourceIds.remove(source.id)
        }

        let didRun = await container.scheduler.runScheduledURLActionNow(sourceId: source.id)
        refreshNonce += 1

        guard didRun else {
            runNowFeedback = RunNowFeedback(
                sourceId: source.id,
                sourceName: source.name,
                title: L10n.scheduledActionsRunNowNoActions,
                message: "",
                allowRetry: false,
                allowOpenSettings: false,
                isFailure: true
            )
            return
        }

        if let latest = await fetchLatestRunWithRetry(actionId: source.id.uuidString) {
            let outcome = buildRunNowOutcome(for: latest, sourceName: source.name)
            runNowFeedback = RunNowFeedback(
                sourceId: source.id,
                sourceName: source.name,
                title: outcome.title,
                message: outcome.message,
                allowRetry: outcome.allowRetry,
                allowOpenSettings: outcome.allowOpenSettings,
                isFailure: latest.status == .failed
            )
        } else {
            runNowFeedback = RunNowFeedback(
                sourceId: source.id,
                sourceName: source.name,
                title: L10n.scheduledActionsRunNowCompleted,
                message: L10n.scheduledActionsRunNowCompletedMessage,
                allowRetry: false,
                allowOpenSettings: false,
                isFailure: false
            )
        }
    }

    private func retryRunNow(sourceId: UUID) async {
        guard let source = container.urlFetcher?.sources.first(where: { $0.id == sourceId }) else { return }
        await runNow(source)
    }

    private func openSystemSettings() {
        guard let url = URL(string: UIApplication.openSettingsURLString) else { return }
        UIApplication.shared.open(url)
    }

    private func fetchLatestRun(actionId: String) -> LocalJSONStorage.ScheduledActionRunRecord? {
        guard let storage = container.localJSONStorage else { return nil }
        return (try? storage.getScheduledActionRuns(limit: 1, actionId: actionId))?.first
    }

    private func fetchLatestRunWithRetry(actionId: String) async -> LocalJSONStorage.ScheduledActionRunRecord? {
        if let run = fetchLatestRun(actionId: actionId) { return run }
        for _ in 0..<4 {
            try? await Task.sleep(nanoseconds: 150_000_000)
            if let run = fetchLatestRun(actionId: actionId) { return run }
        }
        return nil
    }

    private func buildRunNowOutcome(for run: LocalJSONStorage.ScheduledActionRunRecord, sourceName: String) -> (title: String, message: String, allowRetry: Bool, allowOpenSettings: Bool) {
        let base: String
        switch run.status {
        case .success:
            base = L10n.scheduledActionsRunNowCompletedMessage
        case .partial:
            base = L10n.scheduledActionsRunNowResultPartialMessage
        case .skipped:
            base = L10n.scheduledActionsRunNowResultSkippedMessage
        case .failed:
            base = L10n.scheduledActionsRunNowResultFailedMessage
        case .answered, .actionCompleted, .delegationAccepted, .incomplete, .blocked, .cancelled:
            base = ScheduledActionRunText.statusPresentation(run.status).title
        }

        var details: [String] = []
        var allowRetry = false
        var allowOpenSettings = false
        let structuredFailure = structuredFailureText(from: run)
        if let reason = run.reason, !reason.isEmpty {
            let reasonText: String = {
                switch reason {
                case "quiet_hours": return L10n.scheduledActionRunReasonQuietHours
                case "no_events": return L10n.scheduledActionRunReasonNoEvents
                case "deduped": return L10n.scheduledActionRunReasonDeduped
                case "not_eligible_yet": return NSLocalizedString("url_sources.reason.not_eligible_yet", value: "Backoff window active", comment: "Reason not eligible yet")
                case "fetch_failed":
                    allowRetry = true
                    return structuredFailure ?? L10n.scheduledActionRunReasonFetchFailed
                case "fetch_too_large":
                    allowRetry = false
                    return structuredFailure ?? L10n.scheduledActionRunReasonFetchTooLarge
                case "fetch_blocked":
                    allowRetry = false
                    return structuredFailure ?? L10n.scheduledActionRunReasonFetchBlocked
                case "llm_failed":
                    let err = (run.errorMessage ?? "").lowercased()
                    let tlsLike = err.contains("tls") || err.contains("ssl") || err.contains("certificate") || err.contains("-1200") || err.contains("-9816")
                    allowRetry = !tlsLike
                    if let structuredFailure { return structuredFailure }
                    return tlsLike
                        ? NSLocalizedString("url_sources.reason.llm_failed_tls", value: "Summary generation failed (TLS/certificate error, check proxy or certificate config)", comment: "LLM failed with TLS")
                        : NSLocalizedString("url_sources.reason.llm_failed", value: "Summary generation failed", comment: "LLM failed")
                case "notification_failed":
                    allowRetry = true
                    allowOpenSettings = true
                    return NSLocalizedString("url_sources.reason.notification_failed", value: "Notification delivery failed", comment: "Notification failed reason")
                case "server_auth_missing":
                    allowRetry = true
                    return NSLocalizedString("url_sources.reason.server_auth_missing", value: "Server execution failed: push account is not signed in", comment: "Server auth missing reason")
                case "server_run_failed":
                    allowRetry = true
                    return structuredFailure ?? NSLocalizedString("url_sources.reason.server_run_failed", value: "Server execution failed (gateway/service error)", comment: "Server run failed reason")
                case "server_timeout":
                    allowRetry = true
                    return structuredFailure ?? NSLocalizedString("url_sources.reason.server_timeout", value: "Server execution timed out (task may still finish later and push back results)", comment: "Server timeout reason")
                case "server_processing":
                    allowRetry = false
                    return structuredFailure ?? NSLocalizedString("url_sources.reason.server_processing", value: "Server is processing; results will be returned via push notification", comment: "Server processing reason")
                case "server_processing_loading_source":
                    allowRetry = false
                    return structuredFailure ?? NSLocalizedString("url_sources.reason.server_processing_loading_source", value: "Server processing: loading task configuration", comment: "Server processing loading source")
                case "server_processing_fetching_url":
                    allowRetry = false
                    return structuredFailure ?? NSLocalizedString("url_sources.reason.server_processing_fetching_url", value: "Server processing: reading data source content", comment: "Server processing fetching URL")
                case "server_processing_llm_generating":
                    allowRetry = false
                    return structuredFailure ?? NSLocalizedString("url_sources.reason.server_processing_llm_generating", value: "Server processing: generating summary", comment: "Server processing generating summary")
                case "server_processing_persisting_result":
                    allowRetry = false
                    return structuredFailure ?? NSLocalizedString("url_sources.reason.server_processing_persisting_result", value: "Server processing: writing result", comment: "Server processing persisting result")
                default: return String(format: NSLocalizedString("url_sources.reason.generic", value: "Reason: %@", comment: "Generic reason format"), reason)
                }
            }()
            details.append(reasonText)
        }
        if let count = run.eventsCount {
            details.append(String(format: NSLocalizedString("url_sources.details.events_count", value: "Events: %d", comment: "Events count detail"), count))
        }
        if let rawDetails = run.details, !rawDetails.isEmpty {
            if rawDetails.contains("executionPath=server") {
                details.append(NSLocalizedString("url_sources.details.execution_path.server", value: "Execution path: server", comment: "Execution path server"))
            } else if rawDetails.contains("executionPath=local") {
                details.append(NSLocalizedString("url_sources.details.execution_path.local", value: "Execution path: local", comment: "Execution path local"))
            }
        }
        if let next = run.nextEligibleAt {
            details.append(String(format: NSLocalizedString("url_sources.details.next_eligible", value: "Next eligible: %@", comment: "Next eligible detail"), next.formatted(date: .abbreviated, time: .shortened)))
        }
        if let err = run.errorMessage, !err.isEmpty {
            details.append(String(format: NSLocalizedString("url_sources.details.error", value: "Error: %@", comment: "Error detail"), err))
        }

        let message = ([String(format: NSLocalizedString("url_sources.run_now.source_line", value: "Source: %@", comment: "Run now source line"), sourceName), base] + details).joined(separator: "\n")
        let title = ScheduledActionRunText.statusPresentation(run.status).title
        return (title, message, allowRetry, allowOpenSettings)
    }

    private func structuredFailureText(from run: LocalJSONStorage.ScheduledActionRunRecord) -> String? {
        if let reason = run.failureReason?.trimmingCharacters(in: .whitespacesAndNewlines), !reason.isEmpty {
            return reason
        }

        guard let raw = run.details, !raw.isEmpty else {
            if let err = run.errorMessage?.trimmingCharacters(in: .whitespacesAndNewlines), !err.isEmpty {
                return err
            }
            return nil
        }

        var map: [String: String] = [:]
        for part in raw.split(separator: ",") {
            let kv = part.split(separator: "=", maxSplits: 1)
            guard kv.count == 2 else { continue }
            let k = String(kv[0]).trimmingCharacters(in: .whitespacesAndNewlines)
            let v = String(kv[1]).trimmingCharacters(in: .whitespacesAndNewlines)
            if !k.isEmpty, !v.isEmpty { map[k] = v }
        }

        if let msg = map["reasonMessage"], !msg.isEmpty { return msg }
        if let err = map["fetchError"], !err.isEmpty { return err }
        if let err = map["llmError"], !err.isEmpty { return err }
        if let status = map["httpStatus"], !status.isEmpty {
            if let ct = map["contentType"], !ct.isEmpty {
                return String(format: NSLocalizedString("url_sources.fetch_failed.with_content_type", value: "Fetch failed (HTTP %@, %@)", comment: "Fetch failed with status and content type"), status, ct)
            }
            return String(format: NSLocalizedString("url_sources.fetch_failed.with_status", value: "Fetch failed (HTTP %@)", comment: "Fetch failed with status"), status)
        }
        if let err = run.errorMessage?.trimmingCharacters(in: .whitespacesAndNewlines), !err.isEmpty { return err }
        return nil
    }
    
    private func refreshSource(_ source: URLDataSource) {
        guard let fetcher = container.urlFetcher else { return }

        Task {
            do {
                _ = try await fetcher.fetchEvents(from: source)
                await MainActor.run { refreshNonce += 1 }
            } catch {
                Logger().log(.error, "URLDataSourcesView", "Refresh failed", metadata: ["error": error.localizedDescription])
                await MainActor.run {
                    refreshErrorMessage = error.localizedDescription
                }
            }
        }
    }
}

private struct URLActionRow: View {
    let source: URLDataSource
    let isRunningNow: Bool
    let onToggle: () -> Void
    let onEdit: () -> Void
    let onRefresh: () -> Void
    let onRunNow: () -> Void
    let onDelete: () -> Void
    let scheduleText: String?
    let lastRunText: String?

    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            HStack(alignment: .top, spacing: 12) {
                Toggle("", isOn: Binding(get: { source.enabled }, set: { _ in onToggle() }))
                    .labelsHidden()
                    .padding(.top, 2)
                    .accessibilityLabel(
                        Text(
                            String(
                                format: NSLocalizedString("url_sources.row.toggle.accessibility", value: "Enable %@", comment: "Data source toggle accessibility label"),
                                source.name
                            )
                        )
                    )
                    .accessibilityValue(Text(source.enabled ? L10n.yes : L10n.no))

                sourceIdentity
            }

            sourceMetadata

            ViewThatFits(in: .horizontal) {
                HStack(spacing: 10) {
                    runNowButton
                    Spacer(minLength: 8)
                    actionButtons
                }
                VStack(alignment: .leading, spacing: 10) {
                    runNowButton
                    actionButtons
                }
            }
        }
        .padding(.vertical, 12)
        .frame(maxWidth: .infinity, alignment: .leading)
        .contentShape(Rectangle())
    }

    private var sourceIdentity: some View {
        VStack(alignment: .leading, spacing: 6) {
            Text(source.name)
                .font(.headline)
                .foregroundColor(.primary)
                .fixedSize(horizontal: false, vertical: true)

            Text(source.url)
                .font(.caption)
                .foregroundColor(.secondary)
                .lineLimit(2)
                .textSelection(.enabled)
        }
        .frame(maxWidth: .infinity, alignment: .leading)
    }

    private var sourceMetadata: some View {
        ViewThatFits(in: .horizontal) {
            HStack(spacing: 10) { metadataItems }
            VStack(alignment: .leading, spacing: 6) {
                metadataItems
            }
        }
        .font(.caption2)
        .foregroundColor(.secondary)
    }

    @ViewBuilder
    private var metadataItems: some View {
        Label(source.kind.displayName, systemImage: source.kind.iconSystemName)
        if let scheduleText {
            Label(scheduleText, systemImage: "clock")
        }
        if let lastRunText {
            Label(lastRunText, systemImage: "clock.badge.checkmark")
        }
    }

    @ViewBuilder
    private var runNowButton: some View {
        if scheduleText != nil {
            Button(action: onRunNow) {
                HStack(spacing: 8) {
                    if isRunningNow {
                        ProgressView()
                            .controlSize(.small)
                    }
                    Text(isRunningNow ? L10n.scheduledActionsRunning : L10n.scheduledActionsRunNow)
                        .font(.caption.weight(.semibold))
                }
            }
            .buttonStyle(.bordered)
            .disabled(!source.enabled || isRunningNow)
        }
    }

    private var actionButtons: some View {
        HStack(spacing: 16) {
            Button(action: onRefresh) {
                Image(systemName: "arrow.clockwise")
                    .foregroundColor(.secondary)
            }
            .buttonStyle(.plain)
            .disabled(!source.enabled)
            .accessibilityLabel(NSLocalizedString("common.refresh", value: "Refresh", comment: "Refresh action"))

            Button(action: onEdit) {
                Image(systemName: "pencil")
                    .foregroundColor(.secondary)
            }
            .buttonStyle(.plain)
            .accessibilityLabel(NSLocalizedString("common.edit", value: "Edit", comment: "Edit action"))

            Button(action: onDelete) {
                Image(systemName: "trash")
                    .foregroundColor(DesignTokens.dangerColor)
            }
            .buttonStyle(.plain)
            .accessibilityLabel(L10n.urlSourcesDeleteConfirm)
        }
        .font(.callout)
    }
}

private struct AlertState: Identifiable {
    let id = UUID()
    let title: String
    let message: String?
    let sourceId: UUID? = nil
    let allowRetry: Bool = false
    let allowOpenSettings: Bool = false
}

private struct RunNowFeedback {
    let sourceId: UUID?
    let sourceName: String
    let title: String
    let message: String
    let allowRetry: Bool
    let allowOpenSettings: Bool
    let isFailure: Bool
    let time: Date = Date()
}

struct AddURLDataSourceSheet: View {
    @EnvironmentObject var container: ServiceContainer
    @Environment(\.dismiss) var dismiss
    
    var existingSource: URLDataSource? // nil = add, non-nil = edit
    var onSaved: ((URLDataSource) -> Void)? = nil
    
    @State private var name = ""
    @State private var url = ""
    @State private var selectedFormat: URLDataFormat? = nil // User-selected format
    @State private var selectedKind: DataSourceKind = .website
    @State private var detectedFormat: URLDataFormat? = nil // Auto-detected format
    @State private var showFileImporter = false
    @State private var showFolderImporter = false
    @State private var isValidating = false
    @State private var validationTask: Task<Void, Never>?
    @State private var isSaving = false
    @State private var validationResult: ValidationResult?
    @State private var errorMessage: String?
    @State private var previewEvents: [RawEvent] = []
    @State private var sourceDescriptionText: String = ""
    @State private var sourceID = UUID()
    @State private var accessPolicy: URLDataSourceAccessPolicy = .conversationOptIn
    @State private var initialEditorSnapshot = ""
    @State private var validationRevision = 0
    @State private var showsDiscardConfirmation = false

    // Model selection is shared via container.chatManager

    // Schedule
    private var isEditMode: Bool { existingSource != nil }
    private var navTitle: String {
        NSLocalizedString(isEditMode ? "editor.edit" : "editor.add", tableName: "CanonicalDataSourceEditor", value: isEditMode ? "Edit data source" : "Add data source", comment: "Data source editor title")
    }
    private var actionButtonLabel: String { isEditMode ? L10n.urlSourcesSaveLabel : L10n.urlSourcesAddLabel }
    
    var body: some View {
        NavigationStack {
            Form {
                if let error = errorMessage {
                    Section {
                        Text(error)
                            .foregroundColor(DesignTokens.dangerColor)
                            .font(.callout)
                            .accessibilityIdentifier("data.center.editor.error")
                    }
                }
                Section(header: Text(NSLocalizedString("url_sources.form.section_source", value: "Source", comment: "Source section title"))) {
                    TextField(L10n.commonName, text: $name)
                        .accessibilityIdentifier("data.center.editor.name")
                    TextField(L10n.urlSourcesURLField, text: $url)
                        .textInputAutocapitalization(.never)
                        .autocorrectionDisabled()
                        .keyboardType(.URL)
                        .accessibilityIdentifier("data.center.editor.url")

                    Picker(NSLocalizedString("url_sources.form.source_kind", value: "Source type", comment: "Source kind picker title"), selection: $selectedKind) {
                        Text(DataSourceKind.website.displayName).tag(DataSourceKind.website)
                        Text(DataSourceKind.webSearch.displayName).tag(DataSourceKind.webSearch)
                        Text(DataSourceKind.api.displayName).tag(DataSourceKind.api)
                        Text(DataSourceKind.feed.displayName).tag(DataSourceKind.feed)
                        Text(DataSourceKind.calendarFeed.displayName).tag(DataSourceKind.calendarFeed)
                        Text(DataSourceKind.file.displayName).tag(DataSourceKind.file)
                        Text(DataSourceKind.folder.displayName).tag(DataSourceKind.folder)
                        Text(DataSourceKind.custom.displayName).tag(DataSourceKind.custom)
                    }
                    .pickerStyle(.menu)

                    Text(selectedKind.exampleText)
                        .font(.caption2)
                        .foregroundColor(.secondary)

                    if selectedKind == .file || selectedKind == .folder {
                        Button {
                            if selectedKind == .folder {
                                showFolderImporter = true
                            } else {
                                showFileImporter = true
                            }
                        } label: {
                            Label(
                                selectedKind == .folder
                                ? NSLocalizedString("url_sources.form.pick_folder", value: "Select local folder", comment: "Pick local folder")
                                : NSLocalizedString("url_sources.form.pick_file", value: "Select local file", comment: "Pick local file"),
                                systemImage: selectedKind.iconSystemName
                            )
                        }
                        Text(NSLocalizedString("url_sources.form.local_file_note", value: "Files and folders are read only on this device for chat context or local scheduled actions.", comment: "Local file/folder note"))
                            .font(.caption2)
                            .foregroundColor(.secondary)
                    }
                    
                    // Data type selection
                    VStack(alignment: .leading, spacing: 8) {
                        Picker(L10n.urlSourcesDataTypeTitle, selection: $selectedFormat) {
                            Text(L10n.urlSourcesFormatAutoDetect).tag(nil as URLDataFormat?)
                            Text(L10n.urlSourcesFormatHTML).tag(URLDataFormat.html as URLDataFormat?)
                            Text(L10n.urlSourcesFormatRSS).tag(URLDataFormat.rss as URLDataFormat?)
                            Text(L10n.urlSourcesFormatJSON).tag(URLDataFormat.json as URLDataFormat?)
                            Text(L10n.urlSourcesFormatiCalendar).tag(URLDataFormat.icalendar as URLDataFormat?)
                        }
                        .pickerStyle(.menu)
                        
                        if selectedFormat == nil {
                            if let fmt = detectedFormat {
                                HStack {
                                    Image(systemName: "checkmark.circle.fill")
                                        .foregroundColor(.green)
                                        .font(.caption2)
                                    Text(String(format: L10n.urlSourcesDetectedFormat, fmt.rawValue.uppercased()))
                                        .font(.caption2)
                                        .foregroundColor(.secondary)
                                }
                            } else {
                                Text(L10n.urlSourcesDataTypeWillDetect)
                                    .font(.caption2)
                                    .foregroundColor(.secondary)
                            }
                        } else {
                            Text(L10n.format("urlsources.data_type.will_use_format", selectedFormat!.rawValue.uppercased()))
                                .font(.caption2)
                                .foregroundColor(.blue)
                        }
                    }
                }

                Section {
                    Picker(
                        NSLocalizedString("data_source.policy.title", value: "Default access", comment: "Data source policy picker title"),
                        selection: $accessPolicy
                    ) {
                        ForEach(URLDataSourceAccessPolicy.allCases) { policy in
                            Text(policy.displayName).tag(policy)
                        }
                    }
                    .pickerStyle(.menu)
                    .accessibilityIdentifier("data.center.editor.policy")

                    Text(NSLocalizedString("data_source.policy.note", value: "Choose whether this source is used automatically, enabled per conversation, requested only when relevant, or blocked.", comment: "Data source policy explanation"))
                        .font(.caption2)
                        .foregroundColor(.secondary)
                } header: {
                    Text(NSLocalizedString("data_source.policy.section", value: "Context access", comment: "Data source policy section title"))
                }
                
                Section(header: Text(L10n.urlSourcesValidationSectionTitle)) {
                    Button {
                        validate()
                    } label: {
                        HStack {
                            if isValidating {
                                ProgressView()
                                Text(L10n.urlSourcesValidating)
                            } else {
                                Image(systemName: "checkmark.shield")
                                Text(NSLocalizedString("editor.validate", tableName: "CanonicalDataSourceEditor", value: "Validate source", comment: "Validate file or network source"))
                            }
                        }
                    }
                    .disabled(url.isEmpty || isValidating)
                    .accessibilityIdentifier("data.center.editor.validate")
                    
                    if let result = validationResult {
                        switch result {
                        case .success(let count, _):
                            VStack(alignment: .leading, spacing: 6) {
                                Label(String(format: L10n.urlSourcesValidateSuccess, count), systemImage: "checkmark.circle.fill")
                                    .foregroundColor(DesignTokens.successColor)
                                    .font(.subheadline)
                                Text(L10n.urlSourcesValidationReadyNote)
                                    .font(.caption)
                                    .foregroundColor(.secondary)
                            }
                        case .failure(let message):
                            VStack(alignment: .leading, spacing: 6) {
                                Label(message, systemImage: "xmark.circle.fill")
                                    .foregroundColor(DesignTokens.dangerColor)
                                    .font(.subheadline)
                                Text(L10n.urlSourcesValidationFailureNote)
                                    .font(.caption)
                                    .foregroundColor(.secondary)
                            }
                        }
                    }


                    if let contract = displayedContract {
                        VStack(alignment: .leading, spacing: 8) {
                            Label(
                                NSLocalizedString("data_source.contract.title", value: "Verified read contract", comment: "Data source contract title"),
                                systemImage: "checkmark.seal.fill"
                            )
                            .font(.subheadline.weight(.semibold))
                            .foregroundColor(DesignTokens.successColor)

                            Text(contract.readerDescription)
                                .font(.caption.monospaced())
                                .foregroundColor(.secondary)
                                .textSelection(.enabled)

                            Text(contract.schemaFields.joined(separator: " · "))
                                .font(.caption2)
                                .foregroundColor(.secondary)
                                .lineLimit(3)

                            Text(contract.redactedSample)
                                .font(.caption2)
                                .foregroundColor(.secondary)

                            Text(String(
                                format: NSLocalizedString("data_source.contract.summary", value: "%d records · refresh after %d min", comment: "Data source contract summary"),
                                contract.recordCount,
                                contract.maximumAgeMinutes
                            ))
                            .font(.caption2)
                            .foregroundColor(.secondary)
                        }
                        .padding(.vertical, 4)
                        .accessibilityIdentifier("data.center.editor.contract")
                    }
                }
                
                // Preview / description
                    if !previewEvents.isEmpty || !sourceDescriptionText.isEmpty {
                        Section(header: Text(NSLocalizedString("url_sources.preview.title", value: "Verified preview", comment: "Validated data source preview title"))) {
                        if !sourceDescriptionText.isEmpty {
                            Text(sourceDescriptionText)
                                .font(.caption)
                                .foregroundColor(.secondary)
                        }

                        if !previewEvents.isEmpty {
                            ForEach(previewEvents.prefix(5), id: \.id) { ev in
                                VStack(alignment: .leading, spacing: 4) {
                                    Text(ev.title)
                                        .font(.subheadline)
                                    if let notes = ev.notes {
                                        Text(notes)
                                            .font(.caption2)
                                            .foregroundColor(.secondary)
                                            .lineLimit(2)
                                    }
                                }
                            }
                        }
                    }
                }

                // Prompt/template selection, model choice, scheduling, and test-run UI
                // are intentionally not handled in this editor.
            }
            .navigationTitle(navTitle)
            .disabled(isSaving || isValidating)
            .accessibilityIdentifier("data.center.editor")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .cancellationAction) {
                    Button(L10n.cancel) {
                        requestDismiss()
                    }
                    .accessibilityIdentifier("data.center.editor.cancel")
                    .disabled(isSaving)
                }
                
                ToolbarItem(placement: .confirmationAction) {
                    Button {
                        onSaveTapped()
                    } label: {
                        if isSaving {
                            ProgressView()
                                .accessibilityLabel(actionButtonLabel)
                        } else {
                            Text(actionButtonLabel)
                        }
                    }
                    .disabled(!canAdd || isSaving)
                    .accessibilityIdentifier("data.center.editor.save")
                }
            }
            .onAppear {
                if let source = existingSource {
                    name = source.name
                    url = source.url
                    detectedFormat = source.format
                    selectedKind = source.kind
                    sourceDescriptionText = source.sourceDescription ?? ""
                    sourceID = source.id
                    accessPolicy = source.accessPolicy
                }
                DispatchQueue.main.async {
                    initialEditorSnapshot = editorSnapshot
                }
            }
            .onChange(of: url) { _, _ in
                guard url != existingSource?.url else { return }
                validationResult = nil
                previewEvents = []
            }
            .onChange(of: selectedFormat) { _, _ in
                validationResult = nil
                previewEvents = []
            }
            .onChange(of: selectedKind) { _, _ in
                // Validation may infer the kind; editable controls are frozen
                // while that owned operation publishes its preview/result.
                guard !isValidating else { return }
                validationResult = nil
                previewEvents = []
            }
            .interactiveDismissDisabled(isDirty || isValidating || isSaving)
            .onDisappear { validationTask?.cancel() }
            .alert(
                NSLocalizedString("url_sources.discard.title", value: "Discard changes?", comment: "Discard data source changes title"),
                isPresented: $showsDiscardConfirmation
            ) {
                Button(L10n.cancel, role: .cancel) {}
                Button(NSLocalizedString("url_sources.discard.confirm", value: "Discard", comment: "Discard data source changes"), role: .destructive) {
                    dismiss()
                }
            } message: {
                Text(NSLocalizedString("url_sources.discard.message", value: "Unsaved changes to this data source will be lost.", comment: "Discard data source changes message"))
            }
        }
        .fileImporter(
            isPresented: $showFileImporter,
            allowedContentTypes: [.item],
            allowsMultipleSelection: false
        ) { result in
            importLocalSource(result, kind: .file)
        }
        .fileImporter(
            isPresented: $showFolderImporter,
            allowedContentTypes: [.folder],
            allowsMultipleSelection: false
        ) { result in
            importLocalSource(result, kind: .folder)
        }
    }

    private func onSaveTapped() {
        isEditMode ? updateSource() : addSource()
    }

    private var editorSnapshot: String {
        [
            name.trimmingCharacters(in: .whitespacesAndNewlines),
            url.trimmingCharacters(in: .whitespacesAndNewlines),
            selectedFormat?.rawValue ?? "auto",
            String(describing: selectedKind),
            String(describing: accessPolicy),
            String(validationRevision)
        ].joined(separator: "\u{1F}")
    }

    private var isDirty: Bool {
        !initialEditorSnapshot.isEmpty && editorSnapshot != initialEditorSnapshot
    }

    private func requestDismiss() {
        validationTask?.cancel()
        isValidating = false
        if isDirty {
            showsDiscardConfirmation = true
        } else {
            dismiss()
        }
    }

    private var displayedContract: URLDataSourceValidationContract? {
        if case .success(_, let contract) = validationResult {
            return contract
        }
        guard existingSource?.url == url else { return nil }
        return existingSource?.validationContract
    }
    
    private var canAdd: Bool {
        guard !name.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty,
              !url.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty else { return false }

        // Add mode always requires an explicit successful validation.
        if !isEditMode {
            return validationResult?.isSuccess == true
        }

        // Edit mode: allow saving without re-validating if the URL hasn't changed;
        // otherwise require a fresh successful validation.
        if let existing = existingSource, existing.url == url {
            return true
        }
        return validationResult?.isSuccess == true
    }
    
    private func validate() {
        isValidating = true
        validationResult = nil
        errorMessage = nil
        validationTask?.cancel()
        validationTask = Task {
            do {
                guard let fetcher = container.urlFetcher else {
                    await MainActor.run {
                        errorMessage = NSLocalizedString(
                            "data_sources.inline.unavailable.message",
                            value: "The data source service cannot load its configuration right now.",
                            comment: "Data source service unavailable"
                        )
                        isValidating = false
                    }
                    return
                }

                // Detect format and preview first
                // Pass the user-selected format.
                let (fmt, preview, desc) = try await fetcher.detectFormatAndPreview(urlString: url, preferredFormat: selectedFormat)
                guard !Task.isCancelled else { return }
                await MainActor.run {
                    detectedFormat = fmt
                    if selectedKind == .website {
                        selectedKind = URLDataSource.inferKind(format: fmt, url: url)
                    }
                    previewEvents = preview
                    sourceDescriptionText = desc ?? ""
                }

                // Validate using detected format
                var source = URLDataSource(id: sourceID, url: url, format: fmt, kind: selectedKind, name: name)
                source.accessPolicy = accessPolicy
                let result = await fetcher.validateSource(source)
                guard !Task.isCancelled else { return }
                await MainActor.run {
                    validationResult = result
                    validationRevision += 1
                    isValidating = false
                }
            } catch {
                guard !Task.isCancelled else { return }
                await MainActor.run {
                    errorMessage = error.localizedDescription
                    isValidating = false
                }
            }
        }
    }

    private func importLocalSource(_ result: Result<[URL], Error>, kind: DataSourceKind) {
        do {
            guard let selectedURL = try result.get().first else { return }
            selectedKind = kind
            selectedFormat = .html
            detectedFormat = .html
            url = selectedURL.absoluteString
            if name.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
                name = selectedURL.lastPathComponent.isEmpty ? kind.displayName : selectedURL.lastPathComponent
            }
            sourceDescriptionText = kind == .folder
                ? String(format: NSLocalizedString("url_sources.local_source.folder_desc", value: "Local folder: %@", comment: "Local folder source description"), selectedURL.lastPathComponent)
                : String(format: NSLocalizedString("url_sources.local_source.file_desc", value: "Local file: %@", comment: "Local file source description"), selectedURL.lastPathComponent)
            validate()
        } catch {
            if !IOSContextFileImportPolicy.isCancellation(error) { errorMessage = error.localizedDescription }
        }
    }
    
    private func addSource() {
        guard let fetcher = container.urlFetcher else { return }
        // Prefer user-selected format, then detected format, then fallback to HTML.
        let fmt = selectedFormat ?? detectedFormat ?? .html

        var source = URLDataSource(id: sourceID, url: url, format: fmt, kind: selectedKind, name: name)
        source.sourceDescription = sourceDescriptionText.isEmpty ? nil : sourceDescriptionText
        source.accessPolicy = accessPolicy
        source.validationContract = displayedContract
        source.securityBookmarkData = makeSecurityScopedBookmarkIfNeeded()
        // Use the shared ChatManager's selected model by default; keep per-source override if needed later
        if let modelName = container.chatManager?.getCurrentModel()?.name {
            source.preferredModel = modelName
        }

        // Re-check server/client limits before attempting to add (race-safe UX)
        let current = fetcher.sources.count
        let limit = ClientConfig.shared.effectiveURLLimit()
        guard current < limit else {
            errorMessage = L10n.urlSourcesMaxLimitReached.localized("\(limit)")
            return
        }

        commitEditorSource(source, using: fetcher)
    }
    
    private func updateSource() {
        guard let fetcher = container.urlFetcher, let existing = existingSource else { return }
        // Prefer user-selected format, then detected format, then keep existing format.
        let fmt = selectedFormat ?? detectedFormat ?? existing.format

        var updated = existing
        updated.name = name
        updated.url = url
        updated.format = fmt
        updated.kind = selectedKind
        updated.sourceDescription = sourceDescriptionText.isEmpty ? nil : sourceDescriptionText
        updated.accessPolicy = accessPolicy
        updated.validationContract = displayedContract
        if let bookmark = makeSecurityScopedBookmarkIfNeeded() {
            updated.securityBookmarkData = bookmark
        }

        commitEditorSource(updated, using: fetcher)
    }

    private func commitEditorSource(_ source: URLDataSource, using fetcher: URLFetcherService) {
        guard !isSaving else { return }
        isSaving = true
        errorMessage = nil
        Task { @MainActor in
            defer { isSaving = false }
            do {
                let committed = try await fetcher.saveSource(source)
                container.scheduler.scheduleNextRefresh()
                onSaved?(committed)
                dismiss()
            } catch {
                errorMessage = error.localizedDescription
            }
        }
    }
    
    // NOTE: Test-run helpers, schedule helpers and prompt-to-notification helpers
    // were removed as part of the data-sources -> scheduled-actions split.

    private func makeSecurityScopedBookmarkIfNeeded() -> Data? {
        guard selectedKind == .file || selectedKind == .folder,
              let localURL = URL(string: url),
              localURL.isFileURL else {
            return nil
        }

        let didAccess = localURL.startAccessingSecurityScopedResource()
        defer {
            if didAccess {
                localURL.stopAccessingSecurityScopedResource()
            }
        }

        return try? localURL.bookmarkData(
            options: [],
            includingResourceValuesForKeys: nil,
            relativeTo: nil
        )
    }
}


#Preview {
    NavigationStack {
        URLDataSourcesView()
            .environmentObject(ServiceContainer())
    }
}
