// SPDX-License-Identifier: Apache-2.0
// Presentation facts for the Host-owned chat model resolver; no selection policy.
import Foundation

@MainActor
struct MacChatModelCatalog {
    struct Entry {
        let option: MacChatModelOption
        let candidate: MacChatModelCandidate
        let sourceName: String
        var executionCapabilityStatus: MacModelExecutionCapabilityStatus? = nil
    }
    let entries: [Entry]
    var candidates: [MacChatModelCandidate] { entries.map(\.candidate) }

    static func load(application: MacApplicationPort = .shared) throws -> Self {
        let remote = MacAppleChatConfigurationStore(applicationPort: application)
        let endpoints = try remote.endpointSnapshot(includeCredentials: false).endpoints
        let configuredModels = remote.configuredModels()
        let automaticOrder = configuredModels.map(\.profileID)
        let capabilityStatuses = Dictionary(uniqueKeysWithValues: configuredModels.map {
            ($0.profileID, $0.executionCapabilityStatus)
        })
        var entries = endpoints.flatMap { endpoint in
            endpoint.models.map { model in
                Entry(option: MacChatModelOption(source: .configured, modelID: model.modelName,
                    displayName: "\(model.displayName) · \(endpoint.name)", configurationID: model.id),
                    candidate: MacChatModelCandidate(profileID: model.id,
                        allowedForChat: model.capabilities.contains(.chat),
                        availability: endpoint.isEnabled && model.isEnabled ? .available : .disabled),
                    sourceName: endpoint.name,
                    executionCapabilityStatus: capabilityStatuses[model.id])
            }
        }
        let existingIDs = Set(entries.map { $0.candidate.profileID })
        entries += configuredModels.filter { !existingIDs.contains($0.profileID) }.map { model in
            Entry(option: MacChatModelOption(source: .configured, modelID: model.modelName,
                displayName: "\(model.displayName) · \(model.endpointName)", configurationID: model.profileID),
                candidate: MacChatModelCandidate(profileID: model.profileID,
                    allowedForChat: model.capabilities.contains(.chat), availability: .available),
                sourceName: model.endpointName, executionCapabilityStatus: model.executionCapabilityStatus)
        }
        entries.sort {
            (automaticOrder.firstIndex(of: $0.candidate.profileID) ?? Int.max)
                < (automaticOrder.firstIndex(of: $1.candidate.profileID) ?? Int.max)
        }
        let local = MacLocalModelConfigurationStore(applicationPort: application)
        let profiles = try application.listModelProfiles()
        let stored = Dictionary(uniqueKeysWithValues: profiles.map { ($0.id, $0) })
        entries += MacLocalModelProfile.available.map { profile in
            Entry(option: MacChatModelOption(source: .local, modelID: profile.id,
                displayName: profile.displayName),
                candidate: MacChatModelCandidate(profileID: profile.id,
                    availability: stored[profile.id]?.enabled == false ? .disabled
                        : (local.isReady(profile) ? .available : .notInstalled)),
                sourceName: String(localized: "This Mac"))
        }
        let known = Set(entries.map { $0.candidate.profileID })
        entries += profiles.filter { !known.contains($0.id) && $0.deletedAtMs == nil }.map { profile in
            Entry(option: MacChatModelOption(source: .local, modelID: profile.id,
                displayName: profile.displayName),
                candidate: MacChatModelCandidate(profileID: profile.id, availability: .unsupportedRuntime),
                sourceName: profile.provider)
        }
        return Self(entries: entries)
    }
}
