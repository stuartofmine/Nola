// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Implements ModelTestDetailView logic in the iOS UI layer.
// - Interfaces:
//   - ModelTestDetailViewNew
// - APIs:
//   - none
//
import SwiftUI

struct ModelTestDetailViewNew: View {
    @EnvironmentObject var container: ServiceContainer
    @Environment(\.dismiss) var dismiss
    
    let modelName: String
    let isLocal: Bool
    
    @State private var maxTokens: Int = 4096
    @State private var temperature: Double = 0.7
    @State private var promptText: String = ""
    @State private var outputText: String = ""
    @State private var isRunning: Bool = false
    @State private var errorMessage: String?
    @State private var showSettings = false
    
    private var defaultPrompts: [String] {
        [
            L10n.modelTestDetailDefaultPrompt1,
            L10n.modelTestDetailDefaultPrompt2,
            L10n.modelTestDetailDefaultPrompt3
        ]
    }
    
    var body: some View {
        ZStack {
            VStack(spacing: 0) {
                // Comment in English.
                HStack(spacing: DesignTokens.spacing_md) {
                    Button(action: { dismiss() }) {
                        HStack(spacing: DesignTokens.spacing_sm) {
                            Image(systemName: "chevron.left")
                                .font(.system(.subheadline, design: .default))
                                .fontWeight(.semibold)
                            Text(L10n.back)
                                .font(.subheadline)
                        }
                        .foregroundColor(DesignTokens.primaryColor)
                    }
                    
                    VStack(alignment: .leading, spacing: DesignTokens.spacing_xs) {
                        Text(L10n.modelTestTitle)
                            .font(.headline)
                            .fontWeight(.semibold)
                        HStack(spacing: DesignTokens.spacing_sm) {
                            Circle()
                                .fill(isLocal ? Color.green : Color.blue)
                                .frame(width: 8, height: 8)
                            Text(modelName)
                                .font(.caption)
                                .foregroundColor(.secondary)
                        }
                    }
                    
                    Spacer()
                    
                    Button(action: { showSettings.toggle() }) {
                        Image(systemName: "gearshape.fill")
                            .font(.system(.body, design: .default))
                            .foregroundColor(.secondary)
                    }
                }
                .padding(DesignTokens.spacing_md)
                .background(Color(.systemBackground))
                .overlay(
                    Rectangle()
                        .frame(height: 1)
                        .foregroundColor(DesignTokens.borderColor),
                    alignment: .bottom
                )
                
                // Comment in English.
                ScrollView {
                    VStack(spacing: DesignTokens.spacing_lg) {
                        // Comment in English.
                        VStack(alignment: .leading, spacing: DesignTokens.spacing_md) {
                            Label(L10n.modelTestDetailParametersTitle, systemImage: "slider.horizontal.3")
                                .font(.headline)
                                .fontWeight(.semibold)
                            
                            // Comment in English.
                            VStack(alignment: .leading, spacing: DesignTokens.spacing_sm) {
                                HStack {
                                    Text(L10n.modelTestMaxTokens)
                                        .font(.subheadline)
                                    Spacer()
                                    TextField("4096", value: $maxTokens, format: .number.grouping(.never))
                                        .keyboardType(.numberPad)
                                        .multilineTextAlignment(.trailing)
                                        .textFieldStyle(.roundedBorder)
                                        .frame(width: 112)
                                        .monospacedDigit()
                                        .onChange(of: maxTokens) { _, newValue in
                                            let clamped = min(max(newValue, 512), 32768)
                                            if clamped != newValue { maxTokens = clamped }
                                        }
                                }
                                Text("512 ... 32768 tokens")
                                    .font(.caption2.monospacedDigit())
                                    .foregroundStyle(.secondary)
                            }
                            
                            // Comment in English.
                            VStack(alignment: .leading, spacing: DesignTokens.spacing_sm) {
                                HStack {
                                    Text(L10n.modelParametersTemperature)
                                        .font(.subheadline)
                                    Spacer()
                                    TextField("0.7", value: $temperature, format: .number.precision(.fractionLength(2)))
                                        .keyboardType(.decimalPad)
                                        .multilineTextAlignment(.trailing)
                                        .textFieldStyle(.roundedBorder)
                                        .frame(width: 104)
                                        .monospacedDigit()
                                        .onChange(of: temperature) { _, newValue in
                                            let clamped = min(max(newValue, 0), 2)
                                            if clamped != newValue { temperature = clamped }
                                        }
                                }
                                HStack {
                                    Text(L10n.modelTestDetailTemperaturePrecise)
                                        .font(.caption2)
                                        .foregroundColor(.secondary)
                                    Spacer()
                                    Text(L10n.modelTestDetailTemperatureCreative)
                                        .font(.caption2)
                                        .foregroundColor(.secondary)
                                }
                                Text("0.00 ... 2.00")
                                    .font(.caption2.monospacedDigit())
                                    .foregroundStyle(.secondary)
                            }
                        }
                        .padding(DesignTokens.spacing_md)
                        .background(Color(.systemGray6))
                        .overlay(
                            RoundedRectangle(cornerRadius: DesignTokens.radius_md)
                                .stroke(DesignTokens.borderColor, lineWidth: 1)
                        )
                        .cornerRadius(DesignTokens.radius_md)
                        
                        // Comment in English.
                        VStack(alignment: .leading, spacing: DesignTokens.spacing_md) {
                            HStack {
                                Label(L10n.modelTestPromptLabel, systemImage: "bubble.left.fill")
                                    .font(.headline)
                                    .fontWeight(.semibold)
                                Spacer()
                                Button(action: { promptText = "" }) {
                                    Image(systemName: "trash.fill")
                                        .font(.caption)
                                        .foregroundColor(.secondary)
                                }
                            }
                            
                            TextEditor(text: $promptText)
                                .frame(minHeight: 100)
                                .scrollContentBackground(.hidden)
                                .overlay(
                                    RoundedRectangle(cornerRadius: DesignTokens.radius_sm)
                                        .stroke(DesignTokens.borderColor, lineWidth: 1)
                                )
                            
                            // Comment in English.
                            VStack(alignment: .leading, spacing: DesignTokens.spacing_sm) {
                                Text(L10n.modelTestDetailQuickPresets)
                                    .font(.caption)
                                    .fontWeight(.semibold)
                                    .foregroundColor(.secondary)
                                
                                VStack(spacing: DesignTokens.spacing_sm) {
                                    ForEach(0..<defaultPrompts.count, id: \.self) { idx in
                                        Button(action: { promptText = defaultPrompts[idx] }) {
                                            HStack(spacing: DesignTokens.spacing_sm) {
                                                Image(systemName: "sparkles")
                                                    .font(.caption)
                                                Text(defaultPrompts[idx])
                                                    .font(.caption)
                                                Spacer()
                                                Image(systemName: "chevron.right")
                                                    .font(.caption2)
                                                    .opacity(0.5)
                                            }
                                            .frame(maxWidth: .infinity, alignment: .leading)
                                            .padding(DesignTokens.spacing_sm)
                                            .background(DesignTokens.primaryColor.opacity(0.05))
                                            .overlay(
                                                RoundedRectangle(cornerRadius: DesignTokens.radius_sm)
                                                    .stroke(DesignTokens.primaryColor.opacity(0.1), lineWidth: 1)
                                            )
                                            .cornerRadius(DesignTokens.radius_sm)
                                            .foregroundColor(.primary)
                                        }
                                    }
                                }
                            }
                        }
                        .padding(DesignTokens.spacing_md)
                        .background(Color(.systemGray6))
                        .overlay(
                            RoundedRectangle(cornerRadius: DesignTokens.radius_md)
                                .stroke(DesignTokens.borderColor, lineWidth: 1)
                        )
                        .cornerRadius(DesignTokens.radius_md)
                        
                        // Comment in English.
                        VStack(alignment: .leading, spacing: DesignTokens.spacing_md) {
                            HStack {
                                Label(L10n.modelTestResultTitle, systemImage: "sparkles")
                                    .font(.headline)
                                    .fontWeight(.semibold)
                                Spacer()
                                if isRunning {
                                    HStack(spacing: DesignTokens.spacing_sm) {
                                        ProgressView()
                                            .scaleEffect(0.8)
                                        Text(L10n.modelTestGenerating)
                                            .font(.caption2)
                                            .foregroundColor(.secondary)
                                    }
                                }
                            }
                            
                            if !outputText.isEmpty {
                                TextEditor(text: $outputText)
                                    .frame(minHeight: 200)
                                    .scrollContentBackground(.hidden)
                                    .overlay(
                                        RoundedRectangle(cornerRadius: DesignTokens.radius_sm)
                                            .stroke(DesignTokens.borderColor, lineWidth: 1)
                                    )
                            } else if !isRunning {
                                VStack(spacing: DesignTokens.spacing_md) {
                                    Image(systemName: "text.bubble")
                                        .font(.system(size: 32))
                                        .foregroundColor(.secondary)
                                        .opacity(0.5)
                                    Text(L10n.modelTestFillPromptAndRun)
                                        .font(.caption)
                                        .foregroundColor(.secondary)
                                }
                                .frame(height: 150)
                                .frame(maxWidth: .infinity)
                                .background(Color(.systemGray6))
                                .overlay(
                                    RoundedRectangle(cornerRadius: DesignTokens.radius_sm)
                                        .stroke(DesignTokens.borderColor, lineWidth: 1)
                                )
                                .cornerRadius(DesignTokens.radius_sm)
                            }
                        }
                        .padding(DesignTokens.spacing_md)
                        .background(Color(.systemGray6))
                        .overlay(
                            RoundedRectangle(cornerRadius: DesignTokens.radius_md)
                                .stroke(DesignTokens.borderColor, lineWidth: 1)
                        )
                        .cornerRadius(DesignTokens.radius_md)
                        
                        // Comment in English.
                        Spacer()
                            .frame(height: 80)
                    }
                    .padding(DesignTokens.spacing_md)
                }
                
                // Comment in English.
                VStack {
                    Spacer()
                    
                    HStack(spacing: DesignTokens.spacing_md) {
                        Button(role: .destructive, action: {
                            promptText = ""
                            outputText = ""
                        }) {
                            Image(systemName: "trash.fill")
                                .font(.body)
                                .frame(width: 44, height: 44)
                                .background(Color.red.opacity(0.1))
                                .foregroundColor(.red)
                                .cornerRadius(DesignTokens.radius_md)
                        }
                        
                        Button(action: { Task { await runTest() } }) {
                            HStack(spacing: DesignTokens.spacing_sm) {
                                Image(systemName: isRunning ? "stop.fill" : "play.fill")
                                Text(isRunning ? L10n.modelTestStop : L10n.modelTestRun)
                                    .fontWeight(.semibold)
                            }
                            .frame(height: DesignTokens.rowHeightCompact)
                            .frame(maxWidth: .infinity)
                            .background(DesignTokens.primaryColor)
                            .foregroundColor(.white)
                            .cornerRadius(DesignTokens.radius_md)
                        }
                        .disabled(promptText.isEmpty && !isRunning)
                    }
                    .padding(DesignTokens.spacing_md)
                    .background(Color(.systemBackground))
                    .overlay(
                        Rectangle()
                            .frame(height: 1)
                            .foregroundColor(DesignTokens.borderColor),
                        alignment: .top
                    )
                }
            }
            
            // Comment in English.
            if let error = errorMessage {
                VStack(spacing: DesignTokens.spacing_md) {
                    HStack(spacing: DesignTokens.spacing_md) {
                        Image(systemName: "exclamationmark.circle.fill")
                            .font(.title3)
                            .foregroundColor(.white)
                        
                        VStack(alignment: .leading, spacing: DesignTokens.spacing_xs) {
                            Text(L10n.modelTestDetailFailedTitle)
                                .font(.headline)
                                .fontWeight(.semibold)
                            Text(error)
                                .font(.caption)
                        }
                        
                        Spacer()
                        
                        Button(action: { errorMessage = nil }) {
                            Image(systemName: "xmark.circle.fill")
                                .font(.body)
                                .foregroundColor(.white)
                        }
                    }
                    .padding(DesignTokens.spacing_md)
                    .background(DesignTokens.dangerColor)
                    .cornerRadius(DesignTokens.radius_md)
                    
                    Spacer()
                }
                .padding(DesignTokens.spacing_md)
                .transition(.move(edge: .top).combined(with: .opacity))
            }
        }
        .toolbar(.hidden, for: .navigationBar)
        .sheet(isPresented: $showSettings) {
            VStack(spacing: DesignTokens.spacing_md) {
                HStack {
                    Text(L10n.modelTestDetailParametersTitle)
                        .font(.headline)
                        .fontWeight(.semibold)
                    Spacer()
                    Button(action: { showSettings = false }) {
                        Image(systemName: "xmark.circle.fill")
                            .font(.body)
                            .foregroundColor(.secondary)
                    }
                }
                .padding(DesignTokens.spacing_md)
                
                Form {
                    Section(L10n.modelTestDetailModelInfo) {
                        HStack {
                            Text(L10n.modelTestDetailModelName)
                            Spacer()
                            Text(modelName)
                                .foregroundColor(.secondary)
                        }
                        HStack {
                            Text(L10n.modelTestDetailModelType)
                            Spacer()
                            Text(isLocal ? L10n.modelTestLocalLabel : L10n.modelTestCloudLabel)
                                .foregroundColor(.secondary)
                        }
                    }
                }
                
                Spacer()
            }
        }
    }
    
    private func runTest() async {
        await MainActor.run {
            isRunning = true
            outputText = ""
            errorMessage = nil
        }
        
        do {
            try await Task.sleep(nanoseconds: 2_000_000_000)
            
            await MainActor.run {
                outputText = """
                Summary generated:
                
                The model completed analysis based on your prompt "\(promptText.prefix(50))...".
                
                Highlights:
                • This is a simulated test result
                • Actual output will be generated from your prompt content
                • Model parameters:
                  - Max Tokens: \(maxTokens)
                  - Temperature: \(String(format: "%.1f", temperature))
                
                This test run is complete.
                """
                isRunning = false
            }
        } catch {
            await MainActor.run {
                errorMessage = error.localizedDescription
                isRunning = false
            }
        }
    }
}

#Preview {
    ModelTestDetailViewNew(modelName: "DeepSeek API", isLocal: false)
        .environmentObject(ServiceContainer())
}
