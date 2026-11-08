// SPDX-License-Identifier: Apache-2.0
// Platform adapters supply only Core reads and the existing conversation route.
import SwiftUI
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
    @State private var selected: AppleActivityEntry?
    private var actionSize: CGFloat {
        #if os(macOS)
        36
        #else
        44
        #endif
    }
    private var requestKey: String { "\(source)|\(attention)|\(query)" }
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
                Toggle("workspace.today.needsResponse", isOn: $attention)
                    .toggleStyle(.switch)
            }.padding()
            Text("activity.localScope").font(.caption).foregroundStyle(.secondary).padding(.horizontal)
            if let error {
                VStack(alignment: .leading, spacing: 6) {
                    Text(error).foregroundStyle(.secondary)
                    Button("common.retry") { Task { await reload(more: false) } }.disabled(busy)
                }.padding()
            }
            HStack {
                TextField("activity.search", text: $query).textFieldStyle(.roundedBorder)
                Button { Task { await reload(more: false) } } label: {
                    Label("activity.refresh", systemImage: "arrow.clockwise").labelStyle(.iconOnly)
                        .frame(minWidth: actionSize, minHeight: actionSize)
                }.disabled(busy).help(Text("activity.refresh"))
            }.padding(.horizontal).padding(.vertical, 6)
            List {
                if entries.isEmpty && !busy && error == nil {
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
                        }.frame(maxWidth: .infinity, alignment: .leading).padding(.vertical, 4)
                            .contentShape(Rectangle())
                    }.buttonStyle(.plain).listRowBackground(Color.clear)
                        .accessibilityIdentifier("activity.run.\(entry.run_id)")
                }
                if cursor != nil {
                    Button("activity.loadMore") { Task { await reload(more: true) } }.disabled(busy)
                }
            }.listStyle(.plain).scrollContentBackground(.hidden)
            if busy { ProgressView().padding() }
        }
        #if os(macOS)
        .background(MacDesignColor.canvas)
        #else
        .background(DesignTokens.backgroundColor)
        #endif
        .task(id: requestKey) {
            busy = true; cursor = nil; error = nil
            do { try await Task.sleep(nanoseconds: 200_000_000) } catch { return }
            await reload(more: false)
        }
        .sheet(item: $selected) { entry in
            NavigationStack {
                AppleActivityDetailView(entry: entry, load: { target in try await detail(target) }, openSource: openSource, canOpenSource: canOpenSource)
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
            .help(title)
    }
    private var displayEntries: [AppleActivityEntry] {
        #if os(iOS)
        entries.filter(\.needs_response) + entries.filter { !$0.needs_response }
        #else
        entries
        #endif
    }
    private func reload(more: Bool) async {
        let key = requestKey
        busy = true
        error = nil
        if !more { cursor = nil }
        defer { if key == requestKey { busy = false } }
        do {
            let page = try await load(.init(source: source == "all" ? nil : source,
                needs_response: attention, query: query, cursor: more ? cursor : nil, limit: 40))
            try Task.checkCancellation()
            guard key == requestKey else { return }
            entries = more ? entries + page.entries : page.entries
            cursor = page.next_cursor
            error = nil
        } catch is CancellationError { }
        catch { if key == requestKey { self.error = error.localizedDescription } }
    }
}
struct AppleActivityDetailView: View {
    let entry: AppleActivityEntry
    let load: @MainActor (AppleActivityEntry) async throws -> AppleActivityDetail?
    let openSource: (String) -> Void
    var canOpenSource: (String) -> Bool = { _ in true }
    var embedded = false
    @Environment(\.dismiss) private var dismiss
    @State private var value: AppleActivityDetail?
    @State private var error: String?
    @State private var loading = true
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
            if loading { ProgressView() }
            else if let error {
                Text(error).foregroundStyle(.secondary)
                Button("common.retry") { Task { await reload() } }
            }
            else if let value {
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
            } else { Text("activity.unavailable").foregroundStyle(.secondary) }
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
        .task { await reload() }
    }
    private func reload() async {
        loading = true; error = nil
        defer { loading = false }
        do { value = try await load(entry) }
        catch is CancellationError { }
        catch { self.error = error.localizedDescription }
    }
}
