// SPDX-License-Identifier: Apache-2.0
import Foundation

nonisolated struct MacDesktopPluginPage: Decodable, Identifiable, Equatable, Sendable {
    let id: String
    let title: String
    let package: String
}

nonisolated struct MacDesktopPluginDocument: Decodable, Sendable {
    let id: String
    let html: String
}

nonisolated struct MacDesktopPluginInventory: Decodable, Sendable {
    let available: Bool
    let packages: [String]
}

nonisolated struct MacDesktopPluginLibrary: Decodable, Sendable {
    struct Page: Decodable, Identifiable { let id: String; let title: String }
    struct ExternalLink: Decodable { let title: String; let url: URL }
    struct Package: Decodable, Identifiable {
        let source: String
        let pages: [Page]
        let links: [ExternalLink]?
        var id: String { source }
    }
    struct Profile: Decodable, Identifiable {
        let runtime: String
        let profile: String
        let available: Bool
        let packages: [Package]
        let error: String?
        var id: String { runtime + "/" + profile }
        var title: String { runtime == "pi" ? "Pi" : "DeepSeek Harness · " + (profile == "web" ? "Web" : "TUI") }
    }
    let profiles: [Profile]
    var workspacePages: [MacDesktopPluginPage] {
        profiles.flatMap { profile in
            profile.packages.flatMap { package in
                package.pages.map { MacDesktopPluginPage(id: $0.id, title: $0.title, package: package.source) }
            }
        }
    }
}

extension Notification.Name {
    static let macDesktopPluginsChanged = Notification.Name("macDesktopPluginsChanged")
}

nonisolated struct MacDesktopPluginClient: Sendable {
    let environment: MacAppEnvironment
    init(environment: MacAppEnvironment = .shared) { self.environment = environment }

    func library() async throws -> MacDesktopPluginLibrary {
        try await request(["desktop-library"])
    }

    func pages() async throws -> [MacDesktopPluginPage] {
        try await request(["desktop-pages"])
    }

    func document(_ id: String) async throws -> MacDesktopPluginDocument {
        let document: MacDesktopPluginDocument = try await request(["desktop-pages", id])
        guard document.id == id else { throw CocoaError(.fileReadCorruptFile) }
        return document
    }

    func manage(runtime: String, profile: String, action: String, source: String? = nil) async throws -> MacDesktopPluginInventory {
        try await request(["desktop-manage", runtime, profile, action] + (source.map { [$0] } ?? []))
    }

    private func request<T: Decodable & Sendable>(_ arguments: [String]) async throws -> T {
        guard let executable = Bundle.main.url(forAuxiliaryExecutable: "nola-agent-host") else {
            throw CocoaError(.executableNotLoadable)
        }
        let process = Process()
        process.executableURL = executable
        process.arguments = ["plugin"] + arguments
        process.environment = environment.launchAgentEnvironment(projectURL: environment.scratchWorkspaceURL)
        process.currentDirectoryURL = environment.scratchWorkspaceURL
        let session = MacLocalAgentProcessSession(process: process, inputPipe: Pipe(), outputPipe: Pipe(), errorPipe: Pipe())
        try Task.checkCancellation()
        try session.launch()
        try? session.input.close()
        let stdout = Task.detached { Self.drain(session.output, limit: 16 * 1024 * 1024) }
        let stderr = Task.detached { Self.drain(session.error, limit: 8192) }
        let exit = await withTaskCancellationHandler {
            await session.waitForExit()
        } onCancel: {
            Task { await session.shutdownAndDrain() }
        }
        let data = await stdout.value
        let error = await stderr.value
        session.releaseResources()
        try Task.checkCancellation()
        guard exit.status == 0, !exit.signaled else {
            throw MacLocalAgentRuntimeError.processFailed(exit.status, String(decoding: error, as: UTF8.self))
        }
        return try JSONDecoder().decode(T.self, from: data)
    }

    private static func drain(_ handle: FileHandle, limit: Int) -> Data {
        var result = Data()
        while let chunk = try? handle.read(upToCount: 32_768), !chunk.isEmpty {
            result.append(chunk.prefix(max(0, limit - result.count)))
        }
        return result
    }
}
