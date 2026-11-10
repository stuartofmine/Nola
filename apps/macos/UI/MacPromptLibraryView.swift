// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Manages reusable prompts with native desktop editing, import, and export.

import AppKit
import Combine
import SwiftUI
import UniformTypeIdentifiers

enum MacPromptLibraryLayoutPolicy {
    static func usesCompactNavigation(availableWidth: CGFloat) -> Bool {
        availableWidth < 760
    }
}

struct MacPromptLibraryView: View {
    @StateObject private var service = MacPromptLibrary.service
    @State private var selectedID: UUID?
    @State private var defaultTemplateID: UUID?
    @State private var editorTemplate: PromptTemplate?
    @State private var showsEditor = false
    @State private var showsImporter = false
    @State private var errorMessage: String?
    @State private var query = ""
    @State private var pendingDeletion: PromptTemplate?
    @State private var compactDetailIsPresented = false
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    let navigationAction: (() -> Void)?

    init(navigationAction: (() -> Void)? = nil) {
        self.navigationAction = navigationAction
    }

    var body: some View {
        VStack(spacing: 0) {
            promptLibraryHeader

            if let errorMessage = errorMessage ?? service.catalogLoadError {
                MacInlineNotice(
                    title: String(localized: "Prompt Template Error"),
                    message: errorMessage,
                    kind: .failure,
                    dismiss: { self.errorMessage = nil }
                )
                .padding(.horizontal, MacDesignTokens.Spacing.lg)
                .padding(.top, MacDesignTokens.Spacing.sm)
                .transition(.opacity.combined(with: .move(edge: .top)))
            }

            MacPageMetricsBar(metrics: promptMetrics)
                .padding(.horizontal, MacDesignTokens.Spacing.lg)
                .padding(.vertical, MacDesignTokens.Spacing.sm)

            promptLibrarySplitView
        }
        .background(MacPageBackground())
        .task { do { defaultTemplateID = try MacPromptLibrary.defaultTemplateID() } catch { errorMessage = error.localizedDescription } }
        .task {
            await Task.yield()
            if selectedID == nil { selectedID = service.templates.first?.id }
        }
        .onChange(of: filteredTemplates.map(\.id)) { _, ids in
            guard let selectedID, !ids.contains(selectedID) else { return }
            self.selectedID = ids.first
        }
        .sheet(isPresented: $showsEditor) {
            MacPromptTemplateEditor(template: editorTemplate) { template in
                if editorTemplate == nil {
                    try service.addTemplate(template)
                } else {
                    try service.updateTemplate(template)
                }
                selectedID = template.id
                showsEditor = false
            } cancel: {
                showsEditor = false
            }
        }
        .fileImporter(isPresented: $showsImporter, allowedContentTypes: [.json], allowsMultipleSelection: false) { result in
            do {
                guard let url = try result.get().first else { return }
                let accessed = url.startAccessingSecurityScopedResource()
                defer { if accessed { url.stopAccessingSecurityScopedResource() } }
                let template = try service.importTemplate(from: Data(contentsOf: url))
                try service.addTemplate(template)
                selectedID = template.id
            } catch {
                errorMessage = error.localizedDescription
            }
        }
        .confirmationDialog(String(localized: "Delete Prompt Template?"), isPresented: Binding(
            get: { pendingDeletion != nil },
            set: { if !$0 { pendingDeletion = nil } }
        ), titleVisibility: .visible) {
            Button(String(localized: "Cancel"), role: .cancel) { pendingDeletion = nil }
            Button(String(localized: "Delete"), role: .destructive) {
                guard let template = pendingDeletion else { return }
                do {
                    try service.deleteTemplate(id: template.id)
                    pendingDeletion = nil
                    selectedID = filteredTemplates.first?.id
                } catch { errorMessage = error.localizedDescription }
            }
        } message: {
            Text(String(localized: "This removes the custom template from this Mac. This action cannot be undone."))
        }
        .animation(reduceMotion ? nil : .easeOut(duration: 0.18), value: errorMessage)
        .animation(
            reduceMotion ? nil : MacDesignTokens.Motion.disclosureAnimation,
            value: compactDetailIsPresented
        )
    }

    private var filteredTemplates: [PromptTemplate] {
        let normalized = query.trimmingCharacters(in: .whitespacesAndNewlines)
        let readable = service.displayedTemplates.filter { service.canReadTemplate($0) }
        guard !normalized.isEmpty else { return readable }
        return readable.filter { template in
            template.name.localizedCaseInsensitiveContains(normalized)
                || template.description.localizedCaseInsensitiveContains(normalized)
                || template.content.localizedCaseInsensitiveContains(normalized)
                || (template.language?.localizedCaseInsensitiveContains(normalized) ?? false)
        }
    }

    private var selectedTemplate: PromptTemplate? {
        guard let selectedID else { return nil }
        return service.readableTemplate(id: selectedID)
    }

    private var builtInTemplates: [PromptTemplate] {
        filteredTemplates.filter(\.isBuiltIn)
    }

    private var customTemplates: [PromptTemplate] {
        filteredTemplates.filter { !$0.isBuiltIn }
    }

    private var promptTemplateList: some View {
        List {
            promptTemplateListContent
        }
        .listStyle(.sidebar)
        .searchable(
            text: $query,
            placement: .automatic,
            prompt: Text(verbatim: String(localized: "Search templates"))
        )
    }

    private var promptLibrarySplitView: some View {
        GeometryReader { proxy in
            if MacPromptLibraryLayoutPolicy.usesCompactNavigation(availableWidth: proxy.size.width) {
                compactPromptLibrary
            } else {
                MacDesktopSplitView(
                    sidebarMinWidth: 230,
                    sidebarIdealWidth: 260,
                    sidebarMaxWidth: 300
                ) {
                    promptTemplateList
                } detail: {
                    promptTemplateDetail
                }
            }
        }
    }

    @ViewBuilder
    private var compactPromptLibrary: some View {
        if compactDetailIsPresented, selectedTemplate != nil {
            VStack(spacing: 0) {
                HStack {
                    Button {
                        compactDetailIsPresented = false
                    } label: {
                        Label(String(localized: "Prompt Library"), systemImage: "chevron.left")
                    }
                    .buttonStyle(.borderless)
                    Spacer()
                }
                .padding(.horizontal, MacDesignTokens.Spacing.lg)
                .padding(.vertical, MacDesignTokens.Spacing.sm)
                .background(MacDesignColor.chrome.opacity(0.96))
                .overlay(alignment: .bottom) { Divider().opacity(0.55) }

                promptTemplateDetail
            }
            .transition(.move(edge: .trailing).combined(with: .opacity))
        } else {
            promptTemplateList
                .transition(.move(edge: .leading).combined(with: .opacity))
        }
    }

    @ViewBuilder
    private var promptTemplateDetail: some View {
        if let template = selectedTemplate {
            MacPromptTemplateDetail(
                template: template,
                isDefault: defaultTemplateID == template.id,
                useAsDefault: {
                    do { try MacPromptLibrary.setDefaultTemplateID(template.id); defaultTemplateID = template.id; service.objectWillChange.send() }
                    catch { errorMessage = error.localizedDescription }
                },
                duplicate: {
                    do { selectedID = try service.duplicateTemplate(id: template.id).id }
                    catch { errorMessage = error.localizedDescription }
                },
                edit: {
                    do {
                        let fresh = try service.requireReadableTemplate(id: template.id)
                        guard service.canEditTemplate(fresh) else { throw PromptTemplateReadError.denied(template.id) }
                        editorTemplate = fresh
                        showsEditor = true
                    } catch { errorMessage = error.localizedDescription }
                },
                export: { exportTemplate(template) },
                delete: {
                    pendingDeletion = template
                }
            )
        } else {
            MacEmptyState(
                LocalizedStringKey(String(localized: "Prompt Library")),
                description: LocalizedStringKey(String(localized: "Create reusable instructions without adding controls to the composer.")),
                symbol: "text.book.closed"
            ) {}
        }
    }

    private var promptLibraryHeader: some View {
        MacPageHeader(
            LocalizedStringKey(String(localized: "Prompt Library")),
            symbol: "text.book.closed",
            navigationAction: navigationAction
        ) {
            Text(verbatim: String(localized: "Create reusable instructions without adding controls to the composer."))
        } actions: {
            importTemplateButton
            newTemplateButton
        }
    }

    private var importTemplateButton: some View {
        Button {
            showsImporter = true
        } label: {
            Image(systemName: "square.and.arrow.down")
        }
        .help(String(localized: "Import Template"))
        .accessibilityLabel(String(localized: "Import Template"))
        .macToolbarIcon()
    }

    private var newTemplateButton: some View {
        Button {
            editorTemplate = nil
            showsEditor = true
        } label: {
            Image(systemName: "plus")
        }
        .help(String(localized: "New Template"))
        .accessibilityLabel(String(localized: "New Template"))
        .macToolbarIcon()
    }

    @ViewBuilder
    private var promptTemplateListContent: some View {
        if filteredTemplates.isEmpty {
            ContentUnavailableView.search(text: query)
        }
        if !builtInTemplates.isEmpty {
            Section {
                ForEach(builtInTemplates) { template in
                    selectableTemplateRow(template)
                }
            } header: {
                Text(verbatim: String(localized: "Built-in"))
            }
        }
        if !customTemplates.isEmpty {
            Section {
                ForEach(customTemplates) { template in
                    selectableTemplateRow(template)
                }
            } header: {
                Text(verbatim: String(localized: "Custom"))
            }
        }
    }

    private func selectableTemplateRow(_ template: PromptTemplate) -> some View {
        Button {
            selectedID = template.id
            compactDetailIsPresented = true
        } label: {
            templateRow(template)
                .contentShape(Rectangle())
        }
            .buttonStyle(.plain)
            .macSelectableListRow(isSelected: selectedID == template.id)
            .accessibilityAddTraits(selectedID == template.id ? .isSelected : [])
    }

    private var promptMetrics: [MacPageMetric] {
        [
            MacPageMetric(id: "prompts-total", LocalizedStringKey(String(localized: "Prompt Library")), value: "\(service.templates.count)", symbol: "text.book.closed", tint: .blue),
            MacPageMetric(id: "prompts-built-in", LocalizedStringKey(String(localized: "Built-in")), value: "\(service.templates.filter(\.isBuiltIn).count)", symbol: "checkmark.seal.fill", tint: .indigo),
            MacPageMetric(id: "prompts-custom", LocalizedStringKey(String(localized: "Custom")), value: "\(service.templates.filter { !$0.isBuiltIn }.count)", symbol: "person.crop.circle.fill", tint: .green),
        ]
    }

    private func templateRow(_ template: PromptTemplate) -> some View {
        HStack(spacing: 9) {
            Image(systemName: template.isBuiltIn ? "text.book.closed" : "doc.text")
                .foregroundStyle(template.isBuiltIn ? Color.accentColor : Color.green)
                .frame(width: 17)
            VStack(alignment: .leading, spacing: 2) {
                Text(template.name).font(.system(size: 13, weight: .medium)).lineLimit(1)
                Text(template.language ?? String(localized: "Any language"))
                    .font(.system(size: 11)).foregroundStyle(.tertiary)
            }
            Spacer()
            if defaultTemplateID == template.id {
                Image(systemName: "checkmark.circle.fill")
                    .foregroundStyle(.green)
                    .accessibilityLabel(String(localized: "Default"))
            }
        }
        .padding(.vertical, 2)
    }

    private func exportTemplate(_ template: PromptTemplate) {
        do {
            let data = try service.exportTemplate(template)
            let panel = NSSavePanel()
            panel.nameFieldStringValue = "\(template.name).json"
            panel.allowedContentTypes = [.json]
            panel.begin { response in
                guard response == .OK, let url = panel.url else { return }
                do { try data.write(to: url, options: .atomic) }
                catch { Task { @MainActor in errorMessage = error.localizedDescription } }
            }
        } catch {
            errorMessage = error.localizedDescription
        }
    }
}

private struct MacPromptTemplateDetail: View {
    let template: PromptTemplate
    let isDefault: Bool
    let useAsDefault: () -> Void
    let duplicate: () -> Void
    let edit: () -> Void
    let export: () -> Void
    let delete: () -> Void

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 20) {
                ViewThatFits(in: .horizontal) {
                    HStack(alignment: .top, spacing: 20) {
                        templateIdentity
                        Spacer(minLength: 16)
                        defaultControl
                    }
                    VStack(alignment: .leading, spacing: 14) {
                        templateIdentity
                        defaultControl
                    }
                }

                ViewThatFits(in: .horizontal) {
                    HStack(spacing: 8) { actionButtons }
                    VStack(alignment: .leading, spacing: 8) { actionButtons }
                }
                .controlSize(.regular)

                VStack(alignment: .leading, spacing: 8) {
                    Text(String(localized: "Prompt").uppercased()).font(.system(size: 11, weight: .bold)).foregroundStyle(.tertiary).tracking(0.8)
                    Text(template.content)
                        .font(.system(size: 12, design: .monospaced))
                        .textSelection(.enabled)
                        .padding(14)
                        .frame(maxWidth: .infinity, alignment: .leading)
                        .background(Color.primary.opacity(0.035), in: RoundedRectangle(cornerRadius: 11))
                }

                HStack {
                    Label(
                        String(
                            format: String(localized: "%d tokens"),
                            MacPromptLibrary.service.estimateTokenCount(template.content)
                        ),
                        systemImage: "number"
                    )
                    if let language = template.language { Label(language, systemImage: "globe") }
                }
                .font(.system(size: 12)).foregroundStyle(.secondary)
            }
            .padding(28)
            .frame(maxWidth: 860, alignment: .leading)
            .frame(maxWidth: .infinity)
        }
        .background(MacPageBackground())
    }

    private var templateIdentity: some View {
        VStack(alignment: .leading, spacing: 5) {
            Text(template.name)
                .font(.system(size: 19, weight: .semibold))
                .fixedSize(horizontal: false, vertical: true)
            if template.isBuiltIn {
                MacStatusBadge(
                    title: String(localized: "Built-in"),
                    tint: .accentColor,
                    symbol: "checkmark.seal.fill"
                )
            }
            if !template.description.isEmpty {
                Text(template.description)
                    .foregroundStyle(.secondary)
                    .fixedSize(horizontal: false, vertical: true)
            }
        }
    }

    @ViewBuilder
    private var defaultControl: some View {
        if isDefault {
            MacStatusBadge(
                title: String(localized: "Default"),
                tint: .green,
                symbol: "checkmark.circle.fill"
            )
        } else {
            Button(action: useAsDefault) {
                Label(String(localized: "Use as Default"), systemImage: "checkmark.circle")
            }
                .buttonStyle(.borderedProminent)
        }
    }

    @ViewBuilder
    private var actionButtons: some View {
        if !template.isBuiltIn {
            Button(action: edit) {
                Label(String(localized: "Edit"), systemImage: "square.and.pencil")
            }
        }
        Button(action: duplicate) {
            Label(String(localized: "Duplicate"), systemImage: "plus.square.on.square")
        }
        Button(action: export) {
            Label(String(localized: "Export"), systemImage: "square.and.arrow.up")
        }
        if !template.isBuiltIn {
            Menu {
                Button(String(localized: "Delete"), role: .destructive, action: delete)
            } label: {
                Label(String(localized: "More"), systemImage: "ellipsis")
            }
            .menuStyle(.borderlessButton)
        }
    }
}

private struct MacPromptTemplateEditor: View {
    let original: PromptTemplate?
    let save: (PromptTemplate) throws -> Void
    let cancel: () -> Void

    @State private var name: String
    @State private var language: String
    @State private var description: String
    @State private var content: String
    @State private var showsDiscardConfirmation = false
    @State private var saveError: String?
    @FocusState private var nameFocused: Bool
    @Environment(\.accessibilityReduceMotion) private var reduceMotion

    init(template: PromptTemplate?, save: @escaping (PromptTemplate) throws -> Void, cancel: @escaping () -> Void) {
        original = template
        self.save = save
        self.cancel = cancel
        _name = State(initialValue: template?.name ?? "")
        _language = State(initialValue: template?.language ?? "")
        _description = State(initialValue: template?.description ?? "")
        _content = State(initialValue: template?.content ?? "{{context}}")
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 14) {
            HStack(alignment: .top, spacing: 12) {
                MacIconTile(symbol: original == nil ? "doc.badge.plus" : "square.and.pencil", tint: MacDesignColor.brand)
                VStack(alignment: .leading, spacing: 3) {
                    Text(original == nil
                        ? String(localized: "New Prompt Template")
                        : String(localized: "Edit Prompt Template"))
                        .font(.system(size: 19, weight: .semibold))
                    Text(String(localized: "Create reusable instructions for conversations and automations."))
                        .font(.system(size: 11.5))
                        .foregroundStyle(.secondary)
                }
            }
            Form {
                if let saveError { Text(saveError).foregroundStyle(.red) }
                TextField(String(localized: "Name"), text: $name)
                    .focused($nameFocused)
                TextField(String(localized: "Language (optional)"), text: $language, prompt: Text("en, zh-Hans…"))
                TextField(String(localized: "Description"), text: $description, axis: .vertical)
                    .lineLimit(2...4)
                TextEditor(text: $content)
                    .font(.system(size: 12, design: .monospaced))
                    .frame(minHeight: 220)
                LabeledContent(String(localized: "Variables")) {
                    ViewThatFits(in: .horizontal) {
                        HStack {
                            variableButtons
                        }
                        VStack(alignment: .trailing, spacing: 6) {
                            variableButtons
                        }
                    }
                }
                LabeledContent(
                    String(localized: "Estimated size"),
                    value: String(
                        format: String(localized: "%d tokens"),
                        MacPromptLibrary.service.estimateTokenCount(content)
                    )
                )
            }
            .formStyle(.grouped)
            HStack {
                if let validationMessage {
                    Label(validationMessage, systemImage: "exclamationmark.circle.fill")
                        .font(.caption)
                        .foregroundStyle(.orange)
                        .fixedSize(horizontal: false, vertical: true)
                        .transition(.opacity.combined(with: .move(edge: .bottom)))
                }
                Spacer()
                Button(String(localized: "Cancel"), action: requestCancel).keyboardShortcut(.cancelAction)
                Button(String(localized: "Save")) {
                    do { try save(PromptTemplate(
                        id: original?.id ?? UUID(),
                        name: name.trimmingCharacters(in: .whitespacesAndNewlines),
                        language: language.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty ? nil : language,
                        content: content,
                        description: description,
                        isBuiltIn: false,
                        createdAt: original?.createdAt ?? .now,
                        updatedAt: .now
                    )) } catch { saveError = error.localizedDescription }
                }
                .buttonStyle(.borderedProminent)
                .keyboardShortcut(.defaultAction)
                .disabled(validationMessage != nil)
            }
        }
        .padding(22)
        .frame(minWidth: 440, idealWidth: 680, maxWidth: 760)
        .frame(minHeight: 480, idealHeight: 620, maxHeight: 760)
        .interactiveDismissDisabled(hasUnsavedChanges)
        .confirmationDialog(
            String(localized: "Discard unsaved changes?"),
            isPresented: $showsDiscardConfirmation
        ) {
            Button(String(localized: "Discard Changes"), role: .destructive, action: cancel)
            Button(String(localized: "Keep Editing"), role: .cancel) { }
        } message: {
            Text(String(localized: "Your prompt template has not been saved."))
        }
        .task {
            guard original == nil else { return }
            await Task.yield()
            nameFocused = true
        }
        .animation(reduceMotion ? nil : .easeOut(duration: 0.18), value: validationMessage)
    }

    private var validationMessage: String? {
        if name.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
            return String(localized: "Enter a template name to continue.")
        }
        if content.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
            return String(localized: "Enter prompt content to continue.")
        }
        return nil
    }

    private var hasUnsavedChanges: Bool {
        name != (original?.name ?? "")
            || language != (original?.language ?? "")
            || description != (original?.description ?? "")
            || content != (original?.content ?? "{{context}}")
    }

    private func requestCancel() {
        if hasUnsavedChanges {
            showsDiscardConfirmation = true
        } else {
            cancel()
        }
    }

    @ViewBuilder
    private var variableButtons: some View {
        ForEach(PromptVariable.allCases, id: \.rawValue) { variable in
            Button(variable.rawValue) { content += variable.rawValue }
                .font(.system(size: 10.5, design: .monospaced))
                .buttonStyle(.bordered)
                .controlSize(.small)
        }
    }
}
