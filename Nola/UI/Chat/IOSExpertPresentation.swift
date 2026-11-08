// SPDX-License-Identifier: Apache-2.0
// Device-local appearance only. Agent identity and execution remain Core-owned.
import SwiftUI
import Combine

@MainActor
final class IOSExpertColorPreferences: ObservableObject {
    static let shared = IOSExpertColorPreferences()
    private let defaults: UserDefaults
    @Published private var stored: Data
    init(defaults: UserDefaults = .standard) {
        self.defaults = defaults
        stored = defaults.data(forKey: AppleExpertAppearance.storageKey) ?? Data()
    }
    func color(for agentID: String) -> AppleExpertBubbleColor {
        AppleExpertAppearance.selected(stored, agentID: agentID)
    }
    func save(_ color: AppleExpertBubbleColor, for agentID: String) {
        guard let data = AppleExpertAppearance.saving(color, agentID: agentID, in: stored) else { return }
        stored = data
        defaults.set(data, forKey: AppleExpertAppearance.storageKey)
    }
}

struct IOSExpertColorEditor: View {
    let agentID: String
    let name: String
    @Environment(\.dismiss) private var dismiss
    @Environment(\.colorScheme) private var scheme
    @ObservedObject private var preferences = IOSExpertColorPreferences.shared
    @State private var draft: AppleExpertBubbleColor
    init(agentID: String, name: String) {
        self.agentID = agentID
        self.name = name
        _draft = State(initialValue: IOSExpertColorPreferences.shared.color(for: agentID))
    }
    var body: some View {
        NavigationStack {
            ScrollView {
                VStack(alignment: .leading, spacing: 24) {
                    HStack(alignment: .top, spacing: 12) {
                        VStack(spacing: 6) {
                            AppleExpertAvatar(name: name, size: 30)
                            AppleExpertName(name: name, alignment: .center)
                        }.frame(width: 38)
                        Text(String(localized: "expert.color.preview")).font(.body).padding(14)
                            .frame(maxWidth: .infinity, alignment: .leading)
                            .background(draft.fill(scheme), in: UnevenRoundedRectangle(topLeadingRadius: 5, bottomLeadingRadius: 14, bottomTrailingRadius: 14, topTrailingRadius: 14))
                    }
                    VStack(alignment: .leading, spacing: 4) {
                        Text(String(localized: "expert.color.title")).font(.headline)
                        ForEach(AppleExpertBubbleColor.allCases) { color in
                            Button { draft = color } label: {
                                HStack(spacing: 12) {
                                    Circle().fill(color.fill(scheme))
                                        .overlay(Circle().stroke(Color.secondary.opacity(0.5), lineWidth: 0.5))
                                        .frame(width: 24, height: 24).accessibilityHidden(true)
                                    Text(String(localized: String.LocalizationValue(color.localizationKey)))
                                        .foregroundStyle(.primary)
                                    Spacer(minLength: 8)
                                    if draft == color {
                                        Image(systemName: "checkmark").foregroundStyle(.primary)
                                    }
                                }
                                .frame(minHeight: 48).contentShape(Rectangle())
                            }
                            .buttonStyle(.plain)
                            .accessibilityAddTraits(draft == color ? [.isSelected] : [])
                            .accessibilityIdentifier("expert.color." + color.rawValue)
                        }
                    }
                    .accessibilityIdentifier("expert.color.picker")
                    .accessibilityValue(String(localized: String.LocalizationValue(draft.localizationKey)))
                }.padding(20)
            }
            .background(DesignTokens.backgroundColor)
            .navigationTitle(name)
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .cancellationAction) { Button(L10n.cancel) { dismiss() }.frame(minHeight: 44).accessibilityIdentifier("expert.color.cancel") }
                ToolbarItem(placement: .confirmationAction) {
                    Button(String(localized: "common.save")) { preferences.save(draft, for: agentID); dismiss() }.frame(minHeight: 44).accessibilityIdentifier("expert.color.save")
                }
            }
        }
    }
}
