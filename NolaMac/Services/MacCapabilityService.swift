// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Reports system capability readiness for the full local CLI toolset.

import AppKit
import ApplicationServices
import CoreGraphics
import Foundation
import Observation
import Security

struct MacCodeSigningIdentitySnapshot: Sendable, Equatable {
    let bundleIdentifier: String
    let signingIdentifier: String?
    let teamIdentifier: String?
    let designatedRequirement: String?

    var isStable: Bool {
        signingIdentifier == bundleIdentifier
            && !(teamIdentifier?.isEmpty ?? true)
            && !(designatedRequirement?.hasPrefix("cdhash ") ?? false)
    }
}

enum MacCodeSigningIdentityProbe {
    static func current() -> MacCodeSigningIdentitySnapshot {
        let bundleIdentifier = Bundle.main.bundleIdentifier ?? "unknown"
        var code: SecCode?
        guard SecCodeCopySelf(SecCSFlags(), &code) == errSecSuccess,
              let code else {
            return MacCodeSigningIdentitySnapshot(
                bundleIdentifier: bundleIdentifier,
                signingIdentifier: nil,
                teamIdentifier: nil,
                designatedRequirement: nil
            )
        }
        var staticCode: SecStaticCode?
        guard SecCodeCopyStaticCode(code, SecCSFlags(), &staticCode) == errSecSuccess,
              let staticCode else {
            return MacCodeSigningIdentitySnapshot(
                bundleIdentifier: bundleIdentifier,
                signingIdentifier: nil,
                teamIdentifier: nil,
                designatedRequirement: nil
            )
        }

        var signingInformation: CFDictionary?
        _ = SecCodeCopySigningInformation(
            staticCode,
            SecCSFlags(rawValue: kSecCSSigningInformation),
            &signingInformation
        )
        let information = signingInformation as? [String: Any]

        var requirement: SecRequirement?
        var requirementText: CFString?
        if SecCodeCopyDesignatedRequirement(staticCode, SecCSFlags(), &requirement) == errSecSuccess,
           let requirement {
            _ = SecRequirementCopyString(requirement, SecCSFlags(), &requirementText)
        }

        return MacCodeSigningIdentitySnapshot(
            bundleIdentifier: bundleIdentifier,
            signingIdentifier: information?[kSecCodeInfoIdentifier as String] as? String,
            teamIdentifier: information?[kSecCodeInfoTeamIdentifier as String] as? String,
            designatedRequirement: requirementText as String?
        )
    }
}

struct MacAccessibilityPermissionSnapshot: Sendable, Equatable {
    let systemTrusted: Bool
    let eventPostingAuthorized: Bool
    let systemWideReadable: Bool
    let focusedElementReadable: Bool
    let treeReadable: Bool
    let runtimeObserved: Bool

    init(
        systemTrusted: Bool,
        eventPostingAuthorized: Bool = false,
        systemWideReadable: Bool,
        focusedElementReadable: Bool = false,
        treeReadable: Bool,
        runtimeObserved: Bool = false
    ) {
        self.systemTrusted = systemTrusted
        self.eventPostingAuthorized = eventPostingAuthorized
        self.systemWideReadable = systemWideReadable
        self.focusedElementReadable = focusedElementReadable
        self.treeReadable = treeReadable
        self.runtimeObserved = runtimeObserved
    }

    var isAuthorized: Bool {
        systemTrusted
            || eventPostingAuthorized
            || systemWideReadable
            || focusedElementReadable
            || treeReadable
            || runtimeObserved
    }

    func targetState(isReadable: Bool) -> MacAccessibilityTargetState {
        if isReadable { return .readable }
        return isAuthorized ? .targetUnavailable : .permissionRequired
    }
}

struct MacScreenRecordingPermissionSnapshot: Sendable, Equatable {
    let systemPreflight: Bool
    let runtimeCaptureSucceeded: Bool

    var isAuthorized: Bool {
        systemPreflight || runtimeCaptureSucceeded
    }
}

enum MacCapabilityPermissionRefreshPolicy {
    static func intervalSeconds(
        applicationIsActive: Bool,
        hasUnresolvedSystemPermission: Bool
    ) -> UInt64 {
        if applicationIsActive, hasUnresolvedSystemPermission { return 2 }
        if applicationIsActive { return 10 }
        return 30
    }
}

enum MacAccessibilityTargetState: Sendable, Equatable {
    case readable
    case permissionRequired
    case targetUnavailable
}

struct MacAccessibilityElementProbe: Sendable, Equatable {
    let result: AXError
    let attributeCount: Int
    let valueProbeSucceeded: Bool

    var isReadable: Bool {
        MacSystemPermissionProbe.elementProbeIsEffective(
            attributeNamesResult: result,
            attributeCount: attributeCount,
            valueProbeSucceeded: valueProbeSucceeded
        )
    }
}

enum MacAccessibilityApplicationResolver {
    static func matchingApplication(_ requested: String) -> NSRunningApplication? {
        let normalized = requested.trimmingCharacters(in: .whitespacesAndNewlines).lowercased()
        guard !normalized.isEmpty else { return nil }
        return NSWorkspace.shared.runningApplications
            .compactMap { application -> (NSRunningApplication, Int)? in
                let score = applicationMatchScore(
                    requested: normalized,
                    localizedName: application.localizedName,
                    bundleIdentifier: application.bundleIdentifier,
                    isRegularApplication: application.activationPolicy == .regular,
                    isNola: isNolaProcess(application)
                )
                return score.map { (application, $0) }
            }
            .max { $0.1 < $1.1 }?
            .0
    }

    static func applicationMatchScore(
        requested: String,
        localizedName: String?,
        bundleIdentifier: String?,
        isRegularApplication: Bool,
        isNola: Bool
    ) -> Int? {
        guard isRegularApplication, !isNola else { return nil }
        let normalized = requested.trimmingCharacters(in: .whitespacesAndNewlines).lowercased()
        guard !normalized.isEmpty else { return nil }
        let name = localizedName?.lowercased() ?? ""
        let bundleID = bundleIdentifier?.lowercased() ?? ""
        if bundleID == normalized { return 400 }
        if name == normalized { return 300 }
        if bundleID.hasSuffix(".\(normalized)") { return 200 }
        if name.contains(normalized) { return 100 }
        return nil
    }

    static func defaultTargetApplication() -> NSRunningApplication? {
        let currentPID = ProcessInfo.processInfo.processIdentifier
        let runningApplications = NSWorkspace.shared.runningApplications
        let eligiblePIDs = Set(runningApplications.compactMap { application -> pid_t? in
            guard application.processIdentifier != currentPID,
                  application.activationPolicy == .regular,
                  !isNolaProcess(application) else { return nil }
            return application.processIdentifier
        })
        let frontmostPID = NSWorkspace.shared.frontmostApplication?.processIdentifier
        let selectedPID = selectTargetPID(
            frontmostPID: frontmostPID,
            currentPID: currentPID,
            orderedWindowOwnerPIDs: orderedWindowOwnerPIDs(),
            eligiblePIDs: eligiblePIDs
        )
        return selectedPID.flatMap(NSRunningApplication.init(processIdentifier:))
    }

    static func externalProbeApplications() -> [NSRunningApplication] {
        var applications: [NSRunningApplication] = []
        if let target = defaultTargetApplication() {
            applications.append(target)
        }
        applications.append(contentsOf: NSRunningApplication.runningApplications(withBundleIdentifier: "com.apple.finder"))
        applications.append(contentsOf: NSWorkspace.shared.runningApplications.filter {
            $0.activationPolicy == .regular && !isNolaProcess($0)
        })
        var seen: Set<pid_t> = []
        return applications.filter { seen.insert($0.processIdentifier).inserted }
    }

    static func selectTargetPID(
        frontmostPID: pid_t?,
        currentPID: pid_t,
        orderedWindowOwnerPIDs: [pid_t],
        eligiblePIDs: Set<pid_t>
    ) -> pid_t? {
        if let frontmostPID,
           frontmostPID != currentPID,
           eligiblePIDs.contains(frontmostPID) {
            return frontmostPID
        }
        return orderedWindowOwnerPIDs.first {
            $0 != currentPID && eligiblePIDs.contains($0)
        }
    }

    private static func orderedWindowOwnerPIDs() -> [pid_t] {
        guard let windows = CGWindowListCopyWindowInfo(
            [.optionOnScreenOnly, .excludeDesktopElements],
            kCGNullWindowID
        ) as? [[String: Any]] else { return [] }
        return windows.compactMap { window in
            guard (window[kCGWindowLayer as String] as? NSNumber)?.intValue == 0,
                  ((window[kCGWindowAlpha as String] as? NSNumber)?.doubleValue ?? 1) > 0 else {
                return nil
            }
            return (window[kCGWindowOwnerPID as String] as? NSNumber).map { pid_t($0.int32Value) }
        }
    }

    private static func isNolaProcess(_ application: NSRunningApplication) -> Bool {
        let bundleIdentifier = application.bundleIdentifier?.lowercased() ?? ""
        let executableName = application.executableURL?.lastPathComponent.lowercased() ?? ""
        return bundleIdentifier == Bundle.main.bundleIdentifier?.lowercased()
            || bundleIdentifier.hasPrefix("com.gaojc.nola")
            || executableName == "nola-agent-host"
            || executableName == "nola-cli"
    }
}

enum MacRuntimePermissionEvidence {
    private static let lock = NSLock()
    private nonisolated(unsafe) static var accessibilitySucceeded = false
    private nonisolated(unsafe) static var screenCaptureSucceeded = false

    static func recordAccessibilitySuccess() {
        lock.lock()
        defer { lock.unlock() }
        accessibilitySucceeded = true
    }

    static func recordScreenCaptureSuccess() {
        lock.lock()
        defer { lock.unlock() }
        screenCaptureSucceeded = true
    }

    static var hasAccessibilityEvidence: Bool {
        lock.lock()
        defer { lock.unlock() }
        return accessibilitySucceeded
    }

    static var hasScreenCaptureEvidence: Bool {
        lock.lock()
        defer { lock.unlock() }
        return screenCaptureSucceeded
    }
}

nonisolated enum MacProcessRuntimePolicy {
    private static let explicitTestModeKey = "NOLA_TEST_MODE"
    private static let systemPermissionTestKey = "NOLA_REQUIRE_MAC_SYSTEM_PERMISSIONS"
    private static let realKeychainTestKey = "NOLA_USE_REAL_KEYCHAIN_IN_TESTS"

    static var isRunningTests: Bool {
        let environment = ProcessInfo.processInfo.environment
        return environment[explicitTestModeKey] == "1"
            || environment["XCTestConfigurationFilePath"] != nil
            || environment["XCTestBundlePath"] != nil
            || NSClassFromString("XCTestCase") != nil
    }

    static var allowsSystemPermissionPrompts: Bool {
        !isRunningTests
            || ProcessInfo.processInfo.environment[systemPermissionTestKey] == "1"
    }

    static var usesEphemeralCredentials: Bool {
        isRunningTests
            && ProcessInfo.processInfo.environment[realKeychainTestKey] != "1"
    }

    static var allowsStartupIntegrations: Bool {
        !isRunningTests
    }

    /// A login item must only point at an application bundle whose location is
    /// expected to survive restarts. Xcode regularly deletes DerivedData build
    /// products, while launchd keeps retrying the registered bundle forever.
    /// Installed applications keep the persistent scheduler; ephemeral build
    /// products use the app's in-process refresh path instead.
    static var allowsPersistentBackgroundSchedulerRegistration: Bool {
        allowsStartupIntegrations && isStableInstalledApplication(Bundle.main.bundleURL)
    }

    static func isStableInstalledApplication(_ bundleURL: URL) -> Bool {
        let components = bundleURL.standardizedFileURL.pathComponents
        guard !components.contains("DerivedData") else { return false }
        guard !components.contains("Build") || !components.contains("Products") else { return false }
        return bundleURL.pathExtension.caseInsensitiveCompare("app") == .orderedSame
    }
}

enum MacSystemPermissionProbe {
    static func accessibility(prompt: Bool = false) -> Bool {
        accessibilitySnapshot(prompt: prompt).isAuthorized
    }

    static func accessibilitySnapshot(prompt: Bool = false) -> MacAccessibilityPermissionSnapshot {
        if MacProcessRuntimePolicy.isRunningTests,
           !MacProcessRuntimePolicy.allowsSystemPermissionPrompts {
            return MacAccessibilityPermissionSnapshot(
                systemTrusted: false,
                eventPostingAuthorized: false,
                systemWideReadable: false,
                focusedElementReadable: false,
                treeReadable: false,
                runtimeObserved: false
            )
        }
        let shouldPrompt = prompt && MacProcessRuntimePolicy.allowsSystemPermissionPrompts
        let trusted: Bool
        if shouldPrompt {
            let options = [kAXTrustedCheckOptionPrompt.takeUnretainedValue() as String: true] as CFDictionary
            trusted = AXIsProcessTrustedWithOptions(options)
        } else {
            trusted = AXIsProcessTrusted()
        }
        var eventPostingAuthorized = CGPreflightPostEventAccess()
        if shouldPrompt, !trusted, !eventPostingAuthorized {
            eventPostingAuthorized = CGRequestPostEventAccess()
        }
        if trusted || eventPostingAuthorized {
            MacRuntimePermissionEvidence.recordAccessibilitySuccess()
            return MacAccessibilityPermissionSnapshot(
                systemTrusted: trusted,
                eventPostingAuthorized: eventPostingAuthorized,
                systemWideReadable: false,
                focusedElementReadable: false,
                treeReadable: false,
                runtimeObserved: true
            )
        }
        // AXIsProcessTrusted can lag briefly after returning from System
        // Settings. Probe the system-wide focused application as the first
        // behavioral check, but never accept Nola's own AX tree as proof.
        let systemWide = AXUIElementCreateSystemWide()
        var focusedApplicationValue: CFTypeRef?
        let focusedApplicationResult = AXUIElementCopyAttributeValue(
            systemWide,
            kAXFocusedApplicationAttribute as CFString,
            &focusedApplicationValue
        )
        var focusedApplicationPID: pid_t = 0
        let focusedPIDResult: AXError
        if let focusedApplicationValue,
           CFGetTypeID(focusedApplicationValue) == AXUIElementGetTypeID() {
            let focusedApplication = unsafeBitCast(
                focusedApplicationValue,
                to: AXUIElement.self
            )
            focusedPIDResult = AXUIElementGetPid(focusedApplication, &focusedApplicationPID)
        } else {
            focusedPIDResult = .noValue
        }
        let currentPID = ProcessInfo.processInfo.processIdentifier
        let systemWideReadable = systemWideProbeIsEffective(
            focusedApplicationResult: focusedApplicationResult,
            focusedPIDResult: focusedPIDResult,
            focusedApplicationPID: focusedApplicationPID,
            currentPID: currentPID,
            probeTargetsExternalApplication: focusedApplicationPID != currentPID
        )

        // TCC can update the actual AX connection before AXIsProcessTrusted()
        // catches up after the user returns from System Settings. Reading the
        // focused UI element is a second, concrete capability check. Only an
        // element owned by another process counts, so Nola never authorizes
        // itself by reading its own accessibility tree.
        var focusedElementValue: CFTypeRef?
        let focusedElementResult = AXUIElementCopyAttributeValue(
            systemWide,
            kAXFocusedUIElementAttribute as CFString,
            &focusedElementValue
        )
        var focusedElementPID: pid_t = 0
        var focusedElementProbeSucceeded = false
        if focusedElementResult == .success,
           let focusedElementValue,
           CFGetTypeID(focusedElementValue) == AXUIElementGetTypeID() {
            let focusedElement = unsafeBitCast(focusedElementValue, to: AXUIElement.self)
            if AXUIElementGetPid(focusedElement, &focusedElementPID) == .success,
               focusedElementPID > 0,
               focusedElementPID != currentPID {
                focusedElementProbeSucceeded = accessibilityElementProbe(focusedElement).isReadable
            }
        }

        // Nola is usually focused immediately after returning from Settings,
        // so retain an external-process fallback as well.
        var externalProbeResult: AXError = .noValue
        var externalTreeProbeSucceeded = false
        let externalApplications = MacAccessibilityApplicationResolver.externalProbeApplications()
        for application in externalApplications {
            let element = AXUIElementCreateApplication(application.processIdentifier)
            let probe = accessibilityElementProbe(element)
            externalProbeResult = probe.result
            if probe.isReadable {
                externalTreeProbeSucceeded = true
                break
            }
        }
        let treeReadable = accessibilityIsEffective(
            systemTrusted: false,
            focusedApplicationResult: externalProbeResult,
            hasFocusedApplication: !externalApplications.isEmpty,
            focusedTreeProbeSucceeded: externalTreeProbeSucceeded,
            probeTargetsExternalApplication: true
        )
        if trusted || systemWideReadable || focusedElementProbeSucceeded || treeReadable {
            MacRuntimePermissionEvidence.recordAccessibilitySuccess()
        }
        return MacAccessibilityPermissionSnapshot(
            systemTrusted: trusted,
            eventPostingAuthorized: eventPostingAuthorized,
            systemWideReadable: systemWideReadable,
            focusedElementReadable: focusedElementProbeSucceeded,
            treeReadable: treeReadable,
            runtimeObserved: MacRuntimePermissionEvidence.hasAccessibilityEvidence
        )
    }

    static func screenRecordingSnapshot() -> MacScreenRecordingPermissionSnapshot {
        if MacProcessRuntimePolicy.isRunningTests,
           !MacProcessRuntimePolicy.allowsSystemPermissionPrompts {
            return MacScreenRecordingPermissionSnapshot(
                systemPreflight: false,
                runtimeCaptureSucceeded: false
            )
        }
        return MacScreenRecordingPermissionSnapshot(
            systemPreflight: CGPreflightScreenCaptureAccess(),
            runtimeCaptureSucceeded: MacRuntimePermissionEvidence.hasScreenCaptureEvidence
        )
    }

    static func accessibilityElementProbe(_ element: AXUIElement) -> MacAccessibilityElementProbe {
        var attributeNames: CFArray?
        let attributeNamesResult = AXUIElementCopyAttributeNames(element, &attributeNames)
        let attributeCount = attributeNames.map(CFArrayGetCount) ?? 0

        var valueProbeSucceeded = false
        var firstMeaningfulResult: AXError?
        for attribute in [
            kAXRoleAttribute,
            kAXFocusedWindowAttribute,
            kAXWindowsAttribute,
            kAXChildrenAttribute,
        ] {
            var value: CFTypeRef?
            let result = AXUIElementCopyAttributeValue(
                element,
                attribute as CFString,
                &value
            )
            if firstMeaningfulResult == nil, result != .attributeUnsupported {
                firstMeaningfulResult = result
            }
            if result == .success, value != nil {
                valueProbeSucceeded = true
                break
            }
        }

        return MacAccessibilityElementProbe(
            result: attributeNamesResult == .success
                ? attributeNamesResult
                : (firstMeaningfulResult ?? attributeNamesResult),
            attributeCount: attributeCount,
            valueProbeSucceeded: valueProbeSucceeded
        )
    }

    static func elementProbeIsEffective(
        attributeNamesResult: AXError,
        attributeCount: Int,
        valueProbeSucceeded: Bool
    ) -> Bool {
        (attributeNamesResult == .success && attributeCount > 0) || valueProbeSucceeded
    }

    static func systemWideProbeIsEffective(
        focusedApplicationResult: AXError,
        focusedPIDResult: AXError,
        focusedApplicationPID: pid_t,
        currentPID: pid_t,
        probeTargetsExternalApplication: Bool
    ) -> Bool {
        focusedApplicationResult == .success
            && focusedPIDResult == .success
            && focusedApplicationPID > 0
            && focusedApplicationPID != currentPID
            && probeTargetsExternalApplication
    }

    static func accessibilityIsEffective(
        systemTrusted: Bool,
        focusedApplicationResult: AXError,
        hasFocusedApplication: Bool,
        focusedTreeProbeSucceeded: Bool = true,
        probeTargetsExternalApplication: Bool = true
    ) -> Bool {
        systemTrusted || (
            probeTargetsExternalApplication
                &&
            focusedApplicationResult == .success
                && hasFocusedApplication
                && focusedTreeProbeSucceeded
        )
    }

    static func permissionSubject() -> [String: String] {
        let bundleExecutable = Bundle.main.object(forInfoDictionaryKey: "CFBundleExecutable") as? String
        let executableURL = Bundle.main.executableURL
        let signingIdentity = MacCodeSigningIdentityProbe.current()
        return [
            "bundle_id": Bundle.main.bundleIdentifier ?? "unknown",
            "bundle_path": Bundle.main.bundleURL.path,
            "executable": executableURL?.path ?? ProcessInfo.processInfo.processName,
            "process_name": ProcessInfo.processInfo.processName,
            "is_main_app_process": String(executableURL?.lastPathComponent == bundleExecutable),
            "signing_identifier": signingIdentity.signingIdentifier ?? "unknown",
            "team_identifier": signingIdentity.teamIdentifier ?? "none",
            "designated_requirement": signingIdentity.designatedRequirement ?? "none",
            "stable_signing_identity": String(signingIdentity.isStable),
        ]
    }
}

enum MacCapabilityState: String, Sendable, Equatable {
    case ready
    case permissionRequired
    case optional

    var title: String {
        switch self {
        case .ready:
            return NSLocalizedString("mac.capabilities.available", value: "Available", comment: "Available capability state")
        case .permissionRequired:
            return NSLocalizedString("mac.capabilities.permission_required", value: "Permission Required", comment: "Permission required capability state")
        case .optional:
            return NSLocalizedString("mac.capabilities.optional", value: "Optional", comment: "Optional capability state")
        }
    }
}

struct MacCapabilitySnapshot: Identifiable, Sendable, Equatable {
    enum Kind: String, Sendable, Equatable {
        case files
        case shell
        case git
        case network
        case browser
        case accessibility
        case screenRecording
        case automation
        case fullDiskAccess
    }

    let kind: Kind
    let title: String
    let detail: String
    let symbol: String
    let state: MacCapabilityState

    var id: String { kind.rawValue }

    var hasUserAction: Bool {
        if state == .permissionRequired { return true }
        return state == .optional && (kind == .automation || kind == .fullDiskAccess)
    }
}

@MainActor
@Observable
final class MacCapabilityService {
    var capabilities: [MacCapabilitySnapshot] = []
    @ObservationIgnored private var refreshTask: Task<Void, Never>?
    @ObservationIgnored private var permissionProbeTask: Task<Void, Never>?

    init() {}

    func refreshAfterAppActivation() {
        refresh()
        refreshTask?.cancel()
        refreshTask = Task { @MainActor [weak self] in
            for delay in [250, 1_000] {
                try? await Task.sleep(for: .milliseconds(delay))
                guard !Task.isCancelled, let self else { return }
                self.refresh()
            }
        }
    }

    func refresh() {
        permissionProbeTask?.cancel()
        if MacProcessRuntimePolicy.isRunningTests,
           !MacProcessRuntimePolicy.allowsSystemPermissionPrompts {
            applyRefresh(
                accessibilityReady: false,
                screenReady: false,
                hasStableSigningIdentity: false
            )
            return
        }
        permissionProbeTask = Task { @MainActor [weak self] in
            let result = await Task.detached(priority: .utility) {
                (
                    accessibilityReady: await MacSystemPermissionProbe.accessibility(),
                    screenReady: await MacSystemPermissionProbe.screenRecordingSnapshot().isAuthorized,
                    hasStableSigningIdentity: await MacCodeSigningIdentityProbe.current().isStable
                )
            }.value
            guard !Task.isCancelled, let self else { return }
            self.applyRefresh(
                accessibilityReady: result.accessibilityReady,
                screenReady: result.screenReady,
                hasStableSigningIdentity: result.hasStableSigningIdentity
            )
        }
    }

    private func applyRefresh(
        accessibilityReady: Bool,
        screenReady: Bool,
        hasStableSigningIdentity: Bool
    ) {
        let accessibilityDetail = !accessibilityReady && !hasStableSigningIdentity
            ? text("mac.capabilities.item.accessibility.unstable_detail", "This Nola build has an unstable signing identity. macOS may show an older Nola entry as enabled while denying the current build. Run a normally signed build, remove the stale entry, and grant the current Nola app access again.")
            : text("mac.capabilities.item.accessibility.detail", "Required for keyboard, pointer, menu, window, and accessibility-element actions")
        let updated = [
            snapshot(.files, text("mac.capabilities.item.files.title", "Files and Workspaces"), text("mac.capabilities.item.files.detail", "Availability is resolved per workspace and approved run"), "folder", .optional),
            snapshot(.shell, text("mac.capabilities.item.shell.title", "Shell and Processes"), text("mac.capabilities.item.shell.detail", "Availability is resolved by the shared agent policy for each run"), "terminal", .optional),
            snapshot(.git, text("mac.capabilities.item.git.title", "Git and Repositories"), text("mac.capabilities.item.git.detail", "Availability follows the selected workspace and agent policy"), "point.3.connected.trianglepath.dotted", .optional),
            snapshot(.network, text("mac.capabilities.item.network.title", "Network"), text("mac.capabilities.item.network.detail", "Availability follows network state and the shared agent policy"), "network", .optional),
            snapshot(.browser, text("mac.capabilities.item.browser.title", "Chrome Browser Control"), text("mac.capabilities.item.browser.detail", "Available through the installed Nola Chrome extension and agent approval"), "puzzlepiece.extension", .optional),
            snapshot(
                .accessibility,
                text("mac.capabilities.item.accessibility.title", "Accessibility"),
                accessibilityDetail,
                "accessibility",
                accessibilityReady ? .ready : .permissionRequired
            ),
            snapshot(
                .screenRecording,
                text("mac.capabilities.item.screen_recording.title", "Screen Recording"),
                text("mac.capabilities.item.screen_recording.detail", "Required for screenshots and visual computer-use observation"),
                "rectangle.inset.filled.and.person.filled",
                screenReady ? .ready : .permissionRequired
            ),
            snapshot(.automation, text("mac.capabilities.item.automation.title", "App Automation"), text("mac.capabilities.item.automation.detail", "macOS asks separately when Nola controls another application"), "app.badge.checkmark", .optional),
            snapshot(.fullDiskAccess, text("mac.capabilities.item.full_disk_access.title", "Full Disk Access"), text("mac.capabilities.item.full_disk_access.detail", "Optional for workflows outside user-selected workspaces"), "internaldrive", .optional),
        ]
        if capabilities != updated {
            capabilities = updated
        }
    }

    private func text(_ key: String, _ fallback: String) -> String {
        NSLocalizedString(key, value: fallback, comment: "Mac capability")
    }

    func request(_ kind: MacCapabilitySnapshot.Kind) {
        switch kind {
        case .accessibility:
            if MacProcessRuntimePolicy.allowsSystemPermissionPrompts,
               !MacSystemPermissionProbe.accessibility(prompt: true) {
                openPrivacyPane("Accessibility")
            }
        case .screenRecording:
            if MacProcessRuntimePolicy.allowsSystemPermissionPrompts,
               !CGRequestScreenCaptureAccess() {
                openPrivacyPane("ScreenCapture")
            }
        case .automation:
            openPrivacyPane("Automation")
        case .fullDiskAccess:
            openPrivacyPane("AllFiles")
        default:
            break
        }
        refresh()
        refreshTask?.cancel()
        refreshTask = Task { [weak self] in
            for delay in [300, 1_000, 2_500, 5_000] {
                try? await Task.sleep(for: .milliseconds(delay))
                guard !Task.isCancelled else { return }
                self?.refresh()
            }
        }
    }

    private func snapshot(
        _ kind: MacCapabilitySnapshot.Kind,
        _ title: String,
        _ detail: String,
        _ symbol: String,
        _ state: MacCapabilityState
    ) -> MacCapabilitySnapshot {
        MacCapabilitySnapshot(kind: kind, title: title, detail: detail, symbol: symbol, state: state)
    }

    private func openPrivacyPane(_ anchor: String) {
        if let url = URL(string: "x-apple.systempreferences:com.apple.preference.security?Privacy_\(anchor)") {
            NSWorkspace.shared.open(url)
        }
    }
}
