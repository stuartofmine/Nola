// SPDX-License-Identifier: Apache-2.0
// Shared Apple composer presentation. Selection carries IDs, never name-based routing.
import Foundation
import SwiftUI

nonisolated struct ExpertMentionQuery: Equatable {
    let range: NSRange
    let query: String

    static func atEnd(of text: String) -> Self? {
        let value = text as NSString
        guard let match = try? NSRegularExpression(pattern: "(?:^|\\s)@([^\\s@]*)$").firstMatch(
            in: text, range: NSRange(location: 0, length: value.length)) else { return nil }
        let queryRange = match.range(at: 1)
        return Self(range: NSRange(location: queryRange.location - 1, length: queryRange.length + 1),
                    query: value.substring(with: queryRange))
    }
    func removing(from text: String) -> String { (text as NSString).replacingCharacters(in: range, with: "") }
}

struct ExpertMentionCandidate: Identifiable {
    let id: String
    let name: String
    let detail: String
}

struct ExpertMentionPanel: View {
    let candidates: [ExpertMentionCandidate]
    let highlightedID: String?
    let error: String?
    let select: (String) -> Void
    var accessibilityTitle: LocalizedStringKey = "chat.mention.experts"
    var identifierPrefix = "chat.mention."
    var showsAvatar = true

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
            .frame(maxHeight: min(224, CGFloat(max(1, candidates.count)) * 56 + 12))
            .onChange(of: highlightedID) { _, id in if let id { proxy.scrollTo(id) } }
        }
        .background(.background, in: RoundedRectangle(cornerRadius: 14))
        .overlay(RoundedRectangle(cornerRadius: 14).stroke(.secondary.opacity(0.18)))
        .accessibilityLabel(Text(accessibilityTitle))
    }
}
