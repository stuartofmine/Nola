// SPDX-License-Identifier: Apache-2.0
import AppKit
import SwiftUI

struct MacLocalEngineSetupView: View {
    @ObservedObject var acquisition: MacLocalModelAcquisition
    let engine: MacLocalInferenceEngine
    let modelDownloaded: Bool
    var useMLX: (() -> Void)?

    var body: some View {
        VStack(alignment: .leading, spacing: 7) {
            switch acquisition.engines[engine] {
            case nil, .checking:
                HStack {
                    ProgressView().controlSize(.small)
                    Text(String(localized: "Checking engine…", table: "ModelCenter"))
                }
            case .installing:
                HStack {
                    ProgressView().controlSize(.small)
                    Text(String(format: String(localized: "Installing %@ with Homebrew…", table: "ModelCenter"), engine.title))
                }
            case .detected(let status):
                statusContent(status)
            case .failed(let status, let message):
                Text(verbatim: message).foregroundStyle(.red).textSelection(.enabled)
                    .accessibilityIdentifier("local-engine-error")
                if let status, !status.ready { statusContent(status) } else { refresh }
            }
        }
        .font(.callout)
        .frame(maxWidth: .infinity, alignment: .leading)
        .accessibilityElement(children: .contain)
        .accessibilityIdentifier("local-engine-status")
        .onChange(of: engine, initial: true) { _, value in acquisition.checkEngine(value) }
        .onReceive(NotificationCenter.default.publisher(for: NSApplication.didBecomeActiveNotification)) { _ in
            acquisition.checkEngine(engine)
        }
    }

    @ViewBuilder private func statusContent(_ status: MacModelAcquisitionEvent.EngineStatus) -> some View {
        HStack {
            Label(status.ready
                ? String(format: String(localized: "%@ is ready", table: "ModelCenter"), engine.title)
                : String(format: String(localized: "%@ is not ready", table: "ModelCenter"), engine.title),
                  systemImage: status.ready ? "checkmark.circle" : "exclamationmark.circle")
            Spacer()
            refresh
        }
        if !status.ready {
            Text(detail(status.reason)).foregroundStyle(.secondary)
            HStack(spacing: 12) {
                if status.canInstall {
                    Button(String(format: String(localized: "Install %@", table: "ModelCenter"), engine.title)) {
                        acquisition.checkEngine(engine, install: true)
                    }
                    .disabled(acquisition.engineOperations.contains(engine))
                    .accessibilityIdentifier("local-engine-install")
                }
                if let raw = status.guideUrl, let url = URL(string: raw), url.scheme == "https", url.host != nil {
                    Link(String(localized: "Installation guide", table: "ModelCenter"), destination: url)
                }
                #if arch(arm64)
                if engine != .mlx, !status.canInstall, let useMLX {
                    Button(String(localized: "Use built-in MLX", table: "ModelCenter"), action: useMLX)
                        .disabled(acquisition.downloading)
                        .accessibilityIdentifier("local-engine-use-mlx")
                }
                #endif
            }
        }
    }

    private var refresh: some View {
        Button { acquisition.checkEngine(engine) } label: {
            Image(systemName: "arrow.clockwise")
                .frame(width: 22, height: 22)
                .accessibilityLabel(String(localized: "Check engine again", table: "ModelCenter"))
        }
        .buttonStyle(.borderless)
        .disabled(acquisition.engineOperations.contains(engine))
        .help(String(localized: "Check engine again", table: "ModelCenter"))
        .accessibilityIdentifier("local-engine-refresh")
    }

    private func detail(_ reason: String) -> String {
        let instruction = switch reason {
        case "installable": String(localized: "Homebrew can install this engine.", table: "ModelCenter")
        case "homebrew_missing": String(localized: "Homebrew is unavailable on this Mac.", table: "ModelCenter")
        case "unsupported": String(localized: "This engine does not support this Mac.", table: "ModelCenter")
        case "invalid_override": String(format: String(localized: "Check the executable configured by %@.", table: "ModelCenter"),
            engine == .llamaCpp ? "NOLA_LLAMA_SERVER" : "NOLA_VLLM_BIN")
        default: String(localized: "Follow the installation guide for this engine.", table: "ModelCenter")
        }
        let availability = modelDownloaded
            ? String(localized: "The model is downloaded. Chat requires a working engine.", table: "ModelCenter")
            : String(localized: "You can download models now and chat after setup.", table: "ModelCenter")
        return instruction + " " + availability
    }
}
