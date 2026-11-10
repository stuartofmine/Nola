// SPDX-License-Identifier: Apache-2.0
import SwiftUI
import NolaCore
#if os(macOS)
import AppKit
#else
import UIKit
#endif

nonisolated struct SoftwareUpdateArtifact: Decodable, Sendable {
    let kind: String
    let url: String
    let filename: String?
    let size: UInt64?
    let sha256: String?
}
nonisolated struct SoftwareUpdateOffer: Decodable, Sendable {
    let release_id: String
    let version: String
    let build: UInt64
    let notes: String
    let artifact: SoftwareUpdateArtifact
}
nonisolated struct SoftwareUpdateSnapshot: Decodable, Sendable {
    let status: String
    let current_version: String
    let current_build: UInt64
    let channel: String
    let target: String
    let release: SoftwareUpdateOffer?
}
nonisolated enum AppleSoftwareUpdateClient {
    private struct Failure: Decodable { let code: String; let message: String }
    private struct Reply<T: Decodable>: Decodable { let ok: Bool; let value: T?; let error: Failure? }
    struct UpdateFailure: LocalizedError {
        let detail: String
        var errorDescription: String? { detail }
    }
    static var target: String {
        #if os(iOS)
        "ios"
        #elseif arch(arm64)
        "macos-aarch64"
        #else
        "macos-x86_64"
        #endif
    }
    static func call<T: Decodable & Sendable>(_ operation: String, id: String? = nil, directory: URL? = nil) async throws -> T {
        var request: [String: String] = ["operation": operation, "target": target]
        if let id { request["expected_release_id"] = id }
        if let directory { request["directory"] = directory.path }
        let input = try JSONSerialization.data(withJSONObject: request)
        let output = try await Task.detached(priority: .utility) {
            try input.withUnsafeBytes { bytes -> Data in
                guard let result = nola_core_software_update_json(bytes.bindMemory(to: UInt8.self).baseAddress, bytes.count) else {
                    throw UpdateFailure(detail: "Update service unavailable")
                }
                defer { nola_core_free_c_string(result) }
                return Data(String(cString: result).utf8)
            }
        }.value
        try Task.checkCancellation()
        let reply = try JSONDecoder().decode(Reply<T>.self, from: output)
        guard reply.ok, let value = reply.value else { throw UpdateFailure(detail: reply.error?.message ?? "Update service unavailable") }
        return value
    }
}

struct AppleSoftwareUpdateSection: View {
    @State private var snapshot: SoftwareUpdateSnapshot?
    @State private var busy = false
    @State private var operationID = UUID()
    @State private var failure: String?
    @State private var package: URL?
    @State private var actionTask: Task<Void, Never>?
    @Environment(\.openURL) private var openURL

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            Text("softwareUpdate.title").font(.headline)
            if let snapshot {
                Text("Nola \(snapshot.current_version) (\(snapshot.current_build))").font(.subheadline)
                Text(LocalizedStringKey("softwareUpdate.status.\(snapshot.status)")).foregroundStyle(.secondary)
                if let release = snapshot.release {
                    Text("\(release.version) (\(release.build))").font(.subheadline.weight(.medium))
                    if !release.notes.isEmpty { Text(release.notes).textSelection(.enabled) }
                    if release.artifact.kind == "store", let url = URL(string: release.artifact.url) {
                        Button("softwareUpdate.store") { openURL(url) }.disabled(busy)
                    } else {
                        #if os(macOS)
                        Button("softwareUpdate.download") { download(release) }.disabled(busy || package != nil)
                        #endif
                    }
                }
            } else { Text("softwareUpdate.notChecked").foregroundStyle(.secondary) }
            if busy { ProgressView().controlSize(.small).accessibilityLabel(Text("softwareUpdate.working")) }
            if let failure {
                VStack(alignment: .leading, spacing: 4) {
                    Text("softwareUpdate.failed").foregroundStyle(.red)
                    Text(failure).font(.caption).textSelection(.enabled)
                }.accessibilityElement(children: .combine)
            }
            if let package {
                Text("softwareUpdate.verified").foregroundStyle(.secondary)
                #if os(macOS)
                Button("softwareUpdate.reveal") { NSWorkspace.shared.activateFileViewerSelecting([package]) }
                Button("softwareUpdate.install") {
                    // Explicit action: opening an installer never silently quits a
                    // running Host or discards a conversation draft.
                    NSWorkspace.shared.open(package, configuration: .init()) { _, error in
                        if let error { Task { @MainActor in failure = error.localizedDescription } }
                    }
                }
                #endif
            }
            Button("softwareUpdate.check") { check() }.disabled(busy)
                .accessibilityIdentifier("settings.softwareUpdate.check")
            Text("softwareUpdate.dataHint").font(.caption).foregroundStyle(.secondary)
        }
        .frame(maxWidth: .infinity, alignment: .leading)
        .onDisappear { operationID = UUID(); actionTask?.cancel(); busy = false }
    }
    private func check() {
        busy = true; failure = nil; package = nil
        actionTask?.cancel()
        let owner = UUID(); operationID = owner
        actionTask = Task {
            defer { if operationID == owner { busy = false } }
            do { let result: SoftwareUpdateSnapshot = try await AppleSoftwareUpdateClient.call("check"); snapshot = result }
            catch is CancellationError {} catch { failure = error.localizedDescription; snapshot = nil }
        }
    }
    #if os(macOS)
    private func download(_ release: SoftwareUpdateOffer) {
        busy = true; failure = nil
        actionTask?.cancel()
        let owner = UUID(); operationID = owner
        actionTask = Task {
            defer { if operationID == owner { busy = false } }
            do {
                struct Download: Decodable, Sendable { let path: String }
                let directory = try FileManager.default.url(for: .cachesDirectory, in: .userDomainMask, appropriateFor: nil, create: true)
                    .appendingPathComponent("Nola/SoftwareUpdates/\(UUID().uuidString)", isDirectory: true)
                let result: Download = try await AppleSoftwareUpdateClient.call("download", id: release.release_id, directory: directory)
                try Task.checkCancellation()
                let url = URL(fileURLWithPath: result.path)
                // Mark this as an internet download so the normal macOS security
                // assessment remains in the system's installation workflow.
                try (url as NSURL).setResourceValue([
                    "LSQuarantineType": "LSQuarantineTypeOtherDownload",
                    "LSQuarantineAgentName": "Nola",
                    "LSQuarantineTimeStamp": Date()
                ], forKey: .quarantinePropertiesKey)
                package = url
            } catch is CancellationError {} catch { failure = error.localizedDescription }
        }
    }
    #endif
}
