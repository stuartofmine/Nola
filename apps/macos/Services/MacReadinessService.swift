// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Reports only execution-blocking setup issues for the chat-first desktop workspace.

import Foundation

extension Notification.Name {
    nonisolated static let nolaMacConfigurationDidChange = Notification.Name("nola.mac.configurationDidChange")
}

struct MacReadinessIssue: Hashable, Sendable {
    enum Action: Hashable, Sendable {
        case diagnostics
        case models
        case settings
    }

    let title: String
    let detail: String
    let symbol: String
    let actionTitle: String
    let action: Action
}

enum MacReadinessService {
    static func blocksSending(for location: MacExecutionLocation) -> Bool {
        guard location == .localModel else { return blockingIssue(for: location) != nil }
        let profile = MacLocalModelConfigurationStore.shared.selectedProfile
        let runtime = MacMLXOpenAIService.shared.status
        return !(runtime.isStarting && runtime.modelID == profile.id)
            && !MacMLXOpenAIService.shared.isRunning(profile)
    }

    static func blockingIssue(for location: MacExecutionLocation) -> MacReadinessIssue? {
        switch location {
        case .thisMac:
            guard locateAgentRuntime() == nil else { return nil }
            return MacReadinessIssue(
                title: String(localized: "Local Agent Runtime is unavailable"),
                detail: String(localized: "The local agent runtime is part of Nola for Mac. Rebuild the Mac target or configure NOLA_AGENT_HOST_PATH."),
                symbol: "gearshape.2.fill",
                actionTitle: String(localized: "Open Diagnostics"),
                action: .diagnostics
            )
        case .connectedHost:
            guard MacRemoteHostConfigurationStore.shared.load() == nil else { return nil }
            return MacReadinessIssue(
                title: String(localized: "Connected Host is not configured"),
                detail: String(localized: "Add a host URL and access token, or switch execution to This Mac."),
                symbol: "network.slash",
                actionTitle: String(localized: "Open Settings"),
                action: .settings
            )
        case .localModel:
            let store = MacLocalModelConfigurationStore.shared
            let profile = store.selectedProfile
            let runtime = MacMLXOpenAIService.shared.status
            if MacMLXOpenAIService.shared.isRunning(profile) { return nil }
            if runtime.isStarting, runtime.modelID == profile.id {
                return MacReadinessIssue(
                    title: String(localized: "Starting", table: "ModelCenter") + "…",
                    detail: runtime.phase == .loadingWeights
                        ? String(localized: "Loading model weights", table: "ModelCenter") + " · " + profile.displayName
                        : String(localized: "Starting local model service", table: "ModelCenter") + " · " + profile.displayName,
                    symbol: "cpu",
                    actionTitle: String(localized: "Open Model Center"),
                    action: .models
                )
            }
            if runtime.phase == .failed, runtime.modelID == profile.id {
                return MacReadinessIssue(
                    title: String(localized: "Local model failed to start", table: "ModelCenter"),
                    detail: runtime.errorDescription ?? String(localized: "Unable to start the local model service."),
                    symbol: "exclamationmark.triangle.fill",
                    actionTitle: String(localized: "Open Model Center"),
                    action: .models
                )
            }
            guard !store.isSelectedModelReady else {
                return MacReadinessIssue(
                    title: String(localized: "Local model is not running", table: "ModelCenter"),
                    detail: String(localized: "Start the selected local model before using it in chat.", table: "ModelCenter"),
                    symbol: "play.circle",
                    actionTitle: String(localized: "Open Model Center"),
                    action: .models
                )
            }
            return MacReadinessIssue(
                title: String(localized: "Local model is not ready"),
                detail: String(localized: "Download a model and select it for Local Model chat."),
                symbol: "cpu",
                actionTitle: String(localized: "Open Model Center"),
                action: .models
            )
        case .chatOnly:
            guard MacAppleChatConfigurationStore.shared.load() == nil else { return nil }
            return MacReadinessIssue(
                title: String(localized: "Provider Model is not configured"),
                detail: String(localized: "Choose an OpenAI-compatible endpoint and model before sending."),
                symbol: "bubble.left.and.exclamationmark.bubble.right",
                actionTitle: String(localized: "Open Model Center"),
                action: .models
            )
        }
    }

    static var agentRuntimeURL: URL? { locateAgentRuntime() }

    private static func locateAgentRuntime() -> URL? {
        let fileManager = FileManager.default
        if let configured = ProcessInfo.processInfo.environment["NOLA_AGENT_HOST_PATH"] {
            let url = URL(fileURLWithPath: configured)
            if fileManager.isExecutableFile(atPath: url.path) { return url }
        }
        if let bundled = Bundle.main.url(forAuxiliaryExecutable: "nola-agent-host"),
           fileManager.isExecutableFile(atPath: bundled.path) {
            return bundled
        }
        return nil
    }
}
