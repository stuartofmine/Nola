// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Defines workspace activity and user-selectable execution/presentation values.

import Foundation
import SwiftUI

struct MacAgentActivity: Identifiable, Hashable {
    enum State: Hashable {
        case pending
        case running
        case waiting
        case completed
        case failed

        var isTerminal: Bool {
            self == .completed || self == .failed
        }
    }

    let id: UUID
    let title: String
    let detail: String
    let state: State
    let startedAt: Date
    let finishedAt: Date?

    init(
        id: UUID = UUID(),
        title: String,
        detail: String,
        state: State,
        startedAt: Date = .now,
        finishedAt: Date? = nil
    ) {
        let presentation = MacAgentActivityPresentationPolicy.project(title: title, detail: detail)
        self.id = id
        self.title = presentation.title
        self.detail = presentation.detail
        self.state = state
        self.startedAt = startedAt
        self.finishedAt = finishedAt
    }

    func updating(title: String, detail: String, state: State) -> MacAgentActivity {
        MacAgentActivity(
            id: id,
            title: title,
            detail: detail,
            state: state,
            startedAt: startedAt,
            finishedAt: state.isTerminal ? .now : nil
        )
    }
}

enum MacExecutionLocation: String, CaseIterable, Identifiable {
    case thisMac
    case connectedHost
    case localModel
    case chatOnly

    var id: String { rawValue }

    var title: String {
        switch self {
        case .thisMac: return String(localized: "This Mac")
        case .connectedHost: return String(localized: "Connected Host")
        case .localModel: return String(localized: "Local Model")
        case .chatOnly: return String(localized: "Provider Model")
        }
    }

    var symbol: String {
        switch self {
        case .thisMac: return "sparkles"
        case .connectedHost: return "network"
        case .localModel: return "cpu.fill"
        case .chatOnly: return "bubble.left.and.bubble.right"
        }
    }
}

enum MacResponseStyle: String, CaseIterable, Identifiable {
    case concise
    case balanced
    case detailed

    var id: String { rawValue }

    var title: String {
        switch self {
        case .concise: return String(localized: "Concise")
        case .balanced: return String(localized: "Balanced")
        case .detailed: return String(localized: "Detailed")
        }
    }

    /// User-facing response depth is separate from provider-specific reasoning.
    /// Provider effort is selected independently from the model declaration.
    var responseInstruction: String {
        switch self {
        case .concise:
            return "Answer directly and concisely. Include only detail needed to satisfy the request."
        case .balanced:
            return "Give a clear answer with enough explanation to be useful, without unnecessary detail."
        case .detailed:
            return "Give a thorough answer with relevant reasoning, evidence, and important caveats, while staying focused on the request."
        }
    }
}

enum MacWorkspaceSection: String, Identifiable {
    case chat
    case extensions
    case automations
    case memory
    case pluginPage

    static let sidebarSections: [Self] = [.chat, .extensions, .automations, .memory]
    var id: String { rawValue }

    static func initialSection(environment: [String: String] = ProcessInfo.processInfo.environment) -> Self {
        sidebarSections.first { $0.rawValue == environment["NOLA_MAC_INITIAL_SECTION"] } ?? .chat
    }

    var title: LocalizedStringKey {
        switch self {
        case .chat: return "Conversations"
        case .extensions: return "Capability library"
        case .pluginPage: return "Runtime plugins"
        case .memory: return "Memory & Sources"
        case .automations: return "Automations"
        }
    }

    var symbol: String {
        switch self {
        case .chat: return "bubble.left.and.bubble.right"
        case .extensions: return "puzzlepiece.extension"
        case .pluginPage: return "rectangle.on.rectangle"
        case .memory: return "brain.head.profile"
        case .automations: return "clock.arrow.circlepath"
        }
    }
}
