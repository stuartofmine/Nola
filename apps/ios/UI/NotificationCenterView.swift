// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Implements NotificationCenterView logic in the iOS UI layer.
// - Interfaces:
//   - FilterMode
//   - NotificationCenterView
//   - NotificationCenterView_Previews
// - APIs:
//   - none
//
import SwiftUI

enum FilterMode: String {
    case all
    case unread

    var title: String {
        switch self {
        case .all: return L10n.notificationsFilterAll
        case .unread: return L10n.notificationsFilterUnread
        }
    }
}

private struct NotificationCenterRoute: Hashable {
    let entityId: String?
    let lookupKey: String

    init(entity: NotificationEntity) {
        self.entityId = entity.id
        self.lookupKey = entity.dedupeKey ?? entity.id
    }

    init(lookupKey: String) {
        self.entityId = nil
        self.lookupKey = lookupKey
    }
}

struct NotificationCenterView: View {
    @Environment(\.dismiss) private var dismiss
    @EnvironmentObject var container: ServiceContainer
    @State private var items: [NotificationEntity] = []
    @State private var isLoading: Bool = false
    @State private var isLoadingMore: Bool = false
    @State private var hasMoreHistory: Bool = false
    @State private var pageSize: Int = 25
    @State private var offset: Int = 0
    @State private var filter: FilterMode = .all
    @State private var query: String = ""
    @State private var navPath = NavigationPath()
    @State private var suppressNextNotificationRefresh = false
    @State private var queryRefreshTask: Task<Void, Never>? = nil
    @State private var handledDeepLinkRequest: String?
    @State private var embeddedDeepLink: NotificationCenterRoute?
    @State private var loadError: String?
    @State private var operationError: String?
    @State private var isMarkingAllRead = false
    let initialDeepLinkKey: String?
    let ownsNavigationStack: Bool
    let deepLinkRequestID: UUID?

    init(initialDeepLinkKey: String? = nil, ownsNavigationStack: Bool = true, deepLinkRequestID: UUID? = nil) {
        self.initialDeepLinkKey = initialDeepLinkKey
        self.ownsNavigationStack = ownsNavigationStack
        self.deepLinkRequestID = deepLinkRequestID
    }

    var body: some View {
        if ownsNavigationStack {
            NavigationStack(path: $navPath) {
                content
                    .notificationRouteDestination { route in
                        detailDestination(for: route)
                    }
            }
        } else {
            content.navigationDestination(item: $embeddedDeepLink) { route in
                detailDestination(for: route)
            }
        }
    }

    private var content: some View {
        Group {
            if isLoading && items.isEmpty {
                ProgressView()
                    .controlSize(.regular)
                    .tint(DesignTokens.primaryColor)
                    .padding(18)
                    .background(
                        DesignTokens.primaryColor.opacity(0.07),
                        in: RoundedRectangle(cornerRadius: 16, style: .continuous)
                    )
                    .frame(maxWidth: .infinity, maxHeight: .infinity)
                    .transition(.opacity)
            } else if let loadError, items.isEmpty {
                VStack {
                    NolaInlineEmptyState(
                        icon: "exclamationmark.arrow.triangle.2.circlepath",
                        title: NSLocalizedString("notifications.load_failed.title", value: "Couldn't load notifications", comment: "Notification load error title"),
                        message: loadError,
                        tint: .orange,
                        actionTitle: NSLocalizedString("common.retry", value: "Retry", comment: "Retry"),
                        action: {
                            Task { await loadHistory() }
                        }
                    )
                    .padding(.horizontal, DesignTokens.spacing_lg)
                    .padding(.top, DesignTokens.spacing_lg)
                    Spacer()
                }
                .frame(maxWidth: .infinity, alignment: .leading)
            } else if items.isEmpty {
                emptyState
            } else {
                List {
                    if let errorMessage = loadError ?? operationError {
                        Section {
                            HStack(alignment: .top, spacing: 10) {
                                Image(systemName: "exclamationmark.triangle.fill")
                                    .foregroundStyle(.orange)
                                Text(errorMessage)
                                    .font(.footnote)
                                    .foregroundStyle(.secondary)
                                    .frame(maxWidth: .infinity, alignment: .leading)
                                if loadError != nil {
                                    Button(NSLocalizedString("common.retry", value: "Retry", comment: "Retry")) {
                                        Task { await loadHistory(showLoading: false) }
                                    }
                                    .buttonStyle(.borderless)
                                } else {
                                    Button {
                                        operationError = nil
                                    } label: {
                                        Image(systemName: "xmark")
                                    }
                                    .buttonStyle(.borderless)
                                    .accessibilityLabel(NSLocalizedString("common.close", value: "Close", comment: "Close"))
                                }
                            }
                        }
                    }
                    ForEach(groupedByDay(items.sorted(by: { $0.createdAt > $1.createdAt })), id: \.0) { (day, list) in
                        Section(header: Text(day)) {
                            ForEach(list) { item in
                                notificationLink(for: item)
                                .listRowInsets(EdgeInsets(top: 5, leading: 16, bottom: 5, trailing: 16))
                                .listRowSeparator(.hidden)
                                .listRowBackground(Color.clear)
                                .swipeActions(edge: .trailing, allowsFullSwipe: true) {
                                    if !item.isRead {
                                        Button(L10n.notificationsMarkRead) {
                                            Task { await markRead(item) }
                                        }
                                        .tint(DesignTokens.primaryColor)
                                    }
                                }
                                .contextMenu {
                                    if !item.isRead {
                                        Button(L10n.notificationsMarkRead) {
                                            Task { await markRead(item) }
                                        }
                                    }
                                }
                            }
                        }
                    }
                    if hasMoreHistory {
                        Button {
                            Task { await loadMore() }
                        } label: {
                            HStack {
                                Spacer()
                                if isLoadingMore {
                                    ProgressView()
                                } else {
                                    Text(L10n.notificationsLoadMore)
                                }
                                Spacer()
                            }
                        }
                        .disabled(isLoadingMore)
                    }
                }
                .listStyle(.plain)
                .scrollContentBackground(.hidden)
                .background(Color(.systemGroupedBackground))
                .refreshable {
                    await loadHistory(showLoading: false)
                }
            }
        }
        .navigationTitle(L10n.notificationsTitle)
        .toolbar {
            if ownsNavigationStack {
                ToolbarItem(placement: .cancellationAction) {
                    Button(L10n.close) { dismiss() }
                }
            }
            ToolbarItem(placement: .navigationBarLeading) {
                if !items.isEmpty || filter != .all || !query.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
                    Menu {
                        Picker(L10n.notificationsFilterTitle, selection: $filter) {
                            Text(L10n.notificationsFilterAll).tag(FilterMode.all)
                            Text(L10n.notificationsFilterUnread).tag(FilterMode.unread)
                        }
                    } label: {
                        Label(filter.title, systemImage: "line.3.horizontal.decrease.circle")
                    }
                }
            }
            ToolbarItem(placement: .navigationBarTrailing) {
                if isMarkingAllRead || items.contains(where: { !$0.isRead }) {
                    Button {
                        Task {
                            isMarkingAllRead = true
                            await markAllRead()
                            isMarkingAllRead = false
                        }
                    } label: {
                        if isMarkingAllRead {
                            ProgressView()
                        } else {
                            Label(L10n.notificationsMarkAllRead, systemImage: "checkmark.circle")
                        }
                    }
                    .disabled(isMarkingAllRead)
                }
            }
        }
        .searchable(
            text: $query,
            placement: .navigationBarDrawer(displayMode: items.isEmpty ? .automatic : .always),
            prompt: Text(L10n.notificationsSearchPlaceholder)
        )
        .task(id: deepLinkRequestID?.uuidString ?? initialDeepLinkKey ?? "") {
            let request = deepLinkRequestID?.uuidString ?? initialDeepLinkKey
            await loadHistory(showLoading: items.isEmpty)
            guard !Task.isCancelled, let request, request != handledDeepLinkRequest,
                  let key = initialDeepLinkKey else { return }
            handledDeepLinkRequest = request
            let route = NotificationCenterRoute(lookupKey: key)
            if ownsNavigationStack { navPath.append(route) }
            else { embeddedDeepLink = route }
        }
        .onChange(of: filter) { _, _ in Task { await loadHistory(showLoading: false) } }
        .onChange(of: query) { _, _ in
            queryRefreshTask?.cancel()
            queryRefreshTask = Task { @MainActor in
                try? await Task.sleep(nanoseconds: 350_000_000)
                guard !Task.isCancelled else { return }
                await loadHistory(showLoading: false)
            }
        }
        .onReceive(NotificationCenter.default.publisher(for: .nolaNotificationsUpdated)) { _ in
            if suppressNextNotificationRefresh {
                suppressNextNotificationRefresh = false
                return
            }
            Task { await loadHistory(showLoading: false) }
        }
        .onDisappear {
            queryRefreshTask?.cancel()
            queryRefreshTask = nil
        }
    }

    @ViewBuilder
    private var emptyState: some View {
        let normalizedQuery = query.trimmingCharacters(in: .whitespacesAndNewlines)
        VStack {
            if !normalizedQuery.isEmpty {
                NolaInlineEmptyState(
                    icon: "magnifyingglass",
                    title: NSLocalizedString("notifications.empty.search.title", value: "No matching notifications", comment: "No matching notifications title"),
                    message: NSLocalizedString("notifications.empty.search.message", value: "Try another search or clear the current query.", comment: "No matching notifications message"),
                    tint: .blue,
                    actionTitle: NSLocalizedString("notifications.empty.search.clear", value: "Clear search", comment: "Clear notification search"),
                    action: { query = "" }
                )
            } else {
                NolaInlineEmptyState(
                    icon: filter == .unread ? "checkmark.circle" : "bell.slash",
                    title: filter == .unread
                        ? NSLocalizedString("notifications.empty.unread.title", value: "No unread notifications", comment: "No unread notifications")
                        : L10n.notificationsNoPending,
                    message: filter == .unread
                        ? NSLocalizedString("notifications.empty.unread.message", value: "You're all caught up.", comment: "All notifications handled")
                        : L10n.notificationsAllSentOrCancelled,
                    tint: filter == .unread ? .green : .secondary
                )
            }
            Spacer()
        }
        .padding(.horizontal, DesignTokens.spacing_lg)
        .padding(.top, DesignTokens.spacing_lg)
        .frame(maxWidth: .infinity, alignment: .leading)
    }

    @ViewBuilder
    private func notificationLink(for item: NotificationEntity) -> some View {
        let route = NotificationCenterRoute(entity: item)
        if ownsNavigationStack {
            NavigationLink(value: route) {
                NotificationCenterRow(entity: item)
                    .id(item.id)
            }
        } else {
            NavigationLink {
                detailDestination(for: route)
            } label: {
                NotificationCenterRow(entity: item)
                    .id(item.id)
            }
        }
    }

    @ViewBuilder
    private func detailDestination(for route: NotificationCenterRoute) -> some View {
        let initialEntity = initialEntity(for: route)
        NotificationDetailView(
            dedupeKey: route.lookupKey,
            notificationId: route.entityId,
            initialEntity: initialEntity
        )
        .environmentObject(container)
    }

    @MainActor
    private func loadHistory(showLoading: Bool = true) async {
        if showLoading {
            isLoading = true
        }
        do {
            loadError = nil
            offset = 0
            let fetched = try container.notificationManager?.fetchHistory(limit: pageSize, offset: offset, unreadOnly: filter == .unread, query: query) ?? []
            self.items = fetched
            self.offset = fetched.count
            self.hasMoreHistory = fetched.count == pageSize
        } catch {
            loadError = error.localizedDescription
            container.logger.log(.error, "NotificationCenter", "Failed to load notification history", metadata: ["error": error.localizedDescription])
        }
        if showLoading {
            isLoading = false
        }
    }

    @MainActor
    private func markRead(_ item: NotificationEntity, refreshList: Bool = true) async {
        suppressNextNotificationRefresh = true
        operationError = nil
        do {
            try container.notificationManager?.markAsRead(id: item.id)
        } catch {
            operationError = error.localizedDescription
            container.logger.log(.error, "NotificationCenter", "Failed to mark local notification read", metadata: ["error": error.localizedDescription])
        }
        if refreshList {
            await loadHistory(showLoading: false)
        }
    }

    @MainActor
    private func markAllRead() async {
        suppressNextNotificationRefresh = true
        operationError = nil
        do {
            try container.notificationManager?.markAllRead()
        } catch {
            operationError = error.localizedDescription
            container.logger.log(.error, "NotificationCenter", "Failed to mark local notifications read", metadata: ["error": error.localizedDescription])
        }
        await loadHistory(showLoading: false)
    }

    @MainActor
    private func loadMore() async {
        guard !isLoadingMore, hasMoreHistory else { return }
        isLoadingMore = true
        do {
            let fetched = try container.notificationManager?.fetchHistory(limit: pageSize, offset: offset, unreadOnly: filter == .unread, query: query) ?? []
            self.items.append(contentsOf: fetched)
            self.offset += fetched.count
            self.hasMoreHistory = fetched.count == pageSize
        } catch {
            loadError = error.localizedDescription
            container.logger.log(.error, "NotificationCenter", "Failed to load more notifications", metadata: ["error": error.localizedDescription])
        }
        isLoadingMore = false
    }

    private func initialEntity(for route: NotificationCenterRoute) -> NotificationEntity? {
        if let entityId = route.entityId, let exact = items.first(where: { $0.id == entityId }) {
            return exact
        }
        return items.first { $0.id == route.lookupKey || $0.dedupeKey == route.lookupKey }
    }

    private func groupedByDay(_ list: [NotificationEntity]) -> [(String, [NotificationEntity])] {
        var map: [String: [NotificationEntity]] = [:]
        let df = DateFormatter()
        df.dateStyle = .medium
        for it in list {
            let key = df.string(from: it.createdAt)
            var arr = map[key] ?? []
            arr.append(it)
            map[key] = arr
        }
        // Sort keys by most recent day first
        let sorted = map.sorted { a, b in
            // parse back to date for ordering (fallback to string order)
            let dfa = df.date(from: a.key)
            let dfb = df.date(from: b.key)
            if let da = dfa, let db = dfb { return da > db }
            return a.key > b.key
        }
        return sorted.map { ($0.key, $0.value) }
    }

    struct NotificationCenterRow: View {
        let entity: NotificationEntity

        var body: some View {
            HStack(alignment: .top, spacing: 12) {
                ZStack {
                    RoundedRectangle(cornerRadius: 12, style: .continuous)
                        .fill(sourceColor.opacity(0.12))
                        .frame(width: 40, height: 40)
                    Image(systemName: sourceIcon)
                        .font(.system(size: 16, weight: .semibold))
                        .foregroundStyle(sourceColor)
                }

                VStack(alignment: .leading, spacing: 7) {
                    HStack(alignment: .firstTextBaseline, spacing: 8) {
                        Text(entity.title ?? L10n.notificationsTitle)
                            .font(.body.weight(entity.isRead ? .medium : .semibold))
                            .foregroundStyle(.primary)
                            .lineLimit(2)
                        Spacer(minLength: 6)
                        if !entity.isRead {
                            Circle()
                                .fill(DesignTokens.primaryColor)
                                .frame(width: 7, height: 7)
                                .accessibilityLabel(L10n.notificationsFilterUnread)
                        }
                    }

                    if let body = entity.body, !body.isEmpty {
                        Text(body)
                            .font(.subheadline)
                            .foregroundStyle(.secondary)
                            .lineLimit(2)
                    }

                    HStack(spacing: 8) {
                        Text(sourceLabel)
                            .font(.caption.weight(.semibold))
                            .foregroundStyle(sourceColor)
                        Spacer(minLength: 8)
                        Text(RelativeDateTimeFormatter().localizedString(for: entity.createdAt, relativeTo: Date()))
                            .font(.caption)
                            .foregroundStyle(.secondary)
                    }
                }
            }
            .padding(14)
            .background(
                RoundedRectangle(cornerRadius: DesignTokens.radius_lg, style: .continuous)
                    .fill(
                        entity.isRead
                            ? DesignTokens.surfaceColor
                            : DesignTokens.primaryColor.opacity(0.075)
                    )
            )
            .overlay(
                RoundedRectangle(cornerRadius: DesignTokens.radius_lg, style: .continuous)
                    .stroke(
                        entity.isRead
                            ? DesignTokens.borderColor
                            : DesignTokens.primaryColor.opacity(0.16),
                        lineWidth: 1
                    )
            )
            .contentShape(RoundedRectangle(cornerRadius: DesignTokens.radius_lg, style: .continuous))
            .accessibilityElement(children: .combine)
            .accessibilityValue(entity.isRead ? L10n.notificationsRead : L10n.notificationsFilterUnread)
        }

        private var sourceIcon: String {
            switch entity.source?.lowercased() {
            case "calendar": return "calendar"
            case "reminder": return "checkmark.circle"
            case "url", "web": return "globe"
            case "health": return "heart.fill"
            case "contact": return "person.crop.circle"
            default: return "bell.fill"
            }
        }

        private var sourceColor: Color {
            switch entity.source?.lowercased() {
            case "calendar": return .indigo
            case "reminder": return DesignTokens.primaryColor
            case "url", "web": return .cyan
            case "health": return .pink
            case "contact": return .teal
            default: return .secondary
            }
        }

        private var sourceLabel: String {
            switch entity.source?.lowercased() {
            case "calendar":
                return NSLocalizedString("notifications.source.calendar", value: "Calendar", comment: "Notification source calendar")
            case "reminder":
                return NSLocalizedString("notifications.source.reminder", value: "Reminder", comment: "Notification source reminder")
            case "url", "web":
                return NSLocalizedString("notifications.source.web", value: "Web", comment: "Notification source web")
            case "health":
                return NSLocalizedString("notifications.source.health", value: "Health", comment: "Notification source health")
            case "contact":
                return NSLocalizedString("notifications.source.contact", value: "Contacts", comment: "Notification source contacts")
            default:
                return entity.source?.takeIfNotEmpty
                    ?? NSLocalizedString("notifications.source.system", value: "System", comment: "Notification source system")
            }
        }
    }
}

private extension String {
    var takeIfNotEmpty: String? { isEmpty ? nil : self }
}

private extension View {
    func notificationRouteDestination<Destination: View>(
        @ViewBuilder destination: @escaping (NotificationCenterRoute) -> Destination
    ) -> some View {
        navigationDestination(for: NotificationCenterRoute.self, destination: destination)
    }
}

struct NotificationCenterView_Previews: PreviewProvider {
    static var previews: some View {
        NotificationCenterView()
            .environmentObject(ServiceContainer())
    }
}
