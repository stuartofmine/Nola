// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Implements ScheduledActionRunsView logic in the iOS UI layer.
// - Interfaces:
//   - ScheduledActionRunsView
//   - ScheduledActionRunRow
// - APIs:
//   - none
//
import SwiftUI

struct ScheduledActionRunsView: View {
    @EnvironmentObject var container: ServiceContainer

    private let actionId: String?
    private let titleOverride: String?

    @State private var runs: [LocalJSONStorage.ScheduledActionRunRecord] = []
    @State private var loadError: String? = nil
    @State private var isLoading: Bool = false
    @State private var searchText = ""
    @State private var selectedFilter: ScheduledRunFilter = .all

    init(actionId: String? = nil, titleOverride: String? = nil) {
        self.actionId = actionId
        self.titleOverride = titleOverride
    }

    var body: some View {
        List {
            if !runs.isEmpty {
                Section {
                    ScheduledRunsSummary(
                        runs: runs,
                        visibleCount: visibleRuns.count,
                        selectedFilter: $selectedFilter
                    )
                }
                .listRowInsets(EdgeInsets(top: 8, leading: 16, bottom: 8, trailing: 16))
                .listRowBackground(Color.clear)
            }

            if let loadError {
                Section {
                    HStack(alignment: .top, spacing: 10) {
                        Image(systemName: "exclamationmark.triangle.fill")
                            .foregroundStyle(DesignTokens.warningColor)
                        Text(loadError)
                            .font(.caption)
                            .foregroundColor(.secondary)
                            .frame(maxWidth: .infinity, alignment: .leading)
                        Button(NSLocalizedString("common.retry", value: "Retry", comment: "Retry action")) {
                            Task { await reload() }
                        }
                        .buttonStyle(.borderless)
                    }
                }
            }

            if isLoading && runs.isEmpty {
                Section {
                    HStack(spacing: 10) {
                        ProgressView()
                        Text(NSLocalizedString("scheduled.runs.loading", value: "Loading run history…", comment: "Loading scheduled run history"))
                            .font(.callout)
                            .foregroundStyle(.secondary)
                    }
                    .frame(maxWidth: .infinity, minHeight: 80, alignment: .center)
                }
            } else if runs.isEmpty && loadError == nil {
                Section {
                    NolaInlineEmptyState(
                        icon: "clock.arrow.circlepath",
                        title: L10n.scheduledActionRunsEmpty,
                        tint: DesignTokens.primaryColor
                    )
                    .listRowInsets(EdgeInsets(top: 8, leading: 16, bottom: 8, trailing: 16))
                    .listRowBackground(Color.clear)
                }
            } else if visibleRuns.isEmpty {
                Section {
                    NolaCard {
                        NolaInlineEmptyState(
                            icon: "line.3.horizontal.decrease.circle",
                            title: NSLocalizedString("scheduled.runs.filter.empty.title", value: "No matching runs", comment: "No matching scheduled runs title"),
                            message: NSLocalizedString("scheduled.runs.filter.empty.detail", value: "Try another search or status filter.", comment: "No matching scheduled runs detail"),
                            tint: DesignTokens.primaryColor
                        )
                        Button(NSLocalizedString("scheduled.runs.filter.reset", value: "Reset filters", comment: "Reset run history filters")) {
                            searchText = ""
                            selectedFilter = .all
                        }
                        .buttonStyle(.bordered)
                        .frame(maxWidth: .infinity)
                    }
                    .listRowInsets(EdgeInsets(top: 8, leading: 16, bottom: 8, trailing: 16))
                    .listRowBackground(Color.clear)
                }
            } else {
                Section {
                    ForEach(visibleRuns) { run in
                        NavigationLink {
                            ScheduledActionRunDetailScreen(run: run)
                                .environmentObject(container)
                        } label: {
                            ScheduledActionRunRow(run: run)
                        }
                    }
                }
            }
        }
        .listStyle(.insetGrouped)
        .scrollContentBackground(.hidden)
        .background(DesignTokens.backgroundColor)
        .navigationTitle(titleOverride ?? L10n.scheduledActionRunsTitle)
        .navigationBarTitleDisplayMode(.inline)
        .searchable(
            text: $searchText,
            placement: .navigationBarDrawer(displayMode: .always),
            prompt: NSLocalizedString("scheduled.runs.filter.search", value: "Search run history", comment: "Search scheduled run history")
        )
        .task {
            await reload()
        }
        .onReceive(NotificationCenter.default.publisher(for: .nolaScheduledActionRunsUpdated)) { _ in
            Task { await reload() }
        }
        .refreshable {
            await reload()
        }
    }

    private var visibleRuns: [LocalJSONStorage.ScheduledActionRunRecord] {
        let query = searchText.trimmingCharacters(in: .whitespacesAndNewlines)
        return runs.filter { run in
            let matchesQuery = query.isEmpty
                || run.actionName.orEmpty.localizedCaseInsensitiveContains(query)
                || run.actionUrl.orEmpty.localizedCaseInsensitiveContains(query)
                || run.errorMessage.orEmpty.localizedCaseInsensitiveContains(query)
                || run.reason.orEmpty.localizedCaseInsensitiveContains(query)
            let matchesFilter = selectedFilter.matches(run.status)
            return matchesQuery && matchesFilter
        }
    }

    @MainActor
    private func reload() async {
        guard !isLoading else { return }
        isLoading = true
        defer { isLoading = false }

        do {
            guard let storage = container.localJSONStorage else {
                loadError = NSLocalizedString("scheduled_actions.history.unavailable", value: "Run history is unavailable. Retry when local storage is ready.", comment: "Missing run history storage")
                return
            }
            let all = try storage.getScheduledActionRuns(limit: 200, actionId: actionId)
            runs = all
            loadError = nil
        } catch {
            loadError = error.localizedDescription
        }
    }
}


private struct ScheduledActionRunRow: View {
    let run: LocalJSONStorage.ScheduledActionRunRecord

    var body: some View {
        HStack(alignment: .top, spacing: DesignTokens.spacing_md) {
            Circle()
                .fill(statusColor)
                .frame(width: 10, height: 10)
                .padding(.top, 6)

            VStack(alignment: .leading, spacing: 4) {
                // Comment in English.
                ViewThatFits(in: .horizontal) {
                    HStack(alignment: .firstTextBaseline, spacing: 10) {
                        runTitleLabel
                        Spacer(minLength: 6)
                        statusBadge
                    }
                    VStack(alignment: .leading, spacing: 6) {
                        runTitleLabel
                        statusBadge
                    }
                }

                ViewThatFits(in: .horizontal) {
                    HStack(spacing: 6) {
                        primaryMetadata
                        secondaryMetadata(includeLeadingSeparator: true)
                    }
                    VStack(alignment: .leading, spacing: 3) {
                        primaryMetadata
                        secondaryMetadata(includeLeadingSeparator: false)
                    }
                }

                // Comment in English.
                if let err = run.errorMessage, !err.isEmpty {
                    Text(err)
                        .font(.caption)
                        .foregroundColor(DesignTokens.dangerColor)
                        .lineLimit(2)
                } else if let reason = run.reason, !reason.isEmpty {
                    let reasonText = ScheduledActionRunText.reasonSummaryText(for: run)
                    if !reasonText.isEmpty {
                        Text(reasonText)
                            .font(.caption)
                            .foregroundColor(.secondary)
                            .lineLimit(2)
                    }
                }
            }
        }
        .padding(.vertical, 4)
    }

    private var runTitleLabel: some View {
        Text(runTitle)
            .font(.body.weight(.medium))
            .foregroundColor(.primary)
            .lineLimit(2)
            .fixedSize(horizontal: false, vertical: true)
    }

    private var statusBadge: some View {
        Text(statusText)
            .font(.caption2.weight(.semibold))
            .foregroundStyle(statusColor)
            .padding(.horizontal, 8)
            .padding(.vertical, 4)
            .background(statusColor.opacity(0.1), in: Capsule())
    }

    private var primaryMetadata: some View {
        HStack(spacing: 6) {
            Text(Self.relativeTime(run.startedAt))
            Text("·")
            Text(Self.durationText(run.startedAt, run.finishedAt))
        }
        .font(.caption)
        .foregroundStyle(.secondary)
    }

    @ViewBuilder
    private func secondaryMetadata(includeLeadingSeparator: Bool) -> some View {
        let executionPath = run.executionPath ?? (run.isBackground ? "server" : nil)
        HStack(spacing: 6) {
            if let path = executionPath {
                if includeLeadingSeparator {
                    Text("·")
                        .foregroundStyle(.tertiary)
                }
                Text(
                    path == "server"
                    ? NSLocalizedString("scheduled.runs.path.server", value: "Server", comment: "Server execution path")
                    : NSLocalizedString("scheduled.runs.path.local", value: "Local", comment: "Local execution path")
                )
                .foregroundStyle(.secondary)
            }
            if let count = run.eventsCount, count > 0, run.status == .success {
                if includeLeadingSeparator || executionPath != nil {
                    Text("·")
                        .foregroundStyle(.tertiary)
                }
                Text(
                    String(
                        format: NSLocalizedString("scheduled.runs.events_count", value: "%d events", comment: "Run events count"),
                        count
                    )
                )
                .foregroundStyle(DesignTokens.successColor)
            }
        }
        .font(.caption)
    }

    private var runTitle: String {
        if let name = run.actionName, !name.isEmpty {
            return name
        }
        if let url = run.actionUrl, !url.isEmpty {
            return url
        }
        return run.actionType
    }

    private var statusText: String {
        switch run.status {
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

    private var statusColor: Color {
        switch run.status {
        case .success: return DesignTokens.successColor
        case .partial: return DesignTokens.warningColor
        case .skipped: return Color.secondary
        case .failed: return DesignTokens.dangerColor
        case .actionCompleted: return DesignTokens.successColor
        case .answered, .delegationAccepted, .cancelled: return .secondary
        case .incomplete, .blocked: return DesignTokens.warningColor
        }
    }

    private static func relativeTime(_ date: Date) -> String {
        let formatter = RelativeDateTimeFormatter()
        formatter.unitsStyle = .abbreviated
        return formatter.localizedString(for: date, relativeTo: Date())
    }

    private static func durationText(_ start: Date, _ end: Date) -> String {
        let seconds = max(0, end.timeIntervalSince(start))
        if seconds < 1 { return "<1s" }
        if seconds < 60 { return "\(Int(seconds))s" }
        return String(format: "%.1fm", seconds / 60.0)
    }
}

enum ScheduledRunFilter: String, CaseIterable, Identifiable {
    case all
    case success
    case issues
    case skipped
    case cancelled

    var id: String { rawValue }

    func matches(_ status: LocalJSONStorage.ScheduledActionRunRecord.Status) -> Bool {
        let category = ScheduledActionRunText.statusCategory(status)
        switch self {
        case .all: return true
        case .success: return category == .success
        case .issues: return category == .issues
        case .skipped: return category == .skipped
        case .cancelled: return category == .cancelled
        }
    }

    var title: String {
        switch self {
        case .all:
            return NSLocalizedString("scheduled.runs.filter.all", value: "All", comment: "All run history filter")
        case .success:
            return NSLocalizedString("scheduled.runs.filter.success", value: "Success", comment: "Successful run history filter")
        case .issues:
            return NSLocalizedString("scheduled.runs.filter.issues", value: "Issues", comment: "Run history issues filter")
        case .skipped:
            return NSLocalizedString("scheduled.runs.filter.skipped", value: "Skipped", comment: "Skipped run history filter")
        case .cancelled:
            return NSLocalizedString("scheduled_actions.run_status.cancelled", value: "Cancelled", comment: "Cancelled run history filter")
        }
    }
}

private struct ScheduledRunsSummary: View {
    let runs: [LocalJSONStorage.ScheduledActionRunRecord]
    let visibleCount: Int
    @Binding var selectedFilter: ScheduledRunFilter

    var body: some View {
        NolaCard {
            HStack(alignment: .firstTextBaseline) {
                Text(NSLocalizedString("scheduled.runs.filter.summary", value: "Run history", comment: "Run history filter summary"))
                    .font(.callout.weight(.semibold))
                Spacer()
                Text(
                    String(
                        format: NSLocalizedString("scheduled.runs.filter.results", value: "%d of %d", comment: "Run history filtered count"),
                        visibleCount,
                        runs.count
                    )
                )
                .font(.caption.weight(.semibold))
                .foregroundStyle(.secondary)
            }
            ScrollView(.horizontal, showsIndicators: false) {
                HStack(spacing: 8) {
                    ForEach(ScheduledRunFilter.allCases) { filter in
                        let isSelected = selectedFilter == filter
                        Button {
                            selectedFilter = filter
                        } label: {
                            Text("\(filter.title) \(count(for: filter))")
                                .font(.caption.weight(.semibold))
                                .foregroundStyle(isSelected ? Color.primary : Color.secondary)
                                .padding(.horizontal, 11)
                                .padding(.vertical, 7)
                                .background(
                                    isSelected ? DesignTokens.accentSubtleColor : Color.clear,
                                    in: Capsule()
                                )
                        }
                        .buttonStyle(.plain)
                        .accessibilityAddTraits(isSelected ? .isSelected : [])
                    }
                }
            }
        }
    }

    private func count(for filter: ScheduledRunFilter) -> Int {
        runs.filter { filter.matches($0.status) }.count
    }
}

private extension Optional where Wrapped == String {
    var orEmpty: String { self ?? "" }
}
