// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Implements ModelPickerSheet logic in the iOS UI layer.
// - Interfaces:
//   - ModelPickerSheet
// - APIs:
//   - none
//
import SwiftUI

struct ModelPickerSheet: View {
    @Environment(\.dismiss) var dismiss
    @Binding var selectedModel: ModelInfo?
    let availableModels: [ModelInfo]
    var onSelected: ((ModelInfo) -> Void)?
    @State private var showParametersFor: ModelInfo? = nil

    private var localModels: [ModelInfo] { availableModels.filter(\.isLocal) }
    private var hostModels: [ModelInfo] { availableModels.filter(\.isWebBackend) }
    private var cloudModels: [ModelInfo] { availableModels.filter { !$0.isLocal && !$0.isWebBackend } }

    var body: some View {
        NavigationStack {
            List {
                if availableModels.isEmpty {
                    Section(header: Text(L10n.modelManagementTitle)) {
                        NolaInlineEmptyState(
                            icon: "cube.transparent",
                            title: L10n.modelManagementPickerAll,
                            message: L10n.modelManagementLocalOfflineNote,
                            tint: DesignTokens.primaryColor
                        )
                        .listRowInsets(EdgeInsets(top: 8, leading: 16, bottom: 8, trailing: 16))
                        .listRowBackground(Color.clear)
                    }
                } else {
                    modelSection(
                        title: NSLocalizedString("chat.model.group.device", value: "On Device", comment: "Device model group"),
                        icon: "iphone",
                        models: localModels
                    )
                    modelSection(
                        title: NSLocalizedString("chat.model.group.host", value: "Host", comment: "Host model group"),
                        icon: "desktopcomputer",
                        models: hostModels
                    )
                    modelSection(
                        title: NSLocalizedString("chat.model.group.cloud", value: "Cloud", comment: "Cloud model group"),
                        icon: "cloud",
                        models: cloudModels
                    )
                }
            }
            .navigationTitle(L10n.modelPickerTitle)
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .navigationBarTrailing) {
                    Button(L10n.close) { dismiss() }
                }
            }
            .sheet(item: $showParametersFor) { model in
                NavigationStack {
                    ModelParametersView(modelID: model.id, displayName: model.name, isLocal: model.isLocal)
                }
            }
        }
    }

    @ViewBuilder
    private func modelSection(title: String, icon: String, models: [ModelInfo]) -> some View {
        if !models.isEmpty {
            Section {
                ForEach(models) { model in
                    modelRow(model)
                }
            } header: {
                Label(title, systemImage: icon)
            }
        }
    }

    private func modelRow(_ model: ModelInfo) -> some View {
        let selectable = isConfigured(model)
        let selected = selectedModel?.id == model.id && selectedModel?.isLocal == model.isLocal
        return HStack(spacing: DesignTokens.spacing_sm) {
            Button {
                selectedModel = model
                onSelected?(model)
                dismiss()
            } label: {
                HStack(spacing: DesignTokens.spacing_md) {
                    Image(systemName: modelIcon(model))
                        .font(.body)
                        .foregroundStyle(.secondary)
                        .frame(width: 24)
                    VStack(alignment: .leading, spacing: DesignTokens.spacing_xs) {
                        Text(model.name)
                            .font(.body.weight(.medium))
                            .foregroundStyle(selectable ? Color.primary : Color.secondary)
                            .fixedSize(horizontal: false, vertical: true)
                        Text(selectable ? executionLabel(model) : NSLocalizedString("chat.model.not_configured", value: "Not configured", comment: "Model not configured label"))
                            .font(.footnote)
                            .foregroundStyle(.secondary)
                    }
                    Spacer(minLength: 0)
                    if selected {
                        Image(systemName: "checkmark")
                            .font(.body.weight(.semibold))
                            .foregroundStyle(DesignTokens.primaryColor)
                    }
                }
                .frame(maxWidth: .infinity, minHeight: 44, alignment: .leading)
                .contentShape(Rectangle())
            }
            .buttonStyle(NolaActionButtonStyle())
            .disabled(!selectable)
            .accessibilityAddTraits(selected ? .isSelected : [])
            if !model.isWebBackend {
                Button { showParametersFor = model } label: {
                    Image(systemName: "slider.horizontal.3")
                        .font(.body)
                        .foregroundStyle(.secondary)
                        .frame(width: 44, height: 44)
                        .contentShape(Rectangle())
                }
                .buttonStyle(NolaActionButtonStyle())
                .accessibilityLabel(String(format: NSLocalizedString("chat.model.parameters.accessibility", value: "%@ parameters", comment: "Model parameters accessibility label"), model.name))
            }
        }
        .padding(DesignTokens.spacing_sm)
        .background(selected ? DesignTokens.accentSubtleColor : .clear,
                    in: RoundedRectangle(cornerRadius: DesignTokens.radius_md))
        .listRowInsets(EdgeInsets(top: 4, leading: 8, bottom: 4, trailing: 8))
    }

    private func isConfigured(_ model: ModelInfo) -> Bool {
        guard model.isWebBackend else { return true }
        return model.baseURL?.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty == false
            && model.resolvedAPIKey?.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty == false
    }

    private func executionLabel(_ model: ModelInfo) -> String {
        if model.isLocal { return NSLocalizedString("chat.execution.device", value: "On Device", comment: "Device execution label") }
        if model.isWebBackend { return NSLocalizedString("chat.execution.host", value: "Host", comment: "Host execution label") }
        return NSLocalizedString("chat.execution.cloud", value: "Cloud", comment: "Cloud execution label")
    }

    private func modelIcon(_ model: ModelInfo) -> String {
        if model.isLocal { return "iphone" }
        if model.isWebBackend { return "desktopcomputer" }
        return "cloud"
    }

    private func modelTint(_ model: ModelInfo) -> Color {
        if model.isLocal { return DesignTokens.successColor }
        if model.isWebBackend { return DesignTokens.primaryColor }
        return DesignTokens.primaryColor
    }
}
