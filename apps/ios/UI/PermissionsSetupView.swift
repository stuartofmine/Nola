// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Implements PermissionsSetupView logic in the iOS UI layer.
// - Interfaces:
//   - PermissionsSetupView
//   - StepView
//   - PermissionFeature
//   - CompletionView
//   - PermissionSetupInfoCard
//   - PermissionSetupStep
// - APIs:
//   - none
//
//
//  PermissionsSetupView.swift
//  Nola
//
// Comment in English.
//

import SwiftUI

struct PermissionsSetupView: View {
    @EnvironmentObject var container: ServiceContainer
    @Environment(\.dismiss) var dismiss
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    
    @State private var currentStep = 0
    @State private var isRequestingPermissions = false
    @State private var permissionResults: [DataSourceType: Bool] = [:]
    @State private var requestTask: Task<Void, Never>?
    @State private var requestOwner: UUID?
    @State private var showError = false
    @State private var errorMessage: String?
    
    private let steps: [PermissionSetupStep] = [
        PermissionSetupStep(
            type: .contacts,
            title: L10n.settingsDataSourceContacts,
            description: L10n.permissionsSetupContactsDesc,
            icon: "person.2.fill",
            color: .blue
        ),
        PermissionSetupStep(
            type: .calendars,
            title: L10n.settingsDataSourceCalendars,
            description: L10n.permissionsSetupCalendarsDesc,
            icon: "calendar",
            color: .orange
        ),
        PermissionSetupStep(
            type: .reminders,
            title: L10n.settingsDataSourceReminders,
            description: L10n.permissionsSetupRemindersDesc,
            icon: "checklist",
            color: .green
        ),
        PermissionSetupStep(
            type: .health,
            title: L10n.homeSectionHealth,
            description: L10n.permissionsSetupHealthDesc,
            icon: "heart.fill",
            color: .red
        )
    ]
    
    var body: some View {
        VStack(spacing: 0) {
            HStack {
                Text(currentStep < steps.count ? steps[currentStep].title : L10n.permissionsSetupCompletedTitle)
                    .font(.subheadline.weight(.semibold))
                    .lineLimit(1)
                Spacer()
                Text("\(currentStep + 1)/\(steps.count + 1)")
                    .font(.caption.monospacedDigit())
                    .foregroundStyle(.secondary)
                    .contentTransition(.numericText())
            }
            .padding(.horizontal, 20)
            .padding(.top, 14)
            .padding(.bottom, 8)

            ProgressView(value: Double(currentStep + 1), total: Double(steps.count + 1))
                .progressViewStyle(.linear)
                .tint(.accentColor)
                .padding(.horizontal, 20)
                .animation(reduceMotion ? nil : DesignTokens.responsiveAnimation, value: currentStep)
            
            // Comment in English.
            TabView(selection: Binding(
                get: { currentStep },
                set: { if !isRequestingPermissions { currentStep = $0 } }
            )) {
                ForEach(0..<steps.count, id: \.self) { index in
                    StepView(
                        step: steps[index],
                        isGranted: permissionResults[steps[index].type] ?? false
                    )
                    .tag(index)
                }
                
                // Comment in English.
                CompletionView(
                    grantedCount: permissionResults.values.filter { $0 }.count,
                    totalCount: steps.count
                )
                .tag(steps.count)
            }
            .tabViewStyle(.page(indexDisplayMode: .never))
            
            VStack(spacing: 16) {
                if currentStep < steps.count {
                    Button {
                        requestTask = Task { @MainActor in
                            await requestCurrentPermission()
                        }
                    } label: {
                        HStack {
                            if isRequestingPermissions {
                                ProgressView()
                                    .progressViewStyle(.circular)
                                    .tint(.white)
                            } else {
                                Image(systemName: "hand.raised.fill")
                                Text(permissionResults[steps[currentStep].type] == true ? L10n.permissionGranted : L10n.permissionGrant)
                            }
                        }
                        .frame(maxWidth: .infinity)
                        .frame(minHeight: 46)
                    }
                    .buttonStyle(.borderedProminent)
                    .controlSize(.large)
                    .tint(permissionResults[steps[currentStep].type] == true ? .green : .accentColor)
                    .disabled(isRequestingPermissions || permissionResults[steps[currentStep].type] == true)
                    
                    // Comment in English.
                    Button {
                        withAnimation {
                            currentStep += 1
                        }
                    } label: {
                        Text(currentStep == steps.count - 1 ? L10n.permissionsSetupFinishSetup : L10n.skip)
                            .frame(maxWidth: .infinity)
                            .frame(minHeight: 40)
                    }
                    .buttonStyle(.plain)
                    .foregroundStyle(.secondary)
                    .disabled(isRequestingPermissions)
                    .accessibilityIdentifier("permissions.setup.skip")
                } else {
                    Button {
                        dismiss()
                    } label: {
                        HStack {
                            Image(systemName: "checkmark.circle.fill")
                            Text(L10n.permissionsStartUsingNola)
                        }
                        .frame(maxWidth: .infinity)
                        .frame(minHeight: 46)
                    }
                    .buttonStyle(.borderedProminent)
                    .controlSize(.large)
                }
            }
            .padding(.horizontal, 20)
            .padding(.top, 14)
            .padding(.bottom, 16)
            .background(.ultraThinMaterial)
        }
        .interactiveDismissDisabled(isRequestingPermissions)
        .onDisappear {
            requestOwner = nil
            requestTask?.cancel()
            requestTask = nil
            isRequestingPermissions = false
        }
        .alert(L10n.permissionsRequestFailed, isPresented: $showError) {
            Button(L10n.ok, role: .cancel) {}
            Button(L10n.permissionsGoToSettings) {
                openSettings()
            }
        } message: {
            Text(errorMessage ?? L10n.permissionsUnknownError)
        }
    }
    
    @MainActor
    private func requestCurrentPermission() async {
        guard !isRequestingPermissions, currentStep < steps.count else { return }
        let sourceIndex = currentStep
        let owner = UUID()
        requestOwner = owner
        isRequestingPermissions = true
        let step = steps[sourceIndex]
        defer {
            if requestOwner == owner {
                requestOwner = nil
                isRequestingPermissions = false
            }
        }
        
        do {
            let granted: Bool
            
            switch step.type {
            case .contacts:
                granted = try await DataSourcePermissionManager.shared.requestContactsPermission()
                
            case .calendars:
                granted = try await DataSourcePermissionManager.shared.requestCalendarPermission()
                
            case .reminders:
                granted = try await DataSourcePermissionManager.shared.requestRemindersPermission()
                
            case .health:
                granted = try await DataSourcePermissionManager.shared.requestHealthPermission()
                if granted, !Task.isCancelled, requestOwner == owner {
                    container.healthDataSourceEnabled = true
                }
                
            case .url:
                granted = true
            }
            
            guard PermissionSetupRequestCompletion.accepts(
                sourceIndex: sourceIndex, currentIndex: currentStep,
                owner: owner, activeOwner: requestOwner, cancelled: Task.isCancelled
            ) else { return }
            permissionResults[step.type] = granted
            if granted {
                withAnimation(reduceMotion ? nil : DesignTokens.responsiveAnimation) {
                    currentStep = sourceIndex + 1
                }
            } else {
                errorMessage = L10n.format("permissions.setup.error_permission_format", step.title)
                showError = true
            }
        } catch {
            guard PermissionSetupRequestCompletion.accepts(
                sourceIndex: sourceIndex, currentIndex: currentStep,
                owner: owner, activeOwner: requestOwner, cancelled: Task.isCancelled
            ) else { return }
            errorMessage = error.localizedDescription
            showError = true
        }
    }
    
    private func openSettings() {
        if let url = URL(string: UIApplication.openSettingsURLString) {
            UIApplication.shared.open(url)
        }
    }
}

/// An OS permission callback can outlive the page that initiated it.
enum PermissionSetupRequestCompletion {
    static func accepts(sourceIndex: Int, currentIndex: Int, owner: UUID, activeOwner: UUID?, cancelled: Bool) -> Bool {
        !cancelled && activeOwner == owner && currentIndex == sourceIndex
    }
}

// MARK: - Supporting Views

struct StepView: View {
    let step: PermissionSetupStep
    let isGranted: Bool
    
    var body: some View {
        ScrollView {
            VStack(spacing: 24) {
                ZStack {
                    Circle()
                        .fill(
                            LinearGradient(
                                gradient: Gradient(colors: [step.color.opacity(0.24), step.color.opacity(0.07)]),
                                startPoint: .topLeading,
                                endPoint: .bottomTrailing
                            )
                        )
                        .frame(width: 88, height: 88)

                    Image(systemName: step.icon)
                        .font(.system(size: 36, weight: .semibold))
                        .foregroundColor(step.color)
                }

                VStack(spacing: 9) {
                    Text(step.title)
                        .font(.title2.weight(.bold))
                    Text(step.description)
                        .font(.body)
                        .foregroundStyle(.secondary)
                        .multilineTextAlignment(.center)
                        .fixedSize(horizontal: false, vertical: true)
                }

                if isGranted {
                    Label(L10n.permissionGranted, systemImage: "checkmark.circle.fill")
                        .font(.subheadline.weight(.semibold))
                        .foregroundStyle(.green)
                        .padding(.horizontal, 14)
                        .padding(.vertical, 7)
                        .background(Color.green.opacity(0.10), in: Capsule())
                }

                VStack(alignment: .leading, spacing: 14) {
                    PermissionFeature(icon: "shield.fill", title: L10n.permissionsSetupFeaturePrivacyTitle, description: L10n.permissionsSetupFeaturePrivacyDesc)
                    Divider()
                    PermissionFeature(icon: "eye.slash.fill", title: L10n.permissionsSetupFeatureNoUploadTitle, description: L10n.permissionsSetupFeatureNoUploadDesc)
                    Divider()
                    PermissionFeature(icon: "hand.raised.fill", title: L10n.permissionsSetupFeatureControlTitle, description: L10n.permissionsSetupFeatureControlDesc)
                }
                .padding(16)
                .background(Color(.secondarySystemGroupedBackground), in: RoundedRectangle(cornerRadius: 16, style: .continuous))
            }
            .frame(maxWidth: 560)
            .padding(.horizontal, 22)
            .padding(.vertical, 24)
            .frame(maxWidth: .infinity)
        }
    }
}

struct PermissionFeature: View {
    let icon: String
    let title: String
    let description: String
    
    var body: some View {
        HStack(alignment: .top, spacing: 12) {
            Image(systemName: icon)
                .font(.system(size: 13, weight: .semibold))
                .foregroundStyle(Color.accentColor)
                .frame(width: 28, height: 28)
                .background(Color.accentColor.opacity(0.09), in: RoundedRectangle(cornerRadius: 8, style: .continuous))
            
            VStack(alignment: .leading, spacing: 4) {
                Text(title)
                    .font(.subheadline.weight(.semibold))
                
                Text(description)
                    .font(.caption)
                    .foregroundStyle(.secondary)
                    .fixedSize(horizontal: false, vertical: true)
            }
        }
    }
}

struct CompletionView: View {
    let grantedCount: Int
    let totalCount: Int
    
    var body: some View {
        ScrollView {
            VStack(spacing: 24) {
                ZStack {
                    Circle()
                        .fill(LinearGradient(colors: [Color.green.opacity(0.22), Color.blue.opacity(0.06)], startPoint: .topLeading, endPoint: .bottomTrailing))
                        .frame(width: 88, height: 88)
                    Image(systemName: "checkmark")
                        .font(.system(size: 36, weight: .bold))
                        .foregroundStyle(.green)
                }

                VStack(spacing: 9) {
                    Text(L10n.permissionsSetupCompletedTitle)
                        .font(.title2.weight(.bold))
                    Text(L10n.format("permissions.setup.completed.granted_count_format", grantedCount, totalCount))
                        .font(.body)
                        .foregroundStyle(.secondary)
                }

                if grantedCount == totalCount {
                    PermissionSetupInfoCard(icon: "star.fill", color: .yellow, title: L10n.permissionsSetupInfoAllGrantedTitle, description: L10n.permissionsSetupInfoAllGrantedDesc)
                } else if grantedCount > 0 {
                    PermissionSetupInfoCard(icon: "checkmark.circle.fill", color: .green, title: L10n.permissionsSetupInfoSomeGrantedTitle, description: L10n.permissionsSetupInfoSomeGrantedDesc)
                } else {
                    PermissionSetupInfoCard(icon: "info.circle.fill", color: .blue, title: L10n.permissionsSetupInfoNoneGrantedTitle, description: L10n.permissionsSetupInfoNoneGrantedDesc)
                }
            }
            .frame(maxWidth: 560)
            .padding(.horizontal, 22)
            .padding(.vertical, 24)
            .frame(maxWidth: .infinity)
        }
    }
}

struct PermissionSetupInfoCard: View {
    let icon: String
    let color: Color
    let title: String
    let description: String
    
    var body: some View {
        HStack(spacing: 12) {
            Image(systemName: icon)
                .font(.title2)
                .foregroundColor(color)
            
            VStack(alignment: .leading, spacing: 4) {
                Text(title)
                    .font(.headline)
                
                Text(description)
                    .font(.caption)
                    .foregroundColor(.secondary)
                    .fixedSize(horizontal: false, vertical: true)
            }
        }
        .padding()
        .background(
            RoundedRectangle(cornerRadius: 12)
                .fill(Color(.secondarySystemBackground))
        )
    }
}

// MARK: - Supporting Types

struct PermissionSetupStep {
    let type: DataSourceType
    let title: String
    let description: String
    let icon: String
    let color: Color
}

#Preview {
    PermissionsSetupView()
        .environmentObject(ServiceContainer())
}
