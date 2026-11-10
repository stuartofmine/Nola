// SPDX-License-Identifier: Apache-2.0
import SwiftUI

/// The historical Part stays at its canonical position. Core's historical
/// projection enforces public disclosure before any text reaches this view.
nonisolated struct MacHistoricalJournalPartViewKey: Hashable {
    let owner: MacJournalOwnerKey
    let partID: String
    let canonicalRevision: String
}

struct MacJournalHistoricalPartView: View {
    let reference: MacJournalMessageReference
    let part: MacAssistantJournalPart
    let historical: MacAssistantJournalMetadata.Historical
    @Environment(\.uiSurfaceContext) private var uiContext
    @State private var model: MacHistoricalPartDetailModel
    @State private var retry = 0
    private let reader: MacHistoricalPartDetailModel.Reader

    init(reference: MacJournalMessageReference, part: MacAssistantJournalPart,
         historical: MacAssistantJournalMetadata.Historical) {
        self.reference = reference; self.part = part; self.historical = historical
        let databaseURL = reference.databaseURL
        let reader: MacHistoricalPartDetailModel.Reader = { selection in
            let read = Task.detached {
                try Task.checkCancellation()
                return try MacApplicationPort(databaseURL: databaseURL).readHistoricalPart(selection.owner,
                    expectedConversationRevision: selection.canonicalRevision)
            }
            return try await withTaskCancellationHandler { try await read.value } onCancel: { read.cancel() }
        }
        self.reader = reader
        _model = State(initialValue: MacHistoricalPartDetailModel(reader: reader))
    }
    private var selection: MacHistoricalPartSelection? {
        guard part.identity == reference.cursor.identity,
              let revision = Int64(historical.canonical_revision), revision > 0,
              let sequence = UInt64(reference.cursor.sequence) else { return nil }
        return .init(owner: .init(conversationID: part.identity.conversation_id,
            messageID: part.identity.assistant_message_id, partID: part.part_id),
            canonicalRevision: revision, windowSequence: sequence)
    }
    var body: some View {
        Group {
            if selection == nil {
                Text("history.details.failure", tableName: "Chat").foregroundStyle(.secondary)
            } else { switch model.state {
            case .idle, .loading: ProgressView().controlSize(.small)
            case .failed:
                VStack(alignment: .leading) {
                    Text("history.details.failure", tableName: "Chat").foregroundStyle(.secondary)
                    Button { retry += 1 } label: { Text("history.reader.retry", tableName: "Chat") }
                }
            case .absent: EmptyView()
            case .loaded(let projection):
                if let surface = projection.uiSurfaceReference,
                   surface.identity.owner.conversation_id == reference.cursor.identity.conversation_id,
                   surface.identity.owner.message_id == reference.cursor.identity.assistant_message_id,
                   let uiContext {
                    AppleUIHistoryView<MacCanonicalJSONValue>(reference: surface, load: uiContext.readRevision)
                        .id(surface.identity)
                } else if let text = projection.visibleText {
                    VStack(alignment: .leading, spacing: 8) {
                        MacMarkdownView(markdown: text, isStreaming: false, animatesAppends: false)
                        if projection.decodedText == nil,
                           let original = projection.canonicalText, original.totalUTF8Bytes > UInt64(text.utf8.count) {
                            Text("history.preview.label", tableName: "Chat").font(.caption).foregroundStyle(.secondary)
                        }
                    }
                } else if projection.canonicalKind != "text", let selection {
                    MacHistoricalPartDetailsButton(selections: [selection], reader: reader)
                }
            } }
        }
        .accessibilityIdentifier("chat.journal.historical." + part.part_id)
        .task(id: "\(selection?.canonicalRevision ?? 0):\(reference.cursor.sequence):\(retry)") {
            guard let selection else { return }
            await model.load(selection)
        }
        .onDisappear { model.invalidate() }
    }
}
