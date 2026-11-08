// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Desktop medication label lookup backed by the shared Apple-client service.

import SwiftUI

private func macMedicationText(_ key: String, _ fallback: String) -> String {
    let keyed = Bundle.main.localizedString(forKey: key, value: nil, table: nil)
    guard keyed == key else { return keyed }
    return Bundle.main.localizedString(forKey: fallback, value: fallback, table: nil)
}

struct MacMedicationSearchView: View {
    @State private var query = ""
    @State private var result: MedicationInfo?
    @State private var isSearching = false
    @State private var searchedQuery = ""
    @State private var errorMessage: String?

    private let service = MedicationService()

    var body: some View {
        VStack(spacing: 0) {
            header
            Divider().opacity(0.55)
            content
        }
        .background {
            LinearGradient(
                colors: [Color(nsColor: .windowBackgroundColor), Color.accentColor.opacity(0.025)],
                startPoint: .topLeading,
                endPoint: .bottomTrailing
            )
        }
        .navigationTitle(macMedicationText("mac.medication.title", "Medication Lookup"))
    }

    private var header: some View {
        VStack(alignment: .leading, spacing: 14) {
            HStack(alignment: .center, spacing: 13) {
                Image(systemName: "pills.fill")
                    .font(.system(size: 19, weight: .medium))
                    .foregroundStyle(Color.accentColor)
                    .frame(width: 42, height: 42)
                    .background(Color.accentColor.opacity(0.10), in: RoundedRectangle(cornerRadius: 12))
                VStack(alignment: .leading, spacing: 3) {
                    Text(macMedicationText("mac.medication.title", "Medication Lookup"))
                        .font(.system(size: 22, weight: .semibold, design: .rounded))
                    Text(macMedicationText("mac.medication.subtitle", "Search public FDA label information by brand name"))
                        .font(.system(size: 11.5))
                        .foregroundStyle(.secondary)
                }
                Spacer()
            }

            HStack(spacing: 9) {
                Image(systemName: "magnifyingglass")
                    .foregroundStyle(.secondary)
                TextField(macMedicationText("mac.medication.placeholder", "Brand name, for example Aspirin"), text: $query)
                    .textFieldStyle(.plain)
                    .font(.system(size: 13.5))
                    .onSubmit { search() }
                if !query.isEmpty {
                    Button {
                        query = ""
                    } label: {
                        Image(systemName: "xmark.circle.fill")
                            .foregroundStyle(.tertiary)
                    }
                    .buttonStyle(.plain)
                    .accessibilityLabel(macMedicationText("mac.medication.clear", "Clear search"))
                }
                Button(macMedicationText("mac.medication.search", "Search")) { search() }
                    .buttonStyle(.borderedProminent)
                    .disabled(trimmedQuery.isEmpty || isSearching)
            }
            .padding(.horizontal, 13)
            .frame(height: 44)
            .background(.regularMaterial, in: RoundedRectangle(cornerRadius: 12, style: .continuous))
            .overlay {
                RoundedRectangle(cornerRadius: 12, style: .continuous)
                    .stroke(Color.primary.opacity(0.08), lineWidth: 0.5)
            }
        }
        .padding(22)
    }

    @ViewBuilder
    private var content: some View {
        if isSearching {
            VStack(spacing: 12) {
                ProgressView().controlSize(.large)
                Text(macMedicationText("mac.medication.searching", "Searching FDA labels…"))
                    .font(.system(size: 12.5, weight: .medium))
                    .foregroundStyle(.secondary)
            }
            .frame(maxWidth: .infinity, maxHeight: .infinity)
        } else if let result {
            ScrollView {
                VStack(alignment: .leading, spacing: 18) {
                    HStack(alignment: .top, spacing: 14) {
                        Image(systemName: "cross.case.fill")
                            .font(.system(size: 21, weight: .medium))
                            .foregroundStyle(.green)
                            .frame(width: 46, height: 46)
                            .background(Color.green.opacity(0.10), in: RoundedRectangle(cornerRadius: 12))
                        VStack(alignment: .leading, spacing: 4) {
                            Text(result.name)
                                .font(.system(size: 24, weight: .semibold, design: .rounded))
                                .textSelection(.enabled)
                            Label(macMedicationText("mac.medication.source", "openFDA label result"), systemImage: "checkmark.seal.fill")
                                .font(.system(size: 10.5, weight: .medium))
                                .foregroundStyle(.secondary)
                        }
                    }

                    Label(macMedicationText("mac.medication.disclaimer", "Public label information is for reference only and does not replace advice from a doctor or pharmacist."), systemImage: "cross.case")
                        .font(.system(size: 11.5))
                        .foregroundStyle(.secondary)
                        .padding(13)
                        .frame(maxWidth: .infinity, alignment: .leading)
                        .background(Color.accentColor.opacity(0.06), in: RoundedRectangle(cornerRadius: 12, style: .continuous))

                    if let dosage = result.commonDosage, !dosage.isEmpty {
                        informationSection(macMedicationText("mac.medication.dosage", "COMMON DOSAGE"), text: dosage, symbol: "scalemass")
                    }
                    informationSection(
                        macMedicationText("mac.medication.label_information", "LABEL INFORMATION"),
                        text: result.description.isEmpty ? macMedicationText("mac.medication.no_description", "No label description was returned.") : result.description,
                        symbol: "doc.text"
                    )
                    if let warnings = result.warnings, !warnings.isEmpty {
                        informationSection(macMedicationText("mac.medication.warnings", "WARNINGS"), text: warnings, symbol: "exclamationmark.triangle.fill", tint: .orange)
                    }
                }
                .frame(maxWidth: 760, alignment: .leading)
                .padding(28)
                .frame(maxWidth: .infinity)
            }
        } else if let errorMessage {
            ContentUnavailableView(
                macMedicationText("mac.medication.no_result", "No Result"),
                systemImage: "pills.circle",
                description: Text(errorMessage)
            )
        } else {
            ContentUnavailableView(
                macMedicationText("mac.medication.empty.title", "Search Medication Labels"),
                systemImage: "pills",
                description: Text(macMedicationText("mac.medication.empty.detail", "Enter a brand name to retrieve public label information."))
            )
        }
    }

    private func informationSection(
        _ title: String,
        text: String,
        symbol: String,
        tint: Color = .accentColor
    ) -> some View {
        VStack(alignment: .leading, spacing: 10) {
            Label(title, systemImage: symbol)
                .font(.system(size: 11, weight: .bold))
                .foregroundStyle(tint)
                .tracking(0.7)
            Text(text)
                .font(.system(size: 13))
                .lineSpacing(3)
                .textSelection(.enabled)
                .frame(maxWidth: .infinity, alignment: .leading)
        }
        .padding(16)
        .background(.thinMaterial, in: RoundedRectangle(cornerRadius: 13, style: .continuous))
        .overlay {
            RoundedRectangle(cornerRadius: 13, style: .continuous)
                .stroke(Color.primary.opacity(0.07), lineWidth: 0.5)
        }
    }

    private var trimmedQuery: String {
        query.trimmingCharacters(in: .whitespacesAndNewlines)
    }

    private func search() {
        let value = trimmedQuery
        guard !value.isEmpty, !isSearching else { return }
        searchedQuery = value
        isSearching = true
        result = nil
        errorMessage = nil
        Task {
            do {
                let found = try await service.lookupMedication(named: value)
                result = found
                if found == nil {
                    errorMessage = String(format: macMedicationText("mac.medication.not_found", "No public FDA label was found for “%@”."), value)
                }
            } catch {
                errorMessage = error.localizedDescription
            }
            isSearching = false
        }
    }
}
