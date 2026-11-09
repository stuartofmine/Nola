// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Implements HomeComponents logic in the iOS UI layer.
// - Interfaces:
//   - HomeHealthTile
//   - HomeScheduleItem
//   - HomeCustomSourceEntry
//   - HomeCardContainer
//   - HomeIconBadge
//   - HomeSectionHeader
//   - HomePhoneHealthCard
//   - HomeHealthMetricTile
// - APIs:
//   - none
//
//
//  HomeComponents.swift
//  Nola
//
//  Created by Nola AI on 2026/01/17.
//

import Foundation
import SwiftUI

// MARK: - Home: Data Source Models

struct HomeHealthTile: Identifiable {
    enum Kind: String {
        case steps
        case heartRate
        case sleep
        case workout
    }

    let id: Kind
    let title: String
    let icon: String
    let tint: Color
    let value: String
    let subtitle: String?

    static func placeholder() -> [HomeHealthTile] {
        [
            .init(id: .steps, title: L10n.dataSourceDetailHealthMetricSteps, icon: "figure.walk", tint: .green, value: "--", subtitle: nil),
            .init(id: .heartRate, title: L10n.dataSourceDetailHealthMetricHeartRate, icon: "heart.fill", tint: .red, value: "--", subtitle: nil),
            .init(id: .sleep, title: L10n.dataSourceDetailHealthMetricSleep, icon: "bed.double.fill", tint: .purple, value: "--", subtitle: nil),
            .init(id: .workout, title: L10n.dataSourceDetailHealthMetricWorkout, icon: "flame.fill", tint: .orange, value: "--", subtitle: nil)
        ]
    }

    static func from(healthEvents: [RawEvent]) -> [HomeHealthTile] {
        var out = Dictionary(uniqueKeysWithValues: placeholder().map { ($0.id, $0) })

        for ev in healthEvents {
            let t = (ev.metadata["type"] ?? "").trimmingCharacters(in: .whitespacesAndNewlines)
            switch t {
            case "steps":
                let avg = Int(ev.metadata["avg"] ?? "")
                if let avg {
                    out[.steps] = .init(
                        id: .steps,
                        title: L10n.dataSourceDetailHealthMetricSteps,
                        icon: "figure.walk",
                        tint: .green,
                        value: "\(avg)",
                        subtitle: L10n.homeHealthAverageSevenDays
                    )
                }
            case "heart_rate":
                let v = Int(ev.metadata["value"] ?? "")
                if let v {
                    out[.heartRate] = .init(
                        id: .heartRate,
                        title: L10n.dataSourceDetailHealthMetricHeartRate,
                        icon: "heart.fill",
                        tint: .red,
                        value: "\(v)",
                        subtitle: L10n.homeHealthUnitBPM
                    )
                }
            case "sleep":
                let avg = Double(ev.metadata["avg"] ?? "")
                if let avg {
                    out[.sleep] = .init(
                        id: .sleep,
                        title: L10n.dataSourceDetailHealthMetricSleep,
                        icon: "bed.double.fill",
                        tint: .purple,
                        value: String(format: "%.1f", avg),
                        subtitle: L10n.homeHealthUnitHours
                    )
                }
            case "workout":
                let m = Int(ev.metadata["durationMin"] ?? "")
                if let m {
                    out[.workout] = .init(
                        id: .workout,
                        title: L10n.dataSourceDetailHealthMetricWorkout,
                        icon: "flame.fill",
                        tint: .orange,
                        value: "\(m)",
                        subtitle: L10n.homeHealthUnitMinutes
                    )
                }
            default:
                continue
            }
        }

        let order: [HomeHealthTile.Kind] = [.steps, .heartRate, .sleep, .workout]
        return order.compactMap { out[$0] }
    }
}

struct HomeScheduleItem: Identifiable {
    enum Kind {
        case birthday
        case calendar
        case reminder
        case localReminder
    }

    let id: String
    let title: String
    let date: Date
    let kind: Kind

    var icon: String {
        switch kind {
        case .birthday: return "gift.fill"
        case .calendar: return "calendar"
        case .reminder: return "checklist"
        case .localReminder: return "bell.badge.fill"
        }
    }

    var tint: Color {
        switch kind {
        case .birthday: return .pink
        case .calendar: return .blue
        case .reminder: return .green
        case .localReminder: return .orange
        }
    }

    static func placeholder(kind: Kind) -> HomeScheduleItem {
        HomeScheduleItem(
            id: UUID().uuidString,
            title: " ",
            date: Date(),
            kind: kind
        )
    }
}

struct HomeCustomSourceEntry: Identifiable {
    let id: UUID
    let name: String
    let actionId: String
    let latestRun: LocalJSONStorage.ScheduledActionRunRecord?
    let latestSummary: String?
}

// MARK: - Home: Common Card Container

private struct HomeCardContainer<Content: View>: View {
    @Environment(\.colorScheme) private var colorScheme
    let content: Content
    var compact: Bool = false

    init(compact: Bool = false, @ViewBuilder content: () -> Content) {
        self.content = content()
        self.compact = compact
    }

    private var strokeColor: Color {
        DesignTokens.borderColor
    }

    var body: some View {
        VStack(alignment: .leading, spacing: DesignTokens.spacing_md) {
            content
        }
        .padding(compact ? DesignTokens.spacing_sm : DesignTokens.spacing_lg)
        .background {
            if !compact {
                RoundedRectangle(cornerRadius: DesignTokens.radius_md, style: .continuous)
                    .fill(DesignTokens.elevatedColor)
                    .overlay(RoundedRectangle(cornerRadius: DesignTokens.radius_md, style: .continuous).stroke(strokeColor, lineWidth: 1))
            }
        }
    }
}

private struct HomeIconBadge: View {
    let icon: String
    let tint: Color
    var size: CGFloat = 34

    var body: some View {
        ZStack {
            RoundedRectangle(cornerRadius: DesignTokens.radius_md, style: .continuous)
                .fill(DesignTokens.rowFillColor)
            Image(systemName: icon)
                .font(.system(size: size * 0.43, weight: .semibold))
                .foregroundColor(tint)
        }
        .frame(width: size, height: size)
        .accessibilityHidden(true)
    }
}

private struct HomeSectionHeader: View {
    let icon: String
    let title: String
    let subtitle: String?
    let onTapManage: () -> Void

    var body: some View {
        Button(action: onTapManage) {
            HStack(alignment: .center, spacing: DesignTokens.spacing_md) {
                Image(icon).resizable().renderingMode(.template).scaledToFit()
                    .frame(width: 20, height: 20)
                    .foregroundStyle(DesignTokens.actionColor)
                    .frame(width: 34, height: 34)
                    .background(DesignTokens.rowFillColor, in: RoundedRectangle(cornerRadius: DesignTokens.radius_md))
                    .accessibilityHidden(true)

                VStack(alignment: .leading, spacing: 3) {
                    Text(title)
                        .font(.headline)
                        .foregroundColor(.primary)
                        .lineLimit(1)
                    if let subtitle, !subtitle.isEmpty {
                        Text(subtitle)
                            .font(.caption)
                            .foregroundColor(.secondary)
                            .fixedSize(horizontal: false, vertical: true)
                    }
                }
                .frame(maxWidth: .infinity, alignment: .leading)

                Image("nola-chevron").resizable().renderingMode(.template).scaledToFit()
                    .foregroundColor(.secondary)
                    .frame(width: 20, height: 20)
            }
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .accessibilityLabel(title)
    }
}

// MARK: - Section 1: Phone Health

struct HomePhoneHealthCard: View {
    let tiles: [HomeHealthTile]
    let isLoading: Bool
    let error: String?
    let onTapManage: () -> Void
    let onTapAuthorize: () -> Void
    let onTapOpenSettings: () -> Void

    var body: some View {
        HomeCardContainer {
            HomeSectionHeader(
                icon: "nola-heart",
                title: L10n.settingsHealthDataSource,
                subtitle: L10n.settingsHealthDataSourceNote,
                onTapManage: onTapManage
            )

            if let error, !error.isEmpty {
                Text(error)
                    .font(.caption)
                    .foregroundColor(DesignTokens.dangerColor)
            }

            let cols = [GridItem(.flexible(), spacing: 12), GridItem(.flexible(), spacing: 12)]
            LazyVGrid(columns: cols, spacing: 12) {
                let isPlaceholder = tiles.isEmpty && isLoading
                ForEach(tiles.isEmpty ? HomeHealthTile.placeholder() : tiles) { tile in
                    HomeHealthMetricTile(tile: tile)
                        .redacted(reason: isPlaceholder ? .placeholder : [])
                }
            }

            HStack(spacing: 10) {
                if isLoading {
                    ProgressView()
                        .scaleEffect(0.9)
                }
                Button(action: onTapOpenSettings) {
                    Text(L10n.settings)
                        .font(.callout.weight(.semibold))
                }
                .buttonStyle(.bordered)
                Spacer()
            }
        }
    }
}

private struct HomeHealthMetricTile: View {
    let tile: HomeHealthTile

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            HStack(alignment: .firstTextBaseline, spacing: 6) {
                Text(tile.value)
                    .font(.title2.weight(.bold))
                    .foregroundColor(.primary)
                    .lineLimit(1)
                if let sub = tile.subtitle, !sub.isEmpty {
                    Text(sub)
                        .font(.caption)
                        .foregroundColor(.secondary)
                        .lineLimit(1)
                }
                Spacer()
            }

            HStack(spacing: 6) {
                Image(systemName: tile.icon)
                    .font(.caption.weight(.semibold))
                    .foregroundColor(tile.tint)
                    .frame(width: 14)
                Text(tile.title)
                    .font(.caption)
                    .foregroundColor(.secondary)
                    .lineLimit(1)
            }
        }
        .padding(12)
        .background(
            RoundedRectangle(cornerRadius: DesignTokens.radius_lg, style: .continuous)
                .fill(DesignTokens.rowFillColor)
        )
        .overlay(
            RoundedRectangle(cornerRadius: DesignTokens.radius_lg, style: .continuous)
                .stroke(DesignTokens.borderColor, lineWidth: 1)
        )
    }
}

// MARK: - Section 2: Phone Schedule

struct HomePhoneScheduleCard: View {
    let birthdayLeadDays: Int
    let scheduleLeadDays: Int
    let items: [HomeScheduleItem]
    let isLoading: Bool
    let error: String?
    let onTapManage: () -> Void
    let onTapItem: (HomeScheduleItem) -> Void

    var body: some View {
        HomeCardContainer(compact: true) {
            HomeSectionHeader(
                icon: "nola-calendar",
                title: L10n.homeSectionToday,
                subtitle: nil,
                onTapManage: onTapManage
            )

            if let error, !error.isEmpty {
                Label(error, systemImage: "exclamationmark.triangle.fill")
                    .font(.caption)
                    .foregroundColor(DesignTokens.dangerColor)
            }

            VStack(alignment: .leading, spacing: 12) {
                let rows: [HomeScheduleItem] = {
                    if items.isEmpty && isLoading {
                        return [
                            HomeScheduleItem.placeholder(kind: .calendar),
                            HomeScheduleItem.placeholder(kind: .reminder),
                            HomeScheduleItem.placeholder(kind: .birthday)
                        ]
                    }
                    return Array(items.prefix(8))
                }()

                if rows.isEmpty {
                    Text(L10n.settingsCalendarRangeDescription)
                        .font(.caption)
                        .foregroundColor(.secondary)
                } else {
                    VStack(spacing: 10) {
                        ForEach(Array(rows.enumerated()), id: \.offset) { _, item in
                            HomeScheduleRow(item: item, onTap: { onTapItem(item) })
                                .redacted(reason: (items.isEmpty && isLoading) ? .placeholder : [])
                        }
                    }
                }
            }

            if isLoading {
                HStack {
                    ProgressView().scaleEffect(0.9)
                    Spacer()
                }
            }
        }
    }
}

private struct HomeScheduleRow: View {
    let item: HomeScheduleItem
    let onTap: () -> Void

    private var whenText: String {
        let cal = Calendar.current
        let rel = RelativeDateTimeFormatter()
        rel.unitsStyle = .full

        if cal.isDateInToday(item.date) {
            // Explicitly show localized "Today" rather than computing from startOfDay
            let dayLabel = L10n.homeSectionToday
            let timeLabel = item.date.formatted(date: .omitted, time: .shortened)
            return "\(dayLabel) · \(timeLabel)"
        }

        if cal.isDateInTomorrow(item.date) {
            let dayLabel = rel.localizedString(for: item.date, relativeTo: Date())
            let timeLabel = item.date.formatted(date: .omitted, time: .shortened)
            return "\(dayLabel) · \(timeLabel)"
        }

        return item.date.formatted(date: .abbreviated, time: .shortened)
    }

    var body: some View {
        Button(action: onTap) {
            HStack(alignment: .center, spacing: DesignTokens.spacing_md) {
                HomeIconBadge(icon: item.icon, tint: item.tint, size: 32)

                VStack(alignment: .leading, spacing: 3) {
                    Text(item.title)
                        .font(.callout.weight(.semibold))
                        .foregroundColor(.primary)
                        .lineLimit(2)
                    Text(whenText)
                        .font(.caption)
                        .foregroundColor(.secondary)
                        .monospacedDigit()
                }
                .frame(maxWidth: .infinity, alignment: .leading)

                Image(systemName: "chevron.right")
                    .font(.caption.weight(.semibold))
                    .foregroundColor(.secondary.opacity(0.8))
            }
            .padding(10)
            .background(
                RoundedRectangle(cornerRadius: DesignTokens.radius_lg, style: .continuous)
                    .fill(DesignTokens.rowFillColor)
            )
            .overlay(
                RoundedRectangle(cornerRadius: DesignTokens.radius_lg, style: .continuous)
                    .stroke(DesignTokens.borderColor, lineWidth: 1)
            )
        }
        .buttonStyle(.plain)
    }
}

// MARK: - Section 3: Custom URL Sources

struct HomeCustomDataSourcesCard: View {
    let sources: [HomeCustomSourceEntry]
    let isLoading: Bool
    let onTapManage: () -> Void
    let onTapSource: (HomeCustomSourceEntry) -> Void

    var body: some View {
        HomeCardContainer {
            HomeSectionHeader(
                icon: "nola-link",
                title: L10n.homeSectionCustomDataSources,
                subtitle: L10n.homeSectionURLContent,
                onTapManage: onTapManage
            )

            if sources.isEmpty {
                Text(L10n.urlSourcesEmptySubtitle)
                    .font(.callout)
                    .foregroundColor(.secondary)
            } else {
                VStack(spacing: 10) {
                    ForEach(Array(sources.prefix(6).enumerated()), id: \.offset) { _, entry in
                        Button {
                            onTapSource(entry)
                        } label: {
                            HomeCustomSourceRow(entry: entry)
                        }
                        .buttonStyle(.plain)
                    }
                }
            }

            if isLoading {
                HStack {
                    ProgressView().scaleEffect(0.9)
                    Spacer()
                }
            }
        }
    }
}

private struct HomeCustomSourceRow: View {
    let entry: HomeCustomSourceEntry

    private var statusColor: Color {
        guard let run = entry.latestRun else { return .secondary }
        return ScheduledActionRunText.statusPresentation(run.status).tone.color
    }

    private var timeText: String {
        guard let t = entry.latestRun?.finishedAt else { return "" }
        return t.formatted(date: .omitted, time: .shortened)
    }

    private var summaryText: String {
        if let s = entry.latestSummary?.nolaSanitizedSummary(), !s.isEmpty {
            return s
        }
        guard let r = entry.latestRun else {
            return L10n.scheduledActionRunsEmpty
        }

        let mappedReason = ScheduledActionRunText.reasonSummaryText(for: r).nolaSanitizedSummary()
        if !mappedReason.isEmpty {
            return mappedReason
        }

        if let err = r.errorMessage?.nolaSanitizedSummary(), !err.isEmpty {
            return err
        }

        return ScheduledActionRunText.statusPresentation(r.status).title
    }

    var body: some View {
        HStack(alignment: .center, spacing: DesignTokens.spacing_md) {
            ZStack {
                Circle()
                    .fill(statusColor.opacity(DesignTokens.overlaySoftOpacity))
                Circle()
                    .fill(statusColor)
                    .frame(width: 9, height: 9)
            }
            .frame(width: 32, height: 32)

            VStack(alignment: .leading, spacing: 4) {
                HStack(alignment: .firstTextBaseline) {
                    Text(entry.name)
                        .font(.subheadline.weight(.semibold))
                        .foregroundColor(.primary)
                        .lineLimit(1)
                    Spacer()
                    if !timeText.isEmpty {
                        Text(timeText)
                            .font(.caption)
                            .foregroundColor(.secondary)
                    }
                }

                Text(summaryText)
                    .font(.callout)
                    .foregroundColor(.secondary)
                    .lineLimit(4)
            }
            .frame(maxWidth: .infinity, alignment: .leading)
        }
        .padding(12)
        .background(
            RoundedRectangle(cornerRadius: DesignTokens.radius_lg, style: .continuous)
                .fill(DesignTokens.rowFillColor)
        )
        .overlay(
            RoundedRectangle(cornerRadius: DesignTokens.radius_lg, style: .continuous)
                .stroke(DesignTokens.borderColor, lineWidth: 1)
        )
    }
}

// Comment in English.
/// Documentation comment in English.
struct HomeTodayBySourceCard: View {
    let processedEvents: [ProcessedEvent]
    let summary: ModelOutput?
    let isLoading: Bool
    let error: String?
    let onGenerate: () -> Void
    let onTap: () -> Void

    var body: some View {
        VStack(alignment: .leading, spacing: 16) {
            HStack(spacing: 10) {
                Button(action: onTap) {
                    HStack(spacing: 10) {
                        Image(systemName: "sparkles").foregroundStyle(DesignTokens.primaryColor)
                        Text(L10n.homeSummaryTitle).font(.title3.bold())
                        Spacer()
                        Image(systemName: "chevron.right").foregroundStyle(.secondary)
                    }.frame(minHeight: 44).contentShape(Rectangle())
                }.buttonStyle(.plain)

                if isLoading {
                    ProgressView()
                        .scaleEffect(0.85)
                } else {
                    Button(action: onGenerate) {
                        Image(systemName: "arrow.clockwise")
                            .font(.callout.weight(.semibold))
                            .foregroundColor(DesignTokens.primaryColor)
                            .frame(minWidth: 44, minHeight: 44)
                            .background(DesignTokens.mutedFillColor)
                            .clipShape(RoundedRectangle(cornerRadius: 10, style: .continuous))
                    }
                    .buttonStyle(.plain)
                    .accessibilityLabel(L10n.homeRefresh)
                }
            }

            if let error = error {
                Text(error)
                    .font(.callout)
                    .foregroundColor(DesignTokens.dangerColor)
            }

            if let explain = summary?.explain?.nolaSanitizedSummary(), !explain.isEmpty {
                Text(explain)
                    .font(.callout)
                    .foregroundColor(.primary)
                    .lineLimit(4)
                    .padding(12)
                    .background(
                        RoundedRectangle(cornerRadius: DesignTokens.radius_lg, style: .continuous)
                            .fill(DesignTokens.rowFillColor)
                    )
                    .overlay(
                        RoundedRectangle(cornerRadius: DesignTokens.radius_lg, style: .continuous)
                            .stroke(DesignTokens.borderColor, lineWidth: 1)
                    )
            }

            if processedEvents.isEmpty {
                VStack(alignment: .leading, spacing: 10) {
                    Text(L10n.homeNoTodosMessage)
                        .font(.callout)
                        .foregroundColor(.secondary)
                        .multilineTextAlignment(.leading)
                }
                .frame(maxWidth: .infinity, alignment: .leading)
                .padding(.vertical, 8)
            } else {
                let groups = HomeTodayBySourceGrouping.group(processedEvents)
                VStack(alignment: .leading, spacing: 14) {
                    ForEach(Array(groups.enumerated()), id: \.offset) { _, group in
                        HomeSourceGroupSection(group: group)
                    }
                }
            }
        }
        .padding(20)
        .background(
            RoundedRectangle(cornerRadius: DesignTokens.radius_md, style: .continuous)
                .fill(DesignTokens.elevatedColor)
        )
        .overlay(
            RoundedRectangle(cornerRadius: DesignTokens.radius_md, style: .continuous)
                .stroke(DesignTokens.borderColor, lineWidth: 1)
        )

    }
}

private enum HomeTodayBySourceGrouping {
    struct Group {
        let type: ProcessedEvent.SourceType
        let title: String
        let icon: String
        let tint: Color
        let sources: [SourceBucket]
        let totalCount: Int
    }

    struct SourceBucket {
        let name: String
        let items: [ProcessedEvent]
    }

    static func group(_ events: [ProcessedEvent]) -> [Group] {
        // Comment in English.
        let byType = Dictionary(grouping: events) { $0.sourceType }

        let types: [ProcessedEvent.SourceType] = [.url, .calendar, .reminder, .health, .manual]
        var out: [Group] = []

        for type in types {
            guard let list = byType[type], !list.isEmpty else { continue }

            let title = displayTitle(for: type)
            let (icon, tint) = displayStyle(for: type)

            let bySourceName = Dictionary(grouping: list) { ev -> String in
                let name = (ev.metadata["sourceName"] ?? ev.metadata["source"] ?? ev.metadata["name"] ?? "").trimmingCharacters(in: .whitespacesAndNewlines)
                return name.isEmpty ? title : name
            }

            let buckets: [SourceBucket] = bySourceName
                .map { (k, v) in
                    // Comment in English.
                    let sorted = v.sorted {
                        if $0.priority != $1.priority { return $0.priority > $1.priority }
                        return $0.title.localizedCaseInsensitiveCompare($1.title) == .orderedAscending
                    }
                    return SourceBucket(name: k, items: sorted)
                }
                .sorted { lhs, rhs in
                    if lhs.items.count != rhs.items.count { return lhs.items.count > rhs.items.count }
                    return lhs.name.localizedCaseInsensitiveCompare(rhs.name) == .orderedAscending
                }

            out.append(Group(type: type, title: title, icon: icon, tint: tint, sources: buckets, totalCount: list.count))
        }
        return out
    }

    private static func displayTitle(for type: ProcessedEvent.SourceType) -> String {
        switch type {
        case .url:
            return L10n.homeSectionURLContent
        case .calendar:
            return L10n.homeSectionCalendar
        case .reminder:
            return L10n.homeSectionReminders
        case .health:
            return L10n.homeSectionHealth
        case .manual:
            return L10n.homeSectionCustomDataSources
        }
    }

    private static func displayStyle(for type: ProcessedEvent.SourceType) -> (String, Color) {
        switch type {
        case .url:
            return ("link", DesignTokens.primaryColor)
        case .calendar:
            return ("calendar", Color.blue)
        case .reminder:
            return ("checklist", Color.orange)
        case .health:
            return ("heart.text.square", Color.pink)
        case .manual:
            return ("square.and.pencil", Color.secondary)
        }
    }
}

private struct HomeSourceGroupSection: View {
    let group: HomeTodayBySourceGrouping.Group

    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            HStack(spacing: 8) {
                Image(systemName: group.icon)
                    .font(.subheadline.weight(.semibold))
                    .foregroundColor(group.tint)
                Text(group.title)
                    .font(.headline)
                    .foregroundColor(.primary)
                Spacer()
                Text("\(group.totalCount)")
                    .font(.caption.weight(.semibold))
                    .foregroundColor(.secondary)
                    .padding(.horizontal, 8)
                    .padding(.vertical, 4)
                    .background(DesignTokens.mutedFillColor)
                    .clipShape(Capsule())
            }

            ForEach(Array(group.sources.prefix(3).enumerated()), id: \.offset) { _, bucket in
                VStack(alignment: .leading, spacing: 6) {
                    Text(bucket.name)
                        .font(.subheadline.weight(.semibold))
                        .foregroundColor(.primary)
                        .lineLimit(1)

                    ForEach(Array(bucket.items.prefix(2).enumerated()), id: \.offset) { _, ev in
                        HStack(alignment: .top, spacing: 8) {
                            Circle()
                                .fill(group.tint.opacity(0.65))
                                .frame(width: 6, height: 6)
                                .padding(.top, 6)
                            Text(ev.title)
                                .font(.callout)
                                .foregroundColor(.secondary)
                                .lineLimit(2)
                        }
                    }

                    if bucket.items.count > 2 {
                        Text("+\(bucket.items.count - 2)")
                            .font(.caption)
                            .foregroundColor(group.tint)
                    }
                }
                .padding(12)
                .background(
                    RoundedRectangle(cornerRadius: DesignTokens.radius_lg, style: .continuous)
                        .fill(DesignTokens.rowFillColor)
                )
                .overlay(
                    RoundedRectangle(cornerRadius: DesignTokens.radius_lg, style: .continuous)
                        .stroke(DesignTokens.borderColor, lineWidth: 1)
                )
            }
        }
    }
}

// Comment in English.
struct HomeScheduledActionsOverviewCard: View {
    @Environment(\.colorScheme) private var colorScheme

    struct Entry: Identifiable {
        let id: UUID
        let name: String
        let actionId: String
        let latestRun: LocalJSONStorage.ScheduledActionRunRecord?
        let latestText: String?
    }

    let entries: [Entry]
    let onTapAll: () -> Void
    let onTapEntry: (Entry) -> Void

    var body: some View {
        VStack(alignment: .leading, spacing: 14) {
            HStack {
                Image(systemName: "clock.arrow.circlepath")
                    .foregroundColor(DesignTokens.primaryColor)
                Text(L10n.scheduledActionsTitle)
                    .font(.headline)
                    .foregroundColor(.primary)
                Spacer()
                Button(action: onTapAll) {
                    Image(systemName: "chevron.right")
                        .font(.caption.weight(.bold))
                        .foregroundColor(Color(.tertiaryLabel))
                        .padding(6)
                }
                .buttonStyle(.plain)
            }

            if entries.isEmpty {
                Text(L10n.urlSourcesEmptySubtitle)
                    .font(.callout)
                    .foregroundColor(.secondary)
                    .padding(.vertical, 6)
            } else {
                VStack(spacing: 10) {
                    ForEach(Array(entries.prefix(6).enumerated()), id: \.offset) { _, entry in
                        Button {
                            onTapEntry(entry)
                        } label: {
                            HomeScheduledActionRow(entry: entry)
                        }
                        .buttonStyle(.plain)
                    }
                }
            }
        }
        .padding(.vertical, DesignTokens.spacing_sm)
    }
}

private struct HomeScheduledActionRow: View {
    let entry: HomeScheduledActionsOverviewCard.Entry

    private var statusColor: Color {
        guard let run = entry.latestRun else { return .secondary }
        return ScheduledActionRunText.statusPresentation(run.status).tone.color
    }

    private var statusText: String {
        guard let run = entry.latestRun else { return L10n.scheduledActionRunsEmpty }
        return ScheduledActionRunText.statusPresentation(run.status).title
    }

    private var bodyText: String {
        if let t = entry.latestText?.nolaSanitizedSummary(), !t.isEmpty {
            return t
        }
        if let run = entry.latestRun {
            if let reason = run.reason?.nolaSanitizedSummary(), !reason.isEmpty {
                return reason
            }
            if let err = run.errorMessage?.nolaSanitizedSummary(), !err.isEmpty {
                return err
            }
        }
        return statusText
    }

    var body: some View {
        HStack(alignment: .top, spacing: 12) {
            Circle()
                .fill(statusColor)
                .frame(width: 10, height: 10)
                .padding(.top, 6)

            VStack(alignment: .leading, spacing: 4) {
                HStack(alignment: .firstTextBaseline) {
                    Text(entry.name)
                        .font(.subheadline.weight(.semibold))
                        .foregroundColor(.primary)
                        .lineLimit(1)
                    Spacer()
                    if let t = entry.latestRun?.finishedAt {
                        Text(t.formatted(date: .omitted, time: .shortened))
                            .font(.caption)
                            .foregroundColor(.secondary)
                    }
                }

                Text(bodyText)
                    .font(.callout)
                    .foregroundColor(.secondary)
                    .lineLimit(3)
            }
        }
        .padding(12)
        .background(
            RoundedRectangle(cornerRadius: DesignTokens.radius_lg, style: .continuous)
                .fill(DesignTokens.rowFillColor)
        )
        .overlay(
            RoundedRectangle(cornerRadius: DesignTokens.radius_lg, style: .continuous)
                .stroke(DesignTokens.borderColor, lineWidth: 1)
        )
    }
}

// Comment in English.
/// Documentation comment in English.
struct HomeSummaryCard: View {
    let summary: ModelOutput?
    let isLoading: Bool
    let error: String?
    let onGenerate: () -> Void

    var body: some View {
        VStack(alignment: .leading, spacing: 16) {
            // Header
            HStack {
                Image(systemName: "sparkles")
                    .foregroundColor(DesignTokens.primaryColor)
                Text(L10n.homeSummaryTitle)
                    .font(.title3)
                    .fontWeight(.bold)
                Spacer()
                if isLoading {
                    ProgressView()
                        .scaleEffect(0.8)
                }
            }

            if let summary = summary, !summary.items.isEmpty {
                // Content: Explain text
                if let explain = summary.explain, !explain.isEmpty {
                    Text(explain.nolaSanitizedSummary())
                        .font(.body)
                        .foregroundColor(.primary)
                        .lineLimit(4)
                        .fixedSize(horizontal: false, vertical: true)
                }

                NolaDivider()
                    .padding(.vertical, 4)

                // Top 2 items preview
                VStack(alignment: .leading, spacing: 10) {
                    ForEach(Array(summary.items.prefix(2).enumerated()), id: \.offset) { _, item in
                        HStack(alignment: .top, spacing: 8) {
                            Text("•")
                                .foregroundColor(.secondary)
                            Text(item.text.nolaSanitizedSummary())
                                .font(.callout)
                                .foregroundColor(.secondary)
                                .lineLimit(2)
                        }
                    }
                    
                    if summary.items.count > 2 {
                        Text(String(format: L10n.homeSummaryItemsCount, summary.items.count))
                            .font(.caption)
                            .foregroundColor(DesignTokens.primaryColor)
                            .padding(.top, 4)
                    }
                }
            } else if let error = error {
                // Error State
                VStack(alignment: .leading, spacing: 8) {
                    Text(error)
                        .font(.callout)
                        .foregroundColor(DesignTokens.dangerColor)
                    
                    Button(action: onGenerate) {
                        Text(L10n.homeRefresh)
                            .font(.callout.bold())
                            .foregroundColor(DesignTokens.primaryColor)
                    }
                }
            } else {
                // Empty State
                VStack(alignment: .leading, spacing: 12) {
                    Text(L10n.homeNoTodosMessage)
                        .font(.callout)
                        .foregroundColor(.secondary)
                        .multilineTextAlignment(.leading)
                    
                    Button(action: onGenerate) {
                        HStack {
                            Image(systemName: "arrow.clockwise")
                            Text(L10n.homeRefresh)
                        }
                        .fontWeight(.medium)
                        .padding(.vertical, 8)
                        .padding(.horizontal, 16)
                        .background(Color.primary.opacity(0.05))
                        .cornerRadius(8)
                    }
                }
                .frame(maxWidth: .infinity, alignment: .leading)
                .padding(.vertical, 12)
            }
        }
        .padding(20)
        .background(
            RoundedRectangle(cornerRadius: DesignTokens.radius_md, style: .continuous)
                .fill(DesignTokens.elevatedColor)
        )
        .overlay(
            RoundedRectangle(cornerRadius: DesignTokens.radius_md, style: .continuous)
                .stroke(DesignTokens.borderColor, lineWidth: 1)
        )
    }
}

// Comment in English.
/// Documentation comment in English.
struct HomeLatestActivityCard: View {
    let run: LocalJSONStorage.ScheduledActionRunRecord?
    let onTap: () -> Void
    
    // Status helpers
    private var statusIcon: String {
        guard let run = run else { return "clock.arrow.circlepath" }
        return ScheduledActionRunText.statusPresentation(run.status).symbol
    }
    
    private var statusColor: Color {
        guard let run = run else { return .secondary }
        return ScheduledActionRunText.statusPresentation(run.status).tone.color
    }

    var body: some View {
        Button(action: onTap) {
            VStack(alignment: .leading, spacing: 14) {
                // Header
                HStack {
                    Image(systemName: "list.bullet.clipboard")
                        .foregroundColor(DesignTokens.primaryColor)
                    Text(L10n.scheduledActionsTitle) 
                        .font(.headline)
                        .foregroundColor(.primary)
                    Spacer()
                    Image(systemName: "chevron.right")
                        .font(.caption.weight(.bold))
                        .foregroundColor(Color(.tertiaryLabel))
                }

                if let run = run {
                    HStack(alignment: .top, spacing: 12) {
                        // Icon
                        Image(systemName: statusIcon)
                            .font(.system(size: 20))
                            .foregroundColor(statusColor)
                            .frame(width: 24)
                        
                        VStack(alignment: .leading, spacing: 4) {
                            // Title & Time
                            HStack {
                                Text(run.actionName ?? run.actionType)
                                    .font(.subheadline)
                                    .fontWeight(.medium)
                                    .foregroundColor(.primary)
                                Spacer()
                                Text(run.finishedAt.formatted(date: .omitted, time: .shortened))
                                    .font(.caption)
                                    .foregroundColor(.secondary)
                            }
                            
                            // Details
                            Text(detailsText(for: run))
                                .font(.caption)
                                .foregroundColor(.secondary)
                                .lineLimit(2)
                                .multilineTextAlignment(.leading)
                        }
                    }
                    .padding(12)
                    .background(
                        RoundedRectangle(cornerRadius: DesignTokens.radius_lg, style: .continuous)
                            .fill(DesignTokens.rowFillColor)
                    )
                    .overlay(
                        RoundedRectangle(cornerRadius: DesignTokens.radius_lg, style: .continuous)
                            .stroke(DesignTokens.borderColor, lineWidth: 1)
                    )
                } else {
                    // Empty state
                    HStack {
                        Text(L10n.scheduledActionRunsEmpty)
                            .font(.callout)
                            .foregroundColor(.secondary)
                        Spacer()
                    }
                    .padding(12)
                }
            }
            .padding(20)
            .background(
                RoundedRectangle(cornerRadius: DesignTokens.radius_md, style: .continuous)
                    .fill(DesignTokens.elevatedColor)
            )
            .overlay(
                RoundedRectangle(cornerRadius: DesignTokens.radius_md, style: .continuous)
                    .stroke(DesignTokens.borderColor, lineWidth: 1)
            )
        }
        .buttonStyle(.plain)
    }
    
    private func detailsText(for run: LocalJSONStorage.ScheduledActionRunRecord) -> String {
        if let failureReason = run.failureReason?.trimmingCharacters(in: .whitespacesAndNewlines), !failureReason.isEmpty {
            return failureReason
        }
        let mappedReason = ScheduledActionRunText.reasonSummaryText(for: run)
        if !mappedReason.isEmpty { return mappedReason }
        if let count = run.eventsCount, count > 0 {
            return String(format: L10n.homeSummaryItemsCount, count)
        }
        return ScheduledActionRunText.statusPresentation(run.status).title
    }
}

// MARK: - Header Date View
struct HeaderDateView: View {
    @EnvironmentObject var container: ServiceContainer

    var body: some View {
        HStack {
            VStack(alignment: .leading, spacing: 4) {
                Text(DateFormatter.localizedString(from: Date(), dateStyle: .full, timeStyle: .none).uppercased())
                    .font(.caption)
                    .fontWeight(.semibold)
                    .foregroundColor(.secondary)
                
                Text(greetingText())
                    .font(.largeTitle)
                    .fontWeight(.bold)
                    .foregroundColor(.primary)
            }
            Spacer()

            // Inline notification icon aligned with header content
            NotificationIconView()
                .environmentObject(container)
                .padding(.top, 12)
                .padding(.trailing, 16)
        }
        .padding(.vertical, 10)
    }
    
    private func greetingText() -> String {
        let hour = Calendar.current.component(.hour, from: Date())
        if hour < 12 { return L10n.homeGreetingMorning }
        if hour < 18 { return L10n.homeGreetingAfternoon }
        return L10n.homeGreetingEvening
    }
}

// MARK: - Full Summary Detail View
struct HomeSummaryDetailView: View {
    let processedEvents: [ProcessedEvent]
    let summary: ModelOutput
    
    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 20) {
                if let explain = summary.explain {
                    Text(explain.nolaSanitizedSummary())
                        .font(.body)
                        .foregroundColor(.primary)
                        .padding()
                        .background(
                            RoundedRectangle(cornerRadius: DesignTokens.radius_lg, style: .continuous)
                                .fill(DesignTokens.elevatedColor)
                        )
                        .overlay(
                            RoundedRectangle(cornerRadius: DesignTokens.radius_lg, style: .continuous)
                                .stroke(DesignTokens.borderColor, lineWidth: 1)
                        )
                        .padding(.horizontal)
                }
                
                if !summary.items.isEmpty {
                    let sanitizedItems = summary.items.map { item in
                        SummaryItem(text: item.text.nolaSanitizedSummary(), priority: item.priority, dedupeKey: item.dedupeKey, action: item.action)
                    }
                    LazyVStack(alignment: .leading, spacing: 12) {
                        ForEach(Array(sanitizedItems.enumerated()), id: \.offset) { _, item in
                            SummaryItemCard(item: item, prefersEventDetail: false)
                        }
                    }
                    .padding(.horizontal)
                }
            }
            .frame(maxWidth: .infinity, alignment: .leading)
            .padding(.vertical)
        }
        .background(Color(.systemGroupedBackground))
        .navigationTitle(NSLocalizedString("data.title", value: "数据", comment: "Data tab page title"))
    }
}

// MARK: - Text Sanitization
// Used in multiple UI surfaces (Home, Scheduled Actions, notifications).
extension String {
    func nolaSanitizedSummary() -> String {
        self
            .replacingOccurrences(of: "{{", with: "")
            .replacingOccurrences(of: "}}", with: "")
            .trimmingCharacters(in: .whitespacesAndNewlines)
    }
}
