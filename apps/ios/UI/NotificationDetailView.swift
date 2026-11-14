// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Implements NotificationDetailView logic in the iOS UI layer.
// - Interfaces:
//   - NotificationDetailView
//   - SummaryDetailView
//   - JSONValue
//   - TimelineRow
//   - InfoCard
// - APIs:
//   - none
//
import SwiftUI
@preconcurrency import UserNotifications

struct NotificationDetailView: View {
    @EnvironmentObject var container: ServiceContainer
    @Environment(\.dismiss) private var dismiss
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    let dedupeKey: String
    let notificationId: String?
    let initialEntity: NotificationEntity?

    @State private var event: ProcessedEvent? = nil
    @State private var isLoading = true
    @State private var notificationScheduledAt: Date? = nil
    @State private var hasPendingNotification: Bool = false
    @State private var lastRemovedRequest: UNNotificationRequest? = nil
    @State private var lastMarkedEntityIDs: [String] = []
    @State private var showUndoBanner: Bool = false
    @State private var showConfirmMark: Bool = false
    @State private var canonicalCreatedAt: Date? = nil
    @State private var matchedEntity: NotificationEntity? = nil
    @State private var matchedRun: LocalJSONStorage.ScheduledActionRunRecord? = nil
    @State private var isProcessing = false

    init(dedupeKey: String, notificationId: String? = nil, initialEntity: NotificationEntity? = nil) {
        self.dedupeKey = dedupeKey
        self.notificationId = notificationId
        self.initialEntity = initialEntity
        let initialEvent = initialEntity.map { Self.processedEvent(from: $0, dedupeKey: dedupeKey) }
        _event = State(initialValue: initialEvent)
        _isLoading = State(initialValue: initialEvent == nil)
        _matchedEntity = State(initialValue: initialEntity)
        _canonicalCreatedAt = State(initialValue: initialEntity?.createdAt)
    }

    private func extractDoubleCurly(_ s: String) -> String? {
        let text = s.trimmingCharacters(in: .whitespacesAndNewlines)
        guard let start = text.range(of: "{{")?.upperBound,
              let end = text.range(of: "}}", range: start..<text.endIndex)?.lowerBound,
              start <= end
        else { return nil }
        let inner = text[start..<end].trimmingCharacters(in: .whitespacesAndNewlines)
        return inner.isEmpty ? nil : String(inner)
    }

    private static func processedEvent(from entity: NotificationEntity, dedupeKey: String) -> ProcessedEvent {
        let source = EventSource(rawValue: entity.source ?? "") ?? .reminder
        let sourceType: ProcessedEvent.SourceType = {
            switch source {
            case .calendar: return .calendar
            case .reminder: return .reminder
            case .url: return .url
            case .health: return .health
            case .contact: return .manual
            }
        }()
        let title = entity.title ?? entity.body ?? L10n.notificationDetailTitle
        var metadata: [String: String] = [
            "createdAt": ISO8601DateFormatter().string(from: entity.createdAt)
        ]
        if let recordId = entity.recordId {
            metadata["recordId"] = recordId
        }
        return ProcessedEvent(
            id: entity.id,
            source: source,
            sourceType: sourceType,
            sourceId: entity.recordId ?? "notification",
            title: title,
            date: entity.createdAt,
            start: nil,
            due: nil,
            priority: 0.5,
            tags: ["notification"],
            dedupeKey: entity.dedupeKey ?? dedupeKey,
            provenance: nil,
            metadata: metadata,
            notes: entity.body,
            hash: title.hashValue
        )
    }

    private var lookupKeys: [String] {
        var keys: [String] = []
        for candidate in [notificationId, dedupeKey, initialEntity?.id, initialEntity?.dedupeKey, initialEntity?.requestId] {
            guard let trimmed = candidate?.trimmingCharacters(in: .whitespacesAndNewlines), !trimmed.isEmpty else { continue }
            if !keys.contains(trimmed) {
                keys.append(trimmed)
            }
        }
        return keys
    }

    var body: some View {
        Group {
            if isLoading {
                VStack(spacing: 12) {
                    ProgressView()
                        .progressViewStyle(CircularProgressViewStyle(tint: Color.accentColor))
                        .scaleEffect(1.1)
                    Text(L10n.notificationLoading)
                        .font(.subheadline)
                        .foregroundColor(.secondary)
                }
                .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .center)
                .background(Color(.systemBackground))
            } else if let ev = event {
                ScrollView {
                    VStack(alignment: .leading, spacing: 20) {
                        VStack(alignment: .leading, spacing: 16) {
                            HStack(alignment: .top, spacing: 12) {
                                ZStack {
                                    RoundedRectangle(cornerRadius: 13, style: .continuous)
                                        .fill(sourceColor.opacity(0.13))
                                        .frame(width: 44, height: 44)
                                    Image(systemName: sourceIcon)
                                        .font(.system(size: 18, weight: .semibold))
                                        .foregroundStyle(sourceColor)
                                }
                                
                                VStack(alignment: .leading, spacing: 4) {
                                    Text(sourceLabel)
                                        .font(.subheadline.weight(.semibold))
                                        .foregroundStyle(sourceColor)
                                    
                                    if let date = ev.date {
                                        HStack(spacing: 4) {
                                            Image(systemName: "clock")
                                                .font(.caption2)
                                            Text(date.formatted(date: .abbreviated, time: .shortened))
                                                .font(.caption)
                                        }
                                        .foregroundStyle(.secondary)
                                    }
                                }
                                
                                Spacer()
                            }
                            
                            Text(ev.title)
                                .font(.title2.weight(.bold))
                                .foregroundStyle(.primary)
                                .fixedSize(horizontal: false, vertical: true)
                        }
                        .padding(18)
                        .background(
                            RoundedRectangle(cornerRadius: DesignTokens.radius_lg, style: .continuous)
                                .fill(DesignTokens.surfaceColor)
                        )
                        .overlay(
                            RoundedRectangle(cornerRadius: DesignTokens.radius_lg, style: .continuous)
                                .stroke(DesignTokens.borderColor, lineWidth: 1)
                        )
                        .accessibilityElement(children: .combine)

                        // Timeline information card.
                        VStack(alignment: .leading, spacing: 16) {
                            // Task type badge.
                            if let taskTypeRaw = ev.metadata["taskType"] ?? ev.provenance?["taskType"]?.stringValue {
                                HStack(spacing: 8) {
                                    Image(systemName: taskTypeIcon(taskTypeRaw))
                                        .font(.caption)
                                        .foregroundColor(sourceColor)
                                    Text(friendlyTaskType(taskTypeRaw))
                                        .font(.subheadline)
                                        .fontWeight(.semibold)
                                        .foregroundColor(.primary)
                                }
                                .padding(.horizontal, 14)
                                .padding(.vertical, 8)
                                .background(
                                    Capsule()
                                        .fill(sourceColor.opacity(0.12))
                                )
                            }
                            
                            // Timeline rows.
                            VStack(spacing: 12) {
                                if let dt = canonicalCreatedAt ?? (ev.metadata["createdAt"].flatMap(iso8601Date)) {
                                    TimelineRow(
                                        icon: "clock.arrow.circlepath",
                                        title: L10n.timelineAddedTime,
                                        time: dt,
                                        color: .blue
                                    )
                                }
                                
                                if let sched = notificationScheduledAt {
                                    TimelineRow(
                                        icon: "bell.badge",
                                        title: L10n.timelineScheduledNotification,
                                        time: sched,
                                        color: .orange
                                    )
                                }
                            }
                        }
                        .padding(16)
                        .background(
                            RoundedRectangle(cornerRadius: 16)
                                .fill(Color(.secondarySystemBackground))
                        )
                        .overlay(
                            RoundedRectangle(cornerRadius: 16)
                                .stroke(Color.primary.opacity(0.05), lineWidth: 1)
                        )

                        if hasPendingNotification {
                            VStack(alignment: .leading, spacing: 14) {
                                HStack(spacing: 10) {
                                    Image(systemName: "bell.badge.fill")
                                        .font(.system(size: 16, weight: .semibold))
                                        .foregroundStyle(.orange)
                                    Text(L10n.notificationPendingTitle)
                                        .font(.headline)
                                    Spacer()
                                }

                                Text(L10n.notificationScheduledNotice)
                                    .font(.subheadline)
                                    .foregroundStyle(.secondary)
                                    .fixedSize(horizontal: false, vertical: true)

                                Button(action: {
                                    guard !isProcessing else { return }
                                    isProcessing = true
                                    Task {
                                        await performMarkAsProcessedWithUndoSupport()
                                        await MainActor.run { isProcessing = false }
                                    }
                                }) {
                                    HStack(spacing: 8) {
                                        if isProcessing {
                                            ProgressView()
                                                .tint(.white)
                                        } else {
                                            Image(systemName: "checkmark")
                                                .font(.system(size: 14, weight: .bold))
                                        }
                                        Text(L10n.notificationMarkHandled)
                                            .fontWeight(.semibold)
                                    }
                                    .frame(maxWidth: .infinity)
                                }
                                .buttonStyle(.borderedProminent)
                                .tint(.green)
                                .controlSize(.large)
                                .disabled(isProcessing)
                            }
                            .padding(16)
                            .background(
                                RoundedRectangle(cornerRadius: DesignTokens.radius_md, style: .continuous)
                                    .fill(Color.orange.opacity(0.08))
                            )
                            .overlay(
                                RoundedRectangle(cornerRadius: DesignTokens.radius_md, style: .continuous)
                                    .stroke(Color.orange.opacity(0.18), lineWidth: 1)
                            )
                        } else {
                            HStack(spacing: 12) {
                                Image(systemName: "checkmark.circle.fill")
                                    .font(.title3)
                                    .foregroundStyle(.green)
                                Text(L10n.notificationCompleteItem)
                                    .font(.subheadline.weight(.semibold))
                                Spacer()
                            }
                            .padding(16)
                            .background(
                                RoundedRectangle(cornerRadius: DesignTokens.radius_md, style: .continuous)
                                    .fill(Color.green.opacity(0.08))
                            )
                            .accessibilityElement(children: .combine)
                        }

                        // Description and notes section.
                        if let prov = ev.provenance, let explain = prov["explain"]?.stringValue {
                            InfoCard(
                                icon: "lightbulb.fill",
                                iconColor: .yellow,
                                title: L10n.notificationAIExplainTitle,
                                content: extractDoubleCurly(explain) ?? explain
                            )
                        }

                        if let notes = ev.notes, !notes.isEmpty {
                            InfoCard(
                                icon: "note.text",
                                iconColor: .purple,
                                title: L10n.notificationNotesTitle,
                                content: notes
                            )
                        }

                        // Link button.
                        if let url = ev.metadata["url"], let u = URL(string: url) {
                            Button(action: { UIApplication.shared.open(u) }) {
                                HStack(spacing: 12) {
                                    ZStack {
                                        RoundedRectangle(cornerRadius: 11, style: .continuous)
                                            .fill(Color.blue.opacity(0.12))
                                            .frame(width: 40, height: 40)
                                        Image(systemName: "link")
                                            .font(.system(size: 16, weight: .semibold))
                                            .foregroundStyle(.blue)
                                    }
                                    
                                    VStack(alignment: .leading, spacing: 2) {
                                        Text(L10n.notificationOpenRaw)
                                            .font(.subheadline)
                                            .fontWeight(.semibold)
                                            .foregroundColor(.primary)
                                        Text(L10n.notificationViewInBrowser)
                                            .font(.caption)
                                            .foregroundColor(.secondary)
                                    }
                                    
                                    Spacer()
                                    
                                    Image(systemName: "arrow.up.right")
                                        .font(.subheadline.weight(.semibold))
                                        .foregroundStyle(.secondary)
                                }
                                .padding(16)
                                .background(
                                    RoundedRectangle(cornerRadius: 16)
                                        .fill(Color(.secondarySystemBackground))
                                        .overlay(
                                            RoundedRectangle(cornerRadius: 16)
                                                .stroke(DesignTokens.borderColor, lineWidth: 1)
                                        )
                                )
                            }
                            .buttonStyle(PlainButtonStyle())
                        }

                        // Expandable details section.
                        if !ev.metadata.isEmpty {
                            DisclosureGroup {
                                VStack(spacing: 10) {
                                    ForEach(ev.metadata.sorted(by: { $0.key < $1.key }), id: \.key) { pair in
                                        HStack(alignment: .top, spacing: 12) {
                                            Text(pair.key)
                                                .font(.caption)
                                                .fontWeight(.semibold)
                                                .foregroundColor(.secondary)
                                                .frame(width: 100, alignment: .leading)
                                            Text(pair.value)
                                                .font(.caption)
                                                .foregroundColor(.primary)
                                                .multilineTextAlignment(.leading)
                                            Spacer()
                                        }
                                        .padding(.vertical, 6)
                                        
                                        if pair.key != ev.metadata.sorted(by: { $0.key < $1.key }).last?.key {
                                            Divider()
                                        }
                                    }
                                }
                                .padding(.top, 8)
                            } label: {
                                HStack(spacing: 8) {
                                    Image(systemName: "info.circle.fill")
                                        .foregroundColor(.blue)
                                    Text(L10n.notificationDetails)
                                        .font(.subheadline)
                                        .fontWeight(.semibold)
                                }
                            }
                            .disclosureGroupStyle(NolaRowDisclosureStyle())
                            .padding(16)
                            .background(
                                RoundedRectangle(cornerRadius: 16)
                                    .fill(Color(.secondarySystemBackground))
                            )
                        }

                        notificationRecordCard
                        linkedRunCard
                    }
                    .padding(16)
                    .frame(maxWidth: 720)
                    .frame(maxWidth: .infinity)
                }
                .background(Color(.systemGroupedBackground))
            } else {
                VStack(alignment: .leading, spacing: DesignTokens.spacing_md) {
                    NolaInlineEmptyState(
                        icon: "doc.text.fill",
                        title: L10n.notificationContentUnavailable,
                        message: L10n.notificationContentRemoved,
                        tint: .secondary
                    )

                    if matchedEntity != nil || matchedRun != nil {
                        VStack(alignment: .leading, spacing: 12) {
                            notificationRecordCard
                            linkedRunCard
                        }
                        .padding(.top, 8)
                    }
                }
                .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
                .padding()
            }
        }
        .navigationTitle(L10n.notificationDetailTitle)
        #if os(iOS)
        .navigationBarTitleDisplayMode(.inline)
        #endif
        .onAppear {
            markViewedNotificationRead()
        }
        .overlay(alignment: .bottom) {
            if showUndoBanner {
                VStack(spacing: 0) {
                    Spacer()
                    HStack(spacing: 12) {
                        Image(systemName: "checkmark.circle.fill")
                            .font(.system(size: 24))
                            .foregroundColor(.white)
                        
                        VStack(alignment: .leading, spacing: 2) {
                            Text(L10n.notificationMarkedHandled)
                                .font(.subheadline)
                                .fontWeight(.semibold)
                                .foregroundColor(.white)
                            Text(L10n.notificationCancelled)
                                .font(.caption)
                                .foregroundColor(.white.opacity(0.9))
                        }
                        
                        Spacer()
                        
                        Button(action: {
                            undoMarkAsProcessed()
                        }) {
                            HStack(spacing: 6) {
                                Image(systemName: "arrow.uturn.backward.circle.fill")
                                    .font(.system(size: 14))
                                Text(L10n.notificationUndo)
                                    .fontWeight(.semibold)
                            }
                            .padding(.horizontal, 16)
                            .padding(.vertical, 8)
                            .background(Color.white.opacity(0.25))
                            .foregroundColor(.white)
                            .cornerRadius(DesignTokens.radius_pill)
                        }
                    }
                    .padding(16)
                    .background(
                        RoundedRectangle(cornerRadius: DesignTokens.radius_lg)
                            .fill(
                                LinearGradient(
                                    gradient: Gradient(colors: [Color.green, Color.green.opacity(0.85)]),
                                    startPoint: .topLeading,
                                    endPoint: .bottomTrailing
                                )
                            )
                            .shadow(color: Color.black.opacity(0.2), radius: 12, x: 0, y: 4)
                    )
                    .padding(.horizontal, 16)
                    .padding(.bottom, 20)
                    .transition(.move(edge: .bottom).combined(with: .opacity))
                }
                .animation(reduceMotion ? nil : .spring(response: 0.4, dampingFraction: 0.8), value: showUndoBanner)
            }
        }
        .task {
            await loadEvent()
            await loadDiagnosticContext()
        }
    }

    @ViewBuilder
    private var notificationRecordCard: some View {
        if let entity = matchedEntity {
            VStack(alignment: .leading, spacing: 10) {
                Text(NSLocalizedString("notification.detail.record.title", value: "通知记录", comment: "Notification record title"))
                    .font(.subheadline)
                    .fontWeight(.semibold)

                detailRow(NSLocalizedString("notification.detail.field.status", value: "状态", comment: "Notification detail status"), entity.status.rawValue)
                detailRow(NSLocalizedString("notification.detail.field.read", value: "已读", comment: "Notification detail read"), entity.isRead ? NSLocalizedString("common.yes.zh", value: "是", comment: "Yes in Chinese") : NSLocalizedString("common.no.zh", value: "否", comment: "No in Chinese"))
                detailRow(NSLocalizedString("notification.detail.field.source", value: "来源", comment: "Notification detail source"), entity.source ?? "-")
                detailRow(NSLocalizedString("notification.detail.field.created_at", value: "创建时间", comment: "Notification detail created at"), Self.formatMetaDate(entity.createdAt))

                if let sentAt = entity.sentAt {
                    detailRow(NSLocalizedString("notification.detail.field.sent_at", value: "发送时间", comment: "Notification detail sent at"), Self.formatMetaDate(sentAt))
                }
                if let deliveredAt = entity.deliveredAt {
                    detailRow(NSLocalizedString("notification.detail.field.delivered_at", value: "送达时间", comment: "Notification detail delivered at"), Self.formatMetaDate(deliveredAt))
                }
                if entity.attempts > 0 {
                    detailRow(NSLocalizedString("notification.detail.field.retry_count", value: "重试次数", comment: "Notification detail retry count"), "\(entity.attempts)")
                }
                if let userAction = entity.userAction, !userAction.isEmpty {
                    detailRow(NSLocalizedString("notification.detail.field.user_action", value: "用户操作", comment: "Notification detail user action"), userAction)
                }
                if let dedupe = entity.dedupeKey, !dedupe.isEmpty {
                    detailRow(NSLocalizedString("notification.detail.field.dedupe_key", value: "去重键", comment: "Notification detail dedupe key"), dedupe)
                }
                if !entity.requestId.isEmpty {
                    detailRow(NSLocalizedString("notification.detail.field.request_id", value: "请求ID", comment: "Notification detail request id"), entity.requestId)
                }
            }
            .padding(14)
            .background(RoundedRectangle(cornerRadius: 12).fill(Color(.secondarySystemBackground)))
        }
    }

    @ViewBuilder
    private var linkedRunCard: some View {
        if let run = matchedRun {
            VStack(alignment: .leading, spacing: 10) {
                Text(NSLocalizedString("notification.detail.run.title", value: "关联执行记录", comment: "Linked run title"))
                    .font(.subheadline)
                    .fontWeight(.semibold)

                detailRow(NSLocalizedString("notification.detail.run.field.result", value: "结果", comment: "Run result field"), run.status.rawValue)
                if let actionName = run.actionName, !actionName.isEmpty {
                    detailRow(NSLocalizedString("notification.detail.run.field.action", value: "动作", comment: "Run action field"), actionName)
                }
                if let reason = run.reason, !reason.isEmpty {
                    detailRow(NSLocalizedString("notification.detail.run.field.reason", value: "原因码", comment: "Run reason field"), reason)
                }
                if let failureReason = run.failureReason, !failureReason.isEmpty {
                    detailRow(NSLocalizedString("notification.detail.run.field.failure_reason", value: "失败原因", comment: "Run failure reason field"), failureReason)
                }
                if let err = run.errorMessage, !err.isEmpty {
                    detailRow(NSLocalizedString("notification.detail.run.field.error", value: "错误", comment: "Run error field"), err)
                }
                if let details = run.details, !details.isEmpty {
                    detailRow(NSLocalizedString("notification.detail.run.field.details", value: "执行细节", comment: "Run details field"), details)
                }
                if let count = run.eventsCount {
                    detailRow(NSLocalizedString("notification.detail.run.field.events_count", value: "事件数", comment: "Run events count field"), "\(count)")
                }
                detailRow(NSLocalizedString("notification.detail.run.field.started_at", value: "开始", comment: "Run started at field"), Self.formatMetaDate(run.startedAt))
                detailRow(NSLocalizedString("notification.detail.run.field.finished_at", value: "结束", comment: "Run finished at field"), Self.formatMetaDate(run.finishedAt))
            }
            .padding(14)
            .background(RoundedRectangle(cornerRadius: 12).fill(Color(.secondarySystemBackground)))
        }
    }

    private func detailRow(_ key: String, _ value: String) -> some View {
        HStack(alignment: .top, spacing: 10) {
            Text(key)
                .font(.caption)
                .foregroundColor(.secondary)
                .frame(width: 72, alignment: .leading)
            Text(value)
                .font(.caption)
                .foregroundColor(.primary)
                .textSelection(.enabled)
            Spacer(minLength: 0)
        }
    }

    private func loadEvent() async {
        let keys = lookupKeys
        // First: try an in-memory quick match using recentProcessedEvents via Rust
        if !container.recentProcessedEvents.isEmpty {
            if let found = keys.lazy.compactMap({ RustInterop.findProcessedEventInList(processed: container.recentProcessedEvents, key: $0) }).first {
                await MainActor.run {
                    event = found
                    isLoading = false
                }
                await loadCanonicalCreatedAt()
                // Also lookup any pending UNNotificationRequest so we can show scheduled time / actions
                await lookupPendingNotification()
                return
            }
        }

        // Try to find the ProcessedEvent via unifiedStorage
        let store: FileBasedUnifiedStorage? = await MainActor.run {
            container.unifiedStorage as? FileBasedUnifiedStorage
        }
        if let store {
            let all = (try? await store.fetchProcessedEventsAsync(limit: nil)) ?? []

            // Try multiple matching strategies
            let found: ProcessedEvent? = {
                if let direct = all.first(where: { ev in keys.contains(where: { ev.dedupeKey == $0 || ev.id == $0 }) }) { return direct }
                if let partial = all.first(where: { ev in keys.contains(where: { ev.dedupeKey.contains($0) || $0.contains(ev.dedupeKey) }) }) { return partial }
                if let sourceId = all.first(where: { ev in keys.contains(ev.sourceId) }) { return sourceId }
                return nil
            }()

            if let found {
                await MainActor.run {
                    event = found
                    isLoading = false
                }
                await loadCanonicalCreatedAt()
                await lookupPendingNotification()
                return
            }
        }

        // If unified storage didn't find, inspect canonical URLDataSource history.
        let sourcesSnapshot: [URLDataSource]? = await MainActor.run {
            container.urlFetcher?.sources
        }
        if let sourcesSnapshot {
            let store: FileBasedUnifiedStorage? = await MainActor.run {
                container.unifiedStorage as? FileBasedUnifiedStorage
            }
            var foundViaHistory: ProcessedEvent? = nil
            for source in sourcesSnapshot {
                let entries = URLHistoryStore.shared.load(sourceID: source.id.uuidString)
                if let match = entries.first(where: { entry in
                    keys.contains(where: { entry.dedupeKey == $0 || entry.eventId == $0 })
                }) {
                    if let eid = match.eventId, let store {
                        foundViaHistory = try? await store.findProcessedEventAsync(byId: eid)
                        if foundViaHistory != nil { break }
                    }
                }
            }

            if let ev = foundViaHistory {
                await MainActor.run {
                    event = ev
                    isLoading = false
                }
                return
            }
        }

        // Fallback: build a minimal ProcessedEvent from NotificationStore (APNs alerts, debug pushes)
        if let store = container.notificationStore {
            do {
                var entity: NotificationEntity? = nil
                if let notificationId, let byId = try? store.findById(notificationId) {
                    entity = byId
                } else if let byId = try? store.findById(dedupeKey) {
                    entity = byId
                } else if let matches = try? store.findByDedupeKey(dedupeKey), let latest = matches.sorted(by: { $0.createdAt > $1.createdAt }).first {
                    entity = latest
                }

                if let e = entity {
                    let source = EventSource(rawValue: e.source ?? "") ?? .reminder
                    let sourceType: ProcessedEvent.SourceType = {
                        switch source {
                        case .calendar: return .calendar
                        case .reminder: return .reminder
                        case .url: return .url
                        case .health: return .health
                        case .contact: return .manual
                        }
                    }()

                    var metadata: [String: String] = [:]
                    metadata["createdAt"] = ISO8601DateFormatter().string(from: e.createdAt)
                    if let rid = e.recordId { metadata["recordId"] = rid }

                    var notes: String? = e.body
                    if let rid = e.recordId, let storage = container.localJSONStorage {
                        if let journal = try? storage.getJournal(id: rid) {
                            notes = journal.content
                            metadata["journalTitle"] = journal.title
                        }
                    }

                    let title = e.title ?? e.body ?? L10n.notificationDetailTitle
                    let ev = ProcessedEvent(
                        id: e.id,
                        source: source,
                        sourceType: sourceType,
                        sourceId: e.recordId ?? "notification",
                        title: title,
                        date: e.createdAt,
                        start: nil,
                        due: nil,
                        priority: 0.5,
                        tags: ["apns"],
                        dedupeKey: e.dedupeKey ?? dedupeKey,
                        provenance: nil,
                        metadata: metadata,
                        notes: notes,
                        hash: title.hashValue
                    )

                    await MainActor.run {
                        event = ev
                        isLoading = false
                    }
                    await loadCanonicalCreatedAt()
                    await lookupPendingNotification()
                    return
                }
            }
        }

        // If nothing richer was found, keep any initial list snapshot so the
        // detail page does not blink from content back to an empty state.
        await MainActor.run {
            isLoading = false
        }
        // Also attempt to look up pending notification request for dedupeKey to show scheduled time
        await lookupPendingNotification()
    }

    private func markViewedNotificationRead() {
        do {
            _ = try container.notificationManager?.markMatchingAsRead(key: notificationId ?? dedupeKey)
        } catch {
            container.logger.log(.error, "NotificationDetailView", "Failed to mark viewed notification as read", metadata: ["error": error.localizedDescription])
        }

    }

    private func loadDiagnosticContext() async {
        var entity: NotificationEntity? = nil
        if let store = container.notificationStore {
            if let notificationId, let byId = try? store.findById(notificationId) {
                entity = byId
            } else if let byId = try? store.findById(dedupeKey) {
                entity = byId
            } else if let matches = try? store.findByDedupeKey(dedupeKey), let latest = matches.sorted(by: { $0.createdAt > $1.createdAt }).first {
                entity = latest
            }
        }

        var run: LocalJSONStorage.ScheduledActionRunRecord? = nil
        if let storage = container.localJSONStorage {
            let allRuns = (try? storage.getScheduledActionRuns(limit: 500)) ?? []
            let key = entity?.dedupeKey ?? dedupeKey
            run = allRuns.first(where: { ($0.dedupeKey ?? "") == key })
            if run == nil, let sourceId = event?.sourceId, !sourceId.isEmpty {
                run = (try? storage.getScheduledActionRuns(limit: 1, actionId: sourceId))?.first
            }
        }

        await MainActor.run {
            self.matchedEntity = entity
            self.matchedRun = run
        }
    }

    private func lookupPendingNotification() async {
        let keys = lookupKeys
        await withCheckedContinuation { cont in
            UNUserNotificationCenter.current().getPendingNotificationRequests { reqs in
                DispatchQueue.main.async {
                    // Try match by identifier or by userInfo["dedupeKey"]
                    if let req = reqs.first(where: { req in
                        keys.contains(req.identifier) || keys.contains((req.content.userInfo["dedupeKey"] as? String) ?? "")
                    }) {
                        self.hasPendingNotification = true
                        if let trig = req.trigger {
                            if let cal = trig as? UNCalendarNotificationTrigger {
                                self.notificationScheduledAt = cal.nextTriggerDate()
                            } else if let timeTrig = trig as? UNTimeIntervalNotificationTrigger {
                                self.notificationScheduledAt = timeTrig.nextTriggerDate()
                            } else {
                                self.notificationScheduledAt = nil
                            }
                        } else {
                            self.notificationScheduledAt = nil
                        }
                    } else {
                        self.hasPendingNotification = false
                        self.notificationScheduledAt = nil
                    }
                    cont.resume()
                }
            }
        }
    }
    
    // MARK: - Helper functions
    
    private var sourceLabel: String {
        switch event?.sourceType {
        case .calendar: return NSLocalizedString("notification.detail.source.calendar", value: "Calendar", comment: "Calendar source label")
        case .reminder: return NSLocalizedString("autogen.NotificationDetailView.81944e48", comment: "AUTOGEN: Reminder")
        case .health: return NSLocalizedString("autogen.NotificationDetailView.28ff5ffe", comment: "AUTOGEN: Health")
        case .url: return NSLocalizedString("autogen.NotificationDetailView.640e8fc4", comment: "AUTOGEN: Scheduled URL action")
        case .manual: return NSLocalizedString("autogen.NotificationDetailView.2a4a4de8", comment: "AUTOGEN: Manual")
        default: return NSLocalizedString("autogen.NotificationDetailView.1a26edf9", comment: "AUTOGEN: Other")
        }
    }
    
    private var sourceIcon: String {
        switch event?.sourceType {
        case .calendar: return "calendar.circle.fill"
        case .reminder: return "checkmark.circle.fill"
        case .health: return "heart.circle.fill"
        case .url: return "link.circle.fill"
        case .manual: return "pencil.circle.fill"
        default: return "doc.circle.fill"
        }
    }
    
    private var sourceColor: Color {
        switch event?.sourceType {
        case .calendar: return .orange
        case .reminder: return .green
        case .health: return .red
        case .url: return .blue
        case .manual: return .gray
        default: return .gray
        }
    }

    private func friendlyTaskType(_ raw: String) -> String {
        switch raw.lowercased() {
        case "birthday": return NSLocalizedString("autogen.NotificationDetailView.25999591", comment: "AUTOGEN: Birthday reminder")
        case "reminder": return NSLocalizedString("autogen.NotificationDetailView.81944e48", comment: "AUTOGEN: Reminder")
        case "debt": return NSLocalizedString("autogen.NotificationDetailView.b6f9b4bd", comment: "AUTOGEN: Debt")
        case "custom": return NSLocalizedString("autogen.NotificationDetailView.ec7b537e", comment: "AUTOGEN: Custom task")
        default: return raw.capitalized
        }
    }
    
    private func taskTypeIcon(_ raw: String) -> String {
        switch raw.lowercased() {
        case "birthday": return "gift.fill"
        case "reminder": return "bell.fill"
        case "debt": return "dollarsign.circle.fill"
        case "custom": return "star.fill"
        default: return "tag.fill"
        }
    }

    private func iso8601Date(_ s: String) -> Date? {
        let f = ISO8601DateFormatter()
        return f.date(from: s)
    }

    private func loadCanonicalCreatedAt() async {
        guard let store = container.notificationStore else { return }
        do {
            let entity: NotificationEntity?
            if let notificationId {
                entity = try store.findById(notificationId)
            } else if let byId = try store.findById(dedupeKey) {
                entity = byId
            } else {
                entity = try store.findByDedupeKey(dedupeKey).sorted(by: { $0.createdAt > $1.createdAt }).first
            }
            if let entity {
                await MainActor.run {
                    self.canonicalCreatedAt = entity.createdAt
                }
            }
        } catch {
            container.logger.log(.error, "NotificationDetailView", "Failed to load canonical createdAt", metadata: ["error": error.localizedDescription])
        }
    }

    private static func formatMetaDate(_ date: Date) -> String {
        let formatter = DateFormatter()
        formatter.dateStyle = .medium
        formatter.timeStyle = .medium
        return formatter.string(from: date)
    }

    // Mark notification as processed: cancel pending request and notify lists
    private func markAsProcessed() {
        Task {
            await performMarkAsProcessedWithUndoSupport()
        }
    }

    private func performMarkAsProcessedWithUndoSupport() async {
        // Capture pending UNNotificationRequest if present
        let center = UNUserNotificationCenter.current()
        var capturedRequest: UNNotificationRequest? = nil
        await withCheckedContinuation { cont in
            center.getPendingNotificationRequests { reqs in
                let keys = self.lookupKeys
                if let req = reqs.first(where: { req in
                    keys.contains(req.identifier) || keys.contains((req.content.userInfo["dedupeKey"] as? String) ?? "")
                }) {
                    capturedRequest = req
                }
                cont.resume()
            }
        }

        // Store captured request locally so we can re-add if the user undoes
        await MainActor.run {
            self.lastRemovedRequest = capturedRequest
        }

        // Remove pending request from system
        center.removePendingNotificationRequests(withIdentifiers: lookupKeys)

        // Also inform internal notification service to remove tracking.
        await MainActor.run {
            container.notificationService.cancelNotification(withKey: dedupeKey)
        }

        // Update persisted store (if available) and record updated IDs for undo
        var markedIds: [String] = []
        if let store = container.notificationStore {
            do {
                let matches: [NotificationEntity]
                if let notificationId, let exact = try store.findById(notificationId) {
                    matches = [exact]
                } else if let exact = try store.findById(dedupeKey) {
                    matches = [exact]
                } else {
                    matches = try store.findByDedupeKey(dedupeKey)
                }
                for e in matches {
                    try store.markSent(id: e.id)
                    markedIds.append(e.id)
                }
            } catch {
                container.logger.log(.error, "NotificationDetailView", "Failed to mark persisted notifications as sent", metadata: ["error": error.localizedDescription])
            }
        }

        // Update UI first
        await MainActor.run {
            self.hasPendingNotification = false
            self.notificationScheduledAt = nil
            self.lastMarkedEntityIDs = markedIds
        }

        // Notify HomeView to remove this processed event from the list.
        NotificationCenter.default.post(
            name: NSNotification.Name("com.nola.processedEvents.removed"),
            object: nil,
            userInfo: ["dedupeKey": self.dedupeKey]
        )
        AppEventBus.postNotificationsUpdated()

        // Small delay to ensure broadcasts are processed before dismissing
        try? await Task.sleep(nanoseconds: 100_000_000) // 0.1 seconds
        
        // Dismiss and return to previous screen
        await MainActor.run {
            dismiss()
        }
    }

    private func undoMarkAsProcessed() {
        Task { @MainActor in
            // Re-add previously removed UNNotificationRequest if available
            let req = self.lastRemovedRequest
            if let req {
                do {
                    try await UNUserNotificationCenter.current().add(req)
                } catch {
                    container.logger.log(.error, "NotificationDetailView", "Failed to re-add notification request", metadata: ["error": error.localizedDescription])
                }
            }

            // Revert persisted entities to pending
            if let store = container.notificationStore {
                let ids = await MainActor.run { self.lastMarkedEntityIDs }
                for id in ids {
                    do {
                        try store.markPending(id: id)
                    } catch {
                        container.logger.log(.error, "NotificationDetailView", "Failed to revert persisted notification", metadata: ["error": error.localizedDescription])
                    }
                }
            }

            // Broadcast update and reset UI
            AppEventBus.postNotificationsUpdated()
            await MainActor.run {
                self.hasPendingNotification = true
                self.showUndoBanner = false
                self.lastRemovedRequest = nil
                self.lastMarkedEntityIDs = []
            }
        }
    }
}

// MARK: - Summary Detail View (for AI-generated summary items)

struct SummaryDetailView: View {
    @EnvironmentObject var container: ServiceContainer
    let item: SummaryItem

    @State private var relatedEvent: ProcessedEvent? = nil
    
    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 20) {
                // Top card with title and priority.
                VStack(alignment: .leading, spacing: 16) {
                    // Priority badge.
                    HStack(spacing: 12) {
                        ZStack {
                            Circle()
                                .fill(priorityColor.opacity(0.14))
                                .frame(width: 56, height: 56)

                            Image(systemName: prioritySymbol)
                                .font(.system(size: 20, weight: .semibold))
                                .foregroundColor(priorityColor)
                        }
                        
                        VStack(alignment: .leading, spacing: 4) {
                            Text(L10n.notificationPriority)
                                .font(.caption)
                                .foregroundColor(.secondary)
                            priorityDots(for: item.priority)
                            HStack(spacing: 4) {
                                Text(priorityLabel)
                                Text("·")
                                Text(L10n.format("home.priority.percent_format", Int(item.priority * 100)))
                            }
                            .font(.caption2)
                            .foregroundColor(.secondary)
                        }
                        
                        Spacer()
                    }
                    
                    Divider()
                    
                    // Title.
                    Text(item.text)
                        .font(.system(.title2, design: .rounded))
                        .fontWeight(.bold)
                        .foregroundColor(.primary)
                        .lineSpacing(4)
                }
                .padding(20)
                .background(
                    RoundedRectangle(cornerRadius: DesignTokens.radius_lg, style: .continuous)
                        .fill(DesignTokens.surfaceColor)
                )
                .overlay(
                    RoundedRectangle(cornerRadius: DesignTokens.radius_lg, style: .continuous)
                        .stroke(DesignTokens.borderColor, lineWidth: 1)
                )
                
                // AI summary card.
                HStack(spacing: 12) {
                    ZStack {
                        RoundedRectangle(cornerRadius: DesignTokens.radius_sm, style: .continuous)
                            .fill(DesignTokens.primaryColor.opacity(0.12))
                            .frame(width: 48, height: 48)
                        Image(systemName: "sparkles")
                            .font(.system(size: 20, weight: .semibold))
                            .foregroundColor(DesignTokens.primaryColor)
                    }
                    
                    VStack(alignment: .leading, spacing: 4) {
                        Text(L10n.notificationAISummaryTitle)
                            .font(.headline)
                            .fontWeight(.semibold)
                        Text(L10n.notificationAISummaryDesc)
                            .font(.caption)
                            .foregroundColor(.secondary)
                            .lineSpacing(2)
                    }
                    
                    Spacer()
                }
                .padding(16)
                .background(
                    RoundedRectangle(cornerRadius: DesignTokens.radius_md, style: .continuous)
                        .fill(DesignTokens.surfaceColor)
                )
                .overlay(
                    RoundedRectangle(cornerRadius: DesignTokens.radius_md, style: .continuous)
                        .stroke(DesignTokens.borderColor, lineWidth: 1)
                )
                
                // Suggested action card (when an action exists).
                if item.action != nil {
                    VStack(alignment: .leading, spacing: 12) {
                        HStack(spacing: 8) {
                            Image(systemName: "hand.tap.fill")
                                .foregroundColor(.blue)
                            Text(L10n.notificationSuggestedActions)
                                .font(.headline)
                                .fontWeight(.semibold)
                        }
                        
                        HStack(spacing: 10) {
                            Image(systemName: "lightbulb.fill")
                                .foregroundColor(.orange)
                            Text(L10n.notificationTapSummaryAction)
                                .font(.subheadline)
                                .foregroundColor(.secondary)
                        }
                        .padding(12)
                        .frame(maxWidth: .infinity, alignment: .leading)
                        .background(
                            RoundedRectangle(cornerRadius: 10)
                                .fill(DesignTokens.primaryColor.opacity(0.05))
                        )
                    }
                    .padding(16)
                    .background(
                        RoundedRectangle(cornerRadius: 12)
                            .fill(Color(.systemBackground))
                            .shadow(color: Color.black.opacity(0.05), radius: 4, x: 0, y: 2)
                    )
                }

                
                
                // Informational hint.
                HStack(alignment: .top, spacing: 12) {
                    Image(systemName: "info.circle.fill")
                        .font(.title3)
                        .foregroundColor(.blue)
                    
                    VStack(alignment: .leading, spacing: 6) {
                        Text(L10n.notificationAboutSummaryTitle)
                            .font(.caption)
                            .fontWeight(.semibold)
                            .foregroundColor(.secondary)
                        Text(L10n.notificationAboutSummaryDesc)
                            .font(.caption)
                            .foregroundColor(.secondary)
                            .lineSpacing(3)
                    }
                }
                .padding(14)
                .background(
                    RoundedRectangle(cornerRadius: 10)
                        .fill(DesignTokens.primaryColor.opacity(0.05))
                        .overlay(
                            RoundedRectangle(cornerRadius: 10)
                                .stroke(DesignTokens.primaryColor.opacity(0.2), lineWidth: 1)
                        )
                )

                if let ev = relatedEvent {
                    VStack(alignment: .leading, spacing: 10) {
                        Text(L10n.notificationRelatedEvent)
                            .font(.caption)
                            .fontWeight(.semibold)
                            .foregroundColor(.secondary)

                        VStack(alignment: .leading, spacing: 6) {
                            Text(ev.title)
                                .font(.headline)
                                .fontWeight(.semibold)
                                .fixedSize(horizontal: false, vertical: true)
                            if let date = ev.date {
                                HStack(spacing: 6) {
                                    Image(systemName: "calendar")
                                        .font(.caption)
                                    Text(date.formatted(date: .abbreviated, time: .shortened))
                                        .font(.caption)
                                }
                                .foregroundColor(.secondary)
                            }
                        }

                        NavigationLink(destination: NotificationDetailView(dedupeKey: ev.dedupeKey).environmentObject(container)) {
                            HStack {
                                Image(systemName: "bell.fill")
                                    .foregroundColor(.orange)
                                Text(L10n.notificationViewDetail)
                                    .foregroundColor(.blue)
                                Spacer()
                            }
                            .padding(12)
                            .background(
                                RoundedRectangle(cornerRadius: 12)
                                    .fill(Color(.secondarySystemBackground))
                            )
                        }
                        .buttonStyle(.plain)
                    }
                    .padding(16)
                    .background(
                        RoundedRectangle(cornerRadius: 12)
                            .fill(Color(.systemBackground))
                            .shadow(color: Color.black.opacity(0.05), radius: 4, x: 0, y: 2)
                    )
                }
                
                Spacer()
            }
            .padding(16)
        }
        .background(Color(.secondarySystemBackground))
        .navigationTitle(L10n.notificationSummaryDetailTitle)
        #if os(iOS)
        .navigationBarTitleDisplayMode(.inline)
        #endif
        .task {
            await loadRelatedEventIfAny()
        }
    }

    private func loadRelatedEventIfAny() async {
        if !container.recentProcessedEvents.isEmpty,
           let found = RustInterop.findProcessedEventInList(processed: container.recentProcessedEvents, key: item.dedupeKey) {
            await MainActor.run { relatedEvent = found }
            return
        }

        guard let store = container.unifiedStorage as? FileBasedUnifiedStorage else { return }
        let key = item.dedupeKey
        let all = (try? await store.fetchProcessedEventsAsync(limit: nil)) ?? []
        let found: ProcessedEvent? = {
            if let direct = all.first(where: { $0.dedupeKey == key || $0.id == key }) { return direct }
            if let partial = all.first(where: { $0.dedupeKey.contains(key) || key.contains($0.dedupeKey) }) { return partial }
            return nil
        }()

        if let found {
            await MainActor.run { relatedEvent = found }
        }
    }
    
    private var priorityColor: Color {
        if item.priority >= 0.8 { return .red }
        if item.priority >= 0.5 { return .orange }
        return .green
    }
    
    private var priorityLabel: String {
        if item.priority >= 0.8 { return L10n.homePriorityHigh }
        if item.priority >= 0.5 { return L10n.homePriorityMedium }
        return L10n.homePriorityLow
    }
    private var prioritySymbol: String {
        if item.priority >= 0.8 { return "flame.fill" }
        if item.priority >= 0.5 { return "bolt.fill" }
        return "checkmark"
    }

    // Helper to render priority dots (extracted to reduce compiler type-checking cost)
    @ViewBuilder
    private func priorityDots(for priority: Double) -> some View {
        let filledCount = Int(priority * 5)
        HStack(spacing: 6) {
            ForEach(0..<5) { index in
                Circle()
                    .fill(index < filledCount ? priorityColor : Color.gray.opacity(0.2))
                    .frame(width: 8, height: 8)
            }
        }
    }
}

// Helper to get string value from JSONValue (duplicate of internal extension)
private extension JSONValue {
    var stringValue: String? {
        switch self {
        case .string(let s): return s
        case .number(let n): return String(n)
        case .bool(let b): return b ? "true" : "false"
        default: return nil
        }
    }
}

// MARK: - Helper view components

/// Timeline row component.
struct TimelineRow: View {
    let icon: String
    let title: String
    let time: Date
    let color: Color
    
    var body: some View {
        HStack(spacing: 12) {
            ZStack {
                Circle()
                    .fill(color.opacity(0.15))
                    .frame(width: 36, height: 36)
                Image(systemName: icon)
                    .font(.system(size: 14))
                    .foregroundColor(color)
            }
            
            VStack(alignment: .leading, spacing: 2) {
                Text(title)
                    .font(.caption)
                    .foregroundColor(.secondary)
                Text(time.formatted(date: .abbreviated, time: .shortened))
                    .font(.subheadline)
                    .fontWeight(.medium)
                    .foregroundColor(.primary)
            }
            
            Spacer()
        }
    }
}

/// Info card component.
struct InfoCard: View {
    let icon: String
    let iconColor: Color
    let title: String
    let content: String
    
    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            HStack(spacing: 10) {
                ZStack {
                    Circle()
                        .fill(iconColor.opacity(0.15))
                        .frame(width: 32, height: 32)
                    Image(systemName: icon)
                        .font(.system(size: 14))
                        .foregroundColor(iconColor)
                }
                Text(title)
                    .font(.subheadline)
                    .fontWeight(.semibold)
                    .foregroundColor(.primary)
                Spacer()
            }
            
            Text(content)
                .font(.body)
                .foregroundColor(.secondary)
                .lineSpacing(6)
                .fixedSize(horizontal: false, vertical: true)
        }
        .padding(16)
        .background(
            RoundedRectangle(cornerRadius: 16)
                .fill(Color(.secondarySystemBackground))
                .overlay(
                    RoundedRectangle(cornerRadius: 16)
                        .stroke(iconColor.opacity(0.1), lineWidth: 1)
                )
        )
    }
}
