// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Defines the app-scoped filesystem and subprocess environment used by Nola for Mac.

import Foundation

nonisolated struct MacAppEnvironment: Sendable {
    // PIDs are reused across launches; a test process must never reopen a
    // previous run's persistence while all environments in this run agree.
    private static let testRunIdentifier = UUID().uuidString
    static let shared = MacAppEnvironment()
    static let canonicalStoreFileName = "nola-v4.db"

    let supportRoot: URL
    let cacheRoot: URL
    let runtimeHome: URL
    let configDirectory: URL
    let skillsDirectory: URL
    let stateDirectory: URL
    let databaseURL: URL
    let scratchWorkspaceURL: URL
    let sessionWorkspacesDirectory: URL
    let temporaryDirectory: URL

    init(
        applicationSupportRoot: URL? = nil,
        cachesRoot: URL? = nil,
        fileManager: FileManager = .default,
        environment: [String: String] = ProcessInfo.processInfo.environment
    ) {
        let testRoot: URL? = {
            guard applicationSupportRoot == nil,
                  environment["NOLA_MAC_APP_SUPPORT"] == nil,
                  MacProcessRuntimePolicy.isRunningTests else { return nil }
            return fileManager.temporaryDirectory
                .appendingPathComponent(
                    "Nola-XCTest-\(ProcessInfo.processInfo.processIdentifier)-\(Self.testRunIdentifier)",
                    isDirectory: true
                )
        }()
        if let applicationSupportRoot {
            supportRoot = applicationSupportRoot.appendingPathComponent("Nola", isDirectory: true)
        } else if let testRoot {
            supportRoot = testRoot.appendingPathComponent("ApplicationSupport/Nola", isDirectory: true)
        } else if let configured = environment["NOLA_MAC_APP_SUPPORT"]?
            .trimmingCharacters(in: .whitespacesAndNewlines),
                  !configured.isEmpty {
            // This value is also handed directly to the Rust Host. It is the
            // final Nola support root, not the parent Application Support URL.
            supportRoot = URL(fileURLWithPath: configured, isDirectory: true)
        } else {
            let support = fileManager.urls(for: .applicationSupportDirectory, in: .userDomainMask).first
                ?? fileManager.temporaryDirectory
            supportRoot = support.appendingPathComponent("Nola", isDirectory: true)
        }
        if let cachesRoot {
            cacheRoot = cachesRoot.appendingPathComponent("Nola", isDirectory: true)
        } else if let testRoot {
            cacheRoot = testRoot.appendingPathComponent("Caches/Nola", isDirectory: true)
        } else if let configured = environment["NOLA_MAC_CACHE_ROOT"]?
            .trimmingCharacters(in: .whitespacesAndNewlines),
                  !configured.isEmpty {
            cacheRoot = URL(fileURLWithPath: configured, isDirectory: true)
        } else {
            let caches = fileManager.urls(for: .cachesDirectory, in: .userDomainMask).first
                ?? fileManager.temporaryDirectory
            cacheRoot = caches.appendingPathComponent("Nola", isDirectory: true)
        }
        runtimeHome = supportRoot
            .appendingPathComponent("Runtime", isDirectory: true)
            .appendingPathComponent("Home", isDirectory: true)
        configDirectory = runtimeHome.appendingPathComponent(".nola", isDirectory: true)
        skillsDirectory = configDirectory.appendingPathComponent("skills", isDirectory: true)
        stateDirectory = supportRoot.appendingPathComponent("State", isDirectory: true)
        databaseURL = stateDirectory.appendingPathComponent(
            Self.canonicalStoreFileName,
            isDirectory: false
        )
        scratchWorkspaceURL = supportRoot
            .appendingPathComponent("Workspaces", isDirectory: true)
            .appendingPathComponent("Default", isDirectory: true)
        sessionWorkspacesDirectory = supportRoot
            .appendingPathComponent("Workspaces", isDirectory: true)
            .appendingPathComponent("Sessions", isDirectory: true)
        temporaryDirectory = cacheRoot.appendingPathComponent("Temporary", isDirectory: true)
        try? prepare(fileManager: fileManager)
    }

    func prepare(fileManager: FileManager = .default) throws {
        for directory in [
            supportRoot,
            cacheRoot,
            runtimeHome,
            configDirectory,
            skillsDirectory,
            stateDirectory,
            scratchWorkspaceURL,
            sessionWorkspacesDirectory,
            temporaryDirectory,
        ] {
            try fileManager.createDirectory(at: directory, withIntermediateDirectories: true)
        }
    }

    func sessionWorkspaceURL(
        for conversationID: String,
        fileManager: FileManager = .default
    ) -> URL {
        let url = sessionWorkspaceLocation(for: conversationID)
        try? fileManager.createDirectory(
            at: url,
            withIntermediateDirectories: true
        )
        return url
    }

    func removeSessionWorkspace(
        for conversationID: String,
        fileManager: FileManager = .default
    ) throws {
        let url = sessionWorkspaceLocation(for: conversationID)
        guard fileManager.fileExists(atPath: url.path) else { return }
        try fileManager.removeItem(at: url)
    }

    func processEnvironment(
        base: [String: String] = ProcessInfo.processInfo.environment,
        projectURL: URL
    ) -> [String: String] {
        try? prepare()
        var environment = base
        for (key, value) in isolatedVariables(projectURL: projectURL) {
            environment[key] = value
        }
        return environment
    }

    func launchAgentEnvironment(projectURL: URL) -> [String: String] {
        var environment = isolatedVariables(projectURL: projectURL)
        let userHome = FileManager.default.homeDirectoryForCurrentUser
        let inheritedPaths = (ProcessInfo.processInfo.environment["PATH"] ?? "")
            .split(separator: ":")
            .map(String.init)
        let desktopPaths = [
            userHome.appendingPathComponent(".local/bin", isDirectory: true).path,
            userHome.appendingPathComponent(".cargo/bin", isDirectory: true).path,
            "/opt/homebrew/bin",
            "/opt/homebrew/sbin",
            "/usr/local/bin",
            "/usr/bin",
            "/bin",
            "/usr/sbin",
            "/sbin",
        ]
        environment["PATH"] = (inheritedPaths + desktopPaths).reduce(into: [String]()) { paths, path in
            guard !path.isEmpty, !paths.contains(path) else { return }
            paths.append(path)
        }.joined(separator: ":")
        // Keep the Nola Host's HOME isolated while allowing an explicitly
        // selected external execution unit to use its own install and auth state.
        environment["NOLA_EXTERNAL_EXECUTION_HOME"] = userHome.path
        environment["NOLA_DESKTOP_HOST"] = "1"
        environment["NOLA_CLIENT_SURFACE"] = "mac"
        environment["NOLA_AGENT_HOST_PROTOCOL"] = "1"
        environment["NOLA_NATIVE_PERMISSION_MODE"] = "1"
        if MacProcessRuntimePolicy.isRunningTests {
            // The application service and Agent Host are real child processes.
            // Some host E2E tests intentionally kill them to verify recovery;
            // keep those expected test exits from opening macOS crash UI.
            environment["CRASHREPORTER_DISABLE"] = "1"
            environment["CRASH_REPORTER_DISABLE"] = "1"
        }
        return environment
    }

    func containsManagedURL(_ url: URL) -> Bool {
        Self.contains(url, in: supportRoot) || Self.contains(url, in: cacheRoot)
    }

    static func contains(_ candidate: URL, in root: URL) -> Bool {
        let candidatePath = candidate.standardizedFileURL.resolvingSymlinksInPath().path
        let rootPath = root.standardizedFileURL.resolvingSymlinksInPath().path
        return candidatePath == rootPath || candidatePath.hasPrefix(rootPath + "/")
    }

    private static func safeWorkspaceIdentifier(_ value: String) -> String {
        let normalized = value.unicodeScalars.map { scalar -> Character in
            if CharacterSet.alphanumerics.contains(scalar)
                || scalar.value == 45
                || scalar.value == 95 {
                return Character(String(scalar))
            }
            return "_"
        }
        let identifier = String(normalized.prefix(80))
            .trimmingCharacters(in: CharacterSet(charactersIn: "_"))
        return identifier.isEmpty ? "conversation" : identifier
    }

    private func sessionWorkspaceLocation(for conversationID: String) -> URL {
        sessionWorkspacesDirectory.appendingPathComponent(
            Self.safeWorkspaceIdentifier(conversationID),
            isDirectory: true
        )
    }

    private func isolatedVariables(projectURL: URL) -> [String: String] {
        [
            "HOME": runtimeHome.path,
            "XDG_CONFIG_HOME": supportRoot.appendingPathComponent("Configuration", isDirectory: true).path,
            "XDG_DATA_HOME": supportRoot.appendingPathComponent("Data", isDirectory: true).path,
            "XDG_CACHE_HOME": cacheRoot.path,
            "TMPDIR": temporaryDirectory.path,
            "NOLA_DB_PATH": databaseURL.path,
            "NOLA_CLI_PROJECT_ROOT": projectURL.path,
            "NOLA_PROJECT_ROOT": projectURL.path,
            "NOLA_CLI_ALLOWED_DIRS": projectURL.path,
            "NOLA_MAC_APP_SUPPORT": supportRoot.path,
        ]
    }
}
