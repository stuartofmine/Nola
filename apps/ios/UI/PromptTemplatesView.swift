// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Implements PromptTemplatesView logic in the iOS UI layer.
// - Interfaces:
//   - PromptTemplatesView
//   - PromptTemplateRow
//   - AddPromptTemplateSheet
//   - EditPromptTemplateSheet
//   - PromptPreviewSheet
// - APIs:
//   - none
//
//
//  PromptTemplatesView.swift
//  Nola
//
//  Prompt template management view.
//

import SwiftUI

struct PromptTemplatesView: View {
    @EnvironmentObject var container: ServiceContainer
    var body: some View {
        if let service = container.promptTemplates {
            PromptTemplateLibraryContent(service: service)
        } else {
            Text(String(localized: "prompt.templates.error.unavailable"))
        }
    }
}

/// The canonical service owns the catalog. Observe it directly so successful
/// create/edit/delete operations update the mounted directory immediately.
private struct PromptTemplateLibraryContent: View {
    @EnvironmentObject var container: ServiceContainer
    @ObservedObject var service: PromptTemplateService
    @State private var showingAddSheet = false
    @State private var editingTemplate: PromptTemplate?
    @State private var previewTemplate: PromptTemplate?
    @State private var pendingDeletion: PromptTemplate?
    @State private var searchText = ""
    @State private var templateError: String?

    var body: some View {
        List {
            let resolution = Result { try service.getCurrentTemplate() }
            let current = (try? resolution.get()).flatMap { matchesSearch($0) ? $0 : nil }
            if case .failure(let error) = resolution { Text(error.localizedDescription).foregroundStyle(.red) }
            if let templateError { Text(templateError).foregroundStyle(.red) }
            let currentLang = Locale.current.language.languageCode?.identifier ?? "en"
            let builtIn = service.templates.filter { template in
                guard template.isBuiltIn && template.id != current?.id && service.canReadTemplate(template) else { return false }
                let language = template.language ?? ""
                return (language.isEmpty || language.hasPrefix(currentLang)) && matchesSearch(template)
            }
            let custom = service.templates.filter {
                !$0.isBuiltIn && $0.id != current?.id && service.canReadTemplate($0) && matchesSearch($0)
            }
            let visibleResultCount = (current == nil ? 0 : 1) + builtIn.count + custom.count

            // Current template
            if let current {
                Section(header: Text(NSLocalizedString("prompt.templates.section.current", value: "Current", comment: "Current prompt template section"))) {
                    PromptTemplateRow(
                        template: current,
                        isSelected: true,
                        onSelect: {},
                        onEdit: {
                            // If current is built-in, duplicate it and edit the copy; otherwise edit in-place
                            if current.isBuiltIn {
                                duplicateAndEdit(current, using: service)
                            } else {
                                openEditor(current, service: service)
                            }
                        },
                        onPreview: {
                            openPreview(current, service: service)
                        },
                        onDuplicate: {
                            duplicateAndEdit(current, using: service)
                        },
                        onDelete: nil
                    )
                }
            }

            // Built-in templates (show only templates matching current language to avoid duplicates)
            if !builtIn.isEmpty {
                Section(header: Text(String(
                    format: NSLocalizedString("prompt.templates.section.builtin_count", value: "Built-in · %@", comment: "Built-in prompt template count"),
                    "\(builtIn.count)"
                ))) {
                    ForEach(builtIn) { template in
                        PromptTemplateRow(
                            template: template,
                            isSelected: false,
                            onSelect: {
                                performTemplateAction { try service.selectTemplate(id: template.id) }
                            },
                            onEdit: {
                                // Duplicate built-in into a custom editable template, then open editor
                                duplicateAndEdit(template, using: service)
                            },
                            onPreview: {
                                openPreview(template, service: service)
                            },
                            onDuplicate: {
                                duplicateAndEdit(template, using: service)
                            },
                            onDelete: nil
                        )
                    }
                }
            }

            // Custom templates
            if !custom.isEmpty {
                Section {
                    ForEach(custom) { template in
                        PromptTemplateRow(
                            template: template,
                            isSelected: false,
                            onSelect: {
                                performTemplateAction { try service.selectTemplate(id: template.id) }
                            },
                            onEdit: {
                                openEditor(template, service: service)
                            },
                            onPreview: {
                                openPreview(template, service: service)
                            },
                            onDuplicate: {
                                duplicateAndEdit(template, using: service)
                            },
                            onDelete: {
                                pendingDeletion = template
                            }
                        )
                    }
                } header: {
                    Text(NSLocalizedString("prompt.templates.section.custom", value: "Custom", comment: "Custom prompt templates section"))
                } footer: {
                    Text(NSLocalizedString("prompt.templates.section.custom_detail", value: "Your templates stay editable and can be duplicated or deleted.", comment: "Custom prompt templates section detail"))
                }
            }

            if visibleResultCount == 0 {
                Section {
                    if normalizedSearch.isEmpty {
                        NolaInlineEmptyState(
                            icon: "text.book.closed",
                            title: NSLocalizedString("prompt.templates.empty.title", value: "No templates yet", comment: "Empty prompt templates title"),
                            message: NSLocalizedString("prompt.templates.empty.message", value: "Create a reusable instruction set for the work you do often.", comment: "Empty prompt templates message"),
                            actionTitle: NSLocalizedString("prompt.templates.add", value: "Add template", comment: "Add prompt template"),
                            action: { showingAddSheet = true }
                        )
                    } else {
                        NolaInlineEmptyState(
                            icon: "magnifyingglass",
                            title: NSLocalizedString("prompt.templates.search.empty.title", value: "No matching templates", comment: "Prompt template search empty title"),
                            message: NSLocalizedString("prompt.templates.search.empty.message", value: "Try another name, description, language, or instruction.", comment: "Prompt template search empty message"),
                            tint: .blue,
                            actionTitle: NSLocalizedString("prompt.templates.search.clear", value: "Clear search", comment: "Clear prompt template search"),
                            action: { searchText = "" }
                        )
                    }
                }
            }

            // Help section
            if normalizedSearch.isEmpty {
                Section {
                    VStack(alignment: .leading, spacing: 8) {
                        Text(L10n.promptTemplatesVariables)
                            .font(.caption)
                            .foregroundColor(.secondary)

                        ForEach(PromptVariable.allCases, id: \.self) { variable in
                            HStack {
                                Text(variable.rawValue)
                                    .font(.caption.monospaced())
                                    .fontDesign(.monospaced)
                                    .foregroundColor(.blue)
                                Text("-")
                                    .font(.caption)
                                    .foregroundColor(.secondary)
                                Text(variable.description)
                                    .font(.caption)
                                    .foregroundColor(.secondary)
                            }
                        }
                    }
                }
            }
        }
        .navigationTitle(L10n.promptTemplatesTitle)
        .navigationBarTitleDisplayMode(.inline)
        .searchable(
            text: $searchText,
            prompt: Text(NSLocalizedString("prompt.templates.search", value: "Search templates", comment: "Search prompt templates"))
        )
        .toolbar {
            ToolbarItem(placement: .primaryAction) {
                Button {
                    showingAddSheet = true
                } label: {
                    Image(systemName: "plus")
                }
                .accessibilityLabel(NSLocalizedString("prompt.templates.add", value: "Add template", comment: "Add prompt template"))
                .accessibilityIdentifier("prompt.templates.add")
            }
        }
        .sheet(isPresented: $showingAddSheet) {
            AddPromptTemplateSheet()
                .environmentObject(container)
        }
        .sheet(item: $editingTemplate) { template in
            EditPromptTemplateSheet(template: template)
                .environmentObject(container)
        }
        .sheet(item: $previewTemplate) { template in
            PromptPreviewSheet(template: template)
                .environmentObject(container)
        }
        .confirmationDialog(
            NSLocalizedString("prompt.templates.delete.title", value: "Delete Prompt Template?", comment: "Delete prompt template confirmation"),
            isPresented: Binding(
                get: { pendingDeletion != nil },
                set: { if !$0 { pendingDeletion = nil } }
            ),
            titleVisibility: .visible
        ) {
            Button(L10n.commonDelete, role: .destructive) {
                guard let template = pendingDeletion else { return }
                performTemplateAction { try service.deleteTemplate(id: template.id) }
                pendingDeletion = nil
            }
            Button(L10n.cancel, role: .cancel) { pendingDeletion = nil }
        } message: {
            Text(NSLocalizedString("prompt.templates.delete.detail", value: "This action cannot be undone.", comment: "Delete prompt template warning"))
        }
    }

    private var normalizedSearch: String {
        searchText.trimmingCharacters(in: .whitespacesAndNewlines)
    }

    private func matchesSearch(_ template: PromptTemplate) -> Bool {
        let query = normalizedSearch
        guard !query.isEmpty else { return true }
        return template.name.localizedCaseInsensitiveContains(query)
            || template.description.localizedCaseInsensitiveContains(query)
            || template.content.localizedCaseInsensitiveContains(query)
            || (template.language?.localizedCaseInsensitiveContains(query) ?? false)
    }

    private func performTemplateAction(_ action: () throws -> Void) {
        do { try action(); templateError = nil } catch { templateError = error.localizedDescription }
    }

    private func openPreview(_ template: PromptTemplate, service: PromptTemplateService) {
        performTemplateAction { previewTemplate = try service.requireReadableTemplate(id: template.id) }
    }

    private func openEditor(_ template: PromptTemplate, service: PromptTemplateService) {
        performTemplateAction {
            let fresh = try service.requireReadableTemplate(id: template.id)
            guard service.canEditTemplate(fresh) else { throw PromptTemplateReadError.denied(template.id) }
            editingTemplate = fresh
        }
    }

    private func duplicateAndEdit(_ template: PromptTemplate, using service: PromptTemplateService) {
        performTemplateAction { editingTemplate = try service.duplicateTemplate(id: template.id) }
    }
}

struct PromptTemplateRow: View {
    let template: PromptTemplate
    let isSelected: Bool
    let onSelect: () -> Void
    let onEdit: (() -> Void)?
    let onPreview: () -> Void
    let onDuplicate: () -> Void
    let onDelete: (() -> Void)?

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            HStack {
                Button(action: onSelect) {
                VStack(alignment: .leading) {
                    HStack(spacing: 6) {
                        Text(template.name)
                            .font(.headline)
                            .lineLimit(2)

                        if template.isBuiltIn {
                            Text(L10n.promptTemplatesBuiltIn)
                                .font(.caption.weight(.semibold))
                                .padding(.horizontal, 7)
                                .padding(.vertical, 3)
                                .background(DesignTokens.primaryColor.opacity(0.12))
                                .foregroundColor(DesignTokens.primaryColor)
                                .clipShape(Capsule())
                        }

                        if isSelected {
                            Image(systemName: "checkmark.circle.fill")
                                .foregroundColor(.green)
                                .accessibilityLabel(NSLocalizedString("prompt.templates.selected", value: "Selected", comment: "Selected prompt template"))
                        }
                    }

                    Text(template.description)
                        .font(.subheadline)
                        .foregroundColor(.secondary)
                        .lineLimit(2)
                }

                .frame(maxWidth: .infinity, minHeight: 44, alignment: .leading)
                }.buttonStyle(.plain)
                    .accessibilityAddTraits(isSelected ? [.isSelected] : [])

                Button {
                    onPreview()
                } label: {
                    Label(L10n.promptTemplatesView, systemImage: "eye")
                        .font(.caption)
                }
                .buttonStyle(.bordered)
                .controlSize(.regular)
            }
        }
        .padding(.vertical, 3)
        .contentShape(Rectangle())
        .contextMenu {
            Button {
                onPreview()
            } label: {
                Label(L10n.preview, systemImage: "eye")
            }

            if let edit = onEdit {
                Button {
                    edit()
                } label: {
                    Label(L10n.edit, systemImage: "pencil")
                }
            }

            Button {
                onDuplicate()
            } label: {
                Label(L10n.commonCopy, systemImage: "doc.on.doc")
            }

            if let delete = onDelete {
                Divider()

                Button(role: .destructive) {
                    delete()
                } label: {
                    Label(L10n.commonDelete, systemImage: "trash")
                }
            }
        }
    }
}

struct AddPromptTemplateSheet: View {
    @EnvironmentObject var container: ServiceContainer
    @Environment(\.dismiss) var dismiss

    @State private var name = ""
    @State private var description = ""
    @State private var content = ""
    @State private var warnings: [String] = []
    @State private var showingDiscardConfirmation = false
    @State private var saveError: String?

    var body: some View {
        NavigationStack {
            Form {
                if let saveError { Text(saveError).foregroundStyle(.red) }
                Section(header: Text(L10n.promptTemplatesBasicInfo)) {
                    TextField(L10n.commonName, text: $name)
                        .accessibilityIdentifier("prompt.templates.editor.name")
                    TextField(L10n.commonDescription, text: $description)
                        .accessibilityIdentifier("prompt.templates.editor.description")
                }

                Section(header: Text(L10n.promptTemplatesContentTitle)) {
                    // Variable insert bar
                    ScrollView(.horizontal, showsIndicators: false) {
                        HStack(spacing: 8) {
                            ForEach(PromptVariable.allCases, id: \.self) { variable in
                                Button(variable.rawValue) {
                                    content += " \(variable.rawValue)"
                                }
                                .font(.caption.monospaced())
                                .padding(.horizontal, 10)
                                .frame(minHeight: 40)
                                .background(Color(.systemGray6))
                                .cornerRadius(6)
                            }
                        }
                        .padding(.vertical, 4)
                    }

                    TextEditor(text: $content)
                        .accessibilityIdentifier("prompt.templates.editor.content")
                        .frame(minHeight: 260)
                        .font(.system(.body, design: .monospaced))
                        .padding(6)
                        .background(Color(.systemGroupedBackground))
                        .cornerRadius(8)

                    HStack(spacing: 12) {
                        if let service = container.promptTemplates {
                            Text(String(format: L10n.promptTemplatesEstimateTokens.localized(), service.estimateTokenCount(content)))
                                .font(.caption)
                                .foregroundColor(.secondary)
                        }

                        Spacer()

                        Text("\(content.count) " + NSLocalizedString("chars", comment: "characters"))
                            .font(.caption)
                            .foregroundColor(.secondary)
                    }
                }

                if !warnings.isEmpty {
                    Section(header: Text(L10n.promptTemplatesWarning)) {
                        ForEach(warnings, id: \.self) { warning in
                            Label(warning, systemImage: "exclamationmark.triangle")
                                .foregroundColor(.orange)
                                .font(.caption)
                        }
                    }
                }
            }
            .navigationTitle(L10n.promptTemplatesNewTitle)
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .cancellationAction) {
                    Button(L10n.cancel) {
                        requestCancel()
                    }
                    .accessibilityIdentifier("prompt.templates.editor.cancel")
                }

                ToolbarItem(placement: .confirmationAction) {
                    Button(L10n.commonSave) {
                        saveTemplate()
                    }
                    .disabled(!canSave)
                    .accessibilityIdentifier("prompt.templates.editor.save")
                }
            }
            .onChange(of: content) { _, newValue in
                validateContent(newValue)
            }
            .interactiveDismissDisabled(hasUnsavedChanges)
            .confirmationDialog(
                NSLocalizedString("prompt.templates.discard.title", value: "Discard unsaved changes?", comment: "Discard prompt template changes title"),
                isPresented: $showingDiscardConfirmation,
                titleVisibility: .visible
            ) {
                Button(NSLocalizedString("prompt.templates.discard.action", value: "Discard Changes", comment: "Discard prompt template changes action"), role: .destructive) { dismiss() }
                Button(L10n.cancel, role: .cancel) { }
            }
        }
    }

    private var canSave: Bool {
        !name.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
            && !content.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
    }

    private var hasUnsavedChanges: Bool {
        !name.isEmpty || !description.isEmpty || !content.isEmpty
    }

    private func requestCancel() {
        if hasUnsavedChanges {
            showingDiscardConfirmation = true
        } else {
            dismiss()
        }
    }

    private func validateContent(_ text: String) {
        if let service = container.promptTemplates {
            let template = PromptTemplate(name: name, content: text, description: description)
            warnings = service.validateTemplate(template)
        }
    }

    private func saveTemplate() {
        guard let service = container.promptTemplates else { return }

        let template = PromptTemplate(
            name: name.trimmingCharacters(in: .whitespacesAndNewlines),
            content: content,
            description: description.trimmingCharacters(in: .whitespacesAndNewlines)
        )

        do { try service.addTemplate(template); dismiss() }
        catch { saveError = error.localizedDescription }
    }
}

struct EditPromptTemplateSheet: View {
    @EnvironmentObject var container: ServiceContainer
    @Environment(\.dismiss) var dismiss

    let template: PromptTemplate
    @State private var name = ""
    @State private var description = ""
    @State private var content = ""
    @State private var warnings: [String] = []
    @State private var showingDiscardConfirmation = false
    @State private var saveError: String?

    var body: some View {
        NavigationStack {
            Form {
                if let saveError { Text(saveError).foregroundStyle(.red) }
                Section(header: Text(L10n.promptTemplatesBasicInfo)) {
                    TextField(L10n.commonName, text: $name)
                    TextField(L10n.commonDescription, text: $description)
                }

                Section(header: Text(L10n.promptTemplatesContentTitle)) {
                    // Variable insert bar
                    ScrollView(.horizontal, showsIndicators: false) {
                        HStack(spacing: 8) {
                            ForEach(PromptVariable.allCases, id: \.self) { variable in
                                Button(variable.rawValue) {
                                    content += " \(variable.rawValue)"
                                }
                                .font(.caption.monospaced())
                                .padding(.horizontal, 10)
                                .frame(minHeight: 40)
                                .background(Color(.systemGray6))
                                .cornerRadius(6)
                            }
                        }
                        .padding(.vertical, 4)
                    }

                    TextEditor(text: $content)
                        .frame(minHeight: 260)
                        .font(.system(.body, design: .monospaced))
                        .padding(6)
                        .background(Color(.systemGroupedBackground))
                        .cornerRadius(8)

                    HStack(spacing: 12) {
                        if let service = container.promptTemplates {
                            Text(String(format: L10n.promptTemplatesEstimateTokens.localized(), service.estimateTokenCount(content)))
                                .font(.caption)
                                .foregroundColor(.secondary)
                        }

                        Spacer()

                        Text("\(content.count) " + NSLocalizedString("chars", comment: "characters"))
                            .font(.caption)
                            .foregroundColor(.secondary)
                    }
                }

                if !warnings.isEmpty {
                    Section(header: Text(L10n.promptTemplatesWarning)) {
                        ForEach(warnings, id: \.self) { warning in
                            Label(warning, systemImage: "exclamationmark.triangle")
                                .foregroundColor(.orange)
                                .font(.caption)
                        }
                    }
                }
            }
            .navigationTitle(L10n.promptTemplatesEditTitle)
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .cancellationAction) {
                    Button(L10n.cancel) {
                        requestCancel()
                    }
                }

                ToolbarItem(placement: .confirmationAction) {
                    Button(L10n.commonSave) {
                        saveChanges()
                    }
                    .disabled(!canSave)
                }
            }
            .onAppear {
                name = template.name
                description = template.description
                content = template.content
            }
            .onChange(of: content) { _, newValue in
                validateContent(newValue)
            }
            .interactiveDismissDisabled(hasUnsavedChanges)
            .confirmationDialog(
                NSLocalizedString("prompt.templates.discard.title", value: "Discard unsaved changes?", comment: "Discard prompt template changes title"),
                isPresented: $showingDiscardConfirmation,
                titleVisibility: .visible
            ) {
                Button(NSLocalizedString("prompt.templates.discard.action", value: "Discard Changes", comment: "Discard prompt template changes action"), role: .destructive) { dismiss() }
                Button(L10n.cancel, role: .cancel) { }
            }
        }
    }

    private var canSave: Bool {
        !name.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
            && !content.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
    }

    private var hasUnsavedChanges: Bool {
        name != template.name || description != template.description || content != template.content
    }

    private func requestCancel() {
        if hasUnsavedChanges {
            showingDiscardConfirmation = true
        } else {
            dismiss()
        }
    }

    private func validateContent(_ text: String) {
        if let service = container.promptTemplates {
            let updated = PromptTemplate(id: template.id, name: name, content: text, description: description)
            warnings = service.validateTemplate(updated)
        }
    }

    private func saveChanges() {
        guard let service = container.promptTemplates else { return }

        var updated = template
        updated.name = name.trimmingCharacters(in: .whitespacesAndNewlines)
        updated.description = description.trimmingCharacters(in: .whitespacesAndNewlines)
        updated.content = content

        do { try service.updateTemplate(updated); dismiss() }
        catch { saveError = error.localizedDescription }
    }
}

struct PromptPreviewSheet: View {
    let template: PromptTemplate
    @EnvironmentObject var container: ServiceContainer
    @Environment(\.dismiss) var dismiss
    @State private var editingTemplateLocal: PromptTemplate?
    @State private var copyError: String?

    var body: some View {
        NavigationStack {
            ScrollView {
                VStack(alignment: .leading, spacing: 16) {
                    if let copyError { Text(copyError).foregroundStyle(.red) }
                    Text(template.content)
                        .font(.system(.body, design: .monospaced))
                        .textSelection(.enabled)
                        .padding(16)
                        .frame(maxWidth: .infinity, alignment: .leading)
                        .background(.thinMaterial, in: RoundedRectangle(cornerRadius: 14, style: .continuous))
                }
                .padding()
                .frame(maxWidth: 760)
                .frame(maxWidth: .infinity)
            }
            .navigationTitle(L10n.preview)
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .navigationBarTrailing) {
                    Button(L10n.promptTemplatesSaveAs) {
                        if let service = container.promptTemplates {
                            do { editingTemplateLocal = try service.duplicateTemplate(id: template.id) }
                            catch { copyError = error.localizedDescription }
                        }
                    }
                }

                ToolbarItem(placement: .confirmationAction) {
                    Button(L10n.done) {
                        dismiss()
                    }
                }
            }
            .sheet(item: $editingTemplateLocal) { t in
                EditPromptTemplateSheet(template: t)
                    .environmentObject(container)
            }
        }
    }
}

#Preview {
    NavigationStack {
        PromptTemplatesView()
            .environmentObject(ServiceContainer())
    }
}
