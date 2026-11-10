// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Implements ServiceContainer+Health logic in the iOS service layer.
// - Interfaces:
//   - ServiceContainer
// - APIs:
//   - none
//
import Foundation
import HealthKit
import UIKit

@MainActor
extension ServiceContainer {
    /// Auto-enable Health data source without checking authorization status.
    /// Per Apple docs: apps don't know authorization status - just read directly.
    func checkAndEnableHealthKitIfAuthorized() async {
        logger.log(.debug, "ServiceContainer", NSLocalizedString("autogen.ServiceContainer.63a2e315", comment: "AUTOGEN"))

        if healthDataSource == nil {
            healthDataSource = HealthDataSource()
        }

        guard let h = healthDataSource else {
            logger.log(.error, "ServiceContainer", NSLocalizedString("autogen.ServiceContainer.37ef41e5", comment: "AUTOGEN"))
            return
        }

        if !dataSources.contains(where: { ($0 as AnyObject) === (h as AnyObject) }) {
            dataSources.append(h)
            logger.log(.info, "ServiceContainer", NSLocalizedString("autogen.ServiceContainer.90b807d8", comment: "AUTOGEN"))
        }

        healthDataSourceEnabled = true
        h.startObserverQueries()
        logger.log(.info, "ServiceContainer", NSLocalizedString("autogen.ServiceContainer.5dacf1e6", comment: "AUTOGEN"))
    }

    /// Return a human-readable authorization summary for HealthKit default types.
    public func getHealthAuthorizationSummary() -> [String: Int] {
        if healthDataSource == nil { healthDataSource = HealthDataSource() }
        guard let h = healthDataSource else { return [:] }
        var out: [String: Int] = [:]
        let summary = h.authorizationSummary()
        for (k, v) in summary {
            out[k] = v.rawValue
        }
        return out
    }

    /// Open the app's Settings page so the user can change Health permissions.
    public func openAppSettings() {
        guard let url = URL(string: UIApplication.openSettingsURLString) else { return }
        if UIApplication.shared.canOpenURL(url) {
            UIApplication.shared.open(url, options: [:], completionHandler: nil)
        }
    }

    /// Run health diagnostics (authorization + sample counts) and log results.
    public func runHealthDiagnostics() async {
        if healthDataSource == nil { healthDataSource = HealthDataSource() }
        guard let h = healthDataSource else { return }
        let report = await h.runDiagnostics()
        for (k, v) in report {
            logger.log(.info, "HealthDiagnostics", "\(k): \(v)")
        }
    }

    /// Attempt a best-effort direct read of HealthKit samples without performing
    /// an explicit authorization request beforehand. Useful for debugging what
    /// samples (if any) the running process can see.
    public func fetchHealthSamplesDirect(
        range: DateInterval = DateInterval(start: Date().addingTimeInterval(-7 * 24 * 3600), end: Date()),
        limit: Int = 10
    ) async -> [String: [String]] {
        if healthDataSource == nil { healthDataSource = HealthDataSource() }
        guard let h = healthDataSource else { return [:] }
        return await h.fetchRawSamplesDirect(range: range, limit: limit)
    }

    /// Enable or disable the HealthDataSource at runtime.
    func setHealthDataSourceEnabled(_ enabled: Bool) {
        if enabled {
            let h: HealthDataSource
            if let existing = healthDataSource {
                h = existing
            } else {
                h = HealthDataSource()
                healthDataSource = h
            }
            if !dataSources.contains(where: { ($0 as AnyObject) === (h as AnyObject) }) {
                dataSources.append(h)
            }
            h.startObserverQueries()
            logger.log(.info, "ServiceContainer", NSLocalizedString("autogen.ServiceContainer.8cfe8d2a", comment: "AUTOGEN"))
        } else {
            if let h = healthDataSource {
                h.stopObserverQueries()
                dataSources.removeAll { ds in
                    return (ds as AnyObject) === (h as AnyObject)
                }
                healthDataSource = nil
                logger.log(.info, "ServiceContainer", NSLocalizedString("autogen.ServiceContainer.e2d798d4", comment: "AUTOGEN"))
            }
        }
    }

    /// Request HealthKit authorization via the HealthDataSource. If granted, ensure the data source is enabled.
    func requestHealthAuthorization() async -> Bool {
        if healthDataSource == nil { healthDataSource = HealthDataSource() }
        guard let h = healthDataSource else { return false }

        do {
            let granted = try await h.requestAuthorization()
            if granted {
                healthDataSourceEnabled = true
                if !dataSources.contains(where: { ($0 as AnyObject) === (h as AnyObject) }) {
                    dataSources.append(h)
                }
                h.startObserverQueries()
                logger.log(.info, "ServiceContainer", NSLocalizedString("autogen.ServiceContainer.8a634bbd", comment: "AUTOGEN"))
                return true
            } else {
                logger.log(.warn, "ServiceContainer", NSLocalizedString("autogen.ServiceContainer.d478d0a6", comment: "AUTOGEN"))
                return false
            }
        } catch {
            logger.log(.error, "ServiceContainer", NSLocalizedString("autogen.ServiceContainer.4d294c08", comment: "AUTOGEN"), metadata: ["error": error.localizedDescription])
            return false
        }
    }
}
