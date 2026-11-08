// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Implements ModelTestView logic in the iOS UI layer.
// - Interfaces:
//   - ModelTestView
//   - SettingsSheet
//   - ModelTestView_Previews
// - APIs:
//   - none
//
import SwiftUI

struct ModelTestView: View {
    @EnvironmentObject var container: ServiceContainer

    @State private var useLocal: Bool = true
    @State private var selectedModel: String = "deepseek-ai/DeepSeek-V3.2"
    @State private var maxTokens: Int = 8192
    @State private var temperature: Double = 0.7
    @State private var promptText: String = ""
    @State private var taskText: String = ""
    @State private var customInputJSON: String = ""

    @State private var outputText: String = ""
    @State private var outputModelOutput: ModelOutput = .empty
    @State private var isRunning: Bool = false
    @State private var isStreaming: Bool = false
    @State private var errorMessage: String?
    @State private var currentTokens: Int = 0
    
    @State private var showSettings: Bool = false
    @State private var showAdvanced: Bool = false

    private enum LogLevel: String, Codable {
        case info
        case error
    }

    private struct LogEntry: Identifiable {
        let id = UUID()
        let level: LogLevel
        let message: String
        let date: Date
    }

    @State private var logs: [LogEntry] = []

    // Comment in English.
    private let defaultPrompts: [String] = [
        NSLocalizedString("model.test.default_prompt.summary_json", value: "Generate a daily summary from the candidate events below and output JSON in ModelOutput format.", comment: "Model test default prompt"),
        NSLocalizedString("autogen.ModelTestView.a94d5104", comment: "AUTOGEN: localized text"),
        NSLocalizedString("autogen.ModelTestView.fff208a3", comment: "AUTOGEN: localized text")
    ]
    
    private var availableModels: [String] {
        return ["deepseek-ai/DeepSeek-V3.2"]
    }

    var body: some View {
        NavigationStack {
            HStack {
                VStack(alignment: .leading, spacing: 4) {
                    if isRunning || isStreaming {
                        HStack(spacing: 6) {
                            ProgressView()
                            Text(L10n.modelTestGenerating)
                                .font(.caption2)
                                .foregroundColor(.secondary)
                        }
                        .fontWeight(.bold)
                    }

                    HStack(spacing: 6) {
                        Circle()
                            .fill(useLocal ? Color.green : Color.blue)
                            .frame(width: 8, height: 8)
                        Text(useLocal ? L10n.modelTestLocalLabel : L10n.modelTestCloudLabel)
                            .font(.caption)
                            .foregroundColor(.secondary)
                    }
                }

                Spacer()

                Button(action: { showSettings.toggle() }) {
                    Image(systemName: "gearshape.fill")
                        .font(.title3)
                        .foregroundColor(.secondary)
                }
            }
            .padding()
            .background(Color(.systemBackground))
            .border(width: 1, edges: [.bottom], color: Color.gray.opacity(0.2))
                
                // Comment in English.
                ZStack {
                    ScrollView {
                        VStack(spacing: 16) {
                            // Comment in English.
                            VStack(spacing: 12) {
                                // Comment in English.
                                VStack(alignment: .leading, spacing: 8) {
                                    Label(L10n.modelTestModelSelect, systemImage: "square.stack.3d.up.fill")
                                        .font(.caption)
                                        .fontWeight(.semibold)
                                        .foregroundColor(.secondary)
                                    
                                    Picker(L10n.modelTestModelSelect, selection: $selectedModel) {
                                        ForEach(availableModels, id: \.self) { model in
                                            HStack {
                                                Text(model)
                                                    .lineLimit(1)
                                                    .font(.body)
                                            }
                                            .tag(model)
                                        }
                                    }
                                    .pickerStyle(.menu)
                                    .frame(maxWidth: .infinity, alignment: .leading)
                                    .padding(.vertical, 8)
                                    .padding(.horizontal, 12)
                                    .background(Color(.systemGray6))
                                    .cornerRadius(8)
                                }
                                .padding(12)
                                .background(Color(.systemGray6))
                                .cornerRadius(10)
                                
                                // Comment in English.
                                VStack(alignment: .leading, spacing: 8) {
                                    HStack {
                                            Label(L10n.modelTestMaxTokens, systemImage: "slider.horizontal.3")
                                            .font(.caption)
                                            .fontWeight(.semibold)
                                            .foregroundColor(.secondary)
                                        Spacer()
                                        TextField("4096", value: $maxTokens, format: .number.grouping(.never))
                                            .keyboardType(.numberPad)
                                            .multilineTextAlignment(.trailing)
                                            .textFieldStyle(.roundedBorder)
                                            .frame(width: 112)
                                            .monospacedDigit()
                                            .onChange(of: maxTokens) { _, newValue in
                                                let clamped = min(max(newValue, 4096), 32768)
                                                if clamped != newValue { maxTokens = clamped }
                                            }
                                    }
                                    Text("4096 ... 32768 tokens")
                                        .font(.caption2.monospacedDigit())
                                        .foregroundStyle(.secondary)
                                    
                                    HStack(spacing: 16) {
                                        VStack(alignment: .center, spacing: 4) {
                                            Text(L10n.modelTest4K)
                                                .font(.caption2)
                                                .foregroundColor(.secondary)
                                        }
                                        Spacer()
                                        VStack(alignment: .center, spacing: 4) {
                                            Text(L10n.modelTest32K)
                                                .font(.caption2)
                                                .foregroundColor(.secondary)
                                        }
                                    }
                                }
                                .padding(12)
                                .background(Color(.systemGray6))
                                .cornerRadius(10)
                            }
                            .padding(.horizontal, 12)
                            .padding(.top, 12)
                            
                            // Comment in English.
                            VStack(spacing: 12) {
                                // Comment in English.
                                VStack(alignment: .leading, spacing: 8) {
                                    HStack {
                                        Label(L10n.modelTestPromptLabel, systemImage: "bubble.left.fill")
                                            .font(.caption)
                                            .fontWeight(.semibold)
                                            .foregroundColor(.secondary)
                                        Spacer()
                                        Button(action: { promptText = "" }) {
                                            Image(systemName: "trash.fill")
                                                .font(.caption)
                                        }
                                        .buttonStyle(.bordered)
                                    }
                                    TextEditor(text: $promptText)
                                        .frame(minHeight: 80)
                                        .scrollContentBackground(.hidden)
                                        .overlay(
                                            RoundedRectangle(cornerRadius: 8)
                                                .stroke(Color.secondary.opacity(0.2), lineWidth: 1)
                                        )
                                    
                                    // Comment in English.
                                    HStack(spacing: 8) {
                                        ForEach(0..<min(3, defaultPrompts.count), id: \.self) { idx in
                                            Button(action: { promptText = defaultPrompts[idx] }) {
                                                Text(String(format: L10n.modelTestPreset, idx + 1))
                                                    .font(.caption2)
                                                    .lineLimit(1)
                                            }
                                            .buttonStyle(.bordered)
                                        }
                                        Spacer()
                                    }
                                }
                                .padding(12)
                                .background(Color(.systemGray6))
                                .cornerRadius(10)
                                
                                // Comment in English.
                                VStack(alignment: .leading, spacing: 8) {
                                    Label(L10n.modelTestTaskLabel, systemImage: "list.bullet.fill")
                                        .font(.caption)
                                        .fontWeight(.semibold)
                                        .foregroundColor(.secondary)
                                    TextEditor(text: $taskText)
                                        .frame(height: 60)
                                        .scrollContentBackground(.hidden)
                                        .overlay(
                                            RoundedRectangle(cornerRadius: 8)
                                                .stroke(Color.secondary.opacity(0.2), lineWidth: 1)
                                        )
                                }
                                .padding(12)
                                .background(Color(.systemGray6))
                                .cornerRadius(10)
                            }
                            .padding(.horizontal, 12)
                            .padding(.top, 12)
                            
                            Divider()
                                .padding(.horizontal, 12)
                            
                            // Comment in English.
                            VStack(alignment: .leading, spacing: 12) {
                                    HStack {
                                    Label(L10n.modelTestResultTitle, systemImage: "sparkles.rectangle.stack.fill")
                                        .font(.caption)
                                        .fontWeight(.semibold)
                                        .foregroundColor(.secondary)
                                    Spacer()
                                        if isRunning {
                                        HStack(spacing: 6) {
                                            ProgressView()
                                                .scaleEffect(0.8)
                                            Text(L10n.modelTestGenerating)
                                                .font(.caption2)
                                                .foregroundColor(.secondary)
                                        }
                                    }
                                        // Comment in English.
                                        if currentTokens > 0 {
                                            Text(L10n.format("common.tokens_format", currentTokens))
                                                .font(.caption2)
                                                .foregroundColor(.secondary)
                                        }
                                }
                                
                                if !outputText.isEmpty || !outputModelOutput.items.isEmpty {
                                    if !outputText.isEmpty {
                                        TextEditor(text: $outputText)
                                            .frame(minHeight: 400)
                                            .frame(maxHeight: 600)
                                            .scrollContentBackground(.hidden)
                                            .overlay(
                                                RoundedRectangle(cornerRadius: 8)
                                                    .stroke(Color.secondary.opacity(0.2), lineWidth: 1)
                                            )
                                    } else {
                                        VStack(alignment: .leading, spacing: 12) {
                                            ForEach(outputModelOutput.items.indices, id: \.self) { idx in
                                                VStack(alignment: .leading, spacing: 6) {
                                                    HStack {
                                                        Text(L10n.modelBullet)
                                                            .font(.title3)
                                                            .foregroundColor(.blue)
                                                        Text(outputModelOutput.items[idx].text)
                                                            .font(.body)
                                                    }
                                                    let dedupeKey = outputModelOutput.items[idx].dedupeKey
                                                    if !dedupeKey.isEmpty {
                                                        Text(L10n.modelItemID.localized(dedupeKey))
                                                            .font(.caption2)
                                                            .foregroundColor(.secondary)
                                                    }
                                                }
                                                .padding(10)
                                                .background(Color(.systemGray6))
                                                .cornerRadius(8)
                                            }
                                            
                                            if let explain = outputModelOutput.explain {
                                                Divider()
                                                VStack(alignment: .leading, spacing: 6) {
                                                    Text(L10n.modelTestExplain)
                                                        .font(.caption)
                                                        .fontWeight(.semibold)
                                                        .foregroundColor(.secondary)
                                                    Text(explain)
                                                        .font(.caption)
                                                        .foregroundColor(.secondary)
                                                }
                                                .padding(10)
                                                .background(Color(.systemGray6))
                                                .cornerRadius(8)
                                            }
                                        }
                                        .frame(maxHeight: 600)
                                    }
                                } else if !isRunning {
                                    VStack(spacing: 12) {
                                        Image(systemName: "text.bubble")
                                            .font(.system(size: 40))
                                            .foregroundColor(.secondary.opacity(0.5))
                                        Text(L10n.modelTestFillPromptAndRun)
                                            .font(.caption)
                                            .foregroundColor(.secondary)
                                    }
                                    .frame(height: 200)
                                    .frame(maxWidth: .infinity)
                                    .background(Color(.systemGray6))
                                    .cornerRadius(10)
                                }
                            }
                            .padding(.horizontal, 12)
                            .padding(.bottom, 12)
                        }
                    }
                    
                    // Comment in English.
                    VStack {
                        Spacer()
                        HStack {
                            Spacer()
                            Button(action: { Task { await runStream() } }) {
                                    HStack(spacing: 8) {
                                    Image(systemName: isRunning ? "stop.fill" : "waveform.path")
                                    Text(isRunning ? L10n.modelTestStop : L10n.modelTestRun)
                                        .fontWeight(.semibold)
                                }
                                .frame(height: 50)
                                .frame(maxWidth: .infinity)
                                .background(Color.blue)
                                .foregroundColor(.white)
                                .cornerRadius(12)
                            }
                            .disabled(isRunning && promptText.isEmpty)
                            .padding(16)
                        }
                    }
                }
            }
            .toolbar(.hidden, for: .navigationBar)
            .sheet(isPresented: $showSettings) {
                SettingsSheet()
                    .environmentObject(container)
            }
            .alert(L10n.alertTitle, isPresented: .constant(errorMessage != nil), presenting: errorMessage) { _ in
                Button(L10n.close) { errorMessage = nil }
            } message: { msg in
                Text(msg)
            }
        }

    // MARK: - Private Methods

    private func appendLog(_ msg: String, level: LogLevel = .info) {
        withAnimation {
            logs.append(LogEntry(level: level, message: msg, date: Date()))
        }
        if level == .error {
            Logger.shared.log(.error, "ModelTestView", msg)
        } else {
            Logger.shared.log(.info, "ModelTestView", msg)
        }
    }

    private func runOnce() async {
        isRunning = true
        isStreaming = false
        outputText = ""
        outputModelOutput = .empty
        logs.removeAll()
        errorMessage = nil
        appendLog(NSLocalizedString("autogen.ModelTestView.d0754da8", comment: "AUTOGEN: localized text"))

        container.useLocalModel = useLocal
        container.reconfigureIfPossible()

        if let svc = container.llmClient as? ModelService {
            if useLocal {
                svc.useLocalProvider(maxTokens: maxTokens, temperature: Float(temperature))
            }
        }

        let decoder = JSONDecoder()
        var input: ModelInput
        
        if let data = customInputJSON.trimmingCharacters(in: .whitespaces).data(using: .utf8),
           !customInputJSON.trimmingCharacters(in: .whitespaces).isEmpty,
           let parsed = try? decoder.decode(ModelInput.self, from: data) {
            input = parsed
        } else {
            let base = makeExampleModelInput()
            var prefs = base.userPrefs
            prefs["prompt"] = promptText
            prefs["task"] = taskText
            input = ModelInput(dateRange: base.dateRange, candidates: base.candidates, userPrefs: prefs)
        }

        do {
            let start = Date()
            let out = try await container.llmClient.generateSummary(input: input, timeout: 90)
            let elapsed = Date().timeIntervalSince(start)
            appendLog(String(format: NSLocalizedString("autogen.ModelTestView.5c69f135", comment: "AUTOGEN: localized text"), elapsed))
            // Log raw model response for debugging (truncated)
            if let d = try? JSONEncoder().encode(out), let s = String(data: d, encoding: .utf8) {
                appendLog("RAW_RESPONSE: \(s.prefix(4000))")
            }

            // Comment in English.
            currentTokens = Telemetry.estimateTokens(from: outputText.isEmpty ? (String(data: try JSONEncoder().encode(out), encoding: .utf8) ?? "") : outputText)

            // Prefer a user-facing text view: if the model returned items, show them as bullets;
            // otherwise show the explain text if present; fallback to raw JSON only if nothing else.
            outputModelOutput = out
            if !out.items.isEmpty {
                let lines = out.items.map { "• \($0.text)" }
                outputText = lines.joined(separator: "\n\n")
                appendLog("PARSED_OUTPUT: \(outputText.prefix(2000))")
            } else if let explain = out.explain, !explain.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
                outputText = explain
                appendLog("PARSED_OUTPUT: \(outputText.prefix(2000))")
            } else if let d = try? JSONEncoder().encode(out), let s = String(data: d, encoding: .utf8) {
                outputText = s
            }
        } catch {
            errorMessage = error.localizedDescription
            appendLog("Generation failed: \(error.localizedDescription)", level: .error)
        }

        isRunning = false
    }

    private func runStream() async {
        isRunning = true
        isStreaming = true
        outputText = ""
        outputModelOutput = .empty
        logs.removeAll()
        errorMessage = nil
        appendLog(NSLocalizedString("autogen.ModelTestView.765216a7", comment: "AUTOGEN: localized text"))

        container.useLocalModel = useLocal
        container.reconfigureIfPossible()

        if let svc = container.llmClient as? ModelService {
            if useLocal {
                svc.useLocalProvider(maxTokens: maxTokens, temperature: Float(temperature))
            }
        }

        let decoder = JSONDecoder()
        var input: ModelInput
        
        if let data = customInputJSON.trimmingCharacters(in: .whitespaces).data(using: .utf8),
           !customInputJSON.trimmingCharacters(in: .whitespaces).isEmpty,
           let parsed = try? decoder.decode(ModelInput.self, from: data) {
            input = parsed
        } else {
            let base = makeExampleModelInput()
            var prefs = base.userPrefs
            prefs["prompt"] = promptText
            prefs["task"] = taskText
            input = ModelInput(dateRange: base.dateRange, candidates: base.candidates, userPrefs: prefs)
        }

        let stream = container.llmClient.generateSummaryStream(input: input)
        do {
            let streamStart = Date()
            var accumulated = ""
            var reasoningBuffer = ""
            var answerBuffer = ""
            var doneReasoning = false

            var chunkIdx = 0
            for try await chunk in stream {
                chunkIdx += 1
                accumulated += chunk

                // Record a preview of the raw chunk for diagnostics (truncate large chunks)
                let preview = String(chunk.prefix(2000))
                appendLog("RAW_CHUNK[\(chunkIdx)]: \(preview)")

                // Try parse chunk as JSON and look for choices[].delta.reasoning_content or content
                var handled = false
                if let data = chunk.data(using: .utf8) {
                    if let obj = try? JSONSerialization.jsonObject(with: data, options: []), let dict = obj as? [String: Any] {
                        if let choices = dict["choices"] as? [[String: Any]], let first = choices.first {
                            if let delta = first["delta"] as? [String: Any] {
                                if let reasoningChunk = delta["reasoning_content"] as? String, !reasoningChunk.isEmpty {
                                    reasoningBuffer += reasoningChunk
                                    outputText = reasoningBuffer
                                    handled = true
                                } else if let answerChunk = delta["content"] as? String, !answerChunk.isEmpty {
                                    if !doneReasoning && !reasoningBuffer.isEmpty {
                                        // show separator once when final answer starts
                                        answerBuffer += answerChunk
                                        outputText = reasoningBuffer + "\n\n === Final Answer ===\n\n" + answerBuffer
                                        doneReasoning = true
                                    } else {
                                        answerBuffer += answerChunk
                                        outputText = (doneReasoning ? (reasoningBuffer + "\n\n === Final Answer ===\n\n" + answerBuffer) : (reasoningBuffer + answerBuffer))
                                    }
                                    handled = true
                                }
                            } else if let message = first["message"] as? [String: Any], let content = message["content"] as? String {
                                // Non-delta streaming may include full message
                                answerBuffer += content
                                outputText = (reasoningBuffer.isEmpty ? answerBuffer : reasoningBuffer + "\n\n === Final Answer ===\n\n" + answerBuffer)
                                handled = true
                            }
                        }
                    }
                }

                if !handled {
                    // Fallback: update user-facing text by parsing accumulated content
                    outputText = parseAccumulatedOutput(accumulated)
                }

                // Progressive token estimate
                currentTokens = Telemetry.estimateTokens(from: accumulated)
            }
            let streamDuration = Date().timeIntervalSince(streamStart)
            appendLog(String(format: NSLocalizedString("autogen.ModelTestView.4f92eb12", comment: "AUTOGEN: localized text"), streamDuration))
            // finalize tokens
            currentTokens = Telemetry.estimateTokens(from: outputText)

            // Finalize: ensure outputModelOutput is set if the stream encoded a ModelOutput
            // Try to extract and decode the JSON from accumulated stream (may contain metadata)
            appendLog("FINALIZE: Attempting to parse accumulated stream (\(accumulated.count) bytes)...")
            if let jsonRange = extractJSONStringRange(from: accumulated) {
                let jsonStr = String(accumulated[jsonRange])
                appendLog("FINALIZE: Extracted JSON substring (\(jsonStr.count) bytes)")
                if let data = jsonStr.data(using: .utf8) {
                    if let decoded = try? JSONDecoder().decode(ModelOutput.self, from: data) {
                        outputModelOutput = decoded
                        appendLog("FINALIZE: Successfully decoded ModelOutput with \(decoded.items.count) items")
                    } else {
                        appendLog("FINALIZE: Failed to decode as ModelOutput")
                    }
                }
            } else {
                appendLog("FINALIZE: No JSON range found in accumulated stream")
            }
            // Always ensure outputText is updated (even if decode failed)
            outputText = parseAccumulatedOutput(accumulated)
            appendLog("PARSED_OUTPUT: \(outputText.prefix(2000))")
        } catch {
            errorMessage = error.localizedDescription
            appendLog("Streaming failed: \(error.localizedDescription)", level: .error)
            // On failure, ensure we still only show sanitized text
            outputText = parseAccumulatedOutput(outputText)
        }

        isRunning = false
        isStreaming = false
    }

    private func exampleInputJSON() -> String {
        "{ \"candidates\": [{ \"type\": \"event\", \"title\": \"Team sync\", \"start\": \"2025-11-14T10:00:00\" }], \"userPrefs\": { \"topN\": 3 } }"
    }

    private func makeExampleModelInput() -> ModelInput {
        let iso = ISO8601DateFormatter()
        let now = Date()
        let start = iso.string(from: now)
        let end = iso.string(from: now.addingTimeInterval(2 * 24 * 60 * 60))

        let candidates: [ModelInput.Candidate] = [
            .init(type: "event", title: NSLocalizedString("autogen.ModelTestView.f2cd5ae9", comment: "AUTOGEN: localized text"), name: nil, date: nil, start: iso.string(from: now.addingTimeInterval(2 * 60 * 60)), due: nil, relation: nil, notesTag: nil, durationMin: 60),
            .init(type: "reminder", title: NSLocalizedString("autogen.ModelTestView.73754ade", comment: "AUTOGEN: localized text"), name: nil, date: nil, start: nil, due: iso.string(from: now.addingTimeInterval(24 * 60 * 60)), relation: nil, notesTag: nil, durationMin: nil)
        ]

        return ModelInput(dateRange: .init(start: start, end: end), candidates: candidates, userPrefs: ["topN": "3"])
    }

    /// Try to extract user-facing text from accumulated model output.
    /// - If the text decodes to `ModelOutput`, prefer `items` or `explain`.
    /// - Otherwise, try to parse any JSON and collect leaf string values.
    /// - Fallback: sanitize raw text by stripping JSON-like punctuation and trimming.
    private func parseAccumulatedOutput(_ raw: String) -> String {
        let trimmed = raw.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmed.isEmpty else { return "" }

        // 1) Try decode as ModelOutput
        // If the stream provides metadata around the JSON (e.g. chunked events), try to extract a JSON substring first.
        if let jsonRange = extractJSONStringRange(from: trimmed) {
            let jsonStr = String(trimmed[jsonRange])
            if let data = jsonStr.data(using: .utf8) {
                if let out = try? JSONDecoder().decode(ModelOutput.self, from: data) {
                    if !out.items.isEmpty {
                        return out.items.map { "• \($0.text)" }.joined(separator: "\n\n")
                    }
                    if let explain = out.explain, !explain.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
                        return explain
                    }
                }

                if let obj = try? JSONSerialization.jsonObject(with: data, options: []), let strs = collectStrings(from: obj), !strs.isEmpty {
                    let filtered = strs.filter { $0.count > 2 }
                    if !filtered.isEmpty {
                        return Array(Set(filtered)).joined(separator: "\n\n")
                    }
                }
            }
        } else if let data = trimmed.data(using: .utf8) {
            // Try directly decoding in case the accumulated text is pure JSON
            if let out = try? JSONDecoder().decode(ModelOutput.self, from: data) {
                if !out.items.isEmpty {
                    return out.items.map { "• \($0.text)" }.joined(separator: "\n\n")
                }
                if let explain = out.explain, !explain.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
                    return explain
                }
            }

            if let obj = try? JSONSerialization.jsonObject(with: data, options: []), let strs = collectStrings(from: obj), !strs.isEmpty {
                let filtered = strs.filter { $0.count > 2 }
                if !filtered.isEmpty {
                    return Array(Set(filtered)).joined(separator: "\n\n")
                }
            }
        }

        // 3) If we reach here, try stripping common streaming metadata tokens then sanitize
        let cleaned = stripStreamingMetadata(from: trimmed)
        let sanitized = cleaned
            .replacingOccurrences(of: "\\n", with: "\n")
            .replacingOccurrences(of: "\\r", with: "")
            .replacingOccurrences(of: "\"", with: "")
            .replacingOccurrences(of: "{", with: "")
            .replacingOccurrences(of: "}", with: "")
            .replacingOccurrences(of: "[", with: "")
            .replacingOccurrences(of: "]", with: "")
        let squeezed = sanitized.components(separatedBy: CharacterSet.whitespacesAndNewlines).filter { !$0.isEmpty }.joined(separator: " ")
        // Limit length
        if squeezed.count > 2000 {
            let idx = squeezed.index(squeezed.startIndex, offsetBy: 2000)
            return String(squeezed[..<idx]) + "..."
        }
        return squeezed
    }

    private func collectStrings(from obj: Any) -> [String]? {
        var out: [String] = []
        func walk(_ node: Any) {
            if let s = node as? String {
                out.append(s)
            } else if let arr = node as? [Any] {
                for v in arr { walk(v) }
            } else if let dict = node as? [String: Any] {
                for (_, v) in dict { walk(v) }
            }
        }
        walk(obj)
        return out.isEmpty ? nil : out
    }

    /// Return the range of the first JSON object/array found in the string (balanced braces).
    private func extractJSONStringRange(from s: String) -> Range<String.Index>? {
        let chars = Array(s)
        var stack: [Character] = []
        var startIndex: Int? = nil
        for (i, c) in chars.enumerated() {
            if c == "{" || c == "[" {
                if startIndex == nil { startIndex = i }
                stack.append(c)
            } else if c == "}" || c == "]" {
                if stack.isEmpty { continue }
                let top = stack.removeLast()
                if (top == "{" && c == "}") || (top == "[" && c == "]") {
                    if stack.isEmpty, let sIdx = startIndex {
                        let st = s.index(s.startIndex, offsetBy: sIdx)
                        let ed = s.index(s.startIndex, offsetBy: i)
                        // Convert closed range to half-open range for Swift String.index ranges
                        let endExclusive = s.index(after: ed)
                        return st..<endExclusive
                    }
                }
            }
        }
        return nil
    }

    /// Remove common streaming metadata tokens that wrap JSON chunks.
    private func stripStreamingMetadata(from s: String) -> String {
        // Remove tokens like "reason:null," "message:role:null,", "id:...model:...", "choices:..." etc.
        // Conservative approach: remove key:value pairs of form key:... up to commas/newlines, but keep braces.
        let pattern = "(\\b[id|object|created|model|system_fingerprint|choices|delta|finish_reason|message|reason|tool_calls|function_calls|usage]:[^,\\n]*)"
        if let re = try? NSRegularExpression(pattern: pattern, options: [.caseInsensitive]) {
            let range = NSRange(s.startIndex..<s.endIndex, in: s)
            let cleaned = re.stringByReplacingMatches(in: s, options: [], range: range, withTemplate: "")
            return cleaned
        }
        return s
    }

}

// MARK: - Settings Sheet

struct SettingsSheet: View {
    @EnvironmentObject var container: ServiceContainer
    @Environment(\.dismiss) var dismiss
    @State private var useLocal: Bool = true
    @State private var maxTokens: Int = 256
    
    var body: some View {
        NavigationStack {
            List {
                Section(header: Text(L10n.modelTestTitle)) {
                    Picker(L10n.modelTestExecutionEnvironment, selection: $useLocal) {
                        Text(L10n.modelTestLocalLabel).tag(true)
                        Text(L10n.modelTestCloudLabel).tag(false)
                    }
                    .pickerStyle(.segmented)
                    .onChange(of: useLocal) { _, new in
                        container.useLocalModel = new
                        container.reconfigureIfPossible()
                    }
                }
                
                if !useLocal {
                    Section(header: Text(L10n.modelTestCloudLabel)) {
                        TextField(L10n.settingsBaseURL, text: $container.baseURLString)
                            .textFieldStyle(.roundedBorder)
                        SecureField(L10n.settingsAPIKey, text: $container.apiKey)
                            .textFieldStyle(.roundedBorder)
                        TextField(L10n.settingsModel, text: $container.model)
                            .textFieldStyle(.roundedBorder)
                        Button(action: { container.saveCredentials() }) {
                            HStack {
                                Image(systemName: "checkmark.circle.fill")
                                Text(L10n.settingsSaveAndTest)
                            }
                            .frame(maxWidth: .infinity)
                        }
                        .buttonStyle(.bordered)
                    }
                }
                
                Section(header: Text(L10n.modelTestMaxTokens)) {
                    Stepper(value: $maxTokens, in: 16...2048, step: 16) {
                        HStack {
                            Text(L10n.modelTestMaxTokens)
                            Spacer()
                            Text(String(maxTokens))
                                .foregroundColor(.secondary)
                        }
                    }
                }
            }
            .navigationTitle(L10n.settings)
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .navigationBarTrailing) {
                    Button(L10n.close) {
                        dismiss()
                    }
                }
            }
        }
    }
}

// Helper extension is defined centrally in MainTabView.swift to avoid duplicate declarations.

struct ModelTestView_Previews: PreviewProvider {
    static var previews: some View {
        ModelTestView()
            .environmentObject(ServiceContainer())
    }
}
