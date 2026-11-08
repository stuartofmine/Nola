// SPDX-License-Identifier: Apache-2.0
import SwiftUI
#if os(macOS)
import AppKit
#endif

enum AppleAppearancePreference: String, CaseIterable {
    static let storageKey = "nola.appearance"
    case system, light, dark

    var colorScheme: ColorScheme? {
        switch self { case .system: nil; case .light: .light; case .dark: .dark }
    }

    var title: String {
        switch self {
        case .system: NSLocalizedString("appearance.system", value: "Follow System", comment: "Appearance preference")
        case .light: NSLocalizedString("appearance.light", value: "Light", comment: "Appearance preference")
        case .dark: NSLocalizedString("appearance.dark", value: "Dark", comment: "Appearance preference")
        }
    }

    #if os(macOS)
    @MainActor static func applyToApplication() {
        let preference = Self(rawValue: UserDefaults.standard.string(forKey: storageKey) ?? "system") ?? .system
        switch preference {
        case .system: NSApplication.shared.appearance = nil
        case .light: NSApplication.shared.appearance = NSAppearance(named: .aqua)
        case .dark: NSApplication.shared.appearance = NSAppearance(named: .darkAqua)
        }
    }
    #endif
}

struct AppleAppearancePicker: View {
    @AppStorage(AppleAppearancePreference.storageKey) private var preference = "system"

    var body: some View {
        Picker(NSLocalizedString("appearance.title", value: "Appearance", comment: "Appearance setting"), selection: $preference) {
            ForEach(AppleAppearancePreference.allCases, id: \.rawValue) { value in
                Text(value.title).tag(value.rawValue)
            }
        }
        .accessibilityIdentifier("settings.appearance")
        #if os(macOS)
        .onChange(of: preference) { _, _ in AppleAppearancePreference.applyToApplication() }
        #endif
    }
}
