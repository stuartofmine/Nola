// SPDX-License-Identifier: Apache-2.0
// Device-local Apple presentation contract. No execution or permission authority.
import SwiftUI

nonisolated enum AppleExpertBubbleColor: String, CaseIterable, Identifiable {
    case red, orange, yellow, green, teal, blue, purple, pink
    var id: String { rawValue }
    var localizationKey: String { "expert.color.\(rawValue)" }
    var lightHex: UInt32 {
        switch self {
        case .red: 0xF8F1F1
        case .orange: 0xF8F4EF
        case .yellow: 0xF7F6ED
        case .green: 0xF1F6F2
        case .teal: 0xEFF6F6
        case .blue: 0xF2F6F8
        case .purple: 0xF6F3F7
        case .pink: 0xF8F2F5
        }
    }
    var darkHex: UInt32 {
        switch self {
        case .red: 0x1C1313
        case .orange: 0x1C1712
        case .yellow: 0x1A190F
        case .green: 0x111A14
        case .teal: 0x101A1B
        case .blue: 0x11171B
        case .purple: 0x18141C
        case .pink: 0x1C1318
        }
    }
    func fill(_ scheme: ColorScheme) -> Color {
        let value = scheme == .dark ? darkHex : lightHex
        return Color(red: Double((value >> 16) & 255) / 255,
            green: Double((value >> 8) & 255) / 255, blue: Double(value & 255) / 255)
    }
}

nonisolated enum AppleExpertAppearance {
    static let storageKey = "nola.experts.bubble-colors"
    static func colors(_ data: Data) -> [String: String] {
        (try? JSONDecoder().decode([String: String].self, from: data)) ?? [:]
    }
    static func selected(_ data: Data, agentID: String) -> AppleExpertBubbleColor {
        colors(data)[agentID].flatMap(AppleExpertBubbleColor.init(rawValue:)) ?? .blue
    }
    static func saving(_ color: AppleExpertBubbleColor, agentID: String, in data: Data) -> Data? {
        guard !agentID.isEmpty else { return nil }
        var values = colors(data)
        values[agentID] = color.rawValue
        return try? JSONEncoder().encode(values)
    }
}

/// Default Nola replies use the prototype's compact signature above prose.
/// Expert identities remain outside their tinted bubbles.
struct AppleConversationSignature: View {
    var body: some View {
        HStack(spacing: 7) {
            Text(verbatim: "n").font(.system(size: 13, weight: .medium))
                .frame(width: 19, height: 19)
                .overlay(Circle().stroke(Color.primary.opacity(0.3), lineWidth: 1))
                .accessibilityHidden(true)
            Text(verbatim: "Nola").font(.system(size: 14, weight: .semibold))
        }.padding(.bottom, 4)
    }
}
