// SPDX-License-Identifier: Apache-2.0
import Foundation
import Darwin

nonisolated struct MacModelImportPlan: Decodable, Sendable {
    let plan_id: String
    let candidates: [Candidate]
    let expires_at_ms: Int64
    struct Candidate: Decodable, Identifiable, Sendable {
        let candidate_id: String
        let connection: MacModelConnectionSnapshot.Connection
        let models: [MacModelConnectionSnapshot.Model]
        let credential_status: String
        let issue: String?
        let existing_revision: Int64?
        var id: String { candidate_id }
    }
}
nonisolated struct MacModelImportResult: Decodable, Identifiable, Sendable {
    let candidate_id: String
    let connection_id: String
    let status: String
    let error: String?
    var id: String { candidate_id }
}

nonisolated struct MacModelProbeConfirmation: Codable, Sendable {
    let request_id: String
    let connection_id: String
    let expected_revision: Int64
    let account_id: String?
    let account_revision: Int64?
    let credential_binding: String
    let model_id: String
    let `protocol`: String
    let max_requests: Int
    var acknowledge_possible_charges: Bool
}

/// Private service transport. Secret-bearing frames never reach a View, an
/// Application receipt, diagnostics, stdout logging or the configuration JSON.
nonisolated final class MacModelImportBridge: @unchecked Sendable {
    private let process: Process
    private let input: FileHandle
    private let output: FileHandle
    private let queue = DispatchQueue(label: "com.gaojc.Nola.model-import")
    private let credentials: MacAppleChatConfigurationStore
    private let application: MacApplicationPort
    private var buffer = Data()
    private var approved: MacModelImportPlan?
    private var mayStoreCredentials = false
    private var probeRoute: MacModelConnectionSnapshot?
    private var probeProtocol: String?
    private var probeConfirmation: MacModelProbeConfirmation?
    private let maximumFrame = 4 * 1024 * 1024

    init(application: MacApplicationPort) throws {
        self.application = application
        guard let executable = MacAgentHostExecutableLocator.resolve(), executable.isFileURL,
              FileManager.default.isExecutableFile(atPath: executable.path),
              (try executable.resourceValues(forKeys: [.isRegularFileKey])).isRegularFile == true else {
            throw MacApplicationPortError.rejected("The bundled Agent Host is unavailable. Reinstall or rebuild Nola before importing.")
        }
        credentials = MacAppleChatConfigurationStore(applicationPort: application)
        process = Process()
        let incoming = Pipe(), outgoing = Pipe()
        input = incoming.fileHandleForWriting; output = outgoing.fileHandleForReading
        process.executableURL = executable
        process.arguments = ["model-import-bridge", "--database", application.databaseURL.path]
        process.standardInput = incoming; process.standardOutput = outgoing
        // Child errors are mapped to fixed service errors, never forwarded.
        process.standardError = FileHandle.nullDevice
        try process.run()
    }

    #if DEBUG
    /// Isolated pipe fixture; never selected by product code or executable lookup.
    init(application: MacApplicationPort, fixtureInput: FileHandle, fixtureOutput: FileHandle) {
        self.application = application
        credentials = MacAppleChatConfigurationStore(applicationPort: application)
        process = Process()
        input = fixtureInput
        output = fixtureOutput
    }
    #endif

    deinit { cancel() }
    func cancel() {
        try? input.close()
        if process.isRunning { process.terminate() }
    }

    func preview(source: String, path: String, credentialPath: String?, includeCredentials: Bool, allowEnvironment: Bool) async throws -> MacModelImportPlan {
        let request: [String: Any] = ["source": source, "path": path,
            "credential_path": credentialPath.map { $0 as Any } ?? NSNull(), "include_credentials": includeCredentials,
            "allow_environment": allowEnvironment]
        let bytes = try JSONSerialization.data(withJSONObject: request)
        return try await operation { [self] in
            let response = try exchange(type: "plan", request: bytes, selected: [])
            guard let raw = response["plan"] else { throw invalidResponse() }
            let plan = try JSONDecoder().decode(MacModelImportPlan.self, from: JSONSerialization.data(withJSONObject: raw))
            approved = plan; mayStoreCredentials = includeCredentials
            return plan
        }
    }

    func commit(planID: String, selected: Set<String>, updateExisting: Set<String>, enableModels: Bool) async throws -> [MacModelImportResult] {
        let bytes = try JSONSerialization.data(withJSONObject: ["plan_id": planID, "selections": selected.sorted().map {
            ["candidate_id": $0, "display_name": NSNull(), "update_existing": updateExisting.contains($0), "enable_models": enableModels] as [String: Any]
        }])
        return try await operation { [self] in
            guard approved?.plan_id == planID, !selected.isEmpty,
                  selected.isSubset(of: Set(approved?.candidates.filter { $0.issue == nil }.map(\.id) ?? [])) else { throw invalidResponse() }
            defer { approved = nil; mayStoreCredentials = false }
            let response = try exchange(type: "commit", request: bytes, selected: selected)
            guard let raw = response["results"] else { throw invalidResponse() }
            return try JSONDecoder().decode([MacModelImportResult].self, from: JSONSerialization.data(withJSONObject: raw))
        }
    }

    func prepareProbe(snapshot: MacModelConnectionSnapshot, model: MacModelConnectionSnapshot.Model) async throws -> MacModelProbeConfirmation {
        let bytes = try JSONSerialization.data(withJSONObject: ["connection_id": snapshot.id,
            "expected_revision": snapshot.revision, "model_id": model.model_id, "protocol": model.protocol])
        return try await operation { [self] in
            probeRoute = snapshot; probeProtocol = model.protocol; probeConfirmation = nil
            let response = try exchange(type: "probe_prepare", request: bytes, selected: [])
            guard let raw = response["confirmation"] else { throw invalidResponse() }
            let confirmation = try JSONDecoder().decode(MacModelProbeConfirmation.self,
                from: JSONSerialization.data(withJSONObject: raw))
            let currentAccountRevision = try MacModelConnectionService(application: application).accounts()
                .first(where: { $0.id == snapshot.document.connection.account_id })?.revision
            guard confirmation.connection_id == snapshot.id, confirmation.expected_revision == snapshot.revision,
                  confirmation.account_id == snapshot.document.connection.account_id,
                  confirmation.account_revision == currentAccountRevision,
                  UUID(uuidString: confirmation.request_id) != nil, confirmation.credential_binding.count == 64,
                  confirmation.model_id == model.model_id, confirmation.protocol == model.protocol,
                  confirmation.max_requests == 1, !confirmation.acknowledge_possible_charges else { throw invalidResponse() }
            probeConfirmation = confirmation
            return confirmation
        }
    }

    func confirmProbe(requestID: String, acknowledgePossibleCharges: Bool) async throws -> MacModelConnectionSnapshot {
        try await operation { [self] in
            guard var confirmation = probeConfirmation, confirmation.request_id == requestID,
                  acknowledgePossibleCharges else { throw invalidResponse() }
            confirmation.acknowledge_possible_charges = true
            defer { probeConfirmation = nil; probeRoute = nil; probeProtocol = nil }
            let response = try exchange(type: "probe", request: JSONEncoder().encode(confirmation), selected: [])
            guard let raw = response["snapshot"] else { throw invalidResponse() }
            let updated = try JSONDecoder().decode(MacModelConnectionSnapshot.self,
                from: JSONSerialization.data(withJSONObject: raw))
            guard updated.id == confirmation.connection_id, updated.revision > confirmation.expected_revision,
                  updated.document.connection.account_id == confirmation.account_id,
                  updated.document.connection.base_url == probeRoute?.document.connection.base_url else { throw invalidResponse() }
            return updated
        }
    }

    private func operation<T: Sendable>(_ action: @escaping @Sendable () throws -> T) async throws -> T {
        try await withCheckedThrowingContinuation { continuation in
            queue.async { continuation.resume(with: Result { try action() }) }
        }
    }

    private func exchange(type: String, request: Data, selected: Set<String>) throws -> [String: Any] {
        let requestID = UUID().uuidString.lowercased()
        let deadline = DispatchWorkItem { [weak self] in self?.cancel() }
        DispatchQueue.global(qos: .utility).asyncAfter(deadline: .now() + 60, execute: deadline)
        var completed = false
        defer {
            deadline.cancel()
            // A rejected credential-read frame otherwise leaves the child
            // blocked on its private reply pipe after the timeout is removed.
            if !completed { cancel() }
        }
        try write(["type": type, "request_id": requestID, "request": JSONSerialization.jsonObject(with: request)])
        var acceptedConnections = Set<String>()
        var credentialReads = 0
        while true {
            let frame = try read()
            guard frame["request_id"] as? String == requestID, let kind = frame["type"] as? String else { throw invalidResponse() }
            if kind == "error" { throw MacApplicationPortError.rejected(type.hasPrefix("probe")
                ? "Text capability was not confirmed. A provider request may have been charged. Refresh before confirming a new request."
                : String(localized: "Import could not finish. Refresh the preview and check the selected source file. No source files were changed.", table: "ModelCenter")) }
            if kind == "credential_read_request" {
                credentialReads += 1
                guard type == "probe_prepare" || type == "probe",
                      credentialReads <= (type == "probe_prepare" ? 1 : 2),
                      let route = probeRoute, let wire = probeProtocol,
                      Self.matchesProbeCredentialBinding(frame, snapshot: route, wireProtocol: wire),
                      let nonce = frame["nonce"] as? String else { throw invalidResponse() }
                let service = MacModelConnectionService(application: application)
                let current = try service.connections().first(where: { $0.id == route.id })
                guard current?.revision == route.revision else { throw invalidResponse() }
                let material = try service.credential(connectionID: route.id,
                    endpoint: route.document.connection.base_url, wireProtocol: wire)
                try write(["type":"credential_read", "request_id":requestID, "nonce":nonce,
                    "secret":material.secret])
                continue
            }
            if kind == "credential_request" {
                guard type == "commit", mayStoreCredentials, let plan = approved,
                      let id = frame["connection_id"] as? String,
                      let candidate = plan.candidates.first(where: { selected.contains($0.id) && $0.connection.id == id }),
                      !acceptedConnections.contains(id),
                      Self.matchesCredentialBinding(frame, candidate: candidate),
                      let nonce = frame["nonce"] as? String, UUID(uuidString: nonce) != nil,
                      let secret = frame["secret"] as? String, !secret.isEmpty else { throw invalidResponse() }
                let reference = try credentials.storeAccountSecret(secret, connectionID: candidate.connection.id,
                    endpoint: candidate.connection.base_url, protocols: candidate.connection.protocols)
                acceptedConnections.insert(id)
                try write(["type": "credential_saved", "request_id": requestID, "nonce": nonce, "reference": reference])
                continue
            }
            guard kind == type else { throw invalidResponse() }
            completed = true
            return frame
        }
    }

    static func matchesCredentialBinding(_ frame: [String: Any], candidate: MacModelImportPlan.Candidate) -> Bool {
        guard frame["connection_id"] as? String == candidate.connection.id,
              frame["endpoint"] as? String == candidate.connection.base_url,
              frame["protocols"] as? [String] == candidate.connection.protocols,
              let account = frame["account_id"] as? String else { return false }
        if let existing = candidate.connection.account_id { return account == existing }
        guard account.hasPrefix("import-account-") else { return false }
        return UUID(uuidString: String(account.dropFirst("import-account-".count))) != nil
    }

    static func matchesProbeCredentialBinding(_ frame: [String: Any], snapshot: MacModelConnectionSnapshot,
        wireProtocol: String) -> Bool {
        guard snapshot.document.connection.protocols.contains(wireProtocol),
              frame["connection_id"] as? String == snapshot.id,
              frame["endpoint"] as? String == snapshot.document.connection.base_url,
              frame["protocol"] as? String == wireProtocol,
              (frame["account_id"] as? String) == snapshot.document.connection.account_id,
              let nonce = frame["nonce"] as? String, UUID(uuidString: nonce) != nil else { return false }
        return true
    }

    private func read() throws -> [String: Any] {
        while true {
            if let newline = buffer.firstIndex(of: 10) {
                let data = buffer.prefix(upTo: newline)
                let frame = try JSONSerialization.jsonObject(with: data) as? [String: Any]
                buffer.removeSubrange(...newline)
                guard let frame else { throw invalidResponse() }
                return frame
            }
            guard buffer.count <= maximumFrame else { throw invalidResponse() }
            // A live child waits for our reply after each frame. Foundation's
            // read(upToCount:) can wait to fill the requested count, deadlocking
            // that exchange; POSIX read returns the bytes currently delivered.
            var bytes = [UInt8](repeating: 0, count: 4096)
            let count = Darwin.read(output.fileDescriptor, &bytes, 4096)
            if count < 0 && errno == EINTR { continue }
            guard count > 0 else { throw invalidResponse() }
            buffer.append(contentsOf: bytes.prefix(count))
            guard buffer.count <= maximumFrame else { throw invalidResponse() }
        }
    }
    private func write(_ value: [String: Any]) throws {
        var data = try JSONSerialization.data(withJSONObject: value)
        guard data.count < maximumFrame else { throw invalidResponse() }
        data.append(10); try input.write(contentsOf: data)
    }
    private func invalidResponse() -> Error {
        MacApplicationPortError.rejected("The import service closed or returned an invalid response. Preview again; check connections before retrying a confirmation.")
    }
}
