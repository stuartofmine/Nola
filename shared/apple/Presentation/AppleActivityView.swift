// SPDX-License-Identifier: Apache-2.0
// Platform adapters supply only Core reads and the existing conversation route.
import SwiftUI
import Observation

/// Read ownership is independent of the title/status fields that may change
/// while a detail page stays open. A late response must not cross this owner.
nonisolated struct AppleActivityReadOwner: Equatable, Sendable {
    let conversation: String
    let run: String
    let assistant: String
    init(_ entry: AppleActivityEntry) {
        conversation = entry.conversation_id; run = entry.run_id; assistant = entry.assistant_message_id
    }
}

@MainActor @Observable
final class AppleActivityDetailReader {
    private var owner: AppleActivityReadOwner?
    private var generation: UInt64 = 0
    private var result: AppleActivityDetail?
    private var failure: String?
    private var pending = false

    func value(for entry: AppleActivityEntry) -> AppleActivityDetail? {
        owner == AppleActivityReadOwner(entry) ? result : nil
    }
    func error(for entry: AppleActivityEntry) -> String? {
        owner == AppleActivityReadOwner(entry) ? failure : nil
    }
    func isLoading(_ entry: AppleActivityEntry) -> Bool {
        owner != AppleActivityReadOwner(entry) || pending
    }
    func load(_ entry: AppleActivityEntry,
        using fetch: @MainActor (AppleActivityEntry) async throws -> AppleActivityDetail?) async {
        let target = AppleActivityReadOwner(entry)
        generation &+= 1
        let ticket = generation
        if owner != target { result = nil }
        owner = target; pending = true; failure = nil
        defer { if generation == ticket { pending = false } }
        do {
            let value = try await fetch(entry)
            try Task.checkCancellation()
            guard generation == ticket else { return }
            guard value.map({ AppleActivityReadOwner($0.entry) == target }) ?? true else {
                failure = String(localized: "activity.unavailable")
                return
            }
            result = value
        } catch is CancellationError { }
        catch { if generation == ticket { failure = error.localizedDescription } }
    }
}

struct AppleActivityView: View {
    let load: @MainActor (AppleActivityQuery) async throws -> AppleActivityPage
    let detail: @MainActor (AppleActivityEntry) async throws -> AppleActivityDetail?
    let openSource: (String) -> Void
    let openDetail: ((AppleActivityEntry) -> Void)?
    let canOpenSource: (String) -> Bool
    @State private var entries: [AppleActivityEntry] = []
    @State private var cursor: AppleActivityCursor?
    @State private var source = "all"
    @State private var query = ""
    @State private var attention = false
    @State private var busy = false
    @State private var error: String?
    @State private var errorRequestKey: String?
    @State private var selected: AppleActivityEntry?
    @State private var loadedRequestKey: String?
    @State private var loadedAt: Date?
    @State private var loadGeneration: UInt64 = 0
    @State private var activeRequestKey: String?
    private var actionSize: CGFloat {
        #if os(macOS)
        36
        #else
        44
        #endif
    }
    private var requestKey: String { "\(source)|\(attention)|\(query)" }
    private var errorForCurrentQuery: String? { errorRequestKey == requestKey ? error : nil }
    private var waitingForCurrentQuery: Bool { loadedRequestKey != requestKey }
    init(source: String = "all", load: @escaping @MainActor (AppleActivityQuery) async throws -> AppleActivityPage,
        detail: @escaping @MainActor (AppleActivityEntry) async throws -> AppleActivityDetail?,
        openSource: @escaping (String) -> Void, openDetail: ((AppleActivityEntry) -> Void)? = nil,
        canOpenSource: @escaping (String) -> Bool = { _ in true }) {
        self.load = load; self.detail = detail; self.openSource = openSource; self.openDetail = openDetail
        self.canOpenSource = canOpenSource
        _source = State(initialValue: source)
    }
    var body: some View {
        VStack(spacing: 0) {
            VStack(alignment: .leading, spacing: 10) {
                HStack(spacing: 2) {
                    sourceButton("all", title: Text("activity.all"))
                    sourceButton("agent", title: Text("activity.agent"))
                    sourceButton("automation", title: Text("activity.plan"))
                }.padding(2).background(Color.primary.opacity(0.04), in: RoundedRectangle(cornerRadius: 8))
                    .accessibilityElement(children: .contain).accessibilityLabel(Text("activity.source"))
                ViewThatFits(in: .horizontal) {
                    HStack {
                        snapshotCount.fixedSize()
                        Spacer(minLength: 8)
                        Toggle("workspace.today.needsResponse", isOn: $attention).toggleStyle(.switch).fixedSize()
                    }
                    VStack(alignment: .leading, spacing: 8) {
                        snapshotCount
                        Toggle("workspace.today.needsResponse", isOn: $attention).toggleStyle(.switch)
                    }
                }.font(.subheadline)
            }.padding()
            Text("activity.localScope").font(.caption).foregroundStyle(.secondary).padding(.horizontal)
            if let error = errorForCurrentQuery {
                VStack(alignment: .leading, spacing: 6) {
                    Text(error).foregroundStyle(.secondary)
                    Button("common.retry") { requestReload(more: false) }.disabled(busy)
                }.padding()
            }
            HStack {
                TextField(text: $query, prompt: Text("activity.search").foregroundStyle(Color.secondary)) {
                    Text("activity.search")
                }.textFieldStyle(.roundedBorder)
                    .accessibilityIdentifier("activity.search")
                Button { requestReload(more: false) } label: {
                    Label("activity.refresh", systemImage: "arrow.clockwise").labelStyle(.iconOnly)
                        .frame(minWidth: actionSize, minHeight: actionSize)
                }.disabled(busy).help(Text("activity.refresh"))
                    .accessibilityIdentifier("activity.refresh")
            }.padding(.horizontal).padding(.vertical, 6)
            List {
                if displayEntries.isEmpty && !busy && !waitingForCurrentQuery && errorForCurrentQuery == nil {
                    Group {
                        if attention { Text("activity.emptyResponse").foregroundStyle(.secondary) }
                        else if !query.isEmpty { ContentUnavailableView.search(text: query) }
                        else { Text("information.activity.empty").foregroundStyle(.secondary) }
                    }.listRowBackground(Color.clear).listRowSeparator(.hidden)
                }
                ForEach(displayEntries) { entry in
                    Button { if let openDetail { openDetail(entry) } else { selected = entry } } label: {
                        VStack(alignment: .leading, spacing: 5) {
                            Text(entry.title.isEmpty ? String(localized: "chat.conversation.default_title") : entry.title)
                                .foregroundStyle(.primary).lineLimit(2)
                            HStack {
                                Text(LocalizedStringKey(entry.source == "agent" ? "activity.agent" : entry.source == "automation" ? "activity.plan" : "activity.conversation"))
                                Text("·").accessibilityHidden(true)
                                Text(LocalizedStringKey("activity.status." + entry.status))
                                Spacer()
                                Text(entry.date, format: .dateTime.month().day().hour().minute())
                            }.font(.caption).foregroundStyle(.secondary)
                            if entry.needs_response { Text("workspace.today.waiting").font(.caption) }
                        }.frame(maxWidth: .infinity, minHeight: actionSize, alignment: .leading).padding(.vertical, 4)
                            .contentShape(Rectangle())
                    }.buttonStyle(.plain).listRowBackground(Color.clear)
                        .accessibilityIdentifier("activity.run.\(entry.run_id)")
                }
                if cursor != nil && !waitingForCurrentQuery {
                    Button("activity.loadMore") { requestReload(more: true) }
                        .disabled(busy).accessibilityIdentifier("activity.loadMore")
                }
            }.listStyle(.plain).scrollContentBackground(.hidden)
                .overlay(alignment: .bottom) {
                    if busy || (waitingForCurrentQuery && errorForCurrentQuery == nil) {
                        ProgressView().padding(10).background(.regularMaterial, in: Capsule()).padding(8)
                            .accessibilityIdentifier("activity.loading")
                    }
                }
        }
        #if os(macOS)
        .background(MacDesignColor.canvas)
        #else
        .background(DesignTokens.backgroundColor)
        #endif
        .task(id: requestKey) {
            guard loadedRequestKey != requestKey || loadedAt.map({ Date().timeIntervalSince($0) >= 30 }) ?? true else { return }
            guard activeRequestKey != requestKey else { return }
            let scheduledGeneration = loadGeneration
            if loadedRequestKey != nil && loadedRequestKey != requestKey {
                do { try await Task.sleep(nanoseconds: 200_000_000) } catch { return }
            }
            // A manual refresh during the debounce already owns this query.
            guard scheduledGeneration == loadGeneration, activeRequestKey != requestKey else { return }
            await reload(more: false)
        }
        .sheet(item: $selected) { entry in
            NavigationStack {
                AppleActivityDetailView(entry: entry, load: { target in try await detail(target) }, openSource: openSource, canOpenSource: canOpenSource)
            }
        }
        .onDisappear {
            // Also invalidate unstructured refresh tasks queued by a button.
            loadGeneration &+= 1
            activeRequestKey = nil; busy = false
        }
    }
    @ViewBuilder private var snapshotCount: some View {
        if !waitingForCurrentQuery && !entries.isEmpty {
            HStack(spacing: 4) {
                Text(entries.count, format: .number).monospacedDigit()
                Text("activity.recentLoaded")
            }
        }
    }
    private func sourceButton(_ value: String, title: Text) -> some View {
        Button { source = value } label: {
            title.font(.callout).frame(maxWidth: .infinity, minHeight: actionSize)
                .background(source == value ? Color.primary.opacity(0.08) : .clear,
                    in: RoundedRectangle(cornerRadius: 6))
                .contentShape(Rectangle())
        }.buttonStyle(.plain).foregroundStyle(.primary)
            .accessibilityAddTraits(source == value ? .isSelected : [])
            .accessibilityIdentifier("activity.source." + value)
            .help(title)
    }
    private var displayEntries: [AppleActivityEntry] {
        guard !waitingForCurrentQuery else { return [] }
        #if os(iOS)
        return entries.filter(\.needs_response) + entries.filter { !$0.needs_response }
        #else
        return entries
        #endif
    }
    private func reload(more: Bool) async {
        let key = requestKey
        guard activeRequestKey != key else { return }
        guard !more || (!busy && !waitingForCurrentQuery && cursor != nil) else { return }
        loadGeneration &+= 1
        let generation = loadGeneration
        busy = true; activeRequestKey = key
        error = nil; errorRequestKey = nil
        defer { if generation == loadGeneration { busy = false; activeRequestKey = nil } }
        do {
            let page = try await load(.init(source: source == "all" ? nil : source,
                needs_response: attention, query: query, cursor: more ? cursor : nil, limit: 40))
            try Task.checkCancellation()
            guard key == requestKey, generation == loadGeneration else { return }
            entries = more ? entries + page.entries : page.entries
            cursor = page.next_cursor
            loadedRequestKey = key; loadedAt = Date()
            error = nil
        } catch is CancellationError { }
        catch { if key == requestKey, generation == loadGeneration {
            self.error = error.localizedDescription; errorRequestKey = key
        } }
    }
    private func requestReload(more: Bool) {
        let ticket = loadGeneration
        Task {
            guard ticket == loadGeneration else { return }
            await reload(more: more)
        }
    }
}
struct AppleActivityDetailView: View {
    let entry: AppleActivityEntry
    let load: @MainActor (AppleActivityEntry) async throws -> AppleActivityDetail?
    let openSource: (String) -> Void
    var canOpenSource: (String) -> Bool = { _ in true }
    var embedded = false
    @Environment(\.dismiss) private var dismiss
    @State private var reader = AppleActivityDetailReader()
    var body: some View {
        if embedded { content }
        else { content.navigationTitle(Text("information.activity")) }
    }
    private var content: some View {
        ScrollView {
        VStack(alignment: .leading, spacing: 20) {
            VStack(alignment: .leading, spacing: 6) {
                Text(entry.title).font(.title3.weight(.semibold))
                Text(entry.date, format: .dateTime.year().month().day().hour().minute())
                    .font(.caption).foregroundStyle(.secondary)
            }
            if reader.isLoading(entry) && reader.value(for: entry) == nil { ProgressView() }
            if let error = reader.error(for: entry) {
                Text(error).foregroundStyle(.secondary)
                Button("common.retry") { Task { await reload() } }.disabled(reader.isLoading(entry))
            }
            if let value = reader.value(for: entry) {
                VStack(alignment: .leading, spacing: 16) {
                VStack(alignment: .leading, spacing: 5) {
                    Text("activity.execution").font(.caption).foregroundStyle(.secondary)
                    Text(LocalizedStringKey("activity.status." + value.entry.status))
                    Text("activity.executionBoundary").font(.caption).foregroundStyle(.secondary)
                }
                VStack(alignment: .leading, spacing: 5) {
                    Text("activity.executionHost").font(.caption).foregroundStyle(.secondary)
                    if let host = value.execution_host_id {
                        Text(verbatim: host).textSelection(.enabled)
                    } else { Text("activity.hostUnknown").foregroundStyle(.secondary) }
                }
                VStack(alignment: .leading, spacing: 5) {
                    Text("activity.contextScope").font(.caption).foregroundStyle(.secondary)
                    Text(LocalizedStringKey("activity.context." + value.context_scope))
                        .font(.subheadline).foregroundStyle(.secondary)
                }
                }.frame(maxWidth: .infinity, alignment: .leading).padding(16)
                    .background(Color.primary.opacity(0.04), in: RoundedRectangle(cornerRadius: 10))
                if let result = value.result_text, !result.isEmpty {
                    VStack(alignment: .leading, spacing: 8) {
                        Text("activity.result").font(.headline)
                        #if os(macOS)
                        MacMarkdownView(markdown: result, isStreaming: false, animatesAppends: false)
                        #else
                        ChatSmoothStreamingMarkdownView(content: result, isStreaming: false)
                        #endif
                    }.textSelection(.enabled)
                }
                if value.result_has_more {
                    Text("activity.fullResultInSource").font(.caption).foregroundStyle(.secondary)
                }
                if let explanation = value.explanation { Text(explanation).foregroundStyle(.secondary).textSelection(.enabled) }
                if let source = value.source_conversation_id, canOpenSource(source) {
                    Button {
                        if !embedded { dismiss() }
                        openSource(source)
                    } label: { Text(LocalizedStringKey(value.review_in_source ? "activity.reviewSource" : "activity.openSource")) }
                } else { Text("activity.sourceUnavailable").foregroundStyle(.secondary) }
                if value.review_in_source, let source = value.source_conversation_id, canOpenSource(source) { Text("activity.sourceActions").font(.caption).foregroundStyle(.secondary) }
            } else if !reader.isLoading(entry) && reader.error(for: entry) == nil {
                Text("activity.unavailable").foregroundStyle(.secondary)
            }
        }
        .frame(maxWidth: 760, alignment: .leading)
        .padding(24)
        .frame(maxWidth: .infinity, alignment: .leading)
        }
        #if os(macOS)
        .background(MacDesignColor.canvas)
        #else
        .background(DesignTokens.backgroundColor)
        .navigationBarTitleDisplayMode(.inline)
        #endif
        .toolbar { if !embedded { Button("common.close") { dismiss() } } }
        .task(id: entry) { await reload() }
    }
    private func reload() async {
        await reader.load(entry, using: load)
    }
}
