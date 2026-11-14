// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: User-visible, provenance-aware management for Agent Host long-term memory.

import SwiftUI

private func macMemoryText(_ key: String, _ fallback: String) -> String {
    let keyed = Bundle.main.localizedString(forKey: key, value: nil, table: nil)
    if keyed != key { return keyed }
    return Bundle.main.localizedString(forKey: fallback, value: fallback, table: nil)
}

private func macMemoryKindTitle(_ kind: String) -> String {
    switch kind {
    case "preference": return macMemoryText("mac.memory.kind.preference", "Preference")
    case "identity": return macMemoryText("mac.memory.kind.identity", "Identity")
    case "decision": return macMemoryText("mac.memory.kind.decision", "Decision")
    case "project": return macMemoryText("mac.memory.kind.project", "Project")
    case "feedback": return macMemoryText("mac.memory.kind.feedback", "Feedback")
    case "reference": return macMemoryText("mac.memory.kind.reference", "Reference")
    case "other": return macMemoryText("mac.memory.kind.other", "Other")
    default: return kind.capitalized
    }
}

struct MacMemoryCenterPage: View {
    var body: some View {
        ScrollView {
            MacMemoryCenterView()
                .padding(.horizontal, MacDesignTokens.Spacing.xxl)
                .padding(.vertical, MacDesignTokens.Spacing.xl)
                .frame(maxWidth: MacDesignTokens.Layout.contentWidth)
                .frame(maxWidth: .infinity)
        }
        .background(MacPageBackground())
    }
}

struct MacMemoryCenterView: View {
    @State private var records: [MacAgentHostMemoryRecord] = []
    @State private var query = ""
    @State private var isLoading = false
    @State private var errorMessage: String?
    @State private var notice: String?
    @State private var editorRecord: MacAgentHostMemoryRecord?
    @State private var isPresentingEditor = false
    @State private var pendingDeletion: MacAgentHostMemoryRecord?
    @State private var isSaving = false
    @State private var editorError: String?
    @State private var isDeleting = false

    private var supersededIDs: Set<String> {
        Set(records.flatMap(\.supersedes))
    }

    private var filteredRecords: [MacAgentHostMemoryRecord] {
        let needle = query.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !needle.isEmpty else { return records }
        return records.filter { record in
            [record.content, record.summary ?? "", record.scope, record.kind, record.source ?? ""]
                .contains { $0.localizedCaseInsensitiveContains(needle) }
        }
    }

    var body: some View {
        LazyVStack(alignment: .leading, spacing: 14) {
            HStack(alignment: .center, spacing: 12) {
                VStack(alignment: .leading, spacing: 4) {
                    MacSectionLabel(LocalizedStringKey(macMemoryText("mac.memory.title", "Long-term Memory")))
                    Text(macMemoryText("mac.memory.subtitle", "Review what Nola carries across conversations, where it came from, and which version is active."))
                        .font(.system(size: 11.5))
                        .foregroundStyle(.secondary)
                }
                Spacer()
                Button {
                    editorRecord = nil
                    editorError = nil
                    isPresentingEditor = true
                } label: {
                    Label(macMemoryText("mac.memory.new", "New Memory"), systemImage: "plus")
                }
                .buttonStyle(.borderedProminent)
                .controlSize(.small)
            }

            HStack(spacing: 10) {
                Image(systemName: "magnifyingglass")
                    .foregroundStyle(.tertiary)
                TextField(macMemoryText("mac.memory.search", "Search memory"), text: $query)
                    .textFieldStyle(.plain)
                if !query.isEmpty {
                    Button { query = "" } label: {
                        Image(systemName: "xmark.circle.fill")
                    }
                    .macToolbarIcon()
                    .foregroundStyle(.tertiary)
                    .help(macMemoryText("mac.memory.search.clear", "Clear Search"))
                    .accessibilityLabel(macMemoryText("mac.memory.search.clear", "Clear Search"))
                }
            }
            .padding(.horizontal, 11)
            .frame(height: 34)
            .background(
                Color.primary.opacity(0.045),
                in: RoundedRectangle(cornerRadius: MacDesignTokens.Radius.field, style: .continuous)
            )
            .overlay {
                RoundedRectangle(cornerRadius: MacDesignTokens.Radius.field, style: .continuous)
                    .stroke(MacDesignColor.borderSubtle, lineWidth: 0.5)
            }

            if let notice {
                MacInlineNotice(message: notice, kind: .success, dismiss: {
                    self.notice = nil
                })
                    .transition(.opacity)
            }

            if let errorMessage, !records.isEmpty {
                MacInlineNotice(message: errorMessage, kind: .failure, dismiss: {
                    self.errorMessage = nil
                })
            }

            if isLoading && records.isEmpty {
                ProgressView(macMemoryText("mac.memory.loading", "Loading memory…"))
                    .frame(maxWidth: .infinity, minHeight: 220)
            } else if let errorMessage, records.isEmpty {
                MacEmptyState(
                    LocalizedStringKey(macMemoryText("mac.memory.unavailable", "Memory is unavailable")),
                    description: LocalizedStringKey(errorMessage),
                    symbol: "exclamationmark.triangle"
                ) {
                    Button(macMemoryText("mac.memory.retry", "Retry")) { Task { await reload() } }
                }
                .frame(minHeight: 240)
            } else if filteredRecords.isEmpty && !query.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
                ContentUnavailableView.search(text: query)
                    .frame(minHeight: 260)
            } else if filteredRecords.isEmpty {
                MacEmptyState(
                    LocalizedStringKey(macMemoryText("mac.memory.empty", "No memories yet")),
                    description: LocalizedStringKey(macMemoryText("mac.memory.empty.detail", "Durable preferences and decisions saved by you or the agent will appear here.")),
                    symbol: "brain.head.profile"
                ) {
                    Button(macMemoryText("mac.memory.new", "New Memory")) {
                        editorRecord = nil
                        editorError = nil
                        isPresentingEditor = true
                    }
                }
                .frame(minHeight: 260)
            } else {
                ForEach(filteredRecords) { record in
                    memoryCard(record, isSuperseded: supersededIDs.contains(record.id))
                }
            }
        }
        .task { await reload() }
        .sheet(isPresented: $isPresentingEditor) {
            MacMemoryEditor(record: editorRecord, isSaving: isSaving, errorMessage: editorError) { draft in
                guard !isSaving else { return }
                isSaving = true
                editorError = nil
                Task { await save(draft, replacing: editorRecord) }
            }
        }
        .confirmationDialog(
            macMemoryText("mac.memory.delete.title", "Delete this memory?"),
            isPresented: Binding(
                get: { pendingDeletion != nil },
                set: { if !$0 { pendingDeletion = nil } }
            )
        ) {
            Button(macMemoryText("mac.memory.delete", "Delete Memory"), role: .destructive) {
                guard let record = pendingDeletion else { return }
                guard !isDeleting else { return }
                isDeleting = true
                Task { await delete(record) }
            }
            Button(macMemoryText("mac.memory.cancel", "Cancel"), role: .cancel) {}
        } message: {
            Text(macMemoryText("mac.memory.delete.detail", "Deleting the active revision may restore the previous version."))
        }
    }

    private func memoryCard(_ record: MacAgentHostMemoryRecord, isSuperseded: Bool) -> some View {
        VStack(alignment: .leading, spacing: 10) {
            HStack(spacing: 7) {
                Label(scopeTitle(record.scope), systemImage: scopeSymbol(record.scope))
                    .font(.system(size: 10.5, weight: .semibold))
                    .foregroundStyle(.blue)
                Text(kindTitle(record.kind))
                    .font(.system(size: 10.5, weight: .medium))
                    .foregroundStyle(.secondary)
                if isSuperseded {
                    MacStatusBadge(
                        title: macMemoryText("mac.memory.superseded", "Superseded"),
                        tint: .orange,
                        symbol: "clock.arrow.circlepath"
                    )
                }
                Spacer()
                Menu {
                    Button(macMemoryText("mac.memory.revise", "Revise"), systemImage: "pencil") {
                        editorRecord = record
                        editorError = nil
                        isPresentingEditor = true
                    }
                    Divider()
                    Button(macMemoryText("mac.memory.delete", "Delete Memory"), systemImage: "trash", role: .destructive) {
                        pendingDeletion = record
                    }
                } label: {
                    Image(systemName: "ellipsis").frame(width: 24, height: 24)
                }
                .menuStyle(.borderlessButton)
                .menuIndicator(.hidden)
                .fixedSize()
                .macToolbarIcon()
                .help(macMemoryText("mac.memory.more", "More"))
                .accessibilityLabel(macMemoryText("mac.memory.more", "More"))
                .disabled(isDeleting)
            }

            Text(record.content)
                .font(.system(size: 13))
                .foregroundStyle(isSuperseded ? .secondary : .primary)
                .textSelection(.enabled)
                .fixedSize(horizontal: false, vertical: true)

            HStack(spacing: 12) {
                Label(sourceTitle(record), systemImage: "arrow.triangle.branch")
                    .lineLimit(1)
                Label(
                    String(format: macMemoryText("mac.memory.confidence", "%.0f%% confidence"), record.confidence * 100),
                    systemImage: "checkmark.seal"
                )
                Spacer()
                Text(updatedAtTitle(record.updatedAt))
                    .lineLimit(1)
            }
            .font(.system(size: 11))
            .foregroundStyle(.secondary)
        }
        .padding(14)
        .macPanel(cornerRadius: 13)
        .opacity(isSuperseded ? 0.72 : 1)
    }

    private func reload() async {
        isLoading = true
        defer { isLoading = false }
        do {
            records = try await MacAgentHostRuntime.shared.listMemoryRecords()
            errorMessage = nil
        } catch {
            errorMessage = error.localizedDescription
        }
    }

    private func save(_ draft: MacAgentHostMemoryDraft, replacing record: MacAgentHostMemoryRecord?) async {
        defer { isSaving = false }
        do {
            if let record {
                _ = try await MacAgentHostRuntime.shared.reviseMemory(id: record.id, draft: draft)
                notice = macMemoryText("mac.memory.revised", "Memory revised")
            } else {
                _ = try await MacAgentHostRuntime.shared.createMemory(draft)
                notice = macMemoryText("mac.memory.saved", "Memory saved")
            }
            isPresentingEditor = false
            await reload()
        } catch {
            editorError = error.localizedDescription
        }
    }

    private func delete(_ record: MacAgentHostMemoryRecord) async {
        defer { isDeleting = false }
        pendingDeletion = nil
        do {
            try await MacAgentHostRuntime.shared.deleteMemory(id: record.id)
            notice = macMemoryText("mac.memory.deleted", "Memory deleted")
            await reload()
        } catch {
            errorMessage = error.localizedDescription
        }
    }

    private func sourceTitle(_ record: MacAgentHostMemoryRecord) -> String {
        let source = record.provenance.source ?? record.source
        if source == "explicit_user_memory_management" { return String(localized: "Manual") }
        return source ?? macMemoryText("mac.memory.source.unknown", "Unknown source")
    }

    private func updatedAtTitle(_ value: String) -> String {
        guard let date = try? Date(value, strategy: Date.ISO8601FormatStyle(includingFractionalSeconds: true)) else { return value }
        return date.formatted(date: .abbreviated, time: .shortened)
    }

    private func scopeTitle(_ scope: String) -> String {
        switch scope {
        case "workspace": return macMemoryText("mac.memory.scope.workspace", "Workspace")
        case "conversation": return macMemoryText("mac.memory.scope.conversation", "Conversation")
        case "turn": return macMemoryText("mac.memory.scope.turn", "Turn")
        default: return macMemoryText("mac.memory.scope.global", "Global")
        }
    }

    private func scopeSymbol(_ scope: String) -> String {
        switch scope {
        case "workspace": return "folder"
        case "conversation", "turn": return "bubble.left.and.bubble.right"
        default: return "person.crop.circle"
        }
    }

    private func kindTitle(_ kind: String) -> String {
        macMemoryKindTitle(kind)
    }
}

private struct MacMemoryEditor: View {
    @Environment(\.dismiss) private var dismiss
    let record: MacAgentHostMemoryRecord?
    let onSave: (MacAgentHostMemoryDraft) -> Void

    @State private var content: String
    @State private var summary: String
    @State private var scope: String
    @State private var kind: String
    @State private var tags: String
    @State private var confidence: Double
    @State private var showsDiscardConfirmation = false
    @State private var allowsDiscardClose = false

    let isSaving: Bool
    let errorMessage: String?

    init(record: MacAgentHostMemoryRecord?, isSaving: Bool, errorMessage: String?, onSave: @escaping (MacAgentHostMemoryDraft) -> Void) {
        self.record = record
        self.isSaving = isSaving
        self.errorMessage = errorMessage
        self.onSave = onSave
        _content = State(initialValue: record?.content ?? "")
        _summary = State(initialValue: record?.summary ?? "")
        _scope = State(initialValue: record?.scope ?? "global")
        _kind = State(initialValue: record?.kind ?? "preference")
        _tags = State(initialValue: record?.tags.joined(separator: ", ") ?? "")
        _confidence = State(initialValue: record?.confidence ?? 1)
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 16) {
            HStack(alignment: .top, spacing: 12) {
                MacIconTile(
                    symbol: record == nil ? "brain.head.profile" : "clock.arrow.circlepath",
                    tint: MacDesignColor.brand
                )
                VStack(alignment: .leading, spacing: 3) {
                    Text(record == nil
                         ? macMemoryText("mac.memory.new", "New Memory")
                         : macMemoryText("mac.memory.revise", "Revise"))
                        .font(.system(size: 19, weight: .semibold))
                    Text(macMemoryText("mac.memory.revision.detail", "Revisions preserve the previous version and its provenance."))
                        .font(.system(size: 11.5))
                        .foregroundStyle(.secondary)
                }
            }

            VStack(alignment: .leading, spacing: 7) {
                Text(macMemoryText("mac.memory.content", "Memory content"))
                    .font(.system(size: 11.5, weight: .semibold))
                TextEditor(text: $content)
                    .font(.system(size: 13))
                    .frame(minHeight: 150)
                    .padding(8)
                    .background(Color(nsColor: .textBackgroundColor), in: RoundedRectangle(cornerRadius: 9))
                    .overlay {
                        RoundedRectangle(cornerRadius: 9, style: .continuous)
                            .strokeBorder(Color.primary.opacity(0.09))
                    }
            }

            TextField(macMemoryText("mac.memory.summary", "Optional summary"), text: $summary)
                .textFieldStyle(.roundedBorder)

            ViewThatFits(in: .horizontal) {
                HStack(spacing: 12) {
                    scopePicker
                    kindPicker
                }
                VStack(alignment: .leading, spacing: 10) {
                    scopePicker
                    kindPicker
                }
            }

            TextField(macMemoryText("mac.memory.tags", "Tags, separated by commas"), text: $tags)
                .textFieldStyle(.roundedBorder)

            HStack {
                Text(macMemoryText("mac.memory.confidence.label", "Confidence"))
                Slider(value: $confidence, in: 0...1, step: 0.05)
                Text(confidence, format: .percent.precision(.fractionLength(0)))
                    .monospacedDigit()
                    .frame(width: 42, alignment: .trailing)
            }
            .font(.system(size: 11.5))

            if let errorMessage {
                MacInlineNotice(message: errorMessage, kind: .failure)
            }

            HStack {
                if isSaving { ProgressView().controlSize(.small) }
                Spacer()
                Button(macMemoryText("mac.memory.cancel", "Cancel"), action: requestDismiss)
                    .keyboardShortcut(.cancelAction)
                    .disabled(isSaving)
                Button(macMemoryText("mac.memory.save", "Save")) {
                    let parsedTags = tags.split(separator: ",")
                        .map { $0.trimmingCharacters(in: .whitespacesAndNewlines) }
                        .filter { !$0.isEmpty }
                    let trimmedSummary = summary.trimmingCharacters(in: .whitespacesAndNewlines)
                    onSave(MacAgentHostMemoryDraft(
                        scope: scope,
                        kind: kind,
                        content: content.trimmingCharacters(in: .whitespacesAndNewlines),
                        summary: trimmedSummary.isEmpty ? nil : trimmedSummary,
                        tags: parsedTags,
                        confidence: confidence,
                        importance: record?.importance ?? 0.5,
                        sensitivity: record?.sensitivity,
                        expiresAt: record?.expiresAt
                    ))
                }
                .keyboardShortcut(.defaultAction)
                .buttonStyle(.borderedProminent)
                .disabled(isSaving || content.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty)
            }
        }
        .padding(22)
        .disabled(isSaving)
        .frame(minWidth: 440, idealWidth: 560, maxWidth: 680)
        .frame(minHeight: 500, idealHeight: 580, maxHeight: 720)
        .interactiveDismissDisabled(hasUnsavedChanges || isSaving)
        .background(MacEditorWindowCloseGuard(
            shouldPreventClose: { (hasUnsavedChanges || isSaving) && !allowsDiscardClose },
            onCloseAttempt: { if !isSaving { requestDismiss() } }
        ))
        .confirmationDialog(
            macMemoryText("mac.memory.discard.title", "Discard unsaved changes?"),
            isPresented: $showsDiscardConfirmation,
            titleVisibility: .visible
        ) {
            Button(macMemoryText("mac.memory.discard", "Discard Changes"), role: .destructive) { allowsDiscardClose = true; dismiss() }
            Button(macMemoryText("mac.memory.cancel", "Cancel"), role: .cancel) {}
        }
    }

    private var scopePicker: some View {
        Picker(macMemoryText("mac.memory.scope", "Scope"), selection: $scope) {
            Text(macMemoryText("mac.memory.scope.global", "Global")).tag("global")
            Text(macMemoryText("mac.memory.scope.workspace", "Workspace")).tag("workspace")
            Text(macMemoryText("mac.memory.scope.conversation", "Conversation")).tag("conversation")
        }
        .frame(maxWidth: .infinity)
    }

    private var kindPicker: some View {
        Picker(macMemoryText("mac.memory.kind", "Kind"), selection: $kind) {
            ForEach(["preference", "identity", "decision", "project", "feedback", "reference", "other"], id: \.self) { value in
                Text(macMemoryKindTitle(value)).tag(value)
            }
        }
        .frame(maxWidth: .infinity)
    }

    private var hasUnsavedChanges: Bool {
        content != (record?.content ?? "")
            || summary != (record?.summary ?? "")
            || scope != (record?.scope ?? "global")
            || kind != (record?.kind ?? "preference")
            || tags != (record?.tags.joined(separator: ", ") ?? "")
            || confidence != (record?.confidence ?? 1)
    }

    private func requestDismiss() {
        guard !isSaving else { return }
        if hasUnsavedChanges {
            showsDiscardConfirmation = true
        } else {
            dismiss()
        }
    }
}
