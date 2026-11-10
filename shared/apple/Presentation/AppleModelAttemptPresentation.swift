// SPDX-License-Identifier: Apache-2.0
import Foundation

/// A successor proves request succession, not provider failure or model fallback.
/// Opaque provider identities participate only in equality; they are not model names.
nonisolated struct AppleModelAttemptPresentation: Equatable, Sendable {
    enum Kind: Equatable, Sendable {
        case pending, running, waiting, completed, cancelled, failed, restarted, routeChanged, recorded
    }
    let kind: Kind
    let modelName: String
    let successorModelName: String?
    let hasSuccessor: Bool

    init(state: String, provider: String?, model: String?, successorAttemptID: String?,
        successorProvider: String?, successorModel: String?) {
        func normalized(_ value: String?) -> String? {
            guard let result = value?.trimmingCharacters(in: .whitespacesAndNewlines), !result.isEmpty else { return nil }
            return result
        }
        let originalModel = normalized(model)
        let nextModel = normalized(successorModel)
        let originalProvider = normalized(provider)
        let nextProvider = normalized(successorProvider)
        hasSuccessor = normalized(successorAttemptID) != nil
        modelName = originalModel ?? "—"
        successorModelName = hasSuccessor && nextModel != originalModel ? nextModel : nil
        if hasSuccessor {
            let modelChanged = originalModel != nil && nextModel != nil && originalModel != nextModel
            let providerChanged = originalProvider != nil && nextProvider != nil && originalProvider != nextProvider
            kind = modelChanged || providerChanged ? .routeChanged : .restarted
        } else {
            switch state {
            case "pending": kind = .pending
            case "running": kind = .running
            case "waiting_for_permission", "waiting_for_input": kind = .waiting
            case "completed", "sealed": kind = .completed
            case "cancelled": kind = .cancelled
            case "failed": kind = .failed
            default: kind = .recorded
            }
        }
    }

    var isFailure: Bool { kind == .failed }
    var titleKey: String {
        switch kind {
        case .pending: "Model request pending"
        case .running: "Model request running"
        case .waiting: "Model request waiting"
        case .completed: "Model request completed"
        case .cancelled: "Response stopped"
        case .failed: "Model request failed"
        case .restarted: "Request restarted"
        case .routeChanged: "Model route changed"
        case .recorded: "Model request recorded"
        }
    }
    func title(bundle: Bundle = .main) -> String {
        bundle.localizedString(forKey: titleKey, value: titleKey, table: nil)
    }
    var symbolName: String {
        switch kind {
        case .failed: "exclamationmark.triangle.fill"
        case .cancelled: "stop.circle"
        case .restarted: "arrow.clockwise"
        case .routeChanged: "arrow.triangle.branch"
        default: "cpu"
        }
    }
}
