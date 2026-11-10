// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Provides bounded loading, readiness, search, empty-state, and user-input views.

import Combine
import Observation
import SwiftUI

struct MacUserInputCard: View {
    let request: MacRemoteUserInputRequest
    let submit: ([MacUserInputAnswer]) -> Void
    let cancel: () -> Void
    let later: () -> Void
    @State private var selectedOptions: [String: String] = [:]
    @State private var writtenAnswers: [String: String] = [:]

    private var answers: [MacUserInputAnswer] {
        request.questions.compactMap { question in
            if let optionID = selectedOptions[question.id], question.options.contains(where: { $0.id == optionID }) {
                return MacUserInputAnswer(questionID: question.id, optionID: optionID, text: nil)
            }
            let text = normalizedText(for: question)
            guard question.allowsFreeform, !text.isEmpty else { return nil }
            return MacUserInputAnswer(questionID: question.id, optionID: nil, text: text)
        }
    }

    private var canSubmit: Bool {
        request.isOpen && !request.questions.isEmpty && request.questions.allSatisfy { question in
            if selectedOptions[question.id].map({ selected in question.options.contains { $0.id == selected } }) == true {
                return true
            }
            let text = normalizedText(for: question)
            if text.isEmpty { return !question.required }
            return question.allowsFreeform && text.unicodeScalars.count <= question.maxTextCharacters
        }
    }

    private func normalizedText(for question: MacRemoteUserInputQuestion) -> String {
        (writtenAnswers[question.id] ?? "").trimmingCharacters(in: .whitespacesAndNewlines)
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 16) {
            HStack(alignment: .top, spacing: 10) {
                Image(systemName: "questionmark.bubble.fill")
                    .font(.system(size: 16, weight: .semibold))
                    .foregroundStyle(Color.accentColor)
                    .frame(width: 28, height: 28)
                    .accessibilityHidden(true)
                VStack(alignment: .leading, spacing: 3) {
                    Text(request.header)
                        .font(.system(size: 15, weight: .semibold))
                        .accessibilityAddTraits(.isHeader)
                    Text(String(localized: "Your answers help Nola continue. You can answer later."))
                        .font(.body)
                        .foregroundStyle(.secondary)
                        .fixedSize(horizontal: false, vertical: true)
                }
                Spacer(minLength: 8)
            }

            ForEach(request.questions) { question in
                questionForm(question)
            }

            Divider()
            ViewThatFits(in: .horizontal) {
                HStack(spacing: 12) {
                    laterButton
                    Spacer(minLength: 12)
                    declineButton
                    submitButton
                }
                VStack(alignment: .trailing, spacing: 12) {
                    submitButton
                    HStack(spacing: 12) { laterButton; declineButton }
                }
            }
        }
        .font(.body)
        .controlSize(.large)
        .padding(16)
        .background(MacDesignColor.canvas, in: RoundedRectangle(cornerRadius: 13, style: .continuous))
        .overlay {
            RoundedRectangle(cornerRadius: 13, style: .continuous)
                .stroke(MacDesignColor.borderSubtle, lineWidth: 1)
        }
        .accessibilityIdentifier("user-input-card-\(request.id)")
        .onChange(of: request) { oldRequest, newRequest in
            guard oldRequest.id != newRequest.id || oldRequest.revision != newRequest.revision
                || oldRequest.schemaDigest != newRequest.schemaDigest else { return }
            selectedOptions = [:]
            writtenAnswers = [:]
        }
    }

    @ViewBuilder
    private func questionForm(_ question: MacRemoteUserInputQuestion) -> some View {
        VStack(alignment: .leading, spacing: 10) {
            Text(question.prompt)
                .font(.system(size: 13, weight: .semibold))
                .fixedSize(horizontal: false, vertical: true)
                .textSelection(.enabled)
            Text(question.required ? String(localized: "Required") : String(localized: "Optional"))
                .foregroundStyle(.secondary)

            ForEach(question.options, id: \.id) { option in
                Button {
                    selectedOptions[question.id] = option.id
                } label: {
                    HStack(alignment: .top, spacing: 10) {
                        Image(systemName: selectedOptions[question.id] == option.id ? "largecircle.fill.circle" : "circle")
                            .foregroundStyle(selectedOptions[question.id] == option.id ? Color.accentColor : Color.secondary)
                            .accessibilityHidden(true)
                        VStack(alignment: .leading, spacing: 4) {
                            Text(option.label).fontWeight(.medium)
                            if !option.detail.isEmpty {
                                Text(option.detail).foregroundStyle(.secondary)
                                    .lineLimit(nil)
                                    .fixedSize(horizontal: false, vertical: true)
                            }
                        }
                        .fixedSize(horizontal: false, vertical: true)
                        .frame(maxWidth: .infinity, alignment: .leading)
                    }
                    .padding(10)
                    .frame(minHeight: 28)
                    .background(selectedOptions[question.id] == option.id
                        ? Color.accentColor.opacity(0.12) : MacDesignColor.canvas,
                        in: RoundedRectangle(cornerRadius: 8))
                    .overlay {
                        RoundedRectangle(cornerRadius: 8)
                            .stroke(selectedOptions[question.id] == option.id
                                ? Color.accentColor : MacDesignColor.borderSubtle, lineWidth: 1)
                    }
                    .contentShape(RoundedRectangle(cornerRadius: 8))
                }
                // Native bordered buttons constrain their title to a single
                // line even when AX reports the full multiline description.
                .buttonStyle(.plain)
                .disabled(!request.isOpen)
                .accessibilityAddTraits(selectedOptions[question.id] == option.id ? .isSelected : [])
                .accessibilityIdentifier("user-input-option-\(question.id)-\(option.id)")
            }

            if question.allowsFreeform {
                TextField(String(localized: "Write an answer"), text: Binding(
                    get: { writtenAnswers[question.id] ?? "" },
                    set: { value in
                        writtenAnswers[question.id] = value
                        selectedOptions[question.id] = nil
                    }
                ), axis: .vertical)
                .textFieldStyle(.roundedBorder)
                .lineLimit(2...6)
                .disabled(!request.isOpen)
                .accessibilityLabel(question.prompt)
                .accessibilityIdentifier("user-input-text-\(question.id)")

                if selectedOptions[question.id] == nil && normalizedText(for: question).unicodeScalars.count > question.maxTextCharacters {
                    Label(String(localized: "Answer is too long. Limit: \(question.maxTextCharacters) characters."), systemImage: "exclamationmark.circle")
                        .foregroundStyle(.red)
                        .fixedSize(horizontal: false, vertical: true)
                }
            }
            if !question.required && (selectedOptions[question.id] != nil || !normalizedText(for: question).isEmpty) {
                Button(String(localized: "Clear answer")) {
                    selectedOptions[question.id] = nil
                    writtenAnswers[question.id] = nil
                }
                .disabled(!request.isOpen)
            }
        }
        .padding(12)
        .background(Color(nsColor: .controlBackgroundColor), in: RoundedRectangle(cornerRadius: 9, style: .continuous))
        .accessibilityElement(children: .contain)
        .accessibilityIdentifier("user-input-question-\(question.id)")
    }

    private var laterButton: some View {
        Button(String(localized: "Answer later"), action: later)
            .help(String(localized: "Hide these questions without sending an answer."))
            .accessibilityIdentifier("user-input-later")
    }

    private var declineButton: some View {
        Button(String(localized: "Decline to answer"), action: cancel)
            .disabled(!request.isOpen)
            .help(String(localized: "Tell Nola you do not want to answer these questions."))
            .accessibilityIdentifier("user-input-decline")
    }

    private var submitButton: some View {
        Button(String(localized: "Send answers")) {
            guard canSubmit else { return }
            submit(answers)
        }
        .buttonStyle(.borderedProminent)
        .disabled(!canSubmit)
        .accessibilityIdentifier("user-input-submit")
    }
}

struct MacConversationLoadingView: View {
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    @State private var isVisible = false

    var body: some View {
        VStack(spacing: 12) {
            MacBreathingStatusLight(diameter: 22)
                .frame(width: 24, height: 24)
            Text(String(localized: "Loading"))
                .font(.system(size: 11.5, weight: .medium))
                .foregroundStyle(.secondary)
        }
        .padding(.horizontal, 20)
        .padding(.vertical, 16)
        .background(.thinMaterial, in: RoundedRectangle(cornerRadius: 14, style: .continuous))
        .overlay {
            RoundedRectangle(cornerRadius: 14, style: .continuous)
                .stroke(Color.primary.opacity(0.06), lineWidth: 0.5)
        }
        .opacity(isVisible ? 1 : 0)
        .offset(y: reduceMotion || isVisible ? 0 : 4)
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        .task {
            guard !isVisible else { return }
            if reduceMotion {
                isVisible = true
            } else {
                withAnimation(.timingCurve(0.18, 0.78, 0.24, 1, duration: 0.28)) {
                    isVisible = true
                }
            }
        }
        .accessibilityElement(children: .combine)
        .accessibilityLabel(String(localized: "Loading"))
    }
}

/// Pending decisions share a bounded, scrollable region; the editor remains
/// outside it and cannot be pushed out of the window by model-provided text.
struct MacConversationInterruptionSurface<Content: View>: View {
    let maxHeight: CGFloat
    @ViewBuilder let content: () -> Content
    @State private var contentHeight: CGFloat?

    var body: some View {
        ScrollView(.vertical) {
            VStack(alignment: .leading, spacing: 10, content: content)
                .frame(maxWidth: .infinity, alignment: .leading)
                .fixedSize(horizontal: false, vertical: true)
                .onGeometryChange(for: CGFloat.self) { geometry in
                    geometry.size.height
                } action: { height in
                    contentHeight = height
                }
        }
        // Start with a usable viewport. A one-point bootstrap clips the
        // controls before AppKit lays out the scroll document, so its geometry
        // callback may never provide the measurement needed to expand it.
        .frame(height: max(1, min(maxHeight, contentHeight ?? maxHeight)))
        .accessibilityIdentifier("chat.pending-decisions")
    }
}

struct MacExecutionReadinessBanner: View {
    let location: MacExecutionLocation
    @State private var refreshID = UUID()
    @Environment(\.openWindow) private var openWindow
    @Environment(\.openSettings) private var openSettings
    @Environment(\.accessibilityReduceMotion) private var reduceMotion

    var body: some View {
        Group {
            if let issue = MacReadinessService.blockingIssue(for: location) {
                HStack(spacing: 11) {
                    Image(systemName: issue.symbol)
                        .font(.system(size: 13, weight: .semibold))
                        .foregroundStyle(.orange)
                        .frame(width: 28, height: 28)
                        .background(Color.orange.opacity(0.10), in: Circle())
                    VStack(alignment: .leading, spacing: 2) {
                        Text(issue.title)
                            .font(.system(size: 11.5, weight: .semibold))
                        Text(issue.detail)
                            .font(.system(size: 10.5))
                            .foregroundStyle(.secondary)
                            .lineLimit(1)
                    }
                    Spacer(minLength: 12)
                    Button(issue.actionTitle) {
                        switch issue.action {
                        case .diagnostics: openWindow(id: "diagnostics-center")
                        case .models: openWindow(id: "model-center")
                        case .settings: openSettings()
                        }
                    }
                    .controlSize(.small)
                }
                .padding(.horizontal, 16)
                .frame(minHeight: 46)
                .background(Color.orange.opacity(0.045))
                .overlay(alignment: .bottom) { Divider().opacity(0.45) }
                .transition(reduceMotion ? .opacity : .move(edge: .top).combined(with: .opacity))
            }
        }
        .id(refreshID)
        .onReceive(NotificationCenter.default.publisher(for: .nolaMacConfigurationDidChange)) { _ in
            refreshID = UUID()
        }
        .animation(reduceMotion ? nil : .snappy(duration: 0.2), value: refreshID)
    }
}

struct MacConversationSearchBar: View {
    @Bindable var model: MacWorkspaceModel
    @FocusState private var focused: Bool

    var body: some View {
        HStack(spacing: 8) {
            Image(systemName: "magnifyingglass")
                .foregroundStyle(.secondary)
            TextField("Find in conversation", text: $model.transcriptSearchText)
                .textFieldStyle(.plain)
                .focused($focused)
            if !model.transcriptSearchText.isEmpty {
                Text("\(model.transcriptMatchCount) found")
                    .font(.system(size: 10.5, weight: .medium))
                    .foregroundStyle(.secondary)
            }
            Button {
                model.transcriptSearchText = ""
                model.isTranscriptSearchVisible = false
            } label: {
                Image(systemName: "xmark")
            }
            .buttonStyle(.plain)
            .help("Close Search")
        }
        .padding(.horizontal, 14)
        .frame(height: 36)
        .background(.bar)
        .overlay(alignment: .bottom) { Divider() }
        .onAppear { focused = true }
    }
}

struct MacEmptyConversationView: View {
    @Bindable var model: MacWorkspaceModel
    var composerFocused: FocusState<Bool>.Binding

    private struct Suggestion {
        let title: String
        let prompt: String
        let symbol: String
    }

    private var suggestions: [Suggestion] {
        [
            Suggestion(
                title: String(localized: "Review a project", table: "Chat"),
                prompt: String(localized: "Inspect this workspace and propose the next concrete changes.", table: "Chat"),
                symbol: "hammer"
            ),
            Suggestion(
                title: String(localized: "Work with a document", table: "Chat"),
                prompt: String(localized: "Help me understand and work with the document I attach.", table: "Chat"),
                symbol: "doc.text"
            ),
            Suggestion(
                title: String(localized: "Plan and execute", table: "Chat"),
                prompt: String(localized: "Break this objective into a clear plan, then execute it step by step.", table: "Chat"),
                symbol: "point.3.connected.trianglepath.dotted"
            ),
        ]
    }

    var body: some View {
        GeometryReader { viewport in
            let compact = viewport.size.height < 280
            ScrollView {
                VStack(spacing: compact ? 12 : 20) {
                    Spacer(minLength: compact ? 12 : 44)

                    if !compact {
                        ZStack {
                            Circle()
                                .fill(Color.accentColor.opacity(0.09))
                                .frame(width: 44, height: 44)
                            Image(systemName: "sparkles")
                                .font(.system(size: 17, weight: .medium))
                                .foregroundStyle(Color.accentColor)
                        }
                    }
                    VStack(spacing: 8) {
                        Text("What should we work on?")
                            .accessibilityIdentifier("chat.empty-state.title")
                            .font(.system(size: 22, weight: .semibold, design: .rounded))
                        Text("Ask a question, open a workspace, or bring in a document.")
                            .font(.system(size: 13))
                            .foregroundStyle(.secondary)
                    }

                    ViewThatFits(in: .horizontal) {
                        HStack(spacing: 8) {
                            ForEach(suggestions, id: \.title) { suggestion in
                                suggestionButton(suggestion)
                                    .frame(maxWidth: .infinity)
                            }
                        }
                        .frame(maxWidth: 620)

                        VStack(spacing: 8) {
                            ForEach(suggestions, id: \.title) { suggestion in
                                suggestionButton(suggestion)
                            }
                        }
                        .frame(maxWidth: 420)
                    }

                    Spacer(minLength: compact ? 16 : 80)
                }
                .frame(maxWidth: .infinity)
                .padding(.horizontal, 32)
            }
        }
    }

    private func suggestionButton(_ suggestion: Suggestion) -> some View {
        Button {
            model.draft = suggestion.prompt
            composerFocused.wrappedValue = true
        } label: {
            HStack(spacing: 9) {
                Image(systemName: suggestion.symbol)
                    .font(.system(size: 14, weight: .semibold))
                    .foregroundStyle(Color.accentColor)
                    .frame(width: 18)
                Text(suggestion.title)
                    .font(.system(size: 12.5, weight: .semibold))
                    .lineLimit(1)
                Spacer(minLength: 6)
                Image(systemName: "chevron.forward")
                    .font(.system(size: 8, weight: .bold))
                    .foregroundStyle(.tertiary)
            }
            .padding(.horizontal, 12)
            .frame(maxWidth: .infinity, minHeight: 42)
            .background(Color.primary.opacity(0.045), in: RoundedRectangle(cornerRadius: 10, style: .continuous))
            .overlay {
                RoundedRectangle(cornerRadius: 10, style: .continuous)
                    .stroke(Color.primary.opacity(0.06), lineWidth: 0.5)
            }
        }
        .buttonStyle(.plain)
        .help(suggestion.prompt)
    }
}
