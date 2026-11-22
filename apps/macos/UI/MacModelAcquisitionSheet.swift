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
        VStack(alignment: .leading, spacing: 14) {
            HStack(alignment: .firstTextBaseline) {
                Text(String(localized: "Download a model", table: "ModelCenter"))
                    .font(.title2.bold())
                Spacer()
                Button {
                    Task {
                        await acquisition.loadHardware(force: true)
                        acquisition.checkEngine(engine, force: true)
                        acquisition.recommend(engine: engine, source: source, name: acquisition.query, force: true)
                    }
                } label: {
                    Label(String(localized: "Refresh", table: "ModelCenter"), systemImage: "arrow.clockwise")
                        .font(.caption.weight(.medium))
                }
                .buttonStyle(.borderless)
                .disabled(acquisition.busy)
                .help(String(localized: "Refresh recommendations and hardware status", table: "ModelCenter"))
                .accessibilityIdentifier("local-acquire-refresh")
            }

            // Unified Search + Direct URL / owner/repo bar
            HStack(spacing: 8) {
                Image(systemName: "magnifyingglass")
                    .foregroundStyle(.secondary)
                TextField(
                    String(localized: "Model name", table: "ModelCenter"),
                    text: $acquisition.query,
                    prompt: Text(String(localized: "搜索模型，或直接粘贴 Hugging Face / ModelScope 链接或 owner/repo", table: "ModelCenter"))
                )
                .textFieldStyle(.plain)
                .focused($nameFocused)
                .accessibilityLabel(String(localized: "Model name", table: "ModelCenter"))
                .accessibilityIdentifier("local-acquire-name")
                .onSubmit { recommend() }
                if !acquisition.query.isEmpty {
                    Button {
                        acquisition.query = ""
                    } label: {
                        Image(systemName: "xmark.circle.fill")
                            .foregroundStyle(.tertiary)
                    }
                    .buttonStyle(.plain)
                    .help(String(localized: "Clear", table: "ModelCenter"))
                }
            }
            .padding(.horizontal, 12)
            .padding(.vertical, 9)
            .background(MacDesignColor.panel, in: RoundedRectangle(cornerRadius: 10, style: .continuous))
            .overlay {
                RoundedRectangle(cornerRadius: 10, style: .continuous)
                    .strokeBorder(nameFocused ? MacDesignColor.brand : MacDesignColor.borderSubtle, lineWidth: nameFocused ? 1.5 : 1)
            }
            .disabled(acquisition.downloading)

            // Compact single-line device summary pill + inline source & engine controls
            compactEnvironmentBar
                .disabled(acquisition.downloading)

            MacLocalEngineSetupView(acquisition: acquisition, engine: engine, modelDownloaded: false) {
                acquisition.query = ""
                engine = .mlx
            }

            switch acquisition.phase {
            case .idle, .installed:
                EmptyView()
            case .searching:
                HStack(spacing: 8) {
                    ProgressView().controlSize(.small)
                    Text(String(localized: "Finding compatible models…", table: "ModelCenter"))
                        .foregroundStyle(.secondary)
                }
                .padding(.vertical, 6)
            case .choosing:
                if acquisition.candidates.isEmpty {
                    VStack(alignment: .leading, spacing: 4) {
                        Text(String(localized: "No compatible model found", table: "ModelCenter"))
                            .font(.callout.weight(.medium))
                        if let reason = acquisition.reason {
                            Text(verbatim: reason).font(.callout).foregroundStyle(.secondary)
                        }
                    }
                    .padding(.vertical, 6)
                } else {
                    Divider()
                    if let origin = acquisition.recommendationSource, ["modelfit", "hub", "mixed"].contains(origin) {
                        HStack(spacing: 8) {
                            if origin != "hub" {
                                Link("ModelFit · CC BY 4.0", destination: URL(string: "https://modelfit.io/data/")!)
                            }
                            if origin != "modelfit" {
                                Text(verbatim: String(localized: "Recommended from", table: "ModelCenter") + " " +
                                    (source == "modelscope" ? "ModelScope" : "Hugging Face"))
                                    .foregroundStyle(.secondary)
                            }
                        }
                        .font(.caption)
                    }
                    ScrollView {
                        LazyVStack(alignment: .leading, spacing: 10) {
                            ForEach(acquisition.candidates.indices, id: \.self) { index in
                                candidateRow(acquisition.candidates[index], index: index)
                            }
                        }
                        .padding(.vertical, 2)
                    }
                    .frame(minHeight: 160, maxHeight: min(360, max(180, CGFloat(acquisition.candidates.count) * 140)))
                }
            case .downloading(let repository):
                downloadProgressCard(repository: repository)
            case .failed(let detail):
                failureCard(detail: detail)
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
                        .buttonStyle(.nolaProminent)
                        .keyboardShortcut(.defaultAction)
                        .accessibilityIdentifier("local-acquire-submit")
                }
            }
        }
        .padding(24)
        .frame(minWidth: 580, idealWidth: 640)
        .onAppear { nameFocused = !acquisition.downloading }
        .task {
            acquisition.updateInputs(engine: engine, source: source)
            await acquisition.loadHardware()
            if !Task.isCancelled && !suppressAutomaticSearch {
                acquisition.recommendOnOpen(engine: engine, source: source)
            }
        }
        .onChange(of: acquisition.query) { _, newValue in
            let parsed = MacLocalModelAcquisition.parseDirectInput(newValue)
            if let detectedSource = parsed.source {
                if source != detectedSource {
                    source = detectedSource
                }
                if parsed.query != newValue {
                    acquisition.query = parsed.query
                    return
                }
            }
            inputsChanged()
        }
        .onChange(of: engine) { inputsChanged() }
        .onChange(of: source) { inputsChanged() }
    }

    private var compactEnvironmentBar: some View {
        VStack(alignment: .leading, spacing: 8) {
            HStack(spacing: 8) {
                hardwarePill
                Spacer(minLength: 8)
                HStack(spacing: 0) {
                    Picker(String(localized: "Download Source", table: "ModelCenter"), selection: $source) {
                        Text(verbatim: "ModelScope").tag("modelscope")
                        Text(verbatim: "Hugging Face").tag("huggingface")
                    }
                    .labelsHidden()
                    .pickerStyle(.segmented)
                    .frame(width: 196)
                }
                .accessibilityElement(children: .contain)
                .accessibilityIdentifier("local-acquire-source")

                HStack(spacing: 0) {
                    Picker(String(localized: "Inference engine", table: "ModelCenter"), selection: $engine) {
                        ForEach(MacLocalInferenceEngine.allCases, id: \.self) { value in
                            Text(verbatim: value.title).tag(value)
                        }
                    }
                    .labelsHidden()
                    .pickerStyle(.segmented)
                    .frame(width: 176)
                }
                .accessibilityElement(children: .contain)
                .accessibilityIdentifier("local-acquire-engine")
            }
        }
    }

    private var hardwarePill: some View {
        HStack(spacing: 6) {
            Image(systemName: "desktopcomputer")
                .font(.caption)
                .foregroundStyle(.secondary)
            if let hw = acquisition.hardware {
                Text(verbatim: compactHardwareText(hw))
                    .font(.caption)
                    .foregroundStyle(.secondary)
                    .lineLimit(1)
                    .truncationMode(.tail)
            } else if let message = acquisition.hardwareError {
                Text(verbatim: message)
                    .font(.caption)
                    .foregroundStyle(.secondary)
                    .lineLimit(1)
            } else {
                ProgressView().controlSize(.mini)
            }
        }
        .padding(.horizontal, 10)
        .padding(.vertical, 5)
        .background(MacDesignColor.subtleFill, in: Capsule())
        .accessibilityElement(children: .contain)
        .accessibilityIdentifier("local-acquire-hardware")
    }

    private func compactHardwareText(_ hw: MacModelAcquisitionEvent.Hardware) -> String {
        var parts: [String] = []
        if let cpu = hw.cpu, !cpu.isEmpty {
            parts.append(cpu)
        }
        if let memory = hw.memoryBytes {
            parts.append(MacLocalModelAcquisition.formatGigabytes(memory) + (hw.unifiedMemory ? " 统一内存" : " RAM"))
        }
        if let disk = hw.freeDiskBytes {
            parts.append("可用 " + MacLocalModelAcquisition.formatGigabytes(disk))
        }
        return parts.isEmpty ? String(localized: "This Mac", table: "ModelCenter") : parts.joined(separator: " · ")
    }

    private func candidateRow(_ candidate: MacModelAcquisitionEvent.Candidate, index: Int) -> some View {
        let fit = candidate.fitBadge(deviceMemoryBytes: acquisition.hardware?.memoryBytes)
        return VStack(alignment: .leading, spacing: 8) {
            HStack(alignment: .top, spacing: 12) {
                VStack(alignment: .leading, spacing: 4) {
                    HStack(spacing: 6) {
                        Text(verbatim: candidate.displayTitle)
                            .font(.headline)
                            .lineLimit(1)
                        Text(fit.label)
                            .font(.caption2.weight(.semibold))
                            .foregroundStyle(fit.isComfortable ? MacDesignColor.success : MacDesignColor.warning)
                            .padding(.horizontal, 7)
                            .padding(.vertical, 2)
                            .background(
                                (fit.isComfortable ? MacDesignColor.success : MacDesignColor.warning).opacity(0.12),
                                in: Capsule()
                            )
                    }
                    HStack(spacing: 10) {
                        Label(MacLocalModelAcquisition.formatGigabytes(candidate.downloadBytes), systemImage: "arrow.down.circle")
                        Text("·")
                        Text(String(localized: "预计内存", table: "ModelCenter") + " " + MacLocalModelAcquisition.formatGigabytes(candidate.estimatedMemoryBytes))
                    }
                    .font(.caption.monospacedDigit())
                    .foregroundStyle(.secondary)
                }
                Spacer(minLength: 8)
                Button(String(localized: "Confirm download", table: "ModelCenter")) {
                    acquisition.confirm(candidate)
                }
                .buttonStyle(.nolaProminent)
                .controlSize(.small)
                .accessibilityIdentifier("local-acquire-confirm-\(index)")
            }

            DisclosureGroup(String(localized: "技术详情", table: "ModelCenter")) {
                VStack(alignment: .leading, spacing: 4) {
                    Text(verbatim: candidate.repository)
                        .font(.caption.monospaced())
                        .foregroundStyle(.secondary)
                        .textSelection(.enabled)
                    if let variant = candidate.variant {
                        Text(verbatim: variant)
                            .font(.caption.monospaced())
                            .foregroundStyle(.secondary)
                            .textSelection(.enabled)
                    }
                    Text(verbatim: [engine.title, source == "modelscope" ? "ModelScope" : "Hugging Face", deviceLabel(candidate.device)]
                        .compactMap { $0 }.joined(separator: " · "))
                        .font(.caption)
                        .foregroundStyle(.secondary)
                    Text(verbatim: candidate.contextDescription)
                        .font(.caption)
                        .foregroundStyle(.secondary)
                        .accessibilityIdentifier("local-acquire-context-\(index)")
                }
                .padding(.top, 4)
            }
            .font(.caption)
            .foregroundStyle(.secondary)
        }
        .padding(12)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(MacDesignColor.panel, in: RoundedRectangle(cornerRadius: 12, style: .continuous))
        .overlay {
            RoundedRectangle(cornerRadius: 12, style: .continuous)
                .strokeBorder(MacDesignColor.borderSubtle, lineWidth: 1)
        }
    }

    private func downloadProgressCard(repository: String) -> some View {
        let fraction = acquisition.total > 0 ? min(1, max(0, Double(acquisition.received) / Double(acquisition.total))) : 0
        return VStack(alignment: .leading, spacing: 8) {
            HStack {
                Text(verbatim: repository)
                    .font(.callout.weight(.semibold))
                    .lineLimit(1)
                Spacer()
                if acquisition.total > 0 {
                    Text("\(Int((fraction * 100).rounded()))%")
                        .font(.callout.weight(.semibold).monospacedDigit())
                        .foregroundStyle(MacDesignColor.brand)
                }
            }
            if acquisition.total > 0 {
                ProgressView(value: fraction)
                HStack(spacing: 8) {
                    Text(verbatim: "\(MacLocalModelAcquisition.formatGigabytes(acquisition.received)) / \(MacLocalModelAcquisition.formatGigabytes(acquisition.total))")
                    if acquisition.downloadSpeedBytesPerSecond > 0 {
                        Text("·")
                        Text(verbatim: "\(MacLocalModelAcquisition.formatGigabytes(Int64(acquisition.downloadSpeedBytesPerSecond)))/s")
                    }
                    if let eta = MacLocalModelAcquisition.formatETA(acquisition.estimatedRemainingSeconds) {
                        Text("·")
                        Text(String(localized: "剩余", table: "ModelCenter") + " " + eta)
                    }
                    Spacer()
                }
                .font(.caption.monospacedDigit())
                .foregroundStyle(.secondary)
            } else {
                ProgressView().controlSize(.small)
            }
        }
        .padding(12)
        .background(MacDesignColor.panel, in: RoundedRectangle(cornerRadius: 12, style: .continuous))
        .overlay {
            RoundedRectangle(cornerRadius: 12, style: .continuous)
                .strokeBorder(MacDesignColor.borderSubtle, lineWidth: 1)
        }
    }

    private func failureCard(detail: String) -> some View {
        HStack(alignment: .top, spacing: 10) {
            Image(systemName: "exclamationmark.triangle.fill")
                .foregroundStyle(MacDesignColor.danger)
            VStack(alignment: .leading, spacing: 4) {
                Text(verbatim: detail)
                    .font(.callout)
                    .foregroundStyle(MacDesignColor.danger)
                    .textSelection(.enabled)
                    .accessibilityIdentifier("local-acquire-error")
            }
            Spacer(minLength: 8)
            if acquisition.lastAttemptedCandidate != nil {
                Button {
                    acquisition.retryLastDownload()
                } label: {
                    Label(String(localized: "Retry", table: "ModelCenter"), systemImage: "arrow.clockwise")
                }
                .buttonStyle(.nolaProminent)
                .controlSize(.small)
                .accessibilityIdentifier("local-acquire-retry")
            }
        }
        .padding(12)
        .background(MacDesignColor.danger.opacity(0.08), in: RoundedRectangle(cornerRadius: 10, style: .continuous))
        .overlay {
            RoundedRectangle(cornerRadius: 10, style: .continuous)
                .strokeBorder(MacDesignColor.danger.opacity(0.24), lineWidth: 1)
        }
    }

    private func deviceLabel(_ raw: String?) -> String? {
        switch raw {
        case "unified": return String(localized: "Unified memory", table: "ModelCenter")
        case "gpu": return "GPU"
        case "cpu": return "CPU"
        default: return nil
        }
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
}
