// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Manages custom JSON, feed, and text sources used by macOS retrieval context.

import SwiftUI

private func macWebSourcesText(_ key: String, _ fallback: String) -> String {
    let keyed = Bundle.main.localizedString(forKey: key, value: nil, table: nil)
    guard keyed == key else { return keyed }
    return Bundle.main.localizedString(forKey: fallback, value: fallback, table: nil)
}

struct MacWebSourcesView: View {
    @State private var store = MacWebSourceStore.shared
    @State private var selectedID: UUID?
    @State private var editingSource: MacWebSource?
    @State private var showsEditor = false
    @State private var pendingDeletion: MacWebSource?
    @State private var successNotice: String?
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    let navigationAction: (() -> Void)?

    init(navigationAction: (() -> Void)? = nil) {
        self.navigationAction = navigationAction
    }

    var body: some View {
        VStack(spacing: 0) {
            webSourcesHeader

            MacPageMetricsBar(metrics: sourceMetrics, isWorking: !store.refreshingIDs.isEmpty)
                .padding(.horizontal, MacDesignTokens.Layout.pageGutter)
                .padding(.vertical, MacDesignTokens.Spacing.sm)

            if let errorMessage = store.errorMessage {
                MacInlineNotice(
                    title: macWebSourcesText("mac.web_sources.error.title", "Web Source Error"),
                    message: errorMessage,
                    kind: .failure,
                    dismiss: { store.errorMessage = nil }
                )
                .padding(.horizontal, MacDesignTokens.Layout.pageGutter)
                .padding(.bottom, MacDesignTokens.Spacing.sm)
                .transition(.move(edge: .top).combined(with: .opacity))
            }

            if let successNotice {
                MacInlineNotice(message: successNotice, kind: .success, dismiss: {
                    withAnimation(reduceMotion ? nil : .easeInOut(duration: 0.18)) {
                        self.successNotice = nil
                    }
                })
                .padding(.horizontal, MacDesignTokens.Layout.pageGutter)
                .padding(.bottom, MacDesignTokens.Spacing.sm)
                .transition(.move(edge: .top).combined(with: .opacity))
            }

            if store.sources.isEmpty {
                MacEmptyState(
                    LocalizedStringKey(macWebSourcesText("mac.web_sources.empty.title", "No Web Sources")),
                    description: LocalizedStringKey(macWebSourcesText("mac.web_sources.empty.detail", "Add JSON, RSS, Atom, or text sources for retrieval context.")),
                    symbol: "text.badge.plus"
                ) {
                    Button {
                        editingSource = nil
                        showsEditor = true
                    } label: {
                        Label(macWebSourcesText("mac.web_sources.add", "Add Source"), systemImage: "plus")
                    }
                    .buttonStyle(.borderedProminent)
                }
            } else {
                webSourcesSplitView
            }
        }
        .background(MacPageBackground())
        .task { if selectedID == nil { selectedID = store.sources.first?.id } }
        .onChange(of: store.sources.map(\.id)) { _, ids in
            guard let selectedID, !ids.contains(selectedID) else { return }
            self.selectedID = ids.first
        }
        .sheet(isPresented: $showsEditor) {
            MacWebSourceEditor(source: editingSource) { source in
                try await store.validateAndUpsert(source)
                selectedID = source.id
                withAnimation(reduceMotion ? nil : .easeInOut(duration: 0.2)) {
                    successNotice = String(
                        format: macWebSourcesText("mac.web_sources.success.ready", "%@ was tested and is ready."),
                        source.name
                    )
                }
                showsEditor = false
            } cancel: {
                showsEditor = false
            }
        }
        .confirmationDialog(macWebSourcesText("mac.web_sources.delete.title", "Delete Web Source?"), isPresented: Binding(
            get: { pendingDeletion != nil },
            set: { if !$0 { pendingDeletion = nil } }
        ), titleVisibility: .visible) {
            Button(macWebSourcesText("mac.web_sources.cancel", "Cancel"), role: .cancel) { pendingDeletion = nil }
            Button(macWebSourcesText("mac.web_sources.delete", "Delete"), role: .destructive) {
                guard let source = pendingDeletion else { return }
                store.delete(source)
                pendingDeletion = nil
            }
        } message: {
            Text(macWebSourcesText("mac.web_sources.delete.detail", "Cached content and run history for this source will be removed from this Mac."))
        }
        .animation(reduceMotion ? nil : .easeInOut(duration: 0.18), value: store.errorMessage)
    }

    private var selectedSource: MacWebSource? {
        guard let selectedID else { return nil }
        return store.sources.first { $0.id == selectedID }
    }

    private var sourceMetrics: [MacPageMetric] {
        [
            MacPageMetric(
                id: "web-sources-total",
                LocalizedStringKey(macWebSourcesText("mac.web_sources.metric.total", "Web Sources")),
                value: "\(store.sources.count)",
                symbol: "network",
                tint: MacDesignColor.brand
            ),
            MacPageMetric(
                id: "web-sources-enabled",
                LocalizedStringKey(macWebSourcesText("mac.web_sources.metric.enabled", "Enabled")),
                value: "\(store.sources.filter(\.isEnabled).count)",
                symbol: "checkmark.circle.fill",
                tint: .green
            ),
            MacPageMetric(
                id: "web-sources-attention",
                LocalizedStringKey(macWebSourcesText("mac.web_sources.metric.attention", "Attention")),
                value: "\(store.sources.filter { $0.lastError != nil }.count)",
                symbol: "exclamationmark.triangle.fill",
                tint: .orange
            ),
        ]
    }

    private var webSourcesSplitView: some View {
        MacDesktopSplitView(
            sidebarIdealWidth: MacDesignTokens.Layout.sidebarWidth,
            sidebarMaxWidth: 320
        ) {
            webSourcesList
        } detail: {
            webSourceDetail
        }
    }

    private var webSourcesList: some View {
        List(selection: $selectedID) {
            ForEach(store.sources) { source in
                webSourceListRow(source)
            }
        }
        .listStyle(.sidebar)
    }

    @ViewBuilder
    private var webSourceDetail: some View {
        if let source = selectedSource {
            MacWebSourceDetail(
                source: source,
                store: store,
                isRefreshing: store.refreshingIDs.contains(source.id)
            ) {
                editingSource = source
                showsEditor = true
            }
        } else {
            MacEmptyState(
                LocalizedStringKey(macWebSourcesText("mac.web_sources.title", "Web Sources")),
                description: LocalizedStringKey(macWebSourcesText("mac.web_sources.select.detail", "Select a source to review its cached content and refresh status.")),
                symbol: "text.magnifyingglass"
            ) {}
        }
    }

    private func webSourceListRow(_ source: MacWebSource) -> some View {
        let isRefreshing = store.refreshingIDs.contains(source.id)
        return MacWebSourceRow(source: source, isRefreshing: isRefreshing)
            .tag(source.id)
            .contextMenu {
                Button {
                    Task { await store.refresh(source) }
                } label: {
                    Text(verbatim: macWebSourcesText("mac.web_sources.refresh", "Refresh"))
                }
                .disabled(isRefreshing)
                Button {
                    editingSource = source
                    showsEditor = true
                } label: {
                    Text(verbatim: macWebSourcesText("mac.web_sources.edit", "Edit"))
                }
                Divider()
                Button(role: .destructive) {
                    pendingDeletion = source
                } label: {
                    Text(verbatim: macWebSourcesText("mac.web_sources.delete", "Delete"))
                }
            }
    }

    private var webSourcesHeader: some View {
        MacPageHeader(
            LocalizedStringKey(macWebSourcesText("mac.web_sources.title", "Web Sources")),
            symbol: "network",
            navigationAction: navigationAction,
            navigationSymbol: "chevron.left",
            navigationLabel: LocalizedStringKey(macWebSourcesText("mac.web_sources.data_center", "Data Center"))
        ) {
            Text(macWebSourcesText("mac.web_sources.subtitle", "Sources available to retrieval context"))
        } actions: {
            refreshSourceButton
            addSourceButton
        }
    }

    private var refreshSourceButton: some View {
        Button {
            Task { await store.refreshAll() }
        } label: {
            refreshButtonLabel
        }
        .disabled(!store.refreshingIDs.isEmpty)
    }

    private var addSourceButton: some View {
        Button {
            editingSource = nil
            showsEditor = true
        } label: {
            Label {
                Text(verbatim: macWebSourcesText("mac.web_sources.add", "Add Source"))
            } icon: {
                Image(systemName: "plus")
            }
        }
        .buttonStyle(.borderedProminent)
    }

    @ViewBuilder
    private var refreshButtonLabel: some View {
        if store.refreshingIDs.isEmpty {
            Label {
                Text(verbatim: macWebSourcesText("mac.web_sources.refresh", "Refresh"))
            } icon: {
                Image(systemName: "arrow.clockwise")
            }
        } else {
            ProgressView()
                .controlSize(.small)
                .accessibilityLabel(macWebSourcesText("mac.web_sources.refreshing", "Refreshing"))
        }
    }
}

private struct MacWebSourceRow: View {
    let source: MacWebSource
    let isRefreshing: Bool

    var body: some View {
        HStack(spacing: 10) {
            if isRefreshing { ProgressView().controlSize(.mini).frame(width: 16) }
            else {
                Image(systemName: source.lastError == nil ? "network" : "exclamationmark.triangle")
                    .foregroundStyle(source.lastError == nil ? Color.accentColor : Color.orange)
                    .frame(width: 16)
            }
            VStack(alignment: .leading, spacing: 2) {
                Text(source.name).font(.system(size: 13, weight: .semibold)).lineLimit(1)
                Text("\(source.format.title) · \(source.scheduleDescription)")
                    .font(.system(size: 11))
                    .foregroundStyle(.secondary)
                    .lineLimit(1)
            }
            Spacer()
            Toggle("", isOn: Binding(
                get: { source.isEnabled },
                set: { MacWebSourceStore.shared.setEnabled($0, for: source) }
            ))
            .labelsHidden()
            .toggleStyle(.switch)
            .controlSize(.mini)
            .accessibilityLabel(String(format: macWebSourcesText("mac.web_sources.enable_format", "Enable %@"), source.name))
        }
        .padding(.vertical, 3)
    }
}

private struct MacWebSourceDetail: View {
    let source: MacWebSource
    @Bindable var store: MacWebSourceStore
    let isRefreshing: Bool
    let edit: () -> Void

    var body: some View {
        VStack(spacing: 0) {
            ViewThatFits(in: .horizontal) {
                HStack(alignment: .top, spacing: 18) {
                    sourceIdentity
                    Spacer(minLength: 16)
                    sourceActions
                }
                VStack(alignment: .leading, spacing: 14) {
                    sourceIdentity
                    sourceActions
                }
            }
            .padding(20)
            Divider()

            ScrollView {
                VStack(alignment: .leading, spacing: 16) {
                    if let error = source.lastError {
                        Label(error, systemImage: "exclamationmark.triangle.fill")
                            .font(.callout)
                            .foregroundStyle(.orange)
                            .textSelection(.enabled)
                            .padding(14)
                            .frame(maxWidth: .infinity, alignment: .leading)
                            .background(.orange.opacity(0.08), in: RoundedRectangle(cornerRadius: 12))
                    }

                    if source.cachedText.isEmpty {
                        MacEmptyState(
                            LocalizedStringKey(macWebSourcesText("mac.web_sources.cache.empty.title", "No Cached Content")),
                            description: LocalizedStringKey(macWebSourcesText("mac.web_sources.cache.empty.detail", "Refresh this source to fetch its current content.")),
                            symbol: "arrow.clockwise"
                        ) {}
                        .frame(maxWidth: .infinity, minHeight: 260)
                    } else {
                        detailSection(macWebSourcesText("mac.web_sources.cache.title", "Cached Content"), symbol: "doc.text") {
                            Text(source.cachedText)
                                .font(.system(size: 11.5, design: .monospaced))
                                .textSelection(.enabled)
                                .fixedSize(horizontal: false, vertical: true)
                                .frame(maxWidth: .infinity, alignment: .leading)
                        }
                    }

                    if let lastFetchedAt = source.lastFetchedAt {
                        Label(
                            String(
                                format: macWebSourcesText("mac.web_sources.updated_format", "Updated %@"),
                                lastFetchedAt.formatted(date: .abbreviated, time: .shortened)
                            ),
                            systemImage: "clock.arrow.circlepath"
                        )
                        .font(.caption)
                        .foregroundStyle(.secondary)
                    }

                    if let runs = source.runs, !runs.isEmpty {
                        detailSection(macWebSourcesText("mac.web_sources.run_history", "Run History"), symbol: "clock.badge.checkmark") {
                            VStack(spacing: 0) {
                                ForEach(Array(runs.prefix(5).enumerated()), id: \.element.id) { index, run in
                                    HStack(alignment: .firstTextBaseline, spacing: 8) {
                                        Image(systemName: run.succeeded ? "checkmark.circle.fill" : "exclamationmark.circle.fill")
                                            .foregroundStyle(run.succeeded ? Color.green : Color.orange)
                                        Text(run.finishedAt, style: .relative)
                                            .foregroundStyle(.secondary)
                                        Text(run.detail)
                                            .lineLimit(2)
                                        Spacer(minLength: 0)
                                    }
                                    .font(.callout)
                                    .padding(.vertical, 9)
                                    if index < min(runs.count, 5) - 1 {
                                        Divider().opacity(0.55)
                                    }
                                }
                            }
                        }
                    }
                }
                .padding(20)
                .frame(maxWidth: 860)
                .frame(maxWidth: .infinity)
            }
        }
    }

    private var sourceIdentity: some View {
        VStack(alignment: .leading, spacing: 5) {
            Text(source.name)
                .font(.system(size: 19, weight: .semibold))
                .fixedSize(horizontal: false, vertical: true)
            Text(source.url)
                .font(.system(size: 10.5, design: .monospaced))
                .foregroundStyle(.secondary)
                .textSelection(.enabled)
                .lineLimit(2)
                .truncationMode(.middle)
            Label(source.scheduleDescription, systemImage: "clock")
                .font(.system(size: 10.5))
                .foregroundStyle(.secondary)
        }
    }

    private var sourceActions: some View {
        HStack(spacing: 8) {
            Button(action: edit) {
                Label(macWebSourcesText("mac.web_sources.edit", "Edit"), systemImage: "square.and.pencil")
            }
            Button {
                Task { await store.refresh(source) }
            } label: {
                if isRefreshing {
                    ProgressView()
                        .controlSize(.small)
                } else {
                    Label(macWebSourcesText("mac.web_sources.refresh", "Refresh"), systemImage: "arrow.clockwise")
                }
            }
            .buttonStyle(.borderedProminent)
            .disabled(isRefreshing)
        }
    }

    private func detailSection<Content: View>(
        _ title: String,
        symbol: String,
        @ViewBuilder content: () -> Content
    ) -> some View {
        VStack(alignment: .leading, spacing: 12) {
            Label(title, systemImage: symbol)
                .font(.headline)
            content()
        }
        .padding(16)
        .frame(maxWidth: .infinity, alignment: .leading)
        .macPanel(cornerRadius: MacDesignTokens.Radius.floating)
    }
}

private struct MacWebSourceEditor: View {
    let original: MacWebSource?
    let save: (MacWebSource) async throws -> Void
    let cancel: () -> Void

    @State private var name: String
    @State private var url: String
    @State private var format: MacWebSource.Format
    @State private var isEnabled: Bool
    @State private var scheduleFrequency: MacWebSource.ScheduleFrequency?
    @State private var scheduledWeekday: Int
    @State private var scheduledTime: Date
    @State private var scheduledDate: Date
    @State private var isSaving = false
    @State private var saveTask: Task<Void, Never>?
    @State private var validationError: String?
    @State private var showsDiscardConfirmation = false
    @FocusState private var nameFocused: Bool
    @Environment(\.accessibilityReduceMotion) private var reduceMotion

    init(source: MacWebSource?, save: @escaping (MacWebSource) async throws -> Void, cancel: @escaping () -> Void) {
        original = source
        self.save = save
        self.cancel = cancel
        _name = State(initialValue: source?.name ?? "")
        _url = State(initialValue: source?.url ?? "")
        _format = State(initialValue: source?.format ?? .automatic)
        _isEnabled = State(initialValue: source?.isEnabled ?? true)
        _scheduleFrequency = State(initialValue: source?.scheduleFrequency)
        _scheduledWeekday = State(initialValue: source?.scheduledWeekday ?? 2)
        var components = DateComponents()
        components.hour = source?.scheduledHour ?? 9
        components.minute = source?.scheduledMinute ?? 0
        _scheduledTime = State(initialValue: Calendar.current.date(from: components) ?? .now)
        _scheduledDate = State(initialValue: source?.scheduledDate ?? Date().addingTimeInterval(3_600))
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 16) {
            HStack(alignment: .top, spacing: 12) {
                MacIconTile(symbol: original == nil ? "network.badge.shield.half.filled" : "square.and.pencil", tint: MacDesignColor.brand)
                VStack(alignment: .leading, spacing: 3) {
                    Text(original == nil
                        ? macWebSourcesText("mac.web_sources.editor.new", "New Web Source")
                        : macWebSourcesText("mac.web_sources.editor.edit", "Edit Web Source"))
                        .font(.system(size: 19, weight: .semibold))
                    Text(macWebSourcesText("mac.web_sources.editor.test_detail", "Nola tests the source before saving its reader and data format."))
                        .font(.system(size: 11.5))
                        .foregroundStyle(.secondary)
                }
            }
            Form {
                TextField(macWebSourcesText("mac.web_sources.form.name", "Name"), text: $name)
                    .focused($nameFocused)
                    .onChange(of: name) { _, _ in validationError = nil }
                TextField(macWebSourcesText("mac.web_sources.form.url", "URL"), text: $url, prompt: Text("https://example.com/feed.json"))
                    .onChange(of: url) { _, _ in validationError = nil }
                Picker(macWebSourcesText("mac.web_sources.form.format", "Format"), selection: $format) {
                    ForEach(MacWebSource.Format.allCases) { format in Text(format.title).tag(format) }
                }
                Toggle(macWebSourcesText("mac.web_sources.form.enabled", "Enabled"), isOn: $isEnabled)
                Picker(macWebSourcesText("mac.web_sources.form.schedule", "Schedule"), selection: $scheduleFrequency) {
                    Text(macWebSourcesText("mac.web_sources.schedule.manual", "Manual")).tag(nil as MacWebSource.ScheduleFrequency?)
                    ForEach(MacWebSource.ScheduleFrequency.allCases) { frequency in
                        Text(frequency.title).tag(Optional(frequency))
                    }
                }
                if scheduleFrequency == .once {
                    DatePicker(macWebSourcesText("mac.web_sources.form.run_at", "Run at"), selection: $scheduledDate)
                } else if let scheduleFrequency {
                    if scheduleFrequency == .weekly {
                        Picker(macWebSourcesText("mac.web_sources.form.weekday", "Weekday"), selection: $scheduledWeekday) {
                            ForEach(1...7, id: \.self) { weekday in
                                Text(weekdayName(weekday)).tag(weekday)
                            }
                        }
                    }
                    DatePicker(macWebSourcesText("mac.web_sources.form.time", "Time"), selection: $scheduledTime, displayedComponents: .hourAndMinute)
                }
            }
            .formStyle(.grouped)
            .disabled(isSaving)
            HStack {
                Group {
                    if isSaving {
                        HStack(spacing: 7) {
                            ProgressView()
                                .controlSize(.small)
                            Text(macWebSourcesText("mac.web_sources.testing", "Testing connection…"))
                                .foregroundStyle(.secondary)
                        }
                    } else if let validationError {
                        Label(validationError, systemImage: "exclamationmark.triangle.fill")
                            .foregroundStyle(.red)
                            .fixedSize(horizontal: false, vertical: true)
                    } else if let inputValidationMessage {
                        Label(inputValidationMessage, systemImage: "exclamationmark.circle.fill")
                            .foregroundStyle(.orange)
                            .fixedSize(horizontal: false, vertical: true)
                    }
                }
                .font(.system(size: 11.5))
                .frame(maxWidth: 300, minHeight: 34, alignment: .leading)
                Spacer()
                Button(isSaving ? macWebSourcesText("mac.web_sources.stop_testing", "Stop Testing") : macWebSourcesText("mac.web_sources.cancel", "Cancel"), action: requestCancel)
                    .keyboardShortcut(.cancelAction)
                Button(
                    original == nil
                        ? macWebSourcesText("mac.web_sources.test_add", "Test & Add")
                        : macWebSourcesText("mac.web_sources.test_save", "Test & Save")
                ) {
                    let source = MacWebSource(
                        id: original?.id ?? UUID(),
                        name: name.trimmingCharacters(in: .whitespacesAndNewlines),
                        url: url.trimmingCharacters(in: .whitespacesAndNewlines),
                        format: format,
                        isEnabled: isEnabled,
                        cachedText: original?.cachedText ?? "",
                        lastFetchedAt: original?.lastFetchedAt,
                        lastError: original?.lastError,
                        createdAt: original?.createdAt ?? .now,
                        scheduleFrequency: scheduleFrequency,
                        scheduledWeekday: scheduleFrequency == .weekly ? scheduledWeekday : nil,
                        scheduledHour: scheduleFrequency == nil || scheduleFrequency == .once
                            ? nil
                            : Calendar.current.component(.hour, from: scheduledTime),
                        scheduledMinute: scheduleFrequency == nil || scheduleFrequency == .once
                            ? nil
                            : Calendar.current.component(.minute, from: scheduledTime),
                        scheduledDate: scheduleFrequency == .once ? scheduledDate : nil,
                        runs: original?.runs
                    )
                    isSaving = true
                    validationError = nil
                    saveTask = Task {
                        defer { isSaving = false; saveTask = nil }
                        do {
                            try await save(source)
                        } catch {
                            guard !Task.isCancelled else { return }
                            validationError = error.localizedDescription
                        }
                    }
                }
                .buttonStyle(.borderedProminent).keyboardShortcut(.defaultAction)
                .disabled(
                    isSaving
                        || inputValidationMessage != nil
                )
            }
        }
        .padding(22)
        .frame(minWidth: 480, idealWidth: 560, maxWidth: 680)
        .frame(minHeight: 500, idealHeight: 560, maxHeight: 700)
        .interactiveDismissDisabled(hasUnsavedChanges || isSaving)
        .onDisappear { saveTask?.cancel() }
        .confirmationDialog(
            macWebSourcesText("mac.web_sources.discard.title", "Discard unsaved changes?"),
            isPresented: $showsDiscardConfirmation,
            titleVisibility: .visible
        ) {
            Button(macWebSourcesText("mac.web_sources.discard", "Discard Changes"), role: .destructive, action: cancel)
            Button(macWebSourcesText("mac.web_sources.cancel", "Cancel"), role: .cancel) {}
        }
        .task {
            guard original == nil else { return }
            await Task.yield()
            nameFocused = true
        }
        .animation(
            reduceMotion ? nil : .easeOut(duration: 0.18),
            value: inputValidationMessage
        )
    }

    private var inputValidationMessage: String? {
        if name.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
            return macWebSourcesText("mac.web_sources.validation.name", "Enter a source name to continue.")
        }
        if url.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
            return macWebSourcesText("mac.web_sources.validation.url", "Enter a source URL to continue.")
        }
        if !hasValidURL {
            return macWebSourcesText("mac.web_sources.validation.url_invalid", "Enter a valid HTTP or HTTPS URL.")
        }
        return nil
    }

    private var hasUnsavedChanges: Bool {
        name != (original?.name ?? "")
            || url != (original?.url ?? "")
            || format != (original?.format ?? .automatic)
            || isEnabled != (original?.isEnabled ?? true)
            || scheduleFrequency != original?.scheduleFrequency
            || scheduledWeekday != (original?.scheduledWeekday ?? 2)
            || scheduledDate != (original?.scheduledDate ?? scheduledDate)
            || Calendar.current.component(.hour, from: scheduledTime) != (original?.scheduledHour ?? 9)
            || Calendar.current.component(.minute, from: scheduledTime) != (original?.scheduledMinute ?? 0)
    }

    private func requestCancel() {
        if isSaving {
            saveTask?.cancel()
            return
        }
        if hasUnsavedChanges {
            showsDiscardConfirmation = true
        } else {
            cancel()
        }
    }

    private var hasValidURL: Bool {
        guard let components = URLComponents(
            string: url.trimmingCharacters(in: .whitespacesAndNewlines)
        ) else { return false }
        return ["http", "https"].contains(components.scheme?.lowercased() ?? "")
            && !(components.host?.isEmpty ?? true)
    }

    private func weekdayName(_ weekday: Int) -> String {
        let symbols = DateFormatter().weekdaySymbols ?? []
        guard symbols.count == 7, (1...7).contains(weekday) else { return weekday.formatted() }
        return symbols[weekday - 1]
    }
}
