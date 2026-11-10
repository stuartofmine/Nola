// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Defines the typed macOS Application boundary for Data Source lifecycle and usage.

import Foundation

/// Canonical Data Source use cases consumed by the macOS registry.
/// Revisions, validation tokens, timestamps, and query limits stay explicit at
/// this boundary so callers cannot silently weaken Application CAS semantics.
nonisolated protocol MacDataSourceApplicationClient: Sendable {
    func createDataSourceDraft(
        _ input: MacCanonicalDataSourceDraft
    ) throws -> MacCanonicalDataSource
    func updateDataSourceConfiguration(
        id: String,
        expectedRevision: Int64,
        input: MacCanonicalDataSourceConfigurationUpdate
    ) throws -> MacCanonicalDataSource
    func requestDataSourceValidation(
        id: String,
        expectedRevision: Int64,
        requestedAtMs: Int64
    ) throws -> MacCanonicalDataSource
    func recordDataSourceValidationSuccess(
        id: String,
        validationToken: String,
        contract: MacDataSourceContract,
        recordedAtMs: Int64
    ) throws -> MacCanonicalDataSource
    func recordDataSourceValidationFailure(
        id: String,
        validationToken: String,
        code: String,
        diagnostic: String,
        recordedAtMs: Int64
    ) throws -> MacCanonicalDataSource
    func setDataSourceDefaultEnabled(
        id: String,
        expectedRevision: Int64,
        enabled: Bool,
        updatedAtMs: Int64
    ) throws -> MacCanonicalDataSource
    func dataSource(id: String) throws -> MacCanonicalDataSource?
    func listDataSources() throws -> [MacCanonicalDataSource]
    func deleteDataSource(
        _ id: String,
        expectedRevision: Int64,
        deletedAtMs: Int64
    ) throws -> Bool
    func setDataSourceOverride(
        conversationID: String,
        sourceID: String,
        value: MacCanonicalDataSourceOverride,
        updatedAtMs: Int64
    ) throws -> Bool
    func dataSourceOverride(
        conversationID: String,
        sourceID: String
    ) throws -> MacCanonicalDataSourceOverride
    func appendDataSourceUsage(_ input: MacCanonicalDataSourceUsageInput) throws -> Bool
    func listDataSourceUsage(limit: Int) throws -> [MacCanonicalDataSourceUsageInput]
}

extension MacApplicationPort: MacDataSourceApplicationClient {}

/// Composition belongs beside the concrete adapter. Registry logic only sees
/// the bounded-context client returned here.
nonisolated enum MacDataSourceApplicationComposition {
    static let sharedClient: any MacDataSourceApplicationClient = MacApplicationPort.shared

    static func client(databaseURL: URL) -> (any MacDataSourceApplicationClient)? {
        try? MacApplicationPort(databaseURL: databaseURL)
    }
}
