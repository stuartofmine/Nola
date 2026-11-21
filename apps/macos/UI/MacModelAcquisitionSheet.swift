// SPDX-License-Identifier: Apache-2.0
import SwiftUI

struct MacModelAcquisitionSheet: View {
    @ObservedObject var acquisition: MacLocalModelAcquisition
    @Binding var source: String
    @AppStorage("nola.local-model.engine") private var engine = MacLocalInferenceEngine.defaultForMac
    @State private var suppressAutomaticSearch = false
    @FocusState private var nameFocused: Bool
    @Environment(\.dismiss) private var dismiss

    var body: some View {
        VStack(alignment: .leading, spacing: 16) {
            Text(String(localized: "Download a model", table: "ModelCenter")).font(.title2.bold())
            hardwareSummary
            Grid(alignment: .leading, horizontalSpacing: 14, verticalSpacing: 10) {
                GridRow {
                    Text(String(localized: "Inference engine", table: "ModelCenter"))
                    Picker(String(localized: "Inference engine", table: "ModelCenter"), selection: $engine) {
                        ForEach(MacLocalInferenceEngine.allCases, id: \.self) { value in Text(verbatim: value.title).tag(value) }
                    }.labelsHidden().accessibilityIdentifier("local-acquire-engine")
                }
                GridRow {
                    Text(String(localized: "Download Source", table: "ModelCenter"))
                    Picker(String(localized: "Download Source", table: "ModelCenter"), selection: $source) {
                        Text(verbatim: "Hugging Face").tag("huggingface")
                        Text(verbatim: "ModelScope").tag("modelscope")
                    }.labelsHidden().accessibilityIdentifier("local-acquire-source")
                }
                GridRow {
                    Text(String(localized: "Model name", table: "ModelCenter"))
                    TextField(String(localized: "Model name", table: "ModelCenter"), text: $acquisition.query,
                        prompt: Text(String(localized: "Optional · recommend a model for this Mac", table: "ModelCenter")))
                        .focused($nameFocused)
                        .accessibilityLabel(String(localized: "Model name", table: "ModelCenter"))
                        .accessibilityIdentifier("local-acquire-name")
                        .onSubmit { recommend() }
                }
            }.disabled(acquisition.downloading)
            MacLocalEngineSetupView(acquisition: acquisition, engine: engine, modelDownloaded: false) {
                acquisition.query = ""
                engine = .mlx
            }
            switch acquisition.phase {
            case .idle, .installed:
                EmptyView()
            case .searching:
                HStack {
                    ProgressView().controlSize(.small)
                    Text(String(localized: "Finding compatible models…", table: "ModelCenter"))
                }
            case .choosing:
                if acquisition.candidates.isEmpty {
                    VStack(alignment: .leading, spacing: 4) {
                        Text(String(localized: "No compatible model found", table: "ModelCenter"))
                        if let reason = acquisition.reason { Text(verbatim: reason).font(.callout).foregroundStyle(.secondary) }
                    }
                } else {
                    Divider()
                    if let origin = acquisition.recommendationSource, ["modelfit", "hub", "mixed"].contains(origin) {
                        HStack(spacing: 8) {
                            if origin != "hub" {
                                Link("ModelFit · CC BY 4.0", destination: URL(string: "https://modelfit.io/data/")!)
                            }
                            if origin != "modelfit" {
                                Text(verbatim: String(localized: "Recommended from", table: "ModelCenter") + " " +
                                    (source == "modelscope" ? "ModelScope" : "Hugging Face")).foregroundStyle(.secondary)
                            }
                        }
                        .font(.caption)
                    }
                    ScrollView {
                        VStack(alignment: .leading, spacing: 12) {
                            ForEach(acquisition.candidates.indices, id: \.self) { index in
                                candidateRow(acquisition.candidates[index], index: index)
                                if index + 1 < acquisition.candidates.count { Divider() }
                            }
                        }
                    }
                    .frame(height: min(240, CGFloat(acquisition.candidates.count) * 132))
                }
            case .downloading(let repository):
                VStack(alignment: .leading, spacing: 6) {
                    Text(verbatim: repository).font(.callout).lineLimit(2)
                    if acquisition.total > 0 {
                        ProgressView(value: Double(acquisition.received), total: Double(acquisition.total))
                        Text(verbatim: "\(bytes(acquisition.received)) / \(bytes(acquisition.total))")
                            .font(.caption.monospacedDigit()).foregroundStyle(.secondary)
                    } else {
                        ProgressView().controlSize(.small)
                    }
                }
            case .failed(let detail):
                Text(verbatim: detail).foregroundStyle(.red).textSelection(.enabled)
                    .accessibilityIdentifier("local-acquire-error")
            }
            HStack {
                if acquisition.busy {
                    Button(String(localized: "Cancel", table: "ModelCenter")) {
                        suppressAutomaticSearch = true
                        acquisition.reset()
                    }
                }
                Spacer()
                Button(String(localized: "Close", table: "ModelCenter")) { dismiss() }
                    .keyboardShortcut(.cancelAction)
                if !acquisition.busy && (acquisition.phase != .choosing || acquisition.candidates.isEmpty) {
                    Button(String(localized: "Find recommendations", table: "ModelCenter"), action: recommend)
                        .keyboardShortcut(.defaultAction)
                        .accessibilityIdentifier("local-acquire-submit")
                }
            }
        }
        .padding(24).frame(minWidth: 540, idealWidth: 600)
        .onAppear { nameFocused = !acquisition.downloading }
        .task {
            acquisition.updateInputs(engine: engine, source: source)
            await acquisition.loadHardware()
            if !Task.isCancelled && !suppressAutomaticSearch {
                acquisition.recommendOnOpen(engine: engine, source: source)
            }
        }
        .onChange(of: acquisition.query) { inputsChanged() }
        .onChange(of: engine) { inputsChanged() }
        .onChange(of: source) { inputsChanged() }
    }

    @ViewBuilder private var hardwareSummary: some View {
        VStack(alignment: .leading, spacing: 6) {
            Label(String(localized: "This Mac", table: "ModelCenter"), systemImage: "desktopcomputer")
                .font(.headline)
            if let hw = acquisition.hardware {
                let cpu = [hw.cpu, hw.cores.map { String(format: String(localized: "%lld CPU cores", table: "ModelCenter"), $0) }]
                    .compactMap { $0 }.joined(separator: " · ")
                if !cpu.isEmpty { Text(verbatim: cpu) }
                if let gpu = hw.gpu {
                    let memory = hw.unifiedMemory ? nil : hw.gpuMemoryBytes.map(memorySize)
                    Text(verbatim: ["GPU: " + gpu, memory].compactMap { $0 }.joined(separator: " · "))
                }
                let memoryTitle = hw.unifiedMemory ? String(localized: "Unified memory", table: "ModelCenter")
                    : String(localized: "Memory", table: "ModelCenter")
                HStack(spacing: 16) {
                    if let memory = hw.memoryBytes { Text(verbatim: memoryTitle + "  " + memorySize(memory)) }
                    if let disk = hw.freeDiskBytes {
                        Text(verbatim: String(localized: "Free disk space", table: "ModelCenter") + "  " + bytes(disk))
                    }
                }
            } else if let message = acquisition.hardwareError {
                Text(verbatim: message).foregroundStyle(.secondary)
            } else {
                ProgressView().controlSize(.small)
            }
        }
        .font(.callout).frame(maxWidth: .infinity, alignment: .leading)
        .padding(12).background(.quaternary.opacity(0.6), in: RoundedRectangle(cornerRadius: 9))
        .accessibilityElement(children: .contain)
        .accessibilityIdentifier("local-acquire-hardware")
    }

    private func candidateRow(_ candidate: MacModelAcquisitionEvent.Candidate, index: Int) -> some View {
        VStack(alignment: .leading, spacing: 5) {
            HStack(alignment: .top, spacing: 12) {
                VStack(alignment: .leading, spacing: 3) {
                    Text(verbatim: candidate.repository).font(.headline).textSelection(.enabled)
                    if let variant = candidate.variant { Text(verbatim: variant).font(.caption).foregroundStyle(.secondary).textSelection(.enabled) }
                }
                Spacer()
                Button(String(localized: "Confirm download", table: "ModelCenter")) { acquisition.confirm(candidate) }
                    .accessibilityIdentifier("local-acquire-confirm-\(index)")
            }
            let device = switch candidate.device {
            case "unified": String(localized: "Unified memory", table: "ModelCenter")
            case "gpu": "GPU"
            case "cpu": "CPU"
            default: nil as String?
            }
            Text(verbatim: [engine.title, source == "modelscope" ? "ModelScope" : "Hugging Face", device]
                .compactMap { $0 }.joined(separator: " · "))
                .font(.caption).foregroundStyle(.secondary)
            ViewThatFits(in: .horizontal) {
                HStack(spacing: 14) { estimates(candidate) }
                VStack(alignment: .leading, spacing: 3) { estimates(candidate) }
            }.font(.callout)
            Text(verbatim: candidate.contextDescription)
                .font(.callout.weight(candidate.estimates128K ? .semibold : .regular))
                .accessibilityIdentifier("local-acquire-context-\(index)")
        }.frame(maxWidth: .infinity, alignment: .leading)
    }

    @ViewBuilder private func estimates(_ candidate: MacModelAcquisitionEvent.Candidate) -> some View {
        Text(verbatim: String(localized: "Download size", table: "ModelCenter") + "  " + bytes(candidate.downloadBytes))
        Text(verbatim: String(localized: "Estimated memory", table: "ModelCenter") + "  " + memorySize(candidate.estimatedMemoryBytes))
    }

    private func recommend() {
        suppressAutomaticSearch = true
        acquisition.recommend(engine: engine, source: source, name: acquisition.query)
    }

    private func inputsChanged() {
        suppressAutomaticSearch = !acquisition.query.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
        acquisition.updateInputs(engine: engine, source: source)
        if !suppressAutomaticSearch {
            acquisition.recommendOnOpen(engine: engine, source: source)
        }
    }

    private func bytes(_ value: Int64) -> String {
        ByteCountFormatter.string(fromByteCount: value, countStyle: .file)
    }

    private func memorySize(_ value: Int64) -> String {
        let gibibytes = Double(value) / 1_073_741_824
        return gibibytes.formatted(.number.precision(.fractionLength(0...1))) + " GiB"
    }
}
