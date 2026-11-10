// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Implements MLXTestView logic in the iOS UI layer.
// - Interfaces:
//   - MLXTestView
// - APIs:
//   - none
//
//
//  MLXTestView.swift
//  Nola
//
// Comment in English.
//

import SwiftUI

struct MLXTestView: View {
    @EnvironmentObject var container: ServiceContainer
    @State private var isRunning = false
    @State private var log: [String] = []
    @State private var output: ModelOutput = .empty
    @State private var maxTokens: Int = 128
    @State private var temperature: Double = 0.7
    @State private var inferenceTime: Double = 0

    private var isModelDownloaded: Bool {
        container.modelDownloader.isModelDownloaded(.qwen3_5_4b)
    }

    private func appendLog(_ msg: String) {
        withAnimation { log.append(msg) }
        Logger.shared.log(.info, "MLXTest", msg)
    }

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 16) {
                statusSection
                paramsSection
                actionSection
                resultSection
                logsSection
                Spacer(minLength: 24)
            }
            .padding()
        }
        .navigationTitle(L10n.modelTestTitle)
    }

    // MARK: - Subviews

    @ViewBuilder
    private var statusSection: some View {
        GroupBox(label: Label(L10n.modelManagementTitle, systemImage: "cpu")) {
            VStack(alignment: .leading, spacing: 8) {
                HStack {
                    Circle()
                        .fill(isModelDownloaded ? Color.green : Color.orange)
                        .frame(width: 10, height: 10)
                    if isModelDownloaded {
                        Text(String(format: L10n.modelDownloadedStatus, "Qwen3 4B (4-bit)"))
                            .fontWeight(.semibold)
                    } else {
                        Text(L10n.modelNotDownloaded)
                            .fontWeight(.semibold)
                    }
                }
                
                if isModelDownloaded {
                    Text(L10n.modelTestSampleInstruction)
                        .font(.caption)
                        .foregroundColor(.secondary)
                } else {
                    Text(L10n.modelNotDownloaded)
                        .font(.caption)
                        .foregroundColor(.orange)
                }
            }
            .padding(.top, 4)
        }
    }

    @ViewBuilder
    private var paramsSection: some View {
        GroupBox(label: Label(L10n.mlxTestParamsTitle, systemImage: "slider.horizontal.3")) {
            VStack(alignment: .leading, spacing: 12) {
                HStack {
                    Text(L10n.modelTestMaxTokens)
                    Spacer()
                    TextField("512", value: $maxTokens, format: .number.grouping(.never))
                        .keyboardType(.numberPad)
                        .multilineTextAlignment(.trailing)
                        .textFieldStyle(.roundedBorder)
                        .frame(width: 104)
                        .monospacedDigit()
                        .onChange(of: maxTokens) { _, newValue in
                            let clamped = min(max(newValue, 16), 1024)
                            if clamped != newValue { maxTokens = clamped }
                        }
                }
                Text("16 ... 1024 tokens")
                    .font(.caption2.monospacedDigit())
                    .foregroundStyle(.secondary)
                HStack {
                    Text(L10n.modelParametersTemperature)
                    Spacer()
                    TextField("0.7", value: $temperature, format: .number.precision(.fractionLength(2)))
                        .keyboardType(.decimalPad)
                        .multilineTextAlignment(.trailing)
                        .textFieldStyle(.roundedBorder)
                        .frame(width: 104)
                        .monospacedDigit()
                        .onChange(of: temperature) { _, newValue in
                            let clamped = min(max(newValue, 0.1), 1.5)
                            if clamped != newValue { temperature = clamped }
                        }
                }
                Text("0.10 ... 1.50")
                    .font(.caption2.monospacedDigit())
                    .foregroundStyle(.secondary)
                Text(L10n.modelTestSampleInstruction)
                    .font(.caption)
                    .foregroundColor(.secondary)
            }
        }
    }

    @ViewBuilder
    private var actionSection: some View {
        HStack {
            Button {
                Task { await runLocalInference() }
            } label: {
                Label(L10n.modelRunLocal, systemImage: "play.circle.fill")
            }
            .buttonStyle(.borderedProminent)
            .disabled(!isModelDownloaded || isRunning)

            if isRunning { ProgressView().padding(.leading, 8) }
        }
    }

    @ViewBuilder
    private var resultSection: some View {
        if !output.items.isEmpty || output.explain != nil {
            GroupBox(label: Label(L10n.modelTestResultTitle, systemImage: "checkmark.seal")) {
                VStack(alignment: .leading, spacing: 8) {
                    HStack {
                        Text(L10n.modelTestGeneratedItems)
                        Text(String(format: "%d", output.items.count))
                            .foregroundColor(.secondary)
                    }
                    if let exp = output.explain, !exp.isEmpty {
                        Text(exp).font(.footnote)
                            .foregroundColor(.secondary)
                    }
                    Divider()
                    HStack {
                        Label(String(format: L10n.modelInferenceTime, inferenceTime), systemImage: "timer")
                        Spacer()
                    }
                    .font(.caption)
                    .foregroundColor(.secondary)
                    
                    if !output.items.isEmpty {
                        Divider()
                        ForEach(output.items.indices, id: \.self) { idx in
                            VStack(alignment: .leading, spacing: 4) {
                                Text(output.items[idx].text)
                                    .font(.body)
                                HStack {
                                    Text(String(format: L10n.modelPriorityFormat, output.items[idx].priority))
                                    Spacer()
                                    Text(output.items[idx].dedupeKey)
                                        .lineLimit(1)
                                }
                                .font(.caption2)
                                .foregroundColor(.secondary)
                            }
                            .padding(.vertical, 4)
                        }
                    }
                }
            }
        }
    }

    @ViewBuilder
    private var logsSection: some View {
        if !log.isEmpty {
            GroupBox(label: Label(L10n.mlxTestLogsTitle, systemImage: "text.justify")) {
                VStack(alignment: .leading, spacing: 6) {
                    ForEach(Array(log.suffix(50)), id: \.self) { line in
                        Text(line)
                            .font(.caption2)
                            .foregroundColor(.secondary)
                            .frame(maxWidth: .infinity, alignment: .leading)
                    }
                }
            }
        }
    }

    // MARK: - Actions

    private func runLocalInference() async {
        guard isModelDownloaded else {
            appendLog(NSLocalizedString("autogen.MLXTestView.6849b829", comment: "AUTOGEN: localized text"))
            return
        }
        
        isRunning = true
        output = .empty
        log.removeAll()
        appendLog(NSLocalizedString("autogen.MLXTestView.48457675", comment: "AUTOGEN: localized text"))
        appendLog(NSLocalizedString("autogen.MLXTestView.1d5258bc", comment: "AUTOGEN: localized text"))

        let modelPath = container.modelDownloader.getModelPath(for: .qwen3_5_4b)
        appendLog("Model path: \(modelPath.path)")
        
        let provider = LocalProvider(
            maxTokens: maxTokens,
            temperature: Float(temperature)
        )

        // Comment in English.
        let input = ModelInput(
            dateRange: .init(
                start: Date().formatted(),
                end: Date().addingTimeInterval(2 * 24 * 60 * 60).formatted()
            ),
            candidates: [
                .init(type: "event", title: NSLocalizedString("autogen.MLXTestView.f2cd5ae9", comment: "AUTOGEN: localized text"), name: nil, date: nil, start: Date().addingTimeInterval(3600).formatted(date: .omitted, time: .shortened), due: nil, relation: nil, notesTag: nil, durationMin: 45),
                .init(type: "reminder", title: NSLocalizedString("autogen.MLXTestView.73754ade", comment: "AUTOGEN: localized text"), name: nil, date: nil, start: nil, due: Date().addingTimeInterval(24 * 60 * 60).formatted(date: .abbreviated, time: .shortened), relation: nil, notesTag: nil, durationMin: nil),
                .init(type: "event", title: NSLocalizedString("autogen.MLXTestView.4949e39c", comment: "AUTOGEN: localized text"), name: nil, date: nil, start: Date().addingTimeInterval(5 * 60 * 60).formatted(date: .omitted, time: .shortened), due: nil, relation: nil, notesTag: nil, durationMin: 60)
            ],
            userPrefs: ["topN": "3"]
        )

        do {
            let start = Date()
            appendLog(NSLocalizedString("autogen.MLXTestView.c36f20a6", comment: "AUTOGEN: localized text"))
            let result = try await provider.generateSummary(input: input, timeout: 90)
            let elapsed = Date().timeIntervalSince(start)
            inferenceTime = elapsed
            appendLog(String(format: NSLocalizedString("autogen.MLXTestView.e051e4e6", comment: "AUTOGEN: localized text"), elapsed))
            appendLog("Generated \(result.items.count) items")
            output = result
        } catch NolaError.llm(.timeout) {
            appendLog(NSLocalizedString("autogen.MLXTestView.c2d273a2", comment: "AUTOGEN: localized text"))
        } catch {
            appendLog("Inference failed: \(error.localizedDescription)")
        }

        isRunning = false
    }
}

#Preview {
    MLXTestView()
        .environmentObject(ServiceContainer())
}
