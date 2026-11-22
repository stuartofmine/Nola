// SPDX-License-Identifier: Apache-2.0
// Shared Apple composer presentation. Selection carries IDs, never name-based routing.
import Foundation
import SwiftUI

nonisolated struct ExpertMentionQuery: Equatable, Sendable {
    let range: NSRange
    let query: String

    private static let tokenExpression = try! NSRegularExpression(pattern: "(?<![A-Za-z0-9._%+@-])@([^\\s@，。！？、；：]*)$")

    static func atEnd(of text: String) -> Self? {
        atCaret(in: text, selection: NSRange(location: text.utf16.count, length: 0))
    }

    static func atCaret(in text: String, selection: NSRange, mentions: [AppleDraftMention] = []) -> Self? {
        let value = text as NSString
        guard selection.length == 0, selection.location >= 0, selection.location <= value.length else { return nil }
        let prefix = value.substring(to: selection.location) as NSString
        // ASCII word characters immediately before @ belong to addresses or identifiers.
        guard let match = tokenExpression.firstMatch(
            in: prefix as String, range: NSRange(location: 0, length: prefix.length)),
              !mentions.contains(where: { NSIntersectionRange($0.range, match.range).length > 0 }) else { return nil }
        return Self(range: match.range, query: prefix.substring(with: match.range(at: 1)))
    }
    func removing(from text: String) -> String { (text as NSString).replacingCharacters(in: range, with: "") }
}

struct ExpertMentionCandidate: Identifiable {
    let id: String
    let name: String
    let detail: String

    static func ordered(_ candidates: [Self], members: Set<String>) -> [Self] {
        let repeated = Dictionary(grouping: candidates, by: { $0.name + "\n" + $0.detail })
        return candidates.sorted {
            if members.contains($0.id) != members.contains($1.id) { return members.contains($0.id) }
            let order = $0.name.localizedStandardCompare($1.name)
            return order == .orderedSame ? $0.id < $1.id : order == .orderedAscending
        }.map { candidate in
            guard let matches = repeated[candidate.name + "\n" + candidate.detail], matches.count > 1 else { return candidate }
            let distinguish = matches.map(\.id)
            var count = 4
            while count < candidate.id.count, distinguish.filter({ $0.suffix(count) == candidate.id.suffix(count) }).count > 1 { count += 1 }
            let detail = [candidate.detail, "#" + String(candidate.id.suffix(count))].filter { !$0.isEmpty }.joined(separator: " · ")
            return .init(id: candidate.id, name: candidate.name, detail: detail)
        }
    }
}

struct ExpertMentionPanel: View {
    let candidates: [ExpertMentionCandidate]
    let highlightedID: String?
    let error: String?
    let select: (String) -> Void
    var accessibilityTitle: LocalizedStringKey = "chat.mention.experts"
    var identifierPrefix = "chat.mention."
    var showsAvatar = true
    @Environment(\.dynamicTypeSize) private var dynamicTypeSize
    @ScaledMetric(relativeTo: .body) private var rowHeight: CGFloat = 56

    private var surfaceFill: Color {
        #if os(iOS)
        DesignTokens.surfaceColor
        #else
        Color("NolaSurface")
        #endif
    }

    private var surfaceBorder: Color {
        #if os(iOS)
        DesignTokens.borderColor
        #else
        Color("NolaDivider")
        #endif
    }

    private var maxPanelHeight: CGFloat {
        let cap: CGFloat = dynamicTypeSize.isAccessibilitySize ? 168 : 196
        return min(cap, CGFloat(max(1, candidates.count)) * rowHeight + 12)
    }

    var body: some View {
        ScrollViewReader { proxy in
            ScrollView {
                LazyVStack(spacing: 2) {
                    if let error { Text(error).font(.callout).foregroundStyle(.red).padding() }
                    else if candidates.isEmpty { Text("chat.mention.no_matches").font(.callout).foregroundStyle(.secondary).padding() }
                    ForEach(candidates) { candidate in
                        Button { select(candidate.id) } label: {
                            HStack(spacing: 12) {
                                if showsAvatar { Text(String(candidate.name.prefix(1)).uppercased())
                                    .font(.callout.weight(.semibold)).frame(width: 32, height: 32)
                                    .background(.secondary.opacity(0.08), in: Circle()) }
                                VStack(alignment: .leading, spacing: 3) {
                                    Text(candidate.name).font(.body).lineLimit(1)
                                    if !candidate.detail.isEmpty {
                                        Text(candidate.detail).font(.caption).foregroundStyle(.secondary).lineLimit(1)
                                    }
                                }
                                Spacer(minLength: 8)
                            }
                            .padding(.horizontal, 12).frame(minHeight: 52)
                            .contentShape(Rectangle())
                            .overlay(RoundedRectangle(cornerRadius: 10).stroke(
                                highlightedID == candidate.id ? Color.accentColor : .clear, lineWidth: 1))
                        }
                        .buttonStyle(.plain)
                        .accessibilityIdentifier(identifierPrefix + candidate.id)
                        .id(candidate.id)
                    }
                }.padding(6)
            }
            .frame(maxHeight: maxPanelHeight)
            .onChange(of: highlightedID) { _, id in if let id { proxy.scrollTo(id) } }
        }
        .background(surfaceFill, in: RoundedRectangle(cornerRadius: 14, style: .continuous))
        .overlay(RoundedRectangle(cornerRadius: 14, style: .continuous).stroke(surfaceBorder, lineWidth: 1))
        .accessibilityLabel(Text(accessibilityTitle))
    }
}

/// Native controls own IME and undo. These hooks publish completed editing operations with their original range.
struct AppleMentionEditor {
    var mentions: [AppleDraftMention] = []
    var caretRequest: Caret?
    var selectionChanged: (NSRange, Bool) -> Void = { _, _ in }
    var applyEdit: (String, NSRange, String) -> Bool = { _, _, _ in false }
    var restore: (String, [AppleDraftMention]) -> Void = { _, _ in }
    var pasted: (String) -> Void = { _ in }

    struct Caret: Equatable {
        let revision = UUID()
        let location: Int
        var before: Snapshot?
    }
    struct Snapshot: Equatable {
        let text: String
        let mentions: [AppleDraftMention]
        let caret: Int
    }

    func caretRange(_ range: NSRange, previous: NSRange?) -> NSRange {
        guard range.length == 0,
              let token = mentions.first(where: { $0.location < range.location && range.location < NSMaxRange($0.range) }) else { return range }
        let movingBack = previous.map { $0.location > range.location } ?? false
        return NSRange(location: movingBack ? token.location : NSMaxRange(token.range), length: 0)
    }
    func deletionRange(_ requested: NSRange, replacement: String) -> NSRange {
        guard replacement.isEmpty else { return requested }
        return mentions.reduce(requested) { range, mention in
            NSIntersectionRange(mention.range, range).length > 0 ? NSUnionRange(range, mention.range) : range
        }
    }
}

/// The platform undo manager replays text. Only a native undo/redo event may restore
/// its previous structured identities; identical text typed or pasted later stays plain.
@MainActor
final class AppleMentionUndoHistory {
    private struct Change {
        let before: String
        let after: String
        let removed: [AppleDraftMention]
        let inserted: [AppleDraftMention]
    }
    private var changes: [Change] = []

    func record(before: String, after: String, original: [AppleDraftMention], current: [AppleDraftMention]) {
        guard original != current else { return }
        changes.append(.init(before: before, after: after, removed: original, inserted: current))
        if changes.count > 24 { changes.removeFirst() }
    }

    func registerCompletion(_ manager: UndoManager?, before: AppleMentionEditor.Snapshot, after: AppleMentionEditor.Snapshot,
                            apply: @escaping (AppleMentionEditor.Snapshot, AppleMentionEditor.Snapshot) -> Bool) {
        manager?.registerUndo(withTarget: self) { [weak manager] history in
            guard apply(after, before) else { return }
            history.registerCompletion(manager, before: after, after: before, apply: apply)
        }
    }

    func restore(before: String, after: String, undo: UndoManager?, editor: AppleMentionEditor) -> Bool {
        if undo?.isUndoing == true, let change = changes.last(where: { $0.before == after && $0.after == before }) {
            editor.restore(after, change.removed)
            return true
        }
        if undo?.isRedoing == true, let change = changes.last(where: { $0.before == before && $0.after == after }) {
            editor.restore(after, change.inserted)
            return true
        }
        return false
    }
}

struct AppleMentionProfileActions {
    var description: (AppleMainExpertSelectionEntry) -> String? = { _ in nil }
    var history: ((AppleMainExpertSelectionEntry) -> Void)?
}
private struct AppleMentionProfileKey: EnvironmentKey {
    static let defaultValue = AppleMentionProfileActions()
}
extension EnvironmentValues {
    var expertMentionProfile: AppleMentionProfileActions {
        get { self[AppleMentionProfileKey.self] }
        set { self[AppleMentionProfileKey.self] = newValue }
    }
}

/// Links are generated exclusively from committed occurrence indexes and are resolved inside this view.
struct ExpertMentionText: View {
    let text: String
    let mentions: AppleExpertMentions?
    @Environment(\.expertMentionProfile) private var profile
    @State private var detail: Int?
    var body: some View {
        if let mentions, mentions.runs(in: text) != nil {
            Text(attributed(mentions)).tint(.accentColor)
                .environment(\.openURL, OpenURLAction { url in
                    guard url.scheme == "nola-mention", let key = url.host, let index = Int(key),
                          mentions.segments.indices.contains(index), mentions.segments[index].agentID != nil else { return .discarded }
                    detail = index
                    return .handled
                })
                .popover(isPresented: Binding(get: { detail != nil }, set: { if !$0 { detail = nil } })) {
                    if let detail, mentions.segments.indices.contains(detail), let id = mentions.segments[detail].agentID,
                       let expert = mentions.selectedExperts?.first(where: { $0.agentID == id }) {
                        VStack(alignment: .leading, spacing: 8) {
                            Text(verbatim: mentions.segments[detail].text).font(.headline)
                            if let description = profile.description(expert), !description.isEmpty { Text(verbatim: description).font(.callout).foregroundStyle(.secondary) }
                            if let open = profile.history, expert.contextConversationID != nil {
                                Button(String(localized: "View expert history")) { self.detail = nil; open(expert) }
                            }
                        }.padding().presentationCompactAdaptation(.popover)
                    }
                }
        } else { Text(verbatim: text) }
    }

    private func attributed(_ mentions: AppleExpertMentions) -> AttributedString {
        var result = AttributedString()
        for (index, segment) in mentions.segments.enumerated() {
            var value = AttributedString(segment.text)
            if segment.agentID != nil {
                value.inlinePresentationIntent = .stronglyEmphasized
                value.link = URL(string: "nola-mention://"+String(index))
            }
            result.append(value)
        }
        return result
    }
}

struct AppleExpertEditCandidate: Identifiable {
    let expert: AppleMainExpertSelectionEntry
    let name: String
    let detail: String
    var id: String { expert.agentID }
}

/// Uses the same native editor, cursor events and completion semantics in the resend sheet.
struct AppleExpertResendEditor: View {
    @Binding var draft: AppleMessageEditDraft
    let candidates: [AppleExpertEditCandidate]
    @FocusState private var focus: Bool
    @State private var selection: NSRange?
    @State private var composing = false
    @State private var caret: AppleMentionEditor.Caret?
    @State private var dismissed: String?
    @State private var highlight: String?

    private var query: ExpertMentionQuery? {
        guard focus, !composing, dismissed != draft.text else { return nil }
        return .atCaret(in: draft.text, selection: selection ?? NSRange(location: draft.text.utf16.count, length: 0), mentions: draft.mentions)
    }
    private var visible: [AppleExpertEditCandidate] {
        guard let query else { return [] }
        return candidates.filter { query.query.isEmpty || $0.name.localizedCaseInsensitiveContains(query.query) || $0.detail.localizedCaseInsensitiveContains(query.query) }
    }

    private var hooks: AppleMentionEditor {
        .init(mentions: draft.mentions, caretRequest: caret, selectionChanged: { range, marked in
            selection = range; composing = marked
        }, applyEdit: { old, range, replacement in
            guard draft.text == old else { return false }
            draft.mentions = AppleDraftMention.replacing(draft.mentions, range: range, with: replacement)
            draft.text = (old as NSString).replacingCharacters(in: range, with: replacement)
            return true
        }, restore: { value, mentions in draft.text = value; draft.mentions = mentions }, pasted: { dismissed = $0 })
    }

    private var textBinding: Binding<String> {
        Binding(get: { draft.text }, set: { text in
            if text != draft.text { draft.text = text; draft.mentions = [] }
        })
    }

    var body: some View {
        VStack(alignment: .leading) {
            if query != nil {
                ExpertMentionPanel(candidates: visible.map { .init(id: $0.id, name: $0.name, detail: $0.detail) },
                    highlightedID: highlight ?? visible.first?.id, error: nil, select: insert)
            }
            #if os(macOS)
            MacComposerTextEditor(text: textBinding, conversationID: draft.sourceMessageID, focused: $focus,
                onPasteAttachments: { false }, onDropAttachments: { _ in false }, onSubmit: {}, submitsReturn: false,
                mentionCommand: handle, mentionEditor: hooks, editorAccessibilityIdentifier: "mac-message-edit-text")
            #elseif os(iOS)
            ChatPasteAwareEditor(text: textBinding, isEditable: true, focus: $focus,
                onPasteFiles: { _ in }, onPasteFailure: { _ in }, mentionsVisible: query != nil,
                mentionCommand: handle, mentionEditor: hooks)
            #endif
        }
        .onChange(of: draft.text) { _, _ in highlight = nil; dismissed = nil }
    }

    private func insert(_ id: String) {
        guard let query, let candidate = visible.first(where: { $0.id == id }) else { return }
        let inserted = AppleDraftMention.insertion(text: draft.text, range: query.range, name: candidate.name,
            expert: candidate.expert, mentions: draft.mentions, sourceID: nil, epoch: nil)
        let previous = AppleMentionEditor.Snapshot(text: draft.text, mentions: draft.mentions, caret: NSMaxRange(query.range))
        draft.text = inserted.text
        draft.mentions = inserted.mentions
        caret = .init(location: inserted.caret, before: previous); selection = NSRange(location: inserted.caret, length: 0)
        focus = true
    }
    private func handle(_ command: String) -> Bool {
        guard query != nil else { return false }
        if command == "dismiss" { dismissed = draft.text; return true }
        guard !visible.isEmpty else { return command == "accept" }
        let index = visible.firstIndex { $0.id == highlight } ?? 0
        if command == "accept" { insert(visible[index].id) }
        else { highlight = visible[(index + (command == "up" ? -1 : 1) + visible.count) % visible.count].id }
        return true
    }
}
