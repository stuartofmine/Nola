// SPDX-License-Identifier: Apache-2.0
// Closed protocol errors; never infer model capabilities from user text.
import Foundation

nonisolated enum AppleMainExpertDispatchRejection: String, LocalizedError, Sendable {
    case unsupportedModel = "main_expert_selection_requires_tool_capable_main_model"
    case disabledDispatch = "main_expert_selection_requires_enabled_expert_dispatch"
    case sourceChanged = "main_expert_selection_source_changed"

    var errorDescription: String? {
        switch self {
        case .unsupportedModel:
            return NSLocalizedString("main.experts.requiresTools", value: "Choose a main model that supports tool calls, or remove the selected experts. Your message is kept.", comment: "Main model cannot delegate")
        case .disabledDispatch:
            return NSLocalizedString("main.experts.enableTools", value: "Enable expert tools for the main Agent, or remove the selected experts. Your message is kept.", comment: "Main expert dispatch is disabled")
        case .sourceChanged:
            return NSLocalizedString("main.experts.sourceChanged", value: "The Host data changed. Reload its expert directory and select the experts again. Your message is kept.", comment: "Do not silently rebase an expert selection")
        }
    }
    var permitsDraftRestore: Bool { self != .sourceChanged }
}
