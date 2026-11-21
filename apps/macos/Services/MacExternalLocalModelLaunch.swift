// SPDX-License-Identifier: Apache-2.0
import Darwin
import Foundation

nonisolated struct MacExternalLocalModelLaunch {
    let executable: URL
    let arguments: [String]
    let port: UInt16

    init(profile: MacLocalModelProfile, token: String, context: Int,
         environment: [String: String] = ProcessInfo.processInfo.environment) throws {
        guard let engine = profile.engine, engine != .mlx, let directory = profile.installedDirectory else {
            throw CocoaError(.fileReadCorruptFile)
        }
        let variable = engine == .llamaCpp ? "NOLA_LLAMA_SERVER" : "NOLA_VLLM_BIN"
        let binaries = engine == .llamaCpp ? ["llama-server", "llama"] : ["vllm"]
        let candidate: String?
        if let configured = environment[variable], engine == .vllm || (configured.hasPrefix("/") && Self.isExecutable(configured)) {
            candidate = configured
        } else {
            let paths = MacAppEnvironment.buildLaunchAgentPath(inheritedPath: environment["PATH"],
                userHome: FileManager.default.homeDirectoryForCurrentUser)
            candidate = paths.split(separator: ":").flatMap { directory in
                binaries.map { URL(fileURLWithPath: String(directory)).appendingPathComponent($0).path }
            }.first(where: Self.isExecutable)
        }
        guard let candidate, candidate.hasPrefix("/"), Self.isExecutable(candidate) else {
            throw MacLocalModelChatStartupError.unavailable(String(format: String(localized:
                "Install %@ and make %@ available in PATH, or set %@ to its executable.", table: "ModelCenter"),
                engine.title, binaries[0], variable))
        }
        executable = URL(fileURLWithPath: candidate)
        port = try Self.availablePort()
        let network = ["--host", "127.0.0.1", "--port", String(port), "--api-key", token]
        if engine == .llamaCpp {
            guard let artifact = profile.artifactURL else { throw CocoaError(.fileReadCorruptFile) }
            let device = profile.recommendedDevice == "cpu" ? ["--n-gpu-layers", "0"] : []
            let subcommand = executable.lastPathComponent == "llama" ? ["serve"] : []
            arguments = subcommand + ["--model", artifact.path, "--alias", profile.id, "--ctx-size", String(context),
                         "--parallel", "1", "--jinja"] + device + network
        } else {
            arguments = ["serve", directory.path, "--served-model-name", profile.id, "--max-model-len", String(context)] + network
        }
    }

    private static func isExecutable(_ path: String) -> Bool {
        var directory: ObjCBool = false
        return FileManager.default.fileExists(atPath: path, isDirectory: &directory) && !directory.boolValue
            && FileManager.default.isExecutableFile(atPath: path)
    }

    private static func availablePort() throws -> UInt16 {
        let socket = Darwin.socket(AF_INET, SOCK_STREAM, 0)
        guard socket >= 0 else { throw POSIXError(.EADDRNOTAVAIL) }
        defer { Darwin.close(socket) }
        var address = sockaddr_in()
        address.sin_len = UInt8(MemoryLayout<sockaddr_in>.size)
        address.sin_family = sa_family_t(AF_INET)
        address.sin_addr.s_addr = inet_addr("127.0.0.1")
        var length = socklen_t(MemoryLayout<sockaddr_in>.size)
        let result = withUnsafeMutablePointer(to: &address) {
            $0.withMemoryRebound(to: sockaddr.self, capacity: 1) {
                guard Darwin.bind(socket, $0, length) == 0 else { return false }
                return getsockname(socket, $0, &length) == 0
            }
        }
        guard result else { throw POSIXError(.EADDRNOTAVAIL) }
        return UInt16(bigEndian: address.sin_port)
    }

    private struct Models: Decodable {
        struct Model: Decodable { let id: String }
        let data: [Model]
    }

    func waitForModel(_ id: String, token: String, process: Process) async throws {
        let configuration = URLSessionConfiguration.ephemeral
        configuration.timeoutIntervalForRequest = 2
        configuration.connectionProxyDictionary = [:]
        let session = URLSession(configuration: configuration)
        defer { session.invalidateAndCancel() }
        let url = URL(string: "http://127.0.0.1:\(port)/v1/models")!
        var request = URLRequest(url: url)
        request.setValue("Bearer " + token, forHTTPHeaderField: "Authorization")
        let deadline = ContinuousClock.now + .seconds(300)
        while ContinuousClock.now < deadline {
            try Task.checkCancellation()
            guard process.isRunning else {
                throw MacLocalModelChatStartupError.unavailable(String(format: String(localized:
                    "The local model service exited with status %d.", table: "ModelCenter"), process.terminationStatus))
            }
            if let (data, response) = try? await session.data(for: request),
               response.url == url, (response as? HTTPURLResponse)?.statusCode == 200,
               let models = try? JSONDecoder().decode(Models.self, from: data),
               models.data.contains(where: { $0.id == id }) { return }
            try await Task.sleep(for: .milliseconds(250))
        }
        throw MacLocalModelChatStartupError.unavailable(String(localized:
            "The engine did not load this model within five minutes.", table: "ModelCenter"))
    }
}
