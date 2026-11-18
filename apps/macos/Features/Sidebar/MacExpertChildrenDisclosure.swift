// SPDX-License-Identifier: Apache-2.0
import SwiftUI

/// Child lists are queried only after expansion; each request is bound to its parent.
struct MacExpertChildrenDisclosure: View {
    let parentID: String
    let refreshRevision: Date
    var roomRevision: Int64? = nil
    let load: (String, Int) async throws -> AppleExpertChildConversations
    let open: (AppleExpertChildConversation) -> Void
    @State private var expanded = false
    @State private var children: [AppleExpertChildConversation] = []
    @State private var firstPage: AppleExpertChildConversations?
    @State private var nextOffset: Int?
    @State private var loading = false
    @State private var error: String?
    @State private var requestID = UUID()
    @State private var request: Task<Void, Never>?

    @Environment(\.accessibilityReduceMotion) private var reduceMotion

    var body: some View {
        VStack(alignment: .leading, spacing: 6) {
            Button {
                withAnimation(reduceMotion ? nil : MacDesignTokens.Motion.disclosureAnimation) {
                    expanded.toggle()
                }
                if expanded { reload() }
                else { cancel() }
            } label: {
                HStack(spacing: 8) {
                    Image(systemName: "person.2")
                        .font(.system(size: 12, weight: .semibold))
                        .foregroundStyle(.secondary)
                    Text("Expert conversations")
                        .font(.system(size: 13, weight: .medium))
                    if loading { ProgressView().controlSize(.mini) }
                    Spacer(minLength: 8)
                    if !children.isEmpty {
                        Text(children.count.formatted())
                            .font(.caption.monospacedDigit())
                            .foregroundStyle(.secondary)
                    }
                    Image(systemName: "chevron.down")
                        .font(.system(size: 10.5, weight: .semibold))
                        .foregroundStyle(.secondary)
                        .rotationEffect(.degrees(expanded ? 180 : 0))
                        .accessibilityHidden(true)
                }
                .frame(maxWidth: .infinity, alignment: .leading)
                .contentShape(Rectangle())
            }
            .buttonStyle(MacHorizontalBarButtonStyle(isSubtle: true, isExpanded: expanded))
            .accessibilityIdentifier("expert.children.toggle." + parentID)
            .accessibilityValue(Text(expanded ? "Expanded" : "Collapsed", tableName: "Chat"))
            if expanded {
                ForEach(children) { child in
                    Button { open(child) } label: {
                        HStack(spacing: 6) {
                            Image(systemName: child.isCurrent && child.active ? "bubble.left" : "clock")
                            Text(child.title).lineLimit(1)
                            Spacer(minLength: 0)
                            Text(child.messageCount.formatted()).foregroundStyle(.tertiary)
                        }.font(.system(size: 11.5)).frame(maxWidth: .infinity, minHeight: 30, alignment: .leading)
                            .contentShape(Rectangle())
                    }.buttonStyle(MacActionButtonStyle(contentInsets: .init(top: 4, leading: 8, bottom: 4, trailing: 6)))
                        .accessibilityIdentifier("expert.child." + child.id)
                }
                if let error {
                    Text(error).font(.caption).foregroundStyle(.secondary).textSelection(.enabled)
                    Button("Retry") { reload() }.buttonStyle(MacActionButtonStyle()).disabled(loading)
                } else if children.isEmpty && !loading {
                    Text("No expert conversations yet").font(.caption).foregroundStyle(.secondary).padding(.vertical, 5)
                }
                if let nextOffset {
                    Button("Load more") { fetch(offset: nextOffset) }.font(.caption).buttonStyle(MacActionButtonStyle()).disabled(loading)
                }
            }
        }
            .onChange(of: refreshRevision) { _, _ in if expanded { reload() } }
            .onChange(of: roomRevision) { _, _ in if expanded { reload() } }
            .onChange(of: parentID) { _, _ in cancel(); expanded = false; children = []; nextOffset = nil; error = nil }
            .onDisappear { cancel() }
    }

    private func cancel() { requestID = UUID(); request?.cancel(); request = nil; loading = false }
    private func reload() { cancel(); fetch(offset: 0) }
    private func fetch(offset: Int) {
        guard expanded, !loading else { return }
        loading = true; error = nil
        let identity = UUID(); requestID = identity
        let parent = parentID
        request = Task { @MainActor in
            do {
                let page = try await load(parent, offset)
                try page.validate(parentID: parent, offset: offset)
                guard requestID == identity, parentID == parent, expanded, !Task.isCancelled else { return }
                if let firstPage, offset > 0 {
                    guard firstPage.roomID == page.roomID, firstPage.roomRevision == page.roomRevision else { throw CocoaError(.coderReadCorrupt) }
                }
                if offset == 0 {
                    firstPage = page; children = page.children
                } else {
                    guard Set(children.map(\.id)).isDisjoint(with: page.children.map(\.id)) else { throw CocoaError(.coderReadCorrupt) }
                    children.append(contentsOf: page.children)
                }
                nextOffset = page.nextOffset; loading = false
            } catch {
                guard requestID == identity, parentID == parent, expanded, !Task.isCancelled else { return }
                self.error = error.localizedDescription; loading = false
            }
        }
    }
}
