// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Implements MedicationSearchView logic in the iOS UI layer.
// - Interfaces:
//   - MedicationSearchView
// - APIs:
//   - none
//
//
//  MedicationSearchView.swift
//  Nola
//
//  Simple medication lookup UI backed by MedicationService
//

import SwiftUI

struct MedicationSearchView: View {
    @EnvironmentObject var container: ServiceContainer
    @State private var query: String = ""
    @State private var result: MedicationInfo? = nil
    @State private var isSearching = false
    @State private var errorMsg: String?

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            HStack {
                TextField(L10n.medicationSearchPlaceholder, text: $query)
                    .textFieldStyle(.roundedBorder)
                Button(action: search) {
                    Image(systemName: "magnifyingglass")
                }
                .disabled(query.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty || isSearching)
            }
            .padding()

            if isSearching {
                ProgressView(L10n.homeAnalyzing)
                    .padding()
            }

            if let r = result {
                VStack(alignment: .leading, spacing: 8) {
                    Text(r.name).font(.title3).fontWeight(.semibold)
                    if let d = r.commonDosage { Text(L10n.medicationCommonDose.localized("\(d)")) .font(.subheadline) }
                    Text(r.description).font(.body)
                    if let w = r.warnings {
                        Text(L10n.medicationNote.localized(w)).font(.caption).foregroundColor(.red)
                    }
                }
                .frame(maxWidth: .infinity, alignment: .leading)
                .padding()
                .background(Color(.systemGray6))
                .cornerRadius(10)
                .padding(.horizontal)
            } else if let err = errorMsg {
                Text(err).foregroundColor(.secondary).padding()
            } else {
                Text(L10n.medicationSearchPlaceholder)
                    .font(.caption)
                    .foregroundColor(.secondary)
                    .padding()
            }

            Spacer()
        }
        .frame(maxWidth: .infinity, alignment: .leading)
        .navigationTitle(L10n.medicationTitle)
    }

    private func search() {
        errorMsg = nil
        result = nil
        isSearching = true
        Task {
            do {
                if let svc = container.medicationService {
                    let found = try await svc.lookupMedication(named: query)
                    await MainActor.run {
                        self.result = found
                        if found == nil {
                            self.errorMsg = L10n.medicationNotFound.localized(query)
                        }
                        self.isSearching = false
                    }
                } else {
                        await MainActor.run {
                            self.errorMsg = L10n.medicationServiceNotInitialized
                            self.isSearching = false
                        }
                }
            } catch {
                await MainActor.run {
                    self.errorMsg = L10n.medicationSearchFailed.localized(error.localizedDescription)
                    self.isSearching = false
                }
            }
        }
    }
}

#Preview {
    NavigationStack {
        MedicationSearchView().environmentObject(ServiceContainer())
    }
}
