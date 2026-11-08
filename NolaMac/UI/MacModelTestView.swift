// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Runs repeatable local-MLX and Chat-Only model checks through production chat services.

import SwiftUI

enum MacModelTestLayoutPolicy {
    static func usesCompactLayout(availableWidth: CGFloat) -> Bool {
        availableWidth < 820
    }
}

/// A model test uses the same stored route contract as a normal chat turn.
/// Keeping this construction in one place makes a test of a Responses or
/// Anthropic profile meaningful, rather than silently testing a default route.
enum MacModelTestRunRequestFactory {
    static func remote(
        prompt: String,
        configuration: MacAppleChatConfiguration,
        environment: MacAppEnvironment = .shared,
        applicationPort: MacApplicationPort = .shared
    ) throws -> MacLocalAgentRunRequest {
        var reasoning = configuration.reasoning ?? .init()
        reasoning.responses = configuration.wireProtocol.usesResponsesTransport
        reasoning.outputLimit = configuration.parameters.maxOutputTokens
        return try MacLocalAgentRunRequest(
            conversationID: "model-test-\(UUID().uuidString.lowercased())",
            prompt: prompt,
            projectURL: environment.scratchWorkspaceURL,
            attachmentURLs: [],
            modelID: configuration.model,
            modelProfileID: configuration.profileID, modelProfileRevision: configuration.profileRevision,
            endpoint: configuration.baseURL.absoluteString,
            apiKey: configuration.apiKey,
            contextWindowTokens: configuration.contextWindowTokens,
            modelPurpose: .chat,
            modelTestMode: true,
            reasoningConfiguration: reasoning,
            reasoningPreference: configuration.parameters.reasoningPreference ?? "automatic",
            modelParameters: configuration.parameters,
            modelExecutionCapabilities: configuration.executionCapabilities,
            wireProtocol: configuration.wireProtocol,
            samplingOverride: (configuration.executionCapabilities.samplingParameters
                && !configuration.executionCapabilities.samplingRequiresNoReasoning)
                || configuration.parameters.temperature != 0.7
                || configuration.parameters.topP != 0.9,
            memoryEnabled: false,
            permissionMode: .cautious,
            appEnvironment: environment,
            applicationPort: applicationPort
        )
    }

    static func local(
        prompt: String,
        profile: MacLocalModelProfile,
        parameters: ModelParameters,
        environment: MacAppEnvironment = .shared
    ) throws -> MacLocalAgentRunRequest {
        try MacLocalAgentRunRequest(
            conversationID: "model-test-\(UUID().uuidString.lowercased())",
            prompt: prompt,
            projectURL: environment.scratchWorkspaceURL,
            attachmentURLs: [],
            modelID: profile.id,
            contextWindowTokens: parameters.maxContextLength,
            modelTestMode: true,
            modelParameters: parameters,
            wireProtocol: .chatCompletions,
            samplingOverride: parameters.temperature != 0.7 || parameters.topP != 0.9,
            memoryEnabled: false,
            permissionMode: .cautious,
            appEnvironment: environment
        )
    }
}

private func macModelTestText(_ key: String, _ fallback: String) -> String {
    let keyed = Bundle.main.localizedString(forKey: key, value: nil, table: nil)
    guard keyed == key else { return keyed }
    return Bundle.main.localizedString(forKey: fallback, value: fallback, table: nil)
}

struct MacModelTestView: View {
    private enum CompactPane: String, CaseIterable, Identifiable {
        case setup
        case output

        var id: String { rawValue }
        var title: String {
            switch self {
            case .setup: macModelTestText("mac.model_test.setup", "Setup")
            case .output: macModelTestText("mac.model_test.output", "Output")
            }
        }
    }

    private enum Target: String, CaseIterable, Identifiable {
        case local
        case provider

        var id: String { rawValue }
        var title: String {
            switch self {
            case .local: return macModelTestText("mac.model_test.target.local", "Local Model")
            case .provider: return macModelTestText("mac.model_test.target.provider", "API Model")
            }
        }
        var symbol: String {
            switch self {
            case .local: return "cpu.fill"
            case .provider: return "cloud.fill"
            }
        }
    }

    @State private var target = Target.local
    @State private var prompt = macModelTestText("mac.model_test.default_prompt", "Explain what Nola can do in three concise bullet points.")
    @State private var response = ""
    @State private var journalReference: MacJournalMessageReference?
    @State private var errorMessage: String?
    @State private var isRunning = false
    @State private var startedAt: Date?
    @State private var firstTokenLatency: TimeInterval?
    @State private var totalDuration: TimeInterval?
    @State private var runTask: Task<Void, Never>?
    @State private var runGeneration = UUID()
    @State private var compactPane = CompactPane.setup
    @Environment(\.openWindow) private var openWindow
    @Environment(\.accessibilityReduceMotion) private var reduceMotion

    var body: some View {
        VStack(spacing: 0) {
            header
            GeometryReader { proxy in
                if MacModelTestLayoutPolicy.usesCompactLayout(availableWidth: proxy.size.width) {
                    compactContent
                } else {
                    HSplitView {
                        configurationPane
                            .frame(minWidth: 300, idealWidth: 340, maxWidth: 420)
                        resultPane
                            .frame(minWidth: 440)
                    }
                }
            }
        }
        .background(MacPageBackground())
        .onDisappear { stop() }
    }

    private var compactContent: some View {
        VStack(spacing: 0) {
            Picker(macModelTestText("mac.model_test.pane", "Model test section"), selection: $compactPane) {
                ForEach(CompactPane.allCases) { pane in
                    Text(pane.title).tag(pane)
                }
            }
            .pickerStyle(.segmented)
            .labelsHidden()
            .padding(.horizontal, MacDesignTokens.Layout.pageGutter)
            .padding(.vertical, MacDesignTokens.Spacing.sm)

            Divider().opacity(0.45)

            Group {
                switch compactPane {
                case .setup: configurationPane
                case .output: resultPane
                }
            }
            .id(compactPane)
            .transition(.opacity)
            .animation(reduceMotion ? nil : MacDesignTokens.Motion.pageChange, value: compactPane)
        }
    }

    private var header: some View {
        MacPageHeader(
            LocalizedStringKey(macModelTestText("mac.model_test.title", "Model Test")),
            symbol: "gauge.with.dots.needle.67percent"
        ) {
            Text(macModelTestText("mac.model_test.subtitle", "Run the production inference path and inspect timing and output"))
        } actions: {
            Button(macModelTestText("mac.model_test.parameters", "Parameters…"), action: openParameters)
            Button(isRunning ? macModelTestText("mac.model_test.stop", "Stop") : macModelTestText("mac.model_test.run", "Run")) {
                isRunning ? stop() : run()
            }
            .buttonStyle(.borderedProminent)
            .disabled(!isRunning && !canRun)
            .keyboardShortcut(.return, modifiers: [.command])
        }
    }

    private var configurationPane: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 16) {
                VStack(alignment: .leading, spacing: 9) {
                    Label(macModelTestText("mac.model_test.target", "Target"), systemImage: target.symbol)
                        .font(.system(size: 13, weight: .semibold))
                    Picker(macModelTestText("mac.model_test.target", "Target"), selection: $target) {
                        ForEach(Target.allCases) { value in
                            Text(value.title).tag(value)
                        }
                    }
                    .labelsHidden()
                    .pickerStyle(.segmented)
                    .disabled(isRunning)
                    statusRow
                }
                .padding(15)
                .macPanel(cornerRadius: MacDesignTokens.Radius.floating)

                VStack(alignment: .leading, spacing: 8) {
                    Text(macModelTestText("mac.model_test.prompt", "Prompt"))
                        .font(.system(size: 13, weight: .semibold))
                    TextEditor(text: $prompt)
                        .font(.system(size: 12.5))
                        .frame(minHeight: 190)
                        .padding(8)
                        .scrollContentBackground(.hidden)
                        .background(MacDesignColor.chrome, in: RoundedRectangle(cornerRadius: MacDesignTokens.Radius.field))
                        .overlay { RoundedRectangle(cornerRadius: MacDesignTokens.Radius.field).stroke(MacDesignColor.borderSubtle, lineWidth: 0.5) }
                        .disabled(isRunning)
                }

                metricsCard
            }
            .padding(18)
        }
    }

    private var statusRow: some View {
        HStack(spacing: 8) {
            Circle()
                .fill(canRun ? Color.green : Color.orange)
                .frame(width: 7, height: 7)
            Text(activeModelName)
                .font(.system(size: 11.5, weight: .medium))
                .lineLimit(1)
            Spacer()
            Text(canRun ? macModelTestText("mac.model_test.ready", "Ready") : macModelTestText("mac.model_test.not_configured", "Not configured"))
                .font(.system(size: 10.5))
                .foregroundStyle(.secondary)
        }
        .accessibilityElement(children: .combine)
    }

    private var metricsCard: some View {
        VStack(alignment: .leading, spacing: 10) {
            Label(macModelTestText("mac.model_test.metrics", "Metrics"), systemImage: "stopwatch")
                .font(.system(size: 13, weight: .semibold))
            metricRow(macModelTestText("mac.model_test.first_token", "First token"), value: duration(firstTokenLatency))
            metricRow(macModelTestText("mac.model_test.total_time", "Total time"), value: duration(totalDuration))
            metricRow(macModelTestText("mac.model_test.characters", "Characters"), value: response.count.formatted())
        }
        .padding(15)
        .macPanel(cornerRadius: MacDesignTokens.Radius.floating)
    }

    private var resultPane: some View {
        VStack(spacing: 0) {
            HStack {
                Label(macModelTestText("mac.model_test.output", "Output"), systemImage: "text.bubble")
                    .font(.system(size: 13.5, weight: .semibold))
                if isRunning {
                    ProgressView().controlSize(.small)
                    Text(macModelTestText("mac.model_test.running", "Running"))
                        .font(.system(size: 10.5))
                        .foregroundStyle(.secondary)
                }
                Spacer()
                Button(macModelTestText("mac.model_test.clear", "Clear")) {
                    response = ""
                    journalReference = nil
                    errorMessage = nil
                    firstTokenLatency = nil
                    totalDuration = nil
                }
                .disabled(isRunning || (journalReference == nil && response.isEmpty && errorMessage == nil))
            }
            .padding(16)
            Divider().opacity(0.45)
            ScrollView {
                VStack(alignment: .leading, spacing: 12) {
                    if let errorMessage {
                        MacInlineNotice(
                            title: macModelTestText("mac.model_test.failed", "Model Test Failed"),
                            message: errorMessage,
                            kind: .failure,
                            actionTitle: macModelTestText("mac.model_test.run_again", "Run Again"),
                            action: run,
                            dismiss: { self.errorMessage = nil }
                        )
                    } else if let journalReference {
                        MacJournalMessageTextView(reference: journalReference)
                    } else if response.isEmpty {
                        ContentUnavailableView(
                            macModelTestText("mac.model_test.empty.title", "No Output Yet"),
                            systemImage: "text.bubble",
                            description: Text(macModelTestText("mac.model_test.empty.detail", "Run a model test to inspect its response here."))
                        )
                        .frame(maxWidth: .infinity, minHeight: 360)
                    } else {
                        MacMarkdownView(markdown: response, isStreaming: isRunning, animatesAppends: isRunning)
                            .textSelection(.enabled)
                    }
                }
                .padding(20)
                .frame(maxWidth: .infinity, alignment: .leading)
            }
        }
    }

    private var canRun: Bool {
        guard !prompt.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty else { return false }
        switch target {
        case .local:
            return MacLocalModelConfigurationStore.shared.isSelectedModelReady
        case .provider:
            return MacAppleChatConfigurationStore.shared.load() != nil
        }
    }

    private var activeModelName: String {
        switch target {
        case .local:
            let store = MacLocalModelConfigurationStore.shared
            return store.isSelectedModelReady ? store.selectedProfile.displayName : macModelTestText("mac.model_test.no_local_model", "No local model downloaded")
        case .provider:
            return MacAppleChatConfigurationStore.shared.displayValues().model.isEmpty
                ? macModelTestText("mac.model_test.not_configured", "Not configured")
                : MacAppleChatConfigurationStore.shared.displayValues().model
        }
    }

    private func run() {
        guard canRun else { return }
        stop()
        let generation = UUID()
        runGeneration = generation
        let runTarget = target
        let runPrompt = prompt
        compactPane = .output
        response = ""
        journalReference = nil
        errorMessage = nil
        firstTokenLatency = nil
        totalDuration = nil
        let start = Date()
        startedAt = start
        isRunning = true
        runTask = Task { @MainActor in
            do {
                let request: MacLocalAgentRunRequest
                switch runTarget {
                case .local:
                    let store = MacLocalModelConfigurationStore.shared
                    let profile = store.selectedProfile
                    _ = try await MacMLXOpenAIService.shared.start(profile)
                    try Task.checkCancellation()
                    request = try MacModelTestRunRequestFactory.local(
                        prompt: runPrompt,
                        profile: profile,
                        parameters: store.parameters(for: profile)
                    )
                case .provider:
                    guard let configuration = MacAppleChatConfigurationStore.shared.load() else {
                        throw MacRemoteModelRoutingError.allConfiguredModelsCoolingDown
                    }
                    request = try MacModelTestRunRequestFactory.remote(
                        prompt: runPrompt,
                        configuration: configuration
                    )
                }

                let stream = await MacAgentHostRuntime.shared.run(request)
                for try await event in stream {
                    guard runGeneration == generation else { return }
                    guard let cursor = event.assistantJournalCursor else { continue }
                    guard cursor.identity.conversation_id == request.conversationID,
                          cursor.identity.run_id == request.submissionID,
                          cursor.identity.assistant_message_id == request.assistantMessageID,
                          let port = request.applicationPort else { throw MacRemoteHostError.invalidResponse }
                    journalReference = MacJournalMessageReference(databaseURL: port.databaseURL, cursor: cursor)
                    if firstTokenLatency == nil {
                        firstTokenLatency = Date().timeIntervalSince(start)
                    }
                }
                guard runGeneration == generation else { return }
                if let journalReference {
                    response = try await MacJournalTextRead.markdown(journalReference, historicalFallback: "")
                }
                guard runGeneration == generation else { return }
                totalDuration = Date().timeIntervalSince(start)
            } catch is CancellationError {
                guard runGeneration == generation else { return }
                totalDuration = Date().timeIntervalSince(start)
            } catch {
                guard runGeneration == generation else { return }
                errorMessage = error.localizedDescription
                totalDuration = Date().timeIntervalSince(start)
            }
            guard runGeneration == generation else { return }
            isRunning = false
            runTask = nil
        }
    }

    private func stop() {
        runGeneration = UUID()
        runTask?.cancel()
        runTask = nil
        if isRunning, let startedAt {
            totalDuration = Date().timeIntervalSince(startedAt)
        }
        isRunning = false
    }

    private func openParameters() {
        switch target {
        case .local:
            let model = MacLocalModelConfigurationStore.shared.selectedProfile
            openWindow(value: MacModelParametersRoute(
                modelName: model.id,
                isLocal: true
            ))
        case .provider:
            guard let configuration = MacAppleChatConfigurationStore.shared.load(),
                  let profileID = configuration.profileID else { return }
            openWindow(value: MacModelParametersRoute(
                modelName: profileID, isLocal: false, displayName: configuration.model
            ))
        }
    }

    private func metricRow(_ title: String, value: String) -> some View {
        HStack {
            Text(title).foregroundStyle(.secondary)
            Spacer()
            Text(value).monospacedDigit()
        }
        .font(.system(size: 11.5))
    }

    private func duration(_ value: TimeInterval?) -> String {
        guard let value else { return "—" }
        if value < 1 { return String(format: macModelTestText("mac.model_test.milliseconds", "%d ms"), Int(value * 1_000)) }
        return String(format: macModelTestText("mac.model_test.seconds", "%.2f s"), value)
    }
}
