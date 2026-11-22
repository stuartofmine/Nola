// SPDX-License-Identifier: Apache-2.0
// Presentation only: workspace section localization keys.
import Foundation

extension MacWorkspaceSection {
    var titleKey: String {
        switch self {
        case .chat: "Conversations"
        case .pluginPage: "Runtime plugins"
        case .extensions: "Capability library"
        case .memory: "Memory & Sources"
        case .automations: "Automations"
        }
    }
}
