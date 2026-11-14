// SPDX-License-Identifier: Apache-2.0
// Presentation only: conversation-scoped content objects never own execution.
import Foundation

enum MacWorkspaceSidebarArea: String, CaseIterable, Identifiable {
    case conversations, resources
    var id: String { rawValue }
    var title: String {
        switch self {
        case .conversations: String(localized: "Conversations")
        case .resources: String(localized: "composer.command.resources")
        }
    }
    var iconAsset: String {
        switch self {
        case .conversations: "nola-chat"
        case .resources: "nola-folder"
        }
    }

}

enum MacWorkspaceContentObject: Hashable, Identifiable {
    case section(MacWorkspaceSection)
    case file(MacCodeReference)
    case activity(AppleActivityEntry)
    case review(URL)
    var id: String {
        switch self {
        case .section(let section): "section:\(section.rawValue)"
        case .review(let url): "review:\(url.standardizedFileURL.absoluteString)"
        case .activity(let entry): "activity:\(entry.run_id)"
        case .file(let reference): "file:\(reference.url.standardizedFileURL.absoluteString)"
        }
    }
    var title: String {
        switch self {
        case .section(let section): String(localized: String.LocalizationValue(section.titleKey))
        case .file(let reference): reference.url.lastPathComponent
        case .activity: String(localized: "Activity")
        case .review: String(localized: "Workspace Changes")
        }
    }
}

extension MacWorkspaceSection {
    var titleKey: String {
        switch self {
        case .chat: "Chat"
        case .data: "Data Center"
        case .models: "Models"
        case .extensions: "Skills and MCP"
        case .capabilities: "System Access"
        case .memory: "Long-term Memory"
        case .automations: "Automations"
        case .settings: "Settings"
        }
    }
}

struct MacConversationContentWorkspace: Equatable {
    static let conversationID = "conversation"
    var objects: [MacWorkspaceContentObject] = []
    var selection = conversationID

    mutating func open(_ object: MacWorkspaceContentObject) {
        if let index = objects.firstIndex(where: { $0.id == object.id }) { objects[index] = object }
        else { objects.append(object) }
        selection = object.id
    }
    mutating func select(_ id: String) {
        guard id == Self.conversationID || objects.contains(where: { $0.id == id }) else { return }
        selection = id
    }
    mutating func close(_ id: String) {
        guard let index = objects.firstIndex(where: { $0.id == id }) else { return }
        if selection == id { selection = index == 0 ? Self.conversationID : objects[index - 1].id }
        objects.remove(at: index)
    }
    var selectedObject: MacWorkspaceContentObject? { objects.first { $0.id == selection } }
}
