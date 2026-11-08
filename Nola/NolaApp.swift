// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Implements NolaApp logic in the iOS app.
// - Interfaces:
//   - NolaApp
// - APIs:
//   - none
//
//
//  NolaApp.swift
//  Nola
//
//

import SwiftUI
import BackgroundTasks
import UIKit
import Foundation
import CryptoKit
import Darwin

@main
struct NolaApp: App {
    private static var isRunningUITests: Bool {
        ProcessInfo.processInfo.arguments.contains("-ui-testing")
    }

    private static var isRunningUnitTests: Bool {
        !isRunningUITests && ProcessInfo.processInfo.environment["XCTestConfigurationFilePath"] != nil
    }

    @State private var container: ServiceContainer?
    @State private var startupFailure: String?
    @AppStorage("hasCompletedOnboarding") private var hasCompletedOnboarding = false
    @AppStorage(AppleAppearancePreference.storageKey) private var appearance = "system"

    private static func makeServiceContainer() throws -> ServiceContainer {
        guard isRunningUnitTests || isRunningUITests else {
            return ServiceContainer(applicationRuntime: try IOSApplicationRuntime.requireShared(retry: true))
        }

        let environment = ProcessInfo.processInfo.environment
        let databaseURL: URL
        if let explicitPath = environment["NOLA_REAL_IOS_UI_DATABASE_PATH"], !explicitPath.isEmpty {
            databaseURL = URL(fileURLWithPath: explicitPath, isDirectory: false)
        } else if isRunningUITests,
                  let timelinePath = environment["NOLA_REAL_IOS_UI_TIMELINE_PATH"],
                  !timelinePath.isEmpty {
            databaseURL = URL(fileURLWithPath: timelinePath, isDirectory: false)
                .deletingLastPathComponent()
                .appendingPathComponent("ios-product.db", isDirectory: false)
        } else {
            let runID = environment["NOLA_UI_TEST_RUN_ID"]
                ?? "process-\(ProcessInfo.processInfo.processIdentifier)-\(UUID().uuidString)"
            databaseURL = FileManager.default.temporaryDirectory
                .appendingPathComponent("nola-ios-xctest-host-\(runID)", isDirectory: true)
                .appendingPathComponent("nola-product.db", isDirectory: false)
        }
        let runtime = try IOSApplicationRuntime(databaseURL: databaseURL)
        IOSApplicationRuntime.installShared(runtime)
        #if DEBUG
        if environment["NOLA_REAL_IOS_UI_RESTORE_FROZEN_CREDENTIAL"] == "1" {
            try restoreIsolatedFrozenCredential(runtime: runtime, environment: environment)
        }
        if environment["NOLA_REAL_IOS_UI_PREPARE_CREDENTIAL"] == "1" {
            prepareIsolatedCloudCredential(runtime: runtime, environment: environment)
        }
        #endif
        return ServiceContainer(applicationRuntime: runtime)
    }
    
    init() {
        let serviceContainer: ServiceContainer?
        let failure: String?
        do {
            serviceContainer = try Self.makeServiceContainer()
            failure = nil
        } catch {
            serviceContainer = nil
            failure = error.localizedDescription
            NSLog("Nola application startup failed: %@", error.localizedDescription)
        }
        _container = State(initialValue: serviceContainer)
        _startupFailure = State(initialValue: failure)

        if let serviceContainer {
            #if DEBUG
            if ProcessInfo.processInfo.arguments.contains("-ui-testing-real-chat-host") {
                Self.installRealChatHost(in: serviceContainer)
            }
            #endif
        }

        guard !Self.isRunningUnitTests, !Self.isRunningUITests else { return }
        // CRITICAL: BGTaskScheduler registration MUST happen during app launch
        // Register once here; Scheduler will set the actual handler when initialized
        Scheduler.registerBackgroundTasksEarly()
    }

    #if DEBUG
    private struct FrozenCredentialFixtureError: LocalizedError {
        let stage: String
        var errorDescription: String? {
            "Isolated frozen credential fixture rejected at: " + stage
        }
    }

    private static func frozenCredentialStage<T>(_ stage: String, _ operation: () throws -> T) throws -> T {
        do { return try operation() }
        catch { throw FrozenCredentialFixtureError(stage: stage) }
    }

    private static func restoreIsolatedFrozenCredential(runtime: IOSApplicationRuntime,
                                                         environment: [String: String]) throws {
        // XCTest clones do not copy Keychain. Restore only a missing test slot;
        // immutable canonical profiles and the admitted Run remain untouched.
        guard isRunningUITests,
              let namespace = environment["NOLA_UI_TEST_RUN_ID"], !namespace.isEmpty, namespace.count <= 100,
              namespace.unicodeScalars.allSatisfy({ CharacterSet(charactersIn: "abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789-").contains($0) }),
              let conversation = environment["NOLA_REAL_IOS_UI_FROZEN_CONVERSATION"], !conversation.isEmpty,
              let run = environment["NOLA_REAL_IOS_UI_FROZEN_RUN"], !run.isEmpty,
              let assistant = environment["NOLA_REAL_IOS_UI_FROZEN_ASSISTANT"], !assistant.isEmpty,
              let secret = environment["NOLA_REAL_IOS_UI_CLOUD_KEY"], !secret.isEmpty else {
            throw FrozenCredentialFixtureError(stage: "launch_environment")
        }
        let expected = URL(fileURLWithPath: "/tmp/nola-ios-" + namespace, isDirectory: true)
            .appendingPathComponent("application.db")
        // Foundation canonicalizes /private/tmp back to /tmp on Apple platforms.
        // POSIX realpath proves the actual location without accepting a fixture
        // directory or database symlink that redirects outside this namespace.
        guard runtime.databaseURL.standardizedFileURL == expected.standardizedFileURL,
              let resolved = realpath(runtime.databaseURL.path, nil) else {
            throw FrozenCredentialFixtureError(stage: "isolated_database_path")
        }
        defer { free(resolved) }
        guard String(cString: resolved) == "/private/tmp/nola-ios-" + namespace + "/application.db" else {
            throw FrozenCredentialFixtureError(stage: "isolated_database_realpath")
        }
        let value = try frozenCredentialStage("load_exact_invocation") { try runtime.executeAgentCommand(.object([
            "type": .string("load_exact_run_invocation"), "conversation_id": .string(conversation),
            "expected_run_id": .string(run), "assistant_message_id": .string(assistant)])) }
        let frozen = try frozenCredentialStage("decode_exact_invocation") { try IOSCanonicalAgentApplication.decodeExactRunInvocation(value,
            conversationID: conversation, runID: run, assistantMessageID: assistant) }
        let digest = SHA256.hash(data: Data(secret.utf8)).map { String(format: "%02x", $0) }.joined()
        guard let route = frozen.hostInvocation.objectFields?["primary"]?.objectFields,
              route["credential_sha256"] == .string(digest),
              let wire = route["protocol"]?.stringScalar else { throw FrozenCredentialFixtureError(stage: "frozen_route_secret_or_protocol") }
        if (try? IOSPreparedRunModel.recover(frozen, using: runtime)) != nil { return }
        let service = IOSModelConnectionService(application: runtime)
        let accounts = try frozenCredentialStage("load_accounts") { try service.accounts() }
        let owner = IOSCloudCredentialOwner.shared
        var matches: [(String, Data)] = []
        for snapshot in try frozenCredentialStage("load_connections", { try service.connections() }) {
            let connection = snapshot.document.connection
            guard connection.enabled, connection.protocols.contains(wire),
                  let account = accounts.first(where: { $0.id == connection.account_id }),
                  account.account.state == "ready", let reference = account.account.credential_ref else { continue }
            let payload = try JSONSerialization.data(withJSONObject: ["connectionID": connection.id,
                "endpoint": connection.base_url, "protocols": connection.protocols, "secret": secret])
            let candidate = IOSCloudCredentialOwner(save: { _, _ in false },
                read: { key in key == reference ? payload : nil }, delete: { _ in })
            guard (try? IOSPreparedRunModel.recover(frozen, using: runtime, credentials: candidate)) != nil else { continue }
            matches.append((reference, payload))
        }
        guard matches.count == 1, let match = matches.first else { throw FrozenCredentialFixtureError(stage: "unique_matching_frozen_credential") }
        if owner.read(match.0) == nil {
            guard owner.save(match.0, match.1) else { throw IOSCloudCredentialError.storageFailed }
        }
        // Existing slots are never overwritten; exact native recovery must also
        // succeed with the actual stored credential after restoration.
        _ = try frozenCredentialStage("recover_stored_original_credential") { try IOSPreparedRunModel.recover(frozen, using: runtime) }
    }

    private static func prepareIsolatedCloudCredential(runtime: IOSApplicationRuntime, environment: [String: String]) {
        // Only a copied, UUID-scoped UI fixture may publish a test credential.
        guard isRunningUITests,
              let path = environment["NOLA_REAL_IOS_UI_DATABASE_PATH"],
              let rootPath = environment["NOLA_REAL_IOS_UI_FIXTURE_ROOT"],
              let endpoint = environment["NOLA_REAL_IOS_UI_CLOUD_ENDPOINT"],
              let upstream = environment["NOLA_REAL_IOS_UI_CLOUD_MODEL"],
              let secret = environment["NOLA_REAL_IOS_UI_CLOUD_KEY"] else {
            fatalError("Isolated cloud fixture preparation requires explicit UI test configuration")
        }
        let root = URL(fileURLWithPath: rootPath).standardizedFileURL
        let database = URL(fileURLWithPath: path).standardizedFileURL
        let prefix = "nola-room-real-ui-"
        guard root.lastPathComponent.hasPrefix(prefix),
              UUID(uuidString: String(root.lastPathComponent.dropFirst(prefix.count))) != nil,
              database == root.appendingPathComponent("nola.db"),
              database == database.resolvingSymlinksInPath(),
              runtime.databaseURL == database,
              FileManager.default.fileExists(atPath: database.path) else {
            fatalError("Cloud fixture credential preparation rejected a non-isolated database")
        }
        do {
            let matches = try ModelSelectionManager.availableModels(using: runtime).filter {
                !$0.isLocal && !$0.isWebBackend && $0.baseURL == endpoint && $0.effectiveUpstreamModelId == upstream
            }
            guard matches.count == 1, let model = matches.first else {
                fatalError("Cloud fixture must contain exactly one matching canonical model")
            }
            // Imported profiles belong to a connection/account. Publish the
            // native secret through that current account port, just as the
            // model center does; a generic profile CAS cannot edit its owner.
            guard let profile = try runtime.modelProfile(id: model.id),
                  let source = try JSONSerialization.jsonObject(with: Data(profile.sourceJSON.utf8)) as? [String: Any],
                  let connectionID = source["connectionID"] as? String else {
                fatalError("Cloud fixture requires a current connection-owned model")
            }
            let service = IOSModelConnectionService(application: runtime)
            guard let connection = try service.connections().first(where: { $0.id == connectionID })?.document.connection,
                  connection.enabled, connection.base_url == endpoint,
                  let accountID = connection.account_id,
                  let account = try service.accounts().first(where: { $0.id == accountID }) else {
                fatalError("Cloud fixture connection or account is unavailable")
            }
            let reference = try service.credentials.storeAccountSecret(secret, connectionID: connectionID,
                endpoint: connection.base_url, protocols: connection.protocols)
            let _: IOSModelAccountSnapshot = try service.command([
                "type": .string("save_account"), "input": .object([
                    "id": .string(account.id), "display_name": .string(account.account.display_name),
                    "source": .string(account.account.source), "credential_ref": .string(reference), "state": .string("ready")]),
                "expected_revision": .signedInteger(account.revision),
                "updated_at_ms": .signedInteger(Int64(Date().timeIntervalSince1970 * 1_000))])
            guard let wire = model.wireProtocol?.rawValue,
                  try service.credential(connectionID: connectionID, endpoint: endpoint, wireProtocol: wire).secret == secret else {
                fatalError("Cloud fixture native credential verification failed")
            }
            if environment["NOLA_REAL_IOS_UI_EXPERT_CAPTURE"] == "1" {
                let custom = "Prefer accurate plain numbered astronomy sentences without headings."
                var parameters = ModelParametersStore(applicationPort: runtime).loadParameters(for: model.id, isLocal: false)
                parameters.responseStyle = .professional
                parameters.customSystemPrompt = custom
                try ModelParametersStore(applicationPort: runtime).saveParameters(parameters, for: model.id, isLocal: false)
                // Expectations only: the runner creates the actual owned
                // template resource by operating the production editor.
                let expected: [String: Any] = ["templateName": "Native Capture Template",
                    "templateContent": "Use accurate astronomy facts and preserve the user's BEGIN and END markers.", "customInstruction": custom,
                    "styleInstruction": PromptTemplates.agentResponseStyle(.professional)]
                let output = root.appendingPathComponent("native-capture-fixture.json")
                try JSONSerialization.data(withJSONObject: expected, options: [.sortedKeys]).write(to: output, options: .atomic)
                try FileManager.default.setAttributes([.posixPermissions: 0o600], ofItemAtPath: output.path)
            }
        } catch {
            // Never include provider input or credentials in launch diagnostics.
            fatalError("Isolated cloud fixture credential publication failed")
        }
    }

    private static func installRealChatHost(in container: ServiceContainer) {
        let environment = ProcessInfo.processInfo.environment
        if environment["NOLA_REAL_IOS_UI_RESET"] == "1",
           let store = container.conversationStore,
           let conversations = try? store.getAllConversations() {
            for conversation in conversations {
                try? store.deleteConversation(conversationId: conversation.id)
            }
        }
        guard
            let rawURL = environment["NOLA_REAL_WEB_AGENT_HOST_URL"],
            let baseURL = URL(string: rawURL),
            let token = environment["NOLA_REAL_WEB_AGENT_HOST_TOKEN"],
            !token.isEmpty,
            let hostID = environment["NOLA_REAL_WEB_AGENT_HOST_ID"],
            ModelSelectionManager.installEphemeralWebHostCredential(
                token: token,
                identity: hostID,
                for: baseURL
            )
        else {
            return
        }
        let model = environment["NOLA_REAL_LLM_MODEL"] ?? "agent-host"
        let hostModel = ModelSelectionManager.webBackendModel(baseURL: rawURL, token: token)
        let provider = WebAgentProvider(config: .init(
            baseURL: baseURL,
            token: token,
            model: model,
            expectedHostId: hostID
        ))
        ModelSelectionManager.addModel(hostModel)
        ModelSelectionManager.saveSelectedModel(hostModel)
        container.chatManager.selectModel(hostModel)
        container.llmClient = ModelService(provider: provider)
    }

    #endif

    var body: some Scene {
        WindowGroup {
            Group {
            if let container {
                if Self.isRunningUITests {
                    #if DEBUG
                    if ProcessInfo.processInfo.arguments.contains("-ui-testing-room-permission") {
                        IOSRoomPermissionUITestFixture(container: container)
                    } else if ProcessInfo.processInfo.arguments.contains("-ui-testing-message-reader") {
                        IOSMessageReaderUITestFixture(container: container)
                    } else {
                        MainTabView().environmentObject(container)
                    }
                    #else
                    MainTabView()
                        .environmentObject(container)
                    #endif
                } else if Self.isRunningUnitTests {
                    Text(NSLocalizedString("app.unit_tests.placeholder", comment: "Placeholder shown while running unit tests"))
                } else if hasCompletedOnboarding {
                    ContentView()
                        .environmentObject(container)
                        .onAppear {
                            // Schedule background refresh if not already scheduled
                            if hasCompletedOnboarding {
                                container.scheduler.scheduleNextRefresh()
                            }
                        }
                } else {
                    OnboardingView()
                        .environmentObject(container)
                }
            } else {
                VStack(alignment: .leading, spacing: 20) {
                    Label(String(localized: "Unable to open Nola data"), systemImage: "externaldrive.badge.exclamationmark")
                        .font(.title2.bold())
                    Text(String(localized: "Your existing data has been preserved. No database was reset or converted."))
                    ScrollView {
                        Text(startupFailure ?? String(localized: "Unknown startup error"))
                            .font(.callout.monospaced())
                            .textSelection(.enabled)
                            .frame(maxWidth: .infinity, alignment: .leading)
                    }
                    Button("Retry") {
                        do {
                            container = try Self.makeServiceContainer()
                            startupFailure = nil
                        } catch {
                            startupFailure = error.localizedDescription
                            NSLog("Nola application startup failed: %@", error.localizedDescription)
                        }
                    }
                    .buttonStyle(.borderedProminent)
                }
                .padding(24)
            }
            }
            .preferredColorScheme((AppleAppearancePreference(rawValue: appearance) ?? .system).colorScheme)
        }
    }
}
