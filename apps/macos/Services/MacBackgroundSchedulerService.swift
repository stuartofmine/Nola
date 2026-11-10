// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Registers the single, bundled, per-user background scheduler agent.

import Foundation
import ServiceManagement

enum MacBackgroundSchedulerService {
    static let label = "com.gaojc.Nola.background-scheduler"
    static let plistName = "\(label).plist"
    private static let registrationIdentityKey = "nola.mac.backgroundScheduler.registrationIdentity"

    static var status: SMAppService.Status {
        SMAppService.agent(plistName: plistName).status
    }

    static func registerIfNeeded() {
        let plistURL = Bundle.main.bundleURL
            .appendingPathComponent("Contents/Library/LaunchAgents", isDirectory: true)
            .appendingPathComponent(plistName)
        let service = SMAppService.agent(plistName: plistName)
        guard MacProcessRuntimePolicy.allowsPersistentBackgroundSchedulerRegistration else {
            unregisterStaleService(service)
            return
        }
        guard FileManager.default.fileExists(atPath: plistURL.path) else {
            record(.warn, "Background scheduler plist is missing")
            unregisterStaleService(service)
            return
        }
        guard let appURL = Bundle.main.executableURL,
              FileManager.default.isExecutableFile(atPath: appURL.path),
              let hostURL = Bundle.main.url(forAuxiliaryExecutable: "nola-agent-host"),
              FileManager.default.isExecutableFile(atPath: hostURL.path) else {
            record(.warn, "Background scheduler runtime is incomplete")
            unregisterStaleService(service)
            return
        }
        let identity = registrationIdentity(
            plistURL: plistURL,
            appURL: appURL,
            hostURL: hostURL
        )
        let storedIdentity = UserDefaults.standard.string(forKey: registrationIdentityKey)
        switch service.status {
        case .enabled, .requiresApproval:
            guard storedIdentity != identity else { return }
            do {
                try service.unregister()
                try service.register()
                UserDefaults.standard.set(identity, forKey: registrationIdentityKey)
                record(.info, "Background scheduler registration refreshed")
            } catch {
                record(.warn, "Background scheduler refresh failed", error: error)
            }
        case .notRegistered, .notFound:
            do {
                try service.register()
                UserDefaults.standard.set(identity, forKey: registrationIdentityKey)
                record(.info, "Background scheduler registered")
            } catch {
                record(.warn, "Background scheduler registration failed", error: error)
            }
        @unknown default:
            return
        }
    }

    private static func unregisterStaleService(_ service: SMAppService) {
        switch service.status {
        case .enabled, .requiresApproval:
            do {
                try service.unregister()
                UserDefaults.standard.removeObject(forKey: registrationIdentityKey)
                record(.info, "Stale background scheduler unregistered")
            } catch {
                record(.warn, "Background scheduler cleanup failed", error: error)
            }
        case .notRegistered, .notFound:
            UserDefaults.standard.removeObject(forKey: registrationIdentityKey)
        @unknown default:
            return
        }
    }

    private static func registrationIdentity(
        plistURL: URL,
        appURL: URL,
        hostURL: URL
    ) -> String {
        let keys: Set<URLResourceKey> = [.contentModificationDateKey, .fileSizeKey]
        let plistValues = try? plistURL.resourceValues(forKeys: keys)
        let hostValues = try? hostURL.resourceValues(forKeys: keys)
        let appValues = try? appURL.resourceValues(forKeys: keys)
        return [
            Bundle.main.bundleURL.standardizedFileURL.path,
            plistURL.standardizedFileURL.path,
            String(plistValues?.fileSize ?? 0),
            String(plistValues?.contentModificationDate?.timeIntervalSince1970 ?? 0),
            appURL.standardizedFileURL.path,
            String(appValues?.fileSize ?? 0),
            String(appValues?.contentModificationDate?.timeIntervalSince1970 ?? 0),
            hostURL.standardizedFileURL.path,
            String(hostValues?.fileSize ?? 0),
            String(hostValues?.contentModificationDate?.timeIntervalSince1970 ?? 0),
        ].joined(separator: "|")
    }

    private static func record(
        _ level: Logger.Level,
        _ message: String,
        error: Error? = nil
    ) {
        var metadata: [String: String]?
        if let error {
            metadata = ["error": error.localizedDescription]
        }
        Logger.shared.log(
            level,
            "MacBackgroundScheduler",
            message,
            metadata: metadata
        )
    }
}
