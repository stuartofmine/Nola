// SPDX-License-Identifier: Apache-2.0
// A historical detail read has no authority to update the live transcript.
import SwiftUI
import Combine

enum IOSHistoricalDetailPresentation {
    static func detailTitle(for kind: String) -> String {
        let key: String
        switch kind {
        case "text": key = "history.detail.open_text"
        case "reasoning": key = "history.detail.open_reasoning"
        case "tool": key = "history.detail.open_tool"
        case "attachment": key = "history.detail.open_attachment"
        default: key = "history.detail.open_record"
        }
        return NSLocalizedString(key, comment: "Read a historical message part")
    }

    static func status(_ value: IOSConversationProjectionV1.HistoricalProjection.Status.State) -> String {
        switch value {
        case .supported: return String(localized: "history.detail.available", defaultValue: "Available")
        case .deferred: return String(localized: "history.detail.partial", defaultValue: "Partially loaded")
        case .notLoaded: return String(localized: "history.detail.not_loaded", defaultValue: "Not loaded")
        case .unsupported: return String(localized: "history.detail.unsupported", defaultValue: "Unrecognized historical format")
        }
    }

    static func fieldTitle(_ name: String) -> String {
        switch name {
        case "text", "markdown": return String(localized: "history.detail.content", defaultValue: "Content")
        case "detail", "summary": return String(localized: "history.detail.summary", defaultValue: "Summary")
        case "input", "arguments": return String(localized: "history.detail.input", defaultValue: "Input")
        case "output", "result": return String(localized: "history.detail.output", defaultValue: "Output")
        case "artifacts": return String(localized: "history.detail.files", defaultValue: "Files")
        case "title", "name": return String(localized: "history.detail.name", defaultValue: "Name")
        case "state": return String(localized: "history.detail.state", defaultValue: "Status")
        default: return name
        }
    }

    static func formatted(_ value: AgentCoreJSONValue) -> String {
        if case .string(let text) = value { return text }
        let encoder = JSONEncoder()
        encoder.outputFormatting = [.prettyPrinted, .sortedKeys, .withoutEscapingSlashes]
        guard let data = try? encoder.encode(value), let text = String(data: data, encoding: .utf8) else {
            return value.displayText
        }
        return text
    }

    static func mainField(_ value: IOSConversationProjectionV1.HistoricalProjection) -> String? {
        guard value.decodedText != nil else { return nil }
        switch (value.schema, value.canonicalKind) {
        case ("agent_part", "text"): return "text"
        case ("message_part_view", "text"): return "markdown"
        case ("agent_part", "reasoning"): return "detail"
        case ("message_part_view", "reasoning"): return "summary"
        default: return nil
        }
    }
}

@MainActor
final class IOSHistoricalPartDetailState: ObservableObject {
    enum Phase: Equatable {
        case loading
        case loaded(IOSConversationProjectionV1.HistoricalProjection)
        case failed(String)
    }
    @Published private(set) var phase: Phase = .loading
    @Published private(set) var previous: IOSConversationProjectionV1.HistoricalProjection?
    private var generation = UUID()
    func invalidate() { generation = UUID() }
    func load(_ request: IOSHistoricalPartRequest,
              reader: @escaping @Sendable (IOSHistoricalPartRequest) async throws -> IOSConversationProjectionV1.HistoricalProjection?) async {
        let token = UUID()
        generation = token
        if let previous, IOSHistoricalPartRequest(previous) != request { self.previous = nil }
        phase = .loading
        do {
            guard let result = try await reader(request) else {
                throw IOSApplicationRuntimeError.rejected(String(localized: "Historical content is unavailable."))
            }
            try Task.checkCancellation()
            guard generation == token else { return }
            guard IOSHistoricalPartRequest(result) == request else {
                phase = .failed(String(localized: "Historical content changed. Reopen its details."))
                return
            }
            previous = result
            phase = .loaded(result)
        } catch is CancellationError {
            return
        } catch {
            guard generation == token, !Task.isCancelled else { return }
            phase = .failed(error.localizedDescription)
        }
    }
}

struct IOSHistoricalPartDetail: View {
    let request: IOSHistoricalPartRequest
    @StateObject private var state = IOSHistoricalPartDetailState()
    @State private var attempt = 0
    @Environment(\.dismiss) private var dismiss
    var body: some View {
        NavigationStack {
            ScrollView {
                VStack(alignment: .leading, spacing: 12) {
                    switch state.phase {
                    case .loading:
                        ProgressView(String(localized: "Loading historical details…"))
                        if let previous = state.previous { content(previous) }
                    case .failed(let message):
                        Text(message).foregroundStyle(.secondary)
                        Button(String(localized: "Retry")) { attempt += 1 }
                        if let previous = state.previous { content(previous) }
                    case .loaded(let projection):
                        content(projection)
                        Button(String(localized: "Reload")) { attempt += 1 }
                    }
                }.frame(maxWidth: .infinity, alignment: .leading).padding()
            }
            .navigationTitle(String(localized: "Historical details"))
            .toolbar { ToolbarItem(placement: .confirmationAction) { Button(String(localized: "Done")) { dismiss() } } }
        }
        .task(id: attempt) {
            await state.load(request) { request in
                try await Task.detached(priority: .userInitiated) {
                    guard let runtime = IOSApplicationRuntime.shared else { throw IOSApplicationRuntimeError.applicationUnavailable }
                    return try runtime.readHistoricalPart(owner: request.owner, revision: request.revision)
                }.value
            }
        }
        .onDisappear { state.invalidate() }
    }

    @ViewBuilder
    private func content(_ projection: IOSConversationProjectionV1.HistoricalProjection) -> some View {
        Text(IOSHistoricalDetailPresentation.status(projection.status.state))
            .font(.subheadline).foregroundStyle(.secondary)
        if let text = projection.decodedText ?? projection.canonicalText?.text, !text.isEmpty {
            ChatSmoothStreamingMarkdownView(content: text, isStreaming: false)
                .textSelection(.enabled)
            if projection.decodedText == nil, let preview = projection.canonicalText,
               UInt64(preview.text.utf8.count) < preview.totalUTF8Bytes {
                Text(String(localized: "history.detail.preview_only", defaultValue: "This is a preview, not the complete original content."))
                    .font(.caption).foregroundStyle(.secondary)
            }
        }
        if projection.status.state == .deferred || projection.status.state == .notLoaded {
            Text(String(localized: "history.detail.limit", defaultValue: "This detail preview is limited to 32 KB. Open the original record to read its raw content page by page."))
                .font(.callout).foregroundStyle(.secondary)
        }
        if projection.source != nil {
            NavigationLink(String(localized: "history.detail.open_original", defaultValue: "Read original record")) {
                IOSHistoricalOriginalReader(request: request)
            }
        }
        ForEach(projection.fields.keys.sorted().filter { $0 != IOSHistoricalDetailPresentation.mainField(projection) }, id: \.self) { name in
            VStack(alignment: .leading, spacing: 6) {
                Text(IOSHistoricalDetailPresentation.fieldTitle(name)).font(.subheadline.bold())
                switch projection.fields[name] {
                case .complete(let value):
                    Text(IOSHistoricalDetailPresentation.formatted(value))
                        .font(.body.monospaced()).textSelection(.enabled)
                case .deferred:
                    Text(String(localized: "Original field not loaded")).foregroundStyle(.secondary)
                case nil: EmptyView()
                }
            }
            .frame(maxWidth: .infinity, alignment: .leading).padding(12)
            .background(Color(.secondarySystemBackground), in: RoundedRectangle(cornerRadius: 12))
        }
        DisclosureGroup(String(localized: "history.detail.diagnostics", defaultValue: "Technical details")) {
            Text([projection.canonicalKind, projection.schema, projection.status.code, projection.status.reason].compactMap { $0 }.joined(separator: " · "))
                .font(.caption.monospaced()).textSelection(.enabled)
        }
        .disclosureGroupStyle(NolaRowDisclosureStyle())
    }
}

@MainActor
final class IOSHistoricalOriginalState: ObservableObject {
    @Published private(set) var page: IOSHistoricalOriginalPage?
    @Published private(set) var loading = false
    @Published private(set) var error: String?
    private(set) var previousOffsets: [UInt64] = []
    private var owner: IOSHistoricalPartRequest?
    private var generation = UUID()
    func invalidate() { generation = UUID() }

    func load(_ request: IOSHistoricalPartRequest, offset: UInt64,
              reader: @escaping @Sendable (IOSHistoricalOriginalQuery) async throws -> IOSHistoricalOriginalPage) async {
        let token = UUID()
        generation = token
        if owner != request {
            owner = request
            page = nil
            previousOffsets = []
        }
        let query = IOSHistoricalOriginalQuery(request, offset: offset)
        loading = true
        error = nil
        defer { if generation == token { loading = false } }
        do {
            let result = try await reader(query)
            try Task.checkCancellation()
            guard generation == token else { return }
            guard result.matches(query), page == nil || page?.totalUTF8Bytes == result.totalUTF8Bytes else {
                throw IOSApplicationRuntimeError.rejected(String(localized: "Historical content changed. Reopen its details."))
            }
            if let page, page.offsetBytes != result.offsetBytes {
                if let index = previousOffsets.firstIndex(of: result.offsetBytes) {
                    previousOffsets.removeSubrange(index...)
                } else {
                    previousOffsets.append(page.offsetBytes)
                }
            }
            page = result
        } catch is CancellationError {
            return
        } catch {
            guard generation == token, !Task.isCancelled else { return }
            self.error = error.localizedDescription
        }
    }
}

private struct IOSHistoricalOriginalReader: View {
    let request: IOSHistoricalPartRequest
    @StateObject private var state = IOSHistoricalOriginalState()
    @State private var offset: UInt64 = 0
    @State private var attempt = 0

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            Text(String(localized: "history.original.explanation", defaultValue: "Read-only original record. Each page contains up to 8 KB; selecting or copying text includes only this page, not the complete record."))
                .font(.caption).foregroundStyle(.secondary)
            if state.loading { ProgressView() }
            if let error = state.error {
                Text(error).font(.callout).foregroundStyle(.secondary)
                Button(String(localized: "Retry")) { attempt += 1 }
            }
            ScrollView {
                Text(state.page.map { $0.text.isEmpty && $0.endOfContent ? String(localized: "history.original.empty", defaultValue: "The original record is empty.") : $0.text } ?? "")
                    .font(.body.monospaced()).textSelection(.enabled)
                    .frame(maxWidth: .infinity, alignment: .leading)
            }
            .id(state.page?.offsetBytes)
            if let page = state.page {
                Text(String(localized: "history.original.range", defaultValue: "Bytes \(page.offsetBytes)–\(page.nextOffsetBytes) of \(page.totalUTF8Bytes)"))
                    .font(.caption).foregroundStyle(.secondary)
                HStack {
                    Button(String(localized: "history.original.previous", defaultValue: "Previous page")) {
                        if let previous = state.previousOffsets.last { offset = previous; attempt += 1 }
                    }.disabled(state.loading || state.previousOffsets.isEmpty)
                    Spacer()
                    Button(String(localized: "history.original.next", defaultValue: "Next page")) {
                        offset = page.nextOffsetBytes
                        attempt += 1
                    }.disabled(state.loading || page.endOfContent)
                }
            }
        }
        .padding()
        .navigationTitle(String(localized: "history.detail.open_original", defaultValue: "Read original record"))
        .navigationBarTitleDisplayMode(.inline)
        .task(id: attempt) {
            await state.load(request, offset: offset) { query in
                try await Task.detached(priority: .userInitiated) {
                    guard let runtime = IOSApplicationRuntime.shared else { throw IOSApplicationRuntimeError.applicationUnavailable }
                    return try runtime.readHistoricalOriginal(query)
                }.value
            }
        }
        .onDisappear { state.invalidate() }
    }
}
