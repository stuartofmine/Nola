// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Owns explicit per-conversation folder consent and selected-folder OS bookmarks,
//   independently of cwd and operation permissions.

import AppKit
import Foundation

@MainActor
protocol MacWorkspaceBookmarkIO: Sendable {
    func chooseFolder(suggested: URL?) async -> URL?
    func bookmark(_ url: URL) throws -> Data
    nonisolated func resolve(_ data: Data) throws -> (url: URL, stale: Bool)
    func start(_ url: URL) -> Bool
    func stop(_ url: URL)
}

@MainActor
struct MacSystemWorkspaceBookmarkIO: MacWorkspaceBookmarkIO {
    func chooseFolder(suggested: URL?) async -> URL? {
        let panel = NSOpenPanel()
        panel.title = String(localized: "Choose a Workspace")
        panel.message = suggested == nil
            ? String(localized: "Choose the default working directory for this conversation.")
            : String(localized: "Authorize this exact folder. This does not approve file modifications.")
        panel.canChooseDirectories = true
        panel.canChooseFiles = false
        panel.allowsMultipleSelection = false
        panel.canCreateDirectories = false
        panel.directoryURL = suggested
        let window = NSApp.keyWindow
        return await withCheckedContinuation { continuation in
            let completion: (NSApplication.ModalResponse) -> Void = { response in
                continuation.resume(returning: response == .OK ? panel.url : nil)
            }
            if let window {
                panel.beginSheetModal(for: window, completionHandler: completion)
            } else {
                panel.begin(completionHandler: completion)
            }
        }
    }

    func bookmark(_ url: URL) throws -> Data {
        try url.bookmarkData(options: [.withSecurityScope], includingResourceValuesForKeys: nil, relativeTo: nil)
    }

    nonisolated func resolve(_ data: Data) throws -> (url: URL, stale: Bool) {
        var stale = false
        let url = try URL(resolvingBookmarkData: data, options: [.withSecurityScope],
                          relativeTo: nil, bookmarkDataIsStale: &stale)
        return (url, stale)
    }

    func start(_ url: URL) -> Bool { url.startAccessingSecurityScopedResource() }
    func stop(_ url: URL) { url.stopAccessingSecurityScopedResource() }
}

@MainActor
final class MacSecurityScopedBookmarkStore {
    static let shared = MacSecurityScopedBookmarkStore()

    struct SubmissionSelection: Sendable {
        let conversationID: String
        let primaryID: UUID?
        let grantIDs: Set<UUID>
        let confirmedDirectories: Set<String>
        let selectedURL: URL?
    }

    func captureSubmissionSelection(conversationID: String, selectedURL: URL?) -> SubmissionSelection {
        let record = read(key(conversationID))
        return SubmissionSelection(conversationID: conversationID, primaryID: record.primaryID,
            grantIDs: Set(record.grants.map(\.id)),
            confirmedDirectories: Set(record.confirmedDirectories ?? []), selectedURL: selectedURL)
    }

    /// Keep the submitted selection, but never revive a revoked grant. New
    /// grants made while preparation suspends belong to later submissions.
    func resolveSubmissionSelection(_ selection: SubmissionSelection) async throws -> (projectURL: URL, authorizedURLs: [URL]) {
        _ = await restoreWorkspaceAsync(conversationID: selection.conversationID)
        let storageKey = key(selection.conversationID)
        let current = read(storageKey)
        let survivingIDs = Set(current.grants.map(\.id)).intersection(selection.grantIDs)
        let authorized = survivingIDs.compactMap { active[storageKey]?[$0] }
            + Set(current.confirmedDirectories ?? []).intersection(selection.confirmedDirectories)
                .sorted().map { URL(fileURLWithPath: $0) }
        let selected = selection.selectedURL
            ?? selection.primaryID.flatMap { survivingIDs.contains($0) ? active[storageKey]?[$0] : nil }
        if let selected {
            guard authorized.contains(where: { $0.standardizedFileURL == selected.standardizedFileURL }) else {
                throw CocoaError(.fileReadNoPermission)
            }
            return (selected, authorized)
        }
        guard selection.primaryID == nil else { throw CocoaError(.fileReadNoPermission) }
        return (MacAppEnvironment.shared.sessionWorkspaceURL(for: selection.conversationID), authorized)
    }

    private struct Grant: Codable {
        let id: UUID
        var bookmark: Data
    }

    private struct Record: Codable {
        var primaryID: UUID?
        var grants: [Grant] = []
        // Explicit app-level consent, not an OS security-scope grant. The Mac
        // product is not App Sandbox enabled; TCC/file permissions still apply.
        var confirmedDirectories: [String]?
    }

    private let defaults: UserDefaults
    private let io: any MacWorkspaceBookmarkIO
    private var active: [String: [UUID: URL]] = [:]

    init(defaults: UserDefaults = .standard, io: (any MacWorkspaceBookmarkIO)? = nil) {
        self.defaults = defaults
        self.io = io ?? MacSystemWorkspaceBookmarkIO()
    }

    private func key(_ conversationID: String?) -> String {
        "nola.mac.workspace.access.v2.\(conversationID ?? "default")"
    }

    private func read(_ key: String) -> Record {
        guard let data = defaults.data(forKey: key),
              let record = try? JSONDecoder().decode(Record.self, from: data) else { return Record() }
        return record
    }

    private func save(_ record: Record, key: String) throws {
        defaults.set(try JSONEncoder().encode(record), forKey: key)
    }

    /// Returns reactivated OS bookmarks plus explicit in-app confirmations.
    /// Broken bookmarks never silently become path-based OS authority. Confirmed
    /// directories authorize only Nola's workspace boundary, not OS access.
    func authorizedWorkspaceURLs(conversationID: String? = nil) -> [URL] {
        let key = key(conversationID)
        var record = read(key)
        var refreshed = false
        for index in record.grants.indices {
            let grant = record.grants[index]
            if active[key]?[grant.id] != nil { continue }
            guard let resolution = try? io.resolve(grant.bookmark),
                  io.start(resolution.url) else { continue }
            active[key, default: [:]][grant.id] = resolution.url
            if resolution.stale, let data = try? io.bookmark(resolution.url) {
                record.grants[index].bookmark = data
                refreshed = true
            }
        }
        if refreshed { try? save(record, key: key) }
        return record.grants.compactMap { active[key]?[$0.id] }
            + (record.confirmedDirectories ?? []).map { URL(fileURLWithPath: $0) }
    }

    func restoreWorkspace(conversationID: String? = nil) -> URL? {
        _ = authorizedWorkspaceURLs(conversationID: conversationID)
        let key = key(conversationID)
        guard let id = read(key).primaryID else { return nil }
        return active[key]?[id]
    }

    func restoreWorkspaceAsync(conversationID: String? = nil) async -> URL? {
        let key = key(conversationID)
        let snapshot = defaults.data(forKey: key)
        var record = read(key)
        let pending = record.grants.filter { active[key]?[$0.id] == nil }
            .map { ($0.id, $0.bookmark) }
        let io = self.io
        let resolutions = await Task.detached(priority: .userInitiated) {
            pending.compactMap { id, data -> (UUID, URL, Bool)? in
                guard let result = try? io.resolve(data) else { return nil }
                return (id, result.url, result.stale)
            }
        }.value
        // A selection or revocation during off-main-thread I/O wins. Never
        // resurrect a grant from the stale snapshot after the user clears it.
        guard defaults.data(forKey: key) == snapshot else {
            return await restoreWorkspaceAsync(conversationID: conversationID)
        }
        var refreshed = false
        for (id, url, stale) in resolutions {
            guard active[key]?[id] == nil, io.start(url) else { continue }
            active[key, default: [:]][id] = url
            if stale, let index = record.grants.firstIndex(where: { $0.id == id }),
               let data = try? io.bookmark(url) {
                record.grants[index].bookmark = data
                refreshed = true
            }
        }
        if refreshed { try? save(record, key: key) }
        guard let primaryID = record.primaryID else { return nil }
        return active[key]?[primaryID]
    }

    func chooseWorkspace(conversationID: String? = nil) async -> URL? {
        guard let selected = await io.chooseFolder(suggested: nil) else { return nil }
        return retain(selected, conversationID: conversationID, makePrimary: true)
    }

    func authorizeFolder(suggestedURL: URL, conversationID: String? = nil) async -> URL? {
        guard let selected = await io.chooseFolder(suggested: suggestedURL),
              selected.resolvingSymlinksInPath().standardizedFileURL
                == suggestedURL.resolvingSymlinksInPath().standardizedFileURL else { return nil }
        return retain(selected, conversationID: conversationID, makePrimary: false)
    }

    /// Called only after the user accepts the native permission card. Unlike
    /// choosing a working directory, confirming an exact request needs no picker.
    func confirmFolderAccess(_ url: URL, conversationID: String) throws {
        guard url.isFileURL, url.path.hasPrefix("/") else {
            throw CocoaError(.fileReadInvalidFileName)
        }
        let path = url.resolvingSymlinksInPath().standardizedFileURL.path
        let recordKey = key(conversationID)
        var record = read(recordKey)
        var directories = record.confirmedDirectories ?? []
        if !directories.contains(path) { directories.append(path) }
        record.confirmedDirectories = directories
        try save(record, key: recordKey)
    }

    private func retain(_ url: URL, conversationID: String?, makePrimary: Bool) -> URL? {
        _ = authorizedWorkspaceURLs(conversationID: conversationID)
        let key = key(conversationID)
        var record = read(key)
        if let existing = active[key]?.first(where: { $0.value.standardizedFileURL == url.standardizedFileURL }) {
            if makePrimary {
                record.primaryID = existing.key
                guard (try? save(record, key: key)) != nil else { return nil }
            }
            return existing.value
        }
        guard io.start(url) else { return nil }
        do {
            let grant = Grant(id: UUID(), bookmark: try io.bookmark(url))
            record.grants.append(grant)
            if makePrimary { record.primaryID = grant.id }
            try save(record, key: key)
            active[key, default: [:]][grant.id] = url
            return url
        } catch {
            io.stop(url)
            return nil
        }
    }

    func clearWorkspace(conversationID: String? = nil) {
        let key = key(conversationID)
        for url in active.removeValue(forKey: key)?.values ?? Dictionary<UUID, URL>().values {
            io.stop(url)
        }
        defaults.removeObject(forKey: key)
    }

    func removeWorkspace(conversationID: String) { clearWorkspace(conversationID: conversationID) }

    func isAuthorizedWorkspace(_ url: URL, conversationID: String? = nil) -> Bool {
        if MacAppEnvironment.shared.containsManagedURL(url) { return true }
        return authorizedWorkspaceURLs(conversationID: conversationID).contains {
            MacAppEnvironment.contains(url, in: $0)
        }
    }

    /// Releases process leases without revoking the persisted user decisions.
    func close() {
        for grants in active.values {
            for url in grants.values { io.stop(url) }
        }
        active.removeAll()
    }
}

/// Decode only the versioned workspace_access capability contract. Arbitrary
/// file/shell arguments are not a request to widen filesystem authority.
nonisolated enum MacWorkspacePermissionPromptPolicy {
    static func coreGrantScope(
        toolName: String, requested: MacPermissionGrantScope, usesLocalRuntime: Bool
    ) -> MacPermissionGrantScope {
        usesLocalRuntime && toolName == "workspace_access" ? .once : requested
    }

    private struct WorkspaceAccess: Decodable {
        let root: String
    }

    static func requestedFolderURL(
        toolName: String,
        arguments: String,
        isAuthorized: (URL) -> Bool
    ) -> URL? {
        guard toolName == "workspace_access",
              let data = arguments.data(using: .utf8),
              let request = try? JSONDecoder().decode(WorkspaceAccess.self, from: data),
              request.root.hasPrefix("/") else { return nil }
        let folder = URL(fileURLWithPath: request.root).standardizedFileURL
        return isAuthorized(folder) ? nil : folder
    }
}
