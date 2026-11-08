// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Native notification inbox with delivery state, cleanup, and conversation deep links.

import AppKit
import SwiftUI
import UserNotifications

enum MacNotificationCenterLayoutPolicy {
    static func usesCompactNavigation(availableWidth: CGFloat) -> Bool {
        availableWidth < 760
    }
}

private func macNotificationText(_ key: String, _ fallback: String) -> String {
    let keyed = Bundle.main.localizedString(forKey: key, value: nil, table: nil)
    guard keyed == key else { return keyed }
    return Bundle.main.localizedString(forKey: fallback, value: fallback, table: nil)
}

struct MacNotificationCenterView: View {
    private enum Filter: String, CaseIterable, Identifiable {
        case all
        case unread
        case scheduled

        var id: String { rawValue }
        var title: String {
            switch self {
            case .all: return macNotificationText("mac.notifications.filter.all", "All")
            case .unread: return macNotificationText("mac.notifications.filter.unread", "Unread")
            case .scheduled: return macNotificationText("mac.notifications.filter.scheduled", "Scheduled")
            }
        }
    }

    @Environment(\.openWindow) private var openWindow
    @State private var store = MacNotificationCenterStore()
    @State private var selection: String?
    @State private var filter = Filter.all
    @State private var query = ""
    @State private var confirmsClearHistory = false
    @State private var pendingRemoval: MacNotificationInboxItem?
    @State private var compactDetailIsPresented = false
    @Environment(\.accessibilityReduceMotion) private var reduceMotion

    var body: some View {
        VStack(spacing: 0) {
            notificationHeader

            if let errorMessage = store.errorMessage {
                MacInlineNotice(
                    title: macNotificationText("mac.notifications.error.title", "Notifications Could Not Be Updated"),
                    message: errorMessage,
                    kind: .failure,
                    actionTitle: macNotificationText("mac.notifications.refresh", "Refresh"),
                    action: { Task { await store.refresh() } },
                    dismiss: { store.errorMessage = nil }
                )
                .padding(.horizontal, MacDesignTokens.Layout.pageGutter)
                .padding(.vertical, MacDesignTokens.Spacing.sm)
                .transition(.opacity.combined(with: .move(edge: .top)))
            }

            Divider().opacity(0.55)
            GeometryReader { proxy in
                if MacNotificationCenterLayoutPolicy.usesCompactNavigation(availableWidth: proxy.size.width) {
                    compactContent
                } else {
                    HSplitView {
                        notificationSidebar
                            .frame(minWidth: 280, idealWidth: 320, maxWidth: 390)
                        detail
                            .frame(minWidth: 420, maxWidth: .infinity, maxHeight: .infinity)
                    }
                }
            }
        }
        .background(MacPageBackground())
        .task {
            await store.refresh()
            if selection == nil { selection = matchingItems.first?.id }
        }
        .onReceive(NotificationCenter.default.publisher(for: .nolaMacNotificationsDidChange)) { _ in
            Task { await store.refresh() }
        }
        .onReceive(NotificationCenter.default.publisher(for: NSApplication.didBecomeActiveNotification)) { _ in
            Task { await store.refresh() }
        }
        .onChange(of: selection) { _, _ in
            guard let selectedItem, !selectedItem.isRead else { return }
            Task { await store.markRead(selectedItem) }
        }
        .onChange(of: matchingItems.map(\.id)) { _, ids in
            if let selection, ids.contains(selection) { return }
            selection = ids.first
        }
        .confirmationDialog(
            macNotificationText("mac.notifications.clear.confirm.title", "Clear Notification History?"),
            isPresented: $confirmsClearHistory,
            titleVisibility: .visible
        ) {
            Button(macNotificationText("base.js.confirm.cancel", "Cancel"), role: .cancel) {}
            Button(macNotificationText("mac.notifications.clear_history", "Clear History"), role: .destructive) {
                Task {
                    await store.clearDelivered()
                    selection = nil
                }
            }
        } message: {
            Text(macNotificationText("mac.notifications.clear.confirm.message", "Delivered notification history will be removed. Scheduled notifications are kept."))
        }
        .confirmationDialog(
            pendingRemovalTitle,
            isPresented: Binding(
                get: { pendingRemoval != nil },
                set: { if !$0 { pendingRemoval = nil } }
            ),
            titleVisibility: .visible,
            presenting: pendingRemoval
        ) { item in
            Button(
                item.deliveryState == .scheduled
                    ? macNotificationText("mac.notifications.cancel", "Cancel Notification")
                    : macNotificationText("mac.notifications.remove", "Remove"),
                role: .destructive
            ) {
                Task {
                    await store.remove(item)
                    if selection == item.id { selection = nil }
                    pendingRemoval = nil
                }
            }
            Button(macNotificationText("base.js.confirm.cancel", "Cancel"), role: .cancel) {
                pendingRemoval = nil
            }
        } message: { item in
            Text(item.deliveryState == .scheduled
                ? macNotificationText("mac.notifications.cancel.confirm.message", "This notification will no longer be delivered at its scheduled time.")
                : macNotificationText("mac.notifications.remove.confirm.message", "This notification will be removed from local history."))
        }
        .animation(reduceMotion ? nil : .easeOut(duration: 0.18), value: store.errorMessage)
        .animation(
            reduceMotion ? nil : MacDesignTokens.Motion.disclosureAnimation,
            value: compactDetailIsPresented
        )
    }

    @ViewBuilder
    private var compactContent: some View {
        if compactDetailIsPresented, selectedItem != nil {
            VStack(spacing: 0) {
                HStack {
                    Button {
                        compactDetailIsPresented = false
                    } label: {
                        Label(
                            macNotificationText("mac.notifications.title", "Notifications"),
                            systemImage: "chevron.left"
                        )
                    }
                    .buttonStyle(.borderless)
                    Spacer()
                }
                .padding(.horizontal, MacDesignTokens.Spacing.lg)
                .padding(.vertical, MacDesignTokens.Spacing.sm)
                .background(MacDesignColor.chrome.opacity(0.96))
                .overlay(alignment: .bottom) { Divider().opacity(0.55) }

                detail
            }
            .transition(.move(edge: .trailing).combined(with: .opacity))
        } else {
            notificationSidebar
                .transition(.move(edge: .leading).combined(with: .opacity))
        }
    }

    private var pendingRemovalTitle: String {
        pendingRemoval?.deliveryState == .scheduled
            ? macNotificationText("mac.notifications.cancel.confirm.title", "Cancel Scheduled Notification?")
            : macNotificationText("mac.notifications.remove.confirm.title", "Remove Notification?")
    }

    private var notificationHeader: some View {
        HStack(spacing: 10) {
            Image(systemName: "bell.badge")
                .font(.system(size: 14, weight: .semibold))
                .foregroundStyle(Color.accentColor)
                .frame(width: 28, height: 28)
                .background(Color.accentColor.opacity(0.1), in: RoundedRectangle(cornerRadius: 8))
            VStack(alignment: .leading, spacing: 2) {
                Text(macNotificationText("mac.notifications.title", "Notifications"))
                    .font(.system(size: 15, weight: .semibold))
                Text(macNotificationText("mac.notifications.empty.detail", "Agent completions and scheduled notifications will appear here."))
                    .font(.system(size: 11.5))
                    .foregroundStyle(.secondary)
                    .lineLimit(1)
            }
            Spacer(minLength: 16)
            if store.isLoading {
                ProgressView().controlSize(.small)
            }
            Button {
                Task { await store.refresh() }
            } label: {
                Image(systemName: "arrow.clockwise")
            }
            .macToolbarIcon()
            .disabled(store.isLoading)
            .help(macNotificationText("mac.notifications.refresh", "Refresh"))

            Menu {
                Button(macNotificationText("mac.notifications.mark_all_read", "Mark All Read")) {
                    Task { await store.markAllRead() }
                }
                .disabled(store.unreadCount == 0)
                Divider()
                Button(macNotificationText("mac.notifications.clear_history", "Clear History"), role: .destructive) {
                    confirmsClearHistory = true
                }
                .disabled(store.items.allSatisfy { $0.deliveryState == .scheduled })
            } label: {
                Image(systemName: "ellipsis")
            }
            .menuStyle(.borderlessButton)
            .frame(width: MacDesignTokens.Control.iconButton, height: MacDesignTokens.Control.iconButton)
            .help(macNotificationText("mac.notifications.actions", "Actions"))
        }
        .padding(.horizontal, MacDesignTokens.Layout.pageGutter)
        .frame(height: MacDesignTokens.Control.pageHeader)
        .background(MacDesignColor.chrome)
    }

    private var notificationSidebar: some View {
        VStack(spacing: 10) {
            Picker(macNotificationText("mac.notifications.filter.label", "Filter"), selection: $filter) {
                ForEach(Filter.allCases) { filter in
                    Text(filterTitle(filter)).tag(filter)
                }
            }
            .pickerStyle(.segmented)
            .labelsHidden()

            TextField(macNotificationText("mac.notifications.search", "Search notifications"), text: $query)
                .textFieldStyle(.roundedBorder)

            List {
                if needsAuthorizationAttention {
                    authorizationSection
                }
                if !inboxItems.isEmpty {
                    Section(macNotificationText("mac.notifications.inbox", "Inbox")) {
                        ForEach(inboxItems) { item in
                            selectableRow(item)
                        }
                    }
                }
                if !scheduledItems.isEmpty {
                    Section(macNotificationText("mac.notifications.scheduled", "Scheduled")) {
                        ForEach(scheduledItems) { item in
                            selectableRow(item)
                        }
                    }
                }
                if matchingItems.isEmpty, !store.items.isEmpty, !store.isLoading {
                    Section {
                        VStack(alignment: .leading, spacing: 8) {
                            Label(
                                macNotificationText("mac.notifications.empty.filtered.title", "No Matching Notifications"),
                                systemImage: "line.3.horizontal.decrease.circle"
                            )
                            .font(.system(size: 13, weight: .semibold))
                            Text(macNotificationText("mac.notifications.empty.filtered.detail", "Adjust the search or filter to see more notifications."))
                                .font(.system(size: 11.5))
                                .foregroundStyle(.secondary)
                            Button(macNotificationText("mac.notifications.empty.filtered.reset", "Reset Filters")) {
                                query = ""
                                filter = .all
                            }
                            .buttonStyle(.borderless)
                        }
                        .padding(.vertical, 8)
                    }
                }
            }
            .listStyle(.sidebar)
            .scrollContentBackground(.hidden)
        }
        .padding(12)
        .background(MacDesignColor.sidebar)
    }

    @ViewBuilder
    private var authorizationSection: some View {
        Section {
            HStack(spacing: 9) {
                Image(systemName: authorizationSymbol)
                    .foregroundStyle(authorizationColor)
                    .frame(width: 18)
                VStack(alignment: .leading, spacing: 2) {
                    Text(authorizationTitle)
                        .font(.system(size: 13, weight: .semibold))
                    Text(macNotificationText("mac.notifications.permission.detail", "System delivery permission"))
                        .font(.system(size: 11.5))
                        .foregroundStyle(.secondary)
                }
                Spacer()
                if store.authorizationStatus == .notDetermined {
                    Button(macNotificationText("mac.notifications.permission.enable", "Enable")) { Task { await store.requestAuthorization() } }
                        .buttonStyle(.borderedProminent)
                        .controlSize(.small)
                } else if store.authorizationStatus == .denied {
                    Button(macNotificationText("mac.notifications.permission.settings", "Open Settings")) {
                        if let url = URL(string: "x-apple.systempreferences:com.apple.Notifications-Settings.extension") {
                            NSWorkspace.shared.open(url)
                        }
                    }
                    .buttonStyle(.bordered)
                    .controlSize(.small)
                }
            }
            .padding(.vertical, 4)
        }
    }

    @ViewBuilder
    private var detail: some View {
        if store.isLoading && store.items.isEmpty {
            MacLoadingState(macNotificationText("mac.notifications.loading", "Loading notifications…"))
            .frame(maxWidth: .infinity, maxHeight: .infinity)
            .background(MacPageBackground())
        } else if let item = selectedItem {
            ScrollView {
                VStack(alignment: .leading, spacing: 22) {
                    ViewThatFits(in: .horizontal) {
                        HStack(alignment: .top, spacing: 16) {
                            notificationIdentity(item)
                            Spacer(minLength: 18)
                            detailActions(item)
                        }
                        VStack(alignment: .leading, spacing: 16) {
                            notificationIdentity(item)
                            detailActions(item)
                        }
                    }

                    Text(item.body.isEmpty ? macNotificationText("mac.notifications.no_details", "No additional details") : item.body)
                        .font(.body)
                        .textSelection(.enabled)
                        .fixedSize(horizontal: false, vertical: true)
                        .padding(18)
                        .frame(maxWidth: .infinity, alignment: .leading)
                        .macPanel(cornerRadius: MacDesignTokens.Radius.floating)
                }
                .padding(34)
                .frame(maxWidth: 860, alignment: .leading)
                .frame(maxWidth: .infinity, alignment: .topLeading)
            }
            .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
            .background(MacPageBackground())
        } else if store.items.isEmpty && !store.isLoading {
            VStack(spacing: MacDesignTokens.Spacing.lg) {
                Image(systemName: "bell.slash")
                    .font(.system(size: 21, weight: .medium))
                    .foregroundStyle(Color.accentColor)
                    .frame(width: 48, height: 48)
                    .background(Color.accentColor.opacity(0.085), in: RoundedRectangle(cornerRadius: 13))
                Text(macNotificationText("mac.notifications.empty.title", "No Notifications"))
                    .font(.title3.weight(.semibold))
                Text(macNotificationText("mac.notifications.empty.detail", "Agent completions and scheduled notifications will appear here."))
                    .font(.callout)
                    .foregroundStyle(.secondary)
                    .multilineTextAlignment(.center)
                    .frame(maxWidth: 420)
            }
            .frame(maxWidth: .infinity, maxHeight: .infinity)
        } else if matchingItems.isEmpty && !store.isLoading {
            ContentUnavailableView {
                Label(
                    macNotificationText("mac.notifications.empty.filtered.title", "No Matching Notifications"),
                    systemImage: "line.3.horizontal.decrease.circle"
                )
            } description: {
                Text(macNotificationText("mac.notifications.empty.filtered.detail", "Adjust the search or filter to see more notifications."))
            } actions: {
                Button(macNotificationText("mac.notifications.empty.filtered.reset", "Reset Filters")) {
                    query = ""
                    filter = .all
                }
                .buttonStyle(.borderedProminent)
            }
        } else {
            ContentUnavailableView(macNotificationText("mac.notifications.select", "Select a Notification"), systemImage: "bell")
        }
    }

    private func notificationIdentity(_ item: MacNotificationInboxItem) -> some View {
        HStack(alignment: .top, spacing: 16) {
            Image(systemName: item.deliveryState == .scheduled ? "clock.badge.fill" : "bell.badge.fill")
                .font(.system(size: 24, weight: .medium))
                .foregroundStyle(item.deliveryState == .scheduled ? Color.orange : Color.accentColor)
                .frame(width: 48, height: 48)
                .background(Color.primary.opacity(0.055), in: RoundedRectangle(cornerRadius: 13))
            VStack(alignment: .leading, spacing: 5) {
                Text(item.title)
                    .font(.system(size: 19, weight: .semibold))
                    .fixedSize(horizontal: false, vertical: true)
                Text(item.date.formatted(date: .abbreviated, time: .shortened))
                    .font(.callout)
                    .foregroundStyle(.secondary)
                Label(
                    item.deliveryState == .scheduled
                        ? macNotificationText("mac.notifications.status.scheduled", "Scheduled")
                        : macNotificationText("mac.notifications.status.delivered", "Delivered"),
                    systemImage: item.deliveryState == .scheduled ? "clock" : "checkmark.circle"
                )
                .font(.caption.weight(.medium))
                .foregroundStyle(item.deliveryState == .scheduled ? Color.orange : Color.green)
            }
        }
    }

    private func detailActions(_ item: MacNotificationInboxItem) -> some View {
        ViewThatFits(in: .horizontal) {
            HStack(spacing: 8) { detailActionButtons(item) }
            VStack(alignment: .leading, spacing: 8) { detailActionButtons(item) }
        }
    }

    @ViewBuilder
    private func detailActionButtons(_ item: MacNotificationInboxItem) -> some View {
        if let conversationID = item.conversationID {
            Button {
                openWindow(value: conversationID)
            } label: {
                Label(
                    macNotificationText("mac.notifications.open_conversation", "Open Conversation"),
                    systemImage: "arrow.up.forward"
                )
            }
            .buttonStyle(.borderedProminent)
        }
        if !item.isRead, item.deliveryState != .scheduled {
            Button(macNotificationText("mac.notifications.mark_read", "Mark Read")) {
                Task { await store.markRead(item) }
            }
        }
        Button(
            item.deliveryState == .scheduled
                ? macNotificationText("mac.notifications.cancel", "Cancel Notification")
                : macNotificationText("mac.notifications.remove", "Remove"),
            role: .destructive
        ) {
            pendingRemoval = item
        }
    }

    private func row(_ item: MacNotificationInboxItem) -> some View {
        HStack(alignment: .top, spacing: 10) {
            ZStack(alignment: .topTrailing) {
                Image(systemName: item.deliveryState == .scheduled ? "clock.fill" : "bell.fill")
                    .font(.system(size: 12, weight: .semibold))
                    .foregroundStyle(item.deliveryState == .scheduled ? Color.orange : Color.accentColor)
                    .frame(width: 30, height: 30)
                    .background(Color.primary.opacity(0.055), in: RoundedRectangle(cornerRadius: 9, style: .continuous))
                if !item.isRead, item.deliveryState != .scheduled {
                    Circle()
                        .fill(Color.accentColor)
                        .frame(width: 7, height: 7)
                        .overlay {
                            Circle().stroke(Color(nsColor: .controlBackgroundColor), lineWidth: 1.5)
                        }
                        .offset(x: 2, y: -2)
                }
            }
            VStack(alignment: .leading, spacing: 4) {
                Text(item.title)
                    .font(.callout.weight(item.isRead ? .regular : .semibold))
                    .lineLimit(1)
                if !item.body.isEmpty {
                    Text(item.body)
                        .font(.caption)
                        .foregroundStyle(.secondary)
                        .lineLimit(2)
                }
                HStack(spacing: 5) {
                    Label(
                        item.deliveryState == .scheduled
                            ? macNotificationText("mac.notifications.status.scheduled", "Scheduled")
                            : macNotificationText("mac.notifications.status.delivered", "Delivered"),
                        systemImage: item.deliveryState == .scheduled ? "clock" : "checkmark.circle"
                    )
                    .foregroundStyle(item.deliveryState == .scheduled ? Color.orange : Color.secondary)
                    Spacer(minLength: 4)
                    Text(RelativeDateTimeFormatter().localizedString(for: item.date, relativeTo: Date()))
                        .foregroundStyle(.secondary)
                }
                .font(.caption.weight(.medium))
            }
            if item.conversationID != nil {
                Image(systemName: "arrow.up.forward")
                    .font(.system(size: 9.5, weight: .semibold))
                    .foregroundStyle(.tertiary)
                    .accessibilityHidden(true)
            }
        }
        .padding(.vertical, 6)
        .onTapGesture(count: 2) {
            if let conversationID = item.conversationID {
                openWindow(value: conversationID)
            }
        }
        .contextMenu {
            if let conversationID = item.conversationID {
                Button {
                    openWindow(value: conversationID)
                } label: {
                    Label(
                        macNotificationText("mac.notifications.open_conversation", "Open Conversation"),
                        systemImage: "arrow.up.forward"
                    )
                }
                Divider()
            }
            if !item.isRead, item.deliveryState != .scheduled {
                Button(macNotificationText("mac.notifications.mark_read", "Mark Read")) { Task { await store.markRead(item) } }
            }
            Button(item.deliveryState == .scheduled ? macNotificationText("mac.notifications.cancel", "Cancel Notification") : macNotificationText("mac.notifications.remove", "Remove"), role: .destructive) {
                pendingRemoval = item
            }
        }
    }

    private func selectableRow(_ item: MacNotificationInboxItem) -> some View {
        Button {
            selection = item.id
            compactDetailIsPresented = true
        } label: {
            row(item)
                .contentShape(Rectangle())
        }
            .buttonStyle(.plain)
            .macSelectableListRow(isSelected: selection == item.id)
            .accessibilityAddTraits(selection == item.id ? .isSelected : [])
    }

    private var selectedItem: MacNotificationInboxItem? {
        guard let selection else { return nil }
        return store.items.first { $0.id == selection }
    }

    private var matchingItems: [MacNotificationInboxItem] {
        let trimmedQuery = query.trimmingCharacters(in: .whitespacesAndNewlines)
        return store.items.filter { item in
            let filterMatches: Bool
            switch filter {
            case .all: filterMatches = true
            case .unread: filterMatches = item.deliveryState != .scheduled && !item.isRead
            case .scheduled: filterMatches = item.deliveryState == .scheduled
            }
            guard filterMatches else { return false }
            guard !trimmedQuery.isEmpty else { return true }
            return item.title.localizedCaseInsensitiveContains(trimmedQuery)
                || item.body.localizedCaseInsensitiveContains(trimmedQuery)
        }
    }

    private var inboxItems: [MacNotificationInboxItem] {
        matchingItems.filter { $0.deliveryState != .scheduled }
    }

    private var scheduledItems: [MacNotificationInboxItem] {
        matchingItems.filter { $0.deliveryState == .scheduled }
    }

    private var needsAuthorizationAttention: Bool {
        switch store.authorizationStatus {
        case .notDetermined, .denied: return true
        case .authorized, .provisional, .ephemeral: return false
        @unknown default: return true
        }
    }

    private var authorizationTitle: String {
        switch store.authorizationStatus {
        case .authorized, .provisional, .ephemeral: return macNotificationText("mac.notifications.status.enabled", "Notifications enabled")
        case .denied: return macNotificationText("mac.notifications.status.disabled", "Notifications disabled")
        case .notDetermined: return macNotificationText("mac.notifications.status.unconfigured", "Notifications not configured")
        @unknown default: return macNotificationText("mac.notifications.status.unavailable", "Notification status unavailable")
        }
    }

    private var authorizationSymbol: String {
        switch store.authorizationStatus {
        case .authorized, .provisional, .ephemeral: return "checkmark.circle.fill"
        case .denied: return "xmark.circle.fill"
        case .notDetermined: return "questionmark.circle.fill"
        @unknown default: return "exclamationmark.circle.fill"
        }
    }

    private var authorizationColor: Color {
        switch store.authorizationStatus {
        case .authorized, .provisional, .ephemeral: return .green
        case .denied: return .red
        case .notDetermined: return .orange
        @unknown default: return .secondary
        }
    }

    private func filterTitle(_ filter: Filter) -> String {
        let count: Int
        switch filter {
        case .all:
            count = store.items.count
        case .unread:
            count = store.unreadCount
        case .scheduled:
            count = store.items.filter { $0.deliveryState == .scheduled }.count
        }
        return "\(filter.title) (\(count))"
    }
}
