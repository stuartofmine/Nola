// SPDX-License-Identifier: Apache-2.0
// Local UI intents shared by the Apple clients; never admitted as agent messages.
import Foundation

enum ComposerCommand: String, CaseIterable, Identifiable {
    case model, experts, resources, permissions, plan, search, stop, help
    var id: String { rawValue }
    var acceptsArgument: Bool { self == .plan || self == .search }
    var title: String { NSLocalizedString("composer.command.\(rawValue)", value: rawValue.capitalized, comment: "Composer command") }
    static func query(_ text: String) -> String? {
        guard text.first == "/", !text.dropFirst().contains(where: { $0.isWhitespace || $0 == "/" }) else { return nil }
        return String(text.dropFirst()).lowercased()
    }
    static func candidates(_ text: String) -> [Self] {
        guard let query = query(text) else { return [] }
        return allCases.filter { query.isEmpty || $0.rawValue.hasPrefix(query) || $0.title.localizedCaseInsensitiveContains(query) }
    }
    static func parse(_ text: String) -> (command: Self?, argument: String)? {
        guard text.first == "/" else { return nil }
        let parts = text.dropFirst().split(whereSeparator: \.isWhitespace)
        let command = parts.first.flatMap { Self(rawValue: $0.lowercased()) }
        let argument = parts.dropFirst().joined(separator: " ")
        return (command?.acceptsArgument == true || argument.isEmpty ? command : nil, argument)
    }
    static var invalidMessage: String { NSLocalizedString("composer.command.invalid", value: "Invalid command or arguments. Type / for commands.", comment: "Command validation") }
}

extension Notification.Name {
    static let nolaOpenWorkspace = Notification.Name("nola.openWorkspace")
    static let nolaSearchConversations = Notification.Name("nola.searchConversations")
}
