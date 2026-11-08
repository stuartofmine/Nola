// SPDX-License-Identifier: Apache-2.0
import Foundation
// Resolve once at launch; an unavailable local route is never a remote route.
nonisolated enum MacLocalModelEndpointSelection {
    enum SelectionError: LocalizedError {
        case instanceChanged
        var errorDescription: String? {
            String(localized: "The local model runtime changed while preparing this request. Please send the request again.")
        }
    }

    static func resolve(
        modelID: String?,
        captured: MacMLXOpenAIService.Endpoint? = nil,
        endpoint: (MacLocalModelProfile) throws -> MacMLXOpenAIService.Endpoint
    ) throws -> MacMLXOpenAIService.Endpoint? {
        guard let modelID, let profile = MacLocalModelProfile.resolve(modelID) else {
            guard captured == nil else { throw SelectionError.instanceChanged }
            return nil
        }
        let current = try endpoint(profile)
        guard current.modelID == profile.id, captured == nil || captured == current else {
            throw SelectionError.instanceChanged
        }
        return captured ?? current
    }
}
