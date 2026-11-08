// SPDX-License-Identifier: Apache-2.0
import SwiftUI

@MainActor
struct MacHistoricalPartDetailsButton: View {
    let selections: [MacHistoricalPartSelection]
    var reader: MacHistoricalPartDetailModel.Reader? = nil
    @Environment(\.journalPresentationOwner) private var owner
    @State private var isPresented = false

    var body: some View {
        let epoch = owner?.readerEpoch
        return Button {
            if let owner, let epoch {
                owner.openReader(.historical(.init(selections: selections, reader: reader)), epoch: epoch)
            } else { isPresented = true }
        } label: {
            Label { Text("history.details.open", tableName: "Chat") } icon: {
                Image(systemName: "doc.text.magnifyingglass")
            }
        }
        .buttonStyle(.link)
        .accessibilityIdentifier("chat.historicalDetails.open")
        .sheet(isPresented: $isPresented) {
            MacHistoricalPartDetailsView(selections: selections, reader: reader).id(selections)
        }
        // A new window, owner or revision invalidates the independent detail view.
        .onChange(of: selections) { _, _ in isPresented = false }
        .onDisappear { isPresented = false }
    }
}

@MainActor
struct MacHistoricalPartDetailsView: View {
    let selections: [MacHistoricalPartSelection]
    @Environment(\.dismiss) private var dismiss
    @State private var model: MacHistoricalPartDetailModel

    init(selections: [MacHistoricalPartSelection], reader: MacHistoricalPartDetailModel.Reader? = nil) {
        self.selections = selections
        _model = State(initialValue: reader.map { MacHistoricalPartDetailModel(reader: $0) } ?? MacHistoricalPartDetailModel())
    }
    @State private var selectedIndex = 0
    @State private var retry = 0

    var body: some View {
        VStack(alignment: .leading, spacing: 14) {
            HStack {
                Text("history.details.title", tableName: "Chat").font(.headline)
                Spacer()
                Button { dismiss() } label: { Text("history.reader.close", tableName: "Chat") }
                    .keyboardShortcut(.cancelAction)
            }
            Text("history.details.readOnly", tableName: "Chat")
                .font(.caption).foregroundStyle(.secondary)
            if selections.count > 1 {
                Picker(selection: $selectedIndex) {
                    ForEach(selections.indices, id: \.self) { index in
                        Text(verbatim: selections[index].owner.partID).tag(index)
                    }
                } label: { Text("history.details.part", tableName: "Chat") }
                .accessibilityIdentifier("chat.historicalDetails.part")
            }
            Divider()
            ScrollView {
                VStack(alignment: .leading, spacing: 12) {
                    switch model.state {
                    case .idle, .loading:
                        ProgressView()
                    case .failed:
                        Text("history.details.failure", tableName: "Chat")
                        Button { retry += 1 } label: { Text("history.reader.retry", tableName: "Chat") }
                    case .absent:
                        Text("history.details.absent", tableName: "Chat")
                    case .loaded(let projection):
                        Text(verbatim: projection.statusLabel).font(.headline)
                        if projection.status.state == .notLoaded || projection.status.state == .deferred {
                            Text("history.details.bounded", tableName: "Chat").foregroundStyle(.secondary)
                        }
                        ForEach(projection.fields.keys.sorted(), id: \.self) { name in
                            VStack(alignment: .leading, spacing: 4) {
                                Text(verbatim: name).font(.caption.bold())
                                switch projection.fields[name] {
                                case .complete(let value):
                                    Text(verbatim: value.displayString).font(.system(.body, design: .monospaced))
                                case .deferred:
                                    Text("history.details.fieldDeferred", tableName: "Chat").foregroundStyle(.secondary)
                                case nil: EmptyView()
                                }
                            }
                        }
                    }
                }
                .textSelection(.enabled)
                .frame(maxWidth: .infinity, alignment: .leading)
            }
            .accessibilityIdentifier("chat.historicalDetails.content")
        }
        .padding(20)
        .frame(minWidth: 560, idealWidth: 760, minHeight: 420, idealHeight: 600)
        .task(id: "\(selectedIndex):\(retry)") {
            guard selections.indices.contains(selectedIndex) else { return }
            await model.load(selections[selectedIndex])
        }
        .onDisappear { model.invalidate() }
    }
}
