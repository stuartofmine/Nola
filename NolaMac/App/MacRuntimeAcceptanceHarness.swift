// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Exercises the embedded CLI host through the macOS application process in Debug builds.

#if DEBUG
import AppKit
import CoreFoundation
import CryptoKit
import Foundation
import SwiftUI

@MainActor
enum MacRuntimeAcceptanceHarness {
    static func runIfRequested() async {
        let environment = ProcessInfo.processInfo.environment
        guard let outputPath = environment["NOLA_MAC_RUNTIME_ACCEPTANCE"], !outputPath.isEmpty,
              let projectPath = environment["NOLA_MAC_RUNTIME_ACCEPTANCE_PROJECT"], !projectPath.isEmpty else {
            return
        }
        let acceptanceProvider: MacAppleChatConfiguration?
        do {
            acceptanceProvider = try self.acceptanceProvider(environment: environment)
        } catch {
            writeEarlyFailure(error, outputPath: outputPath)
            return
        }
        let prompt = environment["NOLA_MAC_RUNTIME_ACCEPTANCE_PROMPT"]
            ?? "Reply exactly: NOLA_MAC_REAL_OK"
        let toolEventRequired =
            environment["NOLA_MAC_RUNTIME_ACCEPTANCE_REQUIRE_TOOL"] == "1"
        let usageEventRequired =
            environment["NOLA_MAC_RUNTIME_ACCEPTANCE_REQUIRE_USAGE"] == "1"
        let thinkingEventRequired =
            environment["NOLA_MAC_RUNTIME_ACCEPTANCE_REQUIRE_THINKING"] == "1"
        let allowPermissionRequests =
            environment["NOLA_MAC_RUNTIME_ACCEPTANCE_PERMISSION_DECISION"]?
                .localizedCaseInsensitiveCompare("deny") != .orderedSame
        let permissionMode: MacPermissionMode
        do {
            permissionMode = try MacPermissionMode.configured(environment["NOLA_MAC_RUNTIME_ACCEPTANCE_PERMISSION_MODE"])
        } catch {
            writeEarlyFailure(error, outputPath: outputPath)
            return
        }
        let permissionFixture: MacRuntimeAcceptancePermissionFixture?
        do {
            if let path = environment["NOLA_MAC_RUNTIME_ACCEPTANCE_PERMISSION_FIXTURE"], !path.isEmpty {
                guard permissionMode == .cautious else {
                    throw MacRuntimeAcceptanceContractError.invalidPermissionFixture
                }
                permissionFixture = try .init(projectURL: URL(fileURLWithPath: projectPath, isDirectory: true),
                                              relativePath: path)
            } else { permissionFixture = nil }
        } catch {
            writeEarlyFailure(error, outputPath: outputPath)
            return
        }
        let permissionScope = environment["NOLA_MAC_RUNTIME_ACCEPTANCE_PERMISSION_SCOPE"]
            .flatMap(MacPermissionGrantScope.init(rawValue:)) ?? .once
        let webSourceURL = environment["NOLA_MAC_RUNTIME_ACCEPTANCE_WEB_SOURCE_URL"]?
            .trimmingCharacters(in: .whitespacesAndNewlines)
        let contextComponents = environment["NOLA_MAC_RUNTIME_ACCEPTANCE_CONTEXT"].map { context in
            [
                MacCoreContextComponent(
                    kind: "platform_data",
                    id: "runtime-acceptance-data-source",
                    content: context
                ),
            ]
        } ?? []

        let configuredConversationID = environment["NOLA_MAC_RUNTIME_ACCEPTANCE_SESSION_ID"]?
            .trimmingCharacters(in: .whitespacesAndNewlines)
        let executionBackend = environment["NOLA_MAC_RUNTIME_ACCEPTANCE_EXECUTION_BACKEND"]?
            .trimmingCharacters(in: .whitespacesAndNewlines)
        let selectedExecutionBackend = executionBackend?.isEmpty == false ? executionBackend! : "nola"
        let conversationID = configuredConversationID?.isEmpty == false
            ? configuredConversationID! : "mac-runtime-acceptance-\(UUID().uuidString.lowercased())"
        let permissionConsumer = MacWorkspaceModel(
            initialConversationID: conversationID,
            canonicalRepository: MacCanonicalConversationRepository(store: .shared),
            localAgentRuntime: MacAgentHostRuntime.shared,
            remoteHost: MacRemoteHostService.shared
        )
        let request: MacLocalAgentRunRequest
        do {
            request = try MacLocalAgentRunRequest(
                conversationID: conversationID,
                expectedConfigurationRevision: permissionConsumer.selectedConversation?.configurationRevision,
                expectedParentMessageID: try MacApplicationPort.shared.loadConversation(conversationID)?.messages.last?.id,
                prompt: prompt,
                contextComponents: contextComponents,
                projectURL: URL(fileURLWithPath: projectPath, isDirectory: true),
                attachmentURLs: [],
                modelID: acceptanceProvider?.model,
                endpoint: acceptanceProvider?.baseURL.absoluteString,
                apiKey: acceptanceProvider?.apiKey,
                reasoningConfiguration: acceptanceProvider?.reasoning,
                reasoningTier: "balanced",
                modelParameters: acceptanceProvider?.parameters,
                modelExecutionCapabilities: acceptanceProvider?.executionCapabilities,
                wireProtocol: acceptanceProvider?.wireProtocol,
                executionBackend: selectedExecutionBackend,
                permissionMode: permissionMode
            )
        } catch {
            writeEarlyFailure(error, outputPath: outputPath)
            return
        }
        var events: [[String: Any]] = []
        var streamAssembler = MacSemanticStreamAssembler()
        var committedStreamNodes: [MacAssistantTextNode] = []
        var presentation = MacAssistantPresentation.processing()
        var committedPrefixIsStable = true
        var mutableTailIDs: Set<UUID> = []
        var mutableTailUpdates = 0
        var assistantDeltaCount = 0
        var usageSnapshotCount = 0
        var coreTextProjectionCount = 0
        var bodyMutationCount = 0
        var projectedAssistantText = ""
        var canonicalAssistantText = ""
        var canonicalProjectionLoaded = false
        var canonicalProjectionMatchesStream = false
        var errorMessage: String?
        var attachmentResults: [[String: Any]] = []
        var attachmentSuccess = true
        var backupResult: [String: Any] = [:]
        var backupSuccess = false
        var cancellationResult: [String: Any] = [:]
        var cancellationSuccess = false
        var webSourceResult: [String: Any] = [:]
        var webSourceSuccess = webSourceURL?.isEmpty != false
        let automationChainRequested =
            environment["NOLA_MAC_RUNTIME_ACCEPTANCE_AUTOMATION_CHAIN"] == "1"
        var automationChainResult: [String: Any] = automationChainRequested
            ? ["matched": false, "error": "Web source fixture was not available"]
            : ["matched": true, "skipped": true]
        var automationChainSuccess = !automationChainRequested
        var followUpResult: [String: Any] = ["matched": true, "skipped": true]
        var followUpSuccess = true
        var externalSessionID: String?
        // Exercise the production permission consumer, not an unconditional
        // reply to the raw challenge (which masks missing Core projections).
        var answeredPermissionIDs = Set<String>()
        var permissionAudits: [[String: Any]] = []
        var permissionFixtureResult: [String: Any] = [:]
        var permissionFixtureSuccess = permissionFixture == nil
        var permissionResponse = ""
        var permissionAssembler = MacSemanticStreamAssembler(identitySeed: "acceptance-permission")
        var permissionSucceeded = true
        let primaryDeadline = acceptanceDeadline(
            environment: environment,
            key: "NOLA_MAC_RUNTIME_ACCEPTANCE_SCENARIO_TIMEOUT_SECONDS",
            defaultSeconds: 90,
            scenario: "primary_agent_turn",
            conversationID: request.conversationID,
            outputPath: outputPath
        )

        do {
            for try await event in await MacAgentHostRuntime.shared.run(request) {
                let eventRecord: [String: Any] = [
                    "kind": kindName(event.kind),
                    "text": event.text,
                    "title": event.title ?? "",
                    "detail": event.detail ?? "",
                    "is_error": event.isError,
                    "event_id": event.eventID,
                    "trace_kind": event.trace?.kind ?? "",
                    "tool_name": event.trace?.payload.toolName ?? event.title ?? "",
                    "tool_call_id": event.toolCallID ?? "",
                ]
                let isProjectionOnlyEvent = event.assistantJournalCursor != nil
                    && event.trace == nil
                    && event.text.isEmpty
                    && (event.title ?? "").isEmpty
                    && (event.detail ?? "").isEmpty
                if !isProjectionOnlyEvent {
                    if events.count >= 500 {
                        events.removeFirst(events.count - 499)
                    }
                    events.append(eventRecord)
                    appendJSONLine(eventRecord, to: outputPath + ".events.jsonl")
                }
                if event.kind == .status,
                   event.detail == "external_session_started",
                   !event.text.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
                    externalSessionID = event.text.trimmingCharacters(in: .whitespacesAndNewlines)
                }
                if let trace = event.trace {
                    if trace.kind == "usage.snapshot", trace.payload.usage != nil {
                        usageSnapshotCount += 1
                    }
                }
                if event.assistantJournalCursor != nil || event.assistantRunTerminalSealDigest != nil {
                    coreTextProjectionCount += 1
                }
                // Trace is an append-only audit log, not the response body.
                // Exercise committed journal reconciliation, including retractions,
                // before independently checking the persisted final message.
                let authoritativeText: String?
                if let cursor = event.assistantJournalCursor,
                   cursor.identity.assistant_message_id == request.assistantMessageID {
                    authoritativeText = try await MacJournalTextRead.markdown(.init(
                        databaseURL: request.databaseURLOverride ?? request.applicationPort?.databaseURL ?? MacApplicationPort.defaultDatabaseURL,
                        cursor: cursor), historicalFallback: "")
                } else { authoritativeText = nil }
                switch authoritativeText.map({
                    AuthoritativeTextReconciliation.reconcile(
                        authoritative: $0, accepted: streamAssembler.rawText
                    )
                }) {
                case .append(let delta):
                    assistantDeltaCount += 1
                    let previousIDs = committedStreamNodes.map(\.id)
                    let nodes = streamAssembler.append(delta)
                    committedStreamNodes.append(contentsOf: nodes)
                    presentation.append(nodes)
                    presentation.updateMutableTextNode(streamAssembler.mutableTail)
                    committedPrefixIsStable = committedPrefixIsStable
                        && Array(committedStreamNodes.prefix(previousIDs.count)).map(\.id) == previousIDs
                    if let mutableTail = streamAssembler.mutableTail {
                        mutableTailUpdates += 1
                        mutableTailIDs.insert(mutableTail.id)
                    }
                    projectedAssistantText += delta
                    bodyMutationCount += 1
                case .replace(let replacement):
                    streamAssembler = MacSemanticStreamAssembler()
                    committedStreamNodes.removeAll(keepingCapacity: true)
                    presentation.replaceText(with: [])
                    mutableTailIDs.removeAll(keepingCapacity: true)
                    mutableTailUpdates = 0
                    projectedAssistantText = replacement
                    bodyMutationCount += 1
                    if !replacement.isEmpty {
                        let nodes = streamAssembler.append(replacement)
                        committedStreamNodes.append(contentsOf: nodes)
                        presentation.append(nodes)
                        presentation.updateMutableTextNode(streamAssembler.mutableTail)
                    }
                default:
                    break
                }
                if event.permissionRequest != nil || event.corePermissionProjection != nil {
                    await permissionConsumer.consumeTracedAgentEvent(
                        event, prompt: prompt, conversationID: request.conversationID,
                        modelID: request.modelID ?? "acceptance",
                        assistantMessageID: MacCanonicalMessageIdentity.stableID(request.assistantMessageID) ?? UUID(),
                        generationID: 0, permissionUsesLocalRuntime: true,
                        permissionAttachmentURLs: [], response: &permissionResponse,
                        assembler: &permissionAssembler, succeeded: &permissionSucceeded
                    )
                }
                if let permissionRequest = permissionConsumer.pendingRemotePermission,
                   answeredPermissionIDs.insert(permissionRequest.id).inserted {
                    if let permissionFixture {
                        let port = request.applicationPort ?? .shared
                        guard let active = try port.activeRunIdentity(conversationID: request.conversationID),
                              active.runID == request.submissionID,
                              active.assistantMessageID == request.assistantMessageID,
                              let projection = try port.loadPermissionProjection(request.conversationID, runID: active.runID),
                              let pending = projection.pending.first(where: { $0.requestID == permissionRequest.id }) else {
                            throw MacRuntimeAcceptanceContractError.invalidPermissionFixture
                        }
                        var audit = try permissionFixture.audit(pending: pending, challenge: permissionRequest,
                            runID: active.runID, currentBytes: Data(contentsOf: permissionFixture.url))
                        audit["phase"] = "before_permission_decision"
                        audit["conversation_id"] = request.conversationID
                        audit["assistant_message_id"] = request.assistantMessageID
                        audit["permission_projection_revision"] = projection.revision
                        audit["decision"] = allowPermissionRequests ? "allow_once" : "deny"
                        permissionAudits.append(audit)
                        appendJSONLine(audit, to: outputPath + ".permission-audit.jsonl")
                    }
                    try await MacAgentHostRuntime.shared.resolvePermission(
                        conversationID: request.conversationID,
                        request: permissionRequest,
                        allow: allowPermissionRequests,
                        scope: permissionScope
                    )
                }
            }
        } catch {
            errorMessage = error.localizedDescription
        }
        primaryDeadline.cancel()
        if let permissionFixture {
            do {
                let finalBytes = try Data(contentsOf: permissionFixture.url)
                let terminal = try (request.applicationPort ?? .shared).canonicalRunTerminal(
                    conversationID: request.conversationID, runID: request.submissionID)
                let expectedKind: MacRunResult.Value.Kind = allowPermissionRequests ? .actionCompleted : .blocked
                permissionFixtureSuccess = permissionAudits.count == 1
                    && terminal?.assistantMessageID == request.assistantMessageID
                    && terminal?.result.value.kind == expectedKind
                    && (allowPermissionRequests ? finalBytes != permissionFixture.originalBytes
                        : finalBytes == permissionFixture.originalBytes)
                permissionFixtureResult = [
                    "relative_path": permissionFixture.relativePath,
                    "original_sha256": permissionFixture.originalSHA256,
                    "original_bytes_base64": permissionFixture.originalBytes.base64EncodedString(),
                    "final_sha256": MacRuntimeAcceptancePermissionFixture.sha256(finalBytes),
                    "final_bytes_base64": finalBytes.base64EncodedString(),
                    "canonical_terminal_sealed": terminal?.status == .sealed,
                    "canonical_result_kind": terminal?.result.value.kind.rawValue ?? "",
                    "matched": permissionFixtureSuccess,
                ]
            } catch {
                errorMessage = errorMessage ?? error.localizedDescription
            }
        }
        let finalNodes = streamAssembler.finish()
        committedStreamNodes.append(contentsOf: finalNodes)
        presentation.append(finalNodes)
        presentation.updateMutableTextNode(nil)
        presentation.finish()

        do {
            let repository = MacCanonicalConversationRepository(store: .shared)
            let projection = try repository.conversationProjection(
                conversationID: request.conversationID
            )
            guard let canonical = projection.messages.first(where: {
                $0.messageID == request.assistantMessageID
            }) else {
                throw MacRuntimeAcceptanceContractError.missingAssistantMessage(
                    request.assistantMessageID
                )
            }
            let projected = try MacCanonicalConversationRepository.projectMessage(canonical)
            canonicalAssistantText = projected.text
            canonicalProjectionLoaded = true
            canonicalProjectionMatchesStream = projected.text == streamAssembler.rawText
            if let canonicalPresentation = projected.presentation {
                presentation = canonicalPresentation
            }
        } catch {
            errorMessage = errorMessage ?? error.localizedDescription
        }

        let screenshotResult = captureAssistantResponseIfRequested(
            environment: environment,
            rawText: canonicalAssistantText,
            presentation: presentation
        )
        let visualObservation = visualObservationResult(
            environment: environment,
            screenshot: screenshotResult
        )

        if let followUpPrompt = environment["NOLA_MAC_RUNTIME_ACCEPTANCE_FOLLOW_UP_PROMPT"]?
            .trimmingCharacters(in: .whitespacesAndNewlines),
           !followUpPrompt.isEmpty {
            let acceptance = await runFollowUpAcceptance(
                conversationID: request.conversationID,
                prompt: followUpPrompt,
                expectedText: environment["NOLA_MAC_RUNTIME_ACCEPTANCE_FOLLOW_UP_EXPECTED"] ?? "",
                projectPath: projectPath,
                provider: acceptanceProvider,
                allowPermissionRequests: allowPermissionRequests,
                executionBackend: selectedExecutionBackend,
                externalSessionID: externalSessionID,
                outputPath: outputPath,
                environment: environment
            )
            followUpSuccess = acceptance.success
            followUpResult = acceptance.result
        }

        let cancellationID = "\(request.conversationID)-cancel"
        let cancellationStartedAt = Date()
        var cancellationAdmitted = false
        let cancellationPrompt = environment["NOLA_MAC_RUNTIME_ACCEPTANCE_CANCELLATION_PROMPT"]?
            .trimmingCharacters(in: .whitespacesAndNewlines)
        let effectiveCancellationPrompt = cancellationPrompt?.isEmpty == false
            ? cancellationPrompt!
            : "Use the available shell tool to run `sleep 30`, wait for it to finish, then reply with CANCEL_TEST_DONE."
        let cancellationTask = Task { () -> String in
            do {
                let cancelRequest = try MacLocalAgentRunRequest(
                    conversationID: cancellationID,
                    prompt: effectiveCancellationPrompt,
                    projectURL: URL(fileURLWithPath: projectPath, isDirectory: true),
                    attachmentURLs: [],
                    modelID: acceptanceProvider?.model,
                    endpoint: acceptanceProvider?.baseURL.absoluteString,
                    apiKey: acceptanceProvider?.apiKey,
                    reasoningConfiguration: acceptanceProvider?.reasoning,
                    reasoningTier: "balanced",
                    modelParameters: acceptanceProvider?.parameters,
                    modelExecutionCapabilities: acceptanceProvider?.executionCapabilities,
                    wireProtocol: acceptanceProvider?.wireProtocol,
                    executionBackend: selectedExecutionBackend
                )
                var cancellationFailure = ""
                for try await event in await MacAgentHostRuntime.shared.run(cancelRequest) {
                    if event.kind == .failed { cancellationFailure = event.text }
                    if event.assistantJournalCursor != nil || event.assistantRunTerminalSealDigest != nil {
                        cancellationAdmitted = true
                    }
                    // Drain through Host cancellation confirmation and the
                    // final Store seal; the Core event alone is not completion.
                }
                guard let terminal = try cancelRequest.applicationPort?.canonicalRunTerminal(
                    conversationID: cancellationID, runID: cancelRequest.submissionID
                ), terminal.result.value.kind == .cancelled else {
                    return "cancellation_missing_canonical_terminal: \(cancellationFailure)"
                }
                return "run.cancelled"
            } catch is CancellationError {
                // Cancellation before admission has no Run to terminate. Do
                // not accept this path if an admitted conversation remains:
                // that would hide a stranded running/cancelling checkpoint.
                do {
                    let conversation = try MacApplicationPort.shared.loadConversation(cancellationID)
                    return conversation == nil
                        ? "cancelled_before_admission"
                        : "cancellation_missing_canonical_terminal"
                } catch {
                    return "cancellation_state_unreadable: \(error.localizedDescription)"
                }
            } catch {
                return error.localizedDescription
            }
        }
        // Exercise cancellation after durable admission, not only cancelling
        // startup before there is any Run to settle.
        let admissionDeadline = Date().addingTimeInterval(15)
        while !cancellationAdmitted && Date() < admissionDeadline {
            try? await Task.sleep(for: .milliseconds(50))
        }
        let cancellationRequestedAt = Date()
        await MacAgentHostRuntime.shared.cancel(conversationID: cancellationID)
        let cancellationDeadline = acceptanceDeadline(
            environment: environment,
            key: "NOLA_MAC_RUNTIME_ACCEPTANCE_CANCELLATION_TIMEOUT_SECONDS",
            defaultSeconds: 5,
            scenario: "cancellation_drain",
            conversationID: cancellationID,
            outputPath: outputPath
        )
        let cancellationOutcome = await cancellationTask.value
        cancellationDeadline.cancel()
        let cancellationDuration = Date().timeIntervalSince(cancellationRequestedAt)
        cancellationSuccess = cancellationAdmitted && cancellationDuration < 5
            && cancellationOutcome == "run.cancelled"
        cancellationResult = [
            "matched": cancellationSuccess,
            "duration_seconds": cancellationDuration,
            "admitted": cancellationAdmitted,
            "admission_wait_seconds": cancellationRequestedAt.timeIntervalSince(cancellationStartedAt),
            "outcome": cancellationOutcome,
        ]

        if let fixturePath = environment["NOLA_MAC_RUNTIME_ACCEPTANCE_FIXTURES"], !fixturePath.isEmpty {
            let fixtures = [
                ("plain.txt", "Nola plain attachment"),
                ("report.docx", "Nola DOCX attachment"),
                ("report.pdf", "Nola PDF attachment"),
            ]
            for (name, expectedText) in fixtures {
                do {
                    let extraction = try await MacAttachmentPipeline.shared.extract(
                        URL(fileURLWithPath: fixturePath, isDirectory: true).appendingPathComponent(name)
                    )
                    let matched = extraction.text.contains(expectedText)
                    attachmentResults.append([
                        "name": name,
                        "method": extraction.method,
                        "text": extraction.text,
                        "matched": matched,
                    ])
                    attachmentSuccess = attachmentSuccess && matched
                } catch {
                    attachmentResults.append([
                        "name": name,
                        "error": error.localizedDescription,
                        "matched": false,
                    ])
                    attachmentSuccess = false
                }
            }
        }

        var backupRoundTripMatched = false
        do {
            let conversationID = "mac-backup-acceptance"
            let fixtureArchives = [
                MacBackupConversation(MacCanonicalConversation(
                        id: conversationID,
                        title: "Backup acceptance",
                        revision: 1,
                        createdAtMs: 1,
                        updatedAtMs: 2,
                        settingsJSON: nil,
                        messages: [
                            ("user", "Persist this message"),
                            ("assistant", "Message persisted"),
                        ].enumerated().map { index, entry in
                            MacCanonicalConversationMessage(
                                id: "\(conversationID):\(index)",
                                runID: nil,
                                role: entry.0,
                                status: "completed",
                                ordinal: Int64(index + 1),
                                createdAtMs: Int64(index + 1),
                                completedAtMs: Int64(index + 1),
                                parts: [MacCanonicalConversationPart(
                                    id: "\(conversationID):\(index):text",
                                    kind: "text",
                                    ordinal: 1,
                                    state: "completed",
                                    contentText: entry.1,
                                    contentJSON: nil,
                                    createdAtMs: Int64(index + 1), updatedAtMs: Int64(index + 1)
                                )]
                            )
                        }
                ))
            ]
            let backupValue = MacDataBackup(
                format: MacDataBackupCodec.format,
                version: MacDataBackupCodec.version,
                exportedAt: .now,
                conversations: fixtureArchives,
                activity: [],
                automations: [],
                settings: [],
                productSettings: [
                    MacCanonicalProductSetting(
                        key: MacProductSettingKeys.contextLimit,
                        valueJSON: #"{"schemaVersion":1,"value":424242}"#,
                        revision: 1,
                        updatedAtMs: 1
                    )
                ],
                webSources: [],
                notifications: []
            )
            let backup = try MacDataBackupCodec.encode(backupValue)
            let backupURL = URL(fileURLWithPath: outputPath + ".backup.json")
            try backup.write(to: backupURL, options: .atomic)
            let restoredBackup = try MacDataBackupCodec.decode(Data(contentsOf: backupURL))
            let restored = restoredBackup.conversations.first {
                $0.canonical.id == conversationID
            }
            let restoredContextLimit = restoredBackup.productSettings.first {
                $0.key == MacProductSettingKeys.contextLimit
            }?.valueJSON
            backupRoundTripMatched = restoredBackup.format == MacDataBackupCodec.format
                && restored?.canonical == fixtureArchives.first?.canonical
                && restored?.canonical.messages.flatMap(\.parts).map(\.contentText) == ["Persist this message", "Message persisted"]
                && restoredContextLimit == #"{"schemaVersion":1,"value":424242}"#
            backupResult = [
                "bytes": backup.count,
                "round_trip_matched": backupRoundTripMatched,
                "conversation_count": restoredBackup.conversations.count,
                "message_count": restored?.canonical.messages.count ?? 0,
                "context_limit": Int(restoredContextLimit ?? "") ?? 0,
                "error": "",
            ]
        } catch {
            backupResult = [
                "round_trip_matched": false,
                "error": error.localizedDescription,
            ]
        }
        let originalStateRestored = true
        backupSuccess = backupRoundTripMatched
        backupResult["matched"] = backupSuccess
        backupResult["original_state_restored"] = originalStateRestored

        if let webSourceURL, !webSourceURL.isEmpty {
            let sharedStore = MacWebSourceStore.shared
            let originalSharedSources = sharedStore.sources
            let acceptanceRoot = URL(fileURLWithPath: outputPath)
                .deletingLastPathComponent()
                .appendingPathComponent("web-source-acceptance", isDirectory: true)
            let registry = MacDataSourceRegistry(
                storeURL: acceptanceRoot.appendingPathComponent("registry.json")
            )
            let store = MacWebSourceStore(
                storeURL: acceptanceRoot.appendingPathComponent("sources.json"),
                registry: registry
            )
            let source = MacWebSource(
                id: UUID(),
                name: "Runtime acceptance source",
                url: webSourceURL,
                format: .automatic,
                isEnabled: true,
                cachedText: "",
                lastFetchedAt: nil,
                lastError: nil,
                createdAt: .now
            )
            do {
                defer {
                    sharedStore.replaceAll(originalSharedSources)
                    try? FileManager.default.removeItem(at: acceptanceRoot)
                }
                try FileManager.default.createDirectory(
                    at: acceptanceRoot,
                    withIntermediateDirectories: true
                )
                try await store.validateAndUpsert(source)
                let injectedPrompt = await store.promptWithSources(
                    "Summarize the verified source.",
                    conversationID: "runtime-source-acceptance"
                )
                if let validated = store.sources.first(where: { $0.id == source.id }) {
                    store.replaceAll([
                        MacWebSource(
                            id: validated.id,
                            name: validated.name,
                            url: validated.url,
                            format: validated.format,
                            isEnabled: validated.isEnabled,
                            cachedText: validated.cachedText,
                            lastFetchedAt: Date().addingTimeInterval(-7_200),
                            lastError: nil,
                            createdAt: validated.createdAt,
                            scheduleFrequency: .once,
                            scheduledDate: Date().addingTimeInterval(-60)
                        ),
                    ])
                }
                await store.refreshDueSources(at: .now)
                if let refreshed = store.sources.first(where: { $0.id == source.id }) {
                    let contract = registry.contract(for: source.dataSourceID)
                    let contractMatched = contract?.isUsable == true
                        && contract?.connectorID == "mac.web.http"
                        && contract?.recordCount == 2
                        && contract?.schemaFields.contains("JSON") == true
                        && contract?.redactedSample.contains("web source refreshed") == false
                    let usageMatched = registry.usageRecords.contains {
                        $0.sourceID == source.dataSourceID
                            && $0.conversationID == "runtime-source-acceptance"
                    }
                    webSourceSuccess = refreshed.cachedText.contains("web source refreshed")
                        && refreshed.lastError == nil
                        && refreshed.lastFetchedAt != nil
                        && refreshed.scheduleFrequency == nil
                        && refreshed.runs?.first?.succeeded == true
                        && injectedPrompt.contains("web source refreshed")
                        && contractMatched
                        && usageMatched
                    webSourceResult = [
                        "matched": webSourceSuccess,
                        "cached_text": refreshed.cachedText,
                        "first_read_validated": contract?.isUsable == true,
                        "connector": contract?.connectorID ?? "",
                        "record_count": contract?.recordCount ?? 0,
                        "schema_fields": contract?.schemaFields ?? [],
                        "sample_redacted": contract?.redactedSample.contains("web source refreshed") == false,
                        "conversation_injected": injectedPrompt.contains("web source refreshed"),
                        "usage_recorded": usageMatched,
                        "schedule_cleared": refreshed.scheduleFrequency == nil,
                        "run_count": refreshed.runs?.count ?? 0,
                        "last_run_succeeded": refreshed.runs?.first?.succeeded ?? false,
                        "error": refreshed.lastError ?? "",
                    ]
                    if webSourceSuccess, automationChainRequested {
                        let acceptance = await runAutomationChainAcceptance(
                            environment: environment,
                            source: refreshed
                        )
                        automationChainSuccess = acceptance.success
                        automationChainResult = acceptance.result
                    }
                } else {
                    webSourceResult = ["matched": false, "error": "Validated source disappeared"]
                }
            } catch {
                webSourceResult = ["matched": false, "error": error.localizedDescription]
            }
        }

        let visibleToolCount = presentation.traceItems.filter { $0.kind == .tool }.count
        let visibleThinkingCount = presentation.traceItems.filter { $0.kind == .thinking }.count
        let kinds = Set(events.compactMap { $0["kind"] as? String })
        let eventFlowSuccess = bodyMutationCount > 0
            && (!toolEventRequired || (
                kinds.contains("toolFinished") && visibleToolCount > 0
            ))
            && (!usageEventRequired || usageSnapshotCount > 0)
            && (!thinkingEventRequired || visibleThinkingCount > 0)
            && kinds.contains("finished")
        let visualObservationRequired =
            environment["NOLA_MAC_RUNTIME_ACCEPTANCE_REQUIRE_VISUAL_OBSERVATION"] == "1"
        let acceptanceGates: [String: Bool] = [
            "error_free": errorMessage == nil,
            "event_flow": eventFlowSuccess,
            "canonical_projection": canonicalProjectionLoaded,
            "stream_matches_canonical_projection": canonicalProjectionMatchesStream,
            "attachments": attachmentSuccess,
            "backup": backupSuccess,
            "cancellation": cancellationSuccess,
            "web_source": webSourceSuccess,
            "automation_chain": automationChainSuccess,
            "follow_up": followUpSuccess,
            "permission_fixture": permissionFixtureSuccess,
            "visual_observation": !visualObservationRequired
                || (visualObservation["passed"] as? Bool == true),
        ]
        let success = acceptanceGates.values.allSatisfy { $0 }
        let markdownNodeCount = committedStreamNodes.reduce(into: 0) { count, node in
            if case .markdownBlock = node.kind { count += 1 }
        }
        let codeNodeCount = committedStreamNodes.reduce(into: 0) { count, node in
            if case .code = node.kind { count += 1 }
        }
        let internalControlToolVisible = presentation.traceItems.contains {
            $0.title.localizedCaseInsensitiveContains("set_thinking_mode")
        }
        let renderProbe: [String: Any] = [
            "rendered_table_count": presentation.renderedTextNodes.reduce(into: 0) { count, node in
                guard node.kind == .markdownBlock else { return }
                count += MacMarkdownTextParser.parse(node.text).filter {
                    if case .table = $0 { return true }
                    return false
                }.count
            },
            "raw_text": streamAssembler.rawText,
            "canonical_text": canonicalAssistantText,
            "assistant_delta_count": assistantDeltaCount,
            "usage_snapshot_count": usageSnapshotCount,
            "core_text_projection_count": coreTextProjectionCount,
            "body_mutation_count": bodyMutationCount,
            "committed_node_count": committedStreamNodes.count,
            "markdown_node_count": markdownNodeCount,
            "code_node_count": codeNodeCount,
            "stable_committed_prefix": committedPrefixIsStable,
            "mutable_tail_update_count": mutableTailUpdates,
            "mutable_tail_identity_count": mutableTailIDs.count,
            "visible_trace_count": presentation.traceItems.count,
            "visible_tool_count": visibleToolCount,
            "visible_thinking_count": visibleThinkingCount,
            "internal_control_tool_visible": internalControlToolVisible,
            "rendered_text_node_count": presentation.renderedTextNodes.count,
            "screenshot": screenshotResult,
            "visual_observation": visualObservation,
        ]
        let payload: [String: Any] = [
            "success": success,
            "conversation_id": request.conversationID,
            "run_id": request.submissionID,
            "assistant_message_id": request.assistantMessageID,
            "execution_backend": selectedExecutionBackend,
            "external_session_id": externalSessionID ?? "",
            "error": errorMessage ?? "",
            "acceptance_gates": acceptanceGates,
            "permission_cards_presented": answeredPermissionIDs.count,
            "permission_audits": permissionAudits,
            "permission_fixture": permissionFixtureResult,
            "events": events,
            "render_probe": renderProbe,
            "attachments": attachmentResults,
            "backup": backupResult,
            "cancellation": cancellationResult,
            "web_source": webSourceResult,
            "automation_chain": automationChainResult,
            "follow_up": followUpResult,
        ]
        if let data = try? JSONSerialization.data(withJSONObject: payload, options: [.prettyPrinted, .sortedKeys]) {
            try? data.write(to: URL(fileURLWithPath: outputPath), options: .atomic)
        }
        NSApplication.shared.terminate(nil)
    }

    private static func appendJSONLine(_ value: [String: Any], to path: String) {
        guard JSONSerialization.isValidJSONObject(value),
              var data = try? JSONSerialization.data(withJSONObject: value, options: [.sortedKeys]) else {
            return
        }
        data.append(0x0A)
        let url = URL(fileURLWithPath: path)
        if !FileManager.default.fileExists(atPath: path) {
            FileManager.default.createFile(atPath: path, contents: data)
            return
        }
        guard let handle = try? FileHandle(forWritingTo: url) else { return }
        defer { try? handle.close() }
        do {
            try handle.seekToEnd()
            try handle.write(contentsOf: data)
        } catch {
            return
        }
    }

    private static func runFollowUpAcceptance(
        conversationID: String,
        prompt: String,
        expectedText: String,
        projectPath: String,
        provider: MacAppleChatConfiguration?,
        allowPermissionRequests: Bool,
        executionBackend: String,
        externalSessionID: String?,
        outputPath: String,
        environment: [String: String]
    ) async -> (success: Bool, result: [String: Any]) {
        let request: MacLocalAgentRunRequest
        do {
            request = try MacLocalAgentRunRequest(
                conversationID: conversationID,
                prompt: prompt,
                projectURL: URL(fileURLWithPath: projectPath, isDirectory: true),
                attachmentURLs: [],
                modelID: provider?.model,
                endpoint: provider?.baseURL.absoluteString,
                apiKey: provider?.apiKey,
                reasoningConfiguration: provider?.reasoning,
                reasoningTier: "balanced",
                modelParameters: provider?.parameters,
                modelExecutionCapabilities: provider?.executionCapabilities,
                wireProtocol: provider?.wireProtocol,
                executionBackend: executionBackend,
                externalSessionID: externalSessionID
            )
        } catch {
            return (false, ["matched": false, "error": error.localizedDescription])
        }
        var assistantText = ""
        var toolCalls = 0
        var finished = false
        var failure = ""
        let deadline = acceptanceDeadline(
            environment: environment,
            key: "NOLA_MAC_RUNTIME_ACCEPTANCE_FOLLOW_UP_TIMEOUT_SECONDS",
            defaultSeconds: 60,
            scenario: "follow_up_agent_turn",
            conversationID: conversationID,
            outputPath: outputPath
        )
        do {
            for try await event in await MacAgentHostRuntime.shared.run(request) {
                if let cursor = event.assistantJournalCursor,
                   cursor.identity.assistant_message_id == request.assistantMessageID {
                    assistantText = try await MacJournalTextRead.markdown(.init(
                        databaseURL: request.databaseURLOverride ?? request.applicationPort?.databaseURL ?? MacApplicationPort.defaultDatabaseURL,
                        cursor: cursor), historicalFallback: "")
                }
                switch event.kind {
                case .toolStarted:
                    toolCalls += 1
                case .permissionRequested:
                    if let permissionRequest = event.permissionRequest {
                        try await MacAgentHostRuntime.shared.resolvePermission(
                            conversationID: conversationID,
                            request: permissionRequest,
                            allow: allowPermissionRequests,
                            scope: .once
                        )
                    }
                case .finished:
                    finished = true
                case .failed:
                    failure = event.detail ?? event.text
                default:
                    break
                }
            }
        } catch {
            failure = error.localizedDescription
        }
        deadline.cancel()
        let normalized = assistantText.trimmingCharacters(in: .whitespacesAndNewlines)
        let matched = failure.isEmpty
            && finished
            && toolCalls == 0
            && (expectedText.isEmpty || normalized.contains(expectedText))
        return (
            matched,
            [
                "matched": matched,
                "assistant_text": assistantText,
                "tool_calls": toolCalls,
                "finished": finished,
                "error": failure,
            ]
        )
    }

    private static func archiveFingerprint(
        _ archives: [MacStoredConversation]
    ) -> String {
        archives.map { archive in
            let messageIdentity = archive.messages.map {
                "\($0.id.uuidString):\($0.role.rawValue):\($0.text.utf8.count)"
            }.joined(separator: ",")
            return [
                archive.conversation.id,
                archive.conversation.title,
                String(archive.conversation.isPinned),
                messageIdentity,
            ].joined(separator: "|")
        }.joined(separator: "\n")
    }

    private static func writeEarlyFailure(_ error: Error, outputPath: String) {
        let payload: [String: Any] = [
            "success": false,
            "error": error.localizedDescription,
            "stage": "prepare_capability_snapshot",
        ]
        guard let data = try? JSONSerialization.data(withJSONObject: payload, options: [.prettyPrinted]) else {
            return
        }
        try? data.write(to: URL(fileURLWithPath: outputPath), options: .atomic)
    }

    private static func acceptanceDeadline(
        environment: [String: String],
        key: String,
        defaultSeconds: Double,
        scenario: String,
        conversationID: String,
        outputPath: String,
        runtime: any MacLocalAgentRuntime = MacAgentHostRuntime.shared
    ) -> Task<Void, Never> {
        let configured = environment[key].flatMap(Double.init)
        let seconds = max(1, configured ?? defaultSeconds)
        return Task { @MainActor in
            try? await Task.sleep(for: .seconds(seconds))
            guard !Task.isCancelled else { return }
            let payload: [String: Any] = [
                "success": false,
                "error": "Acceptance scenario exceeded its wall-clock deadline",
                "stage": scenario,
                "conversation_id": conversationID,
                "timeout_seconds": seconds,
            ]
            if let data = try? JSONSerialization.data(
                withJSONObject: payload,
                options: [.prettyPrinted, .sortedKeys]
            ) {
                try? data.write(to: URL(fileURLWithPath: outputPath), options: .atomic)
            }
            await runtime.cancel(conversationID: conversationID)
        }
    }

    private static func contextPreferenceMatches(_ lhs: Any?, _ rhs: Any?) -> Bool {
        switch (lhs, rhs) {
        case (nil, nil):
            return true
        case let (lhs as NSNumber, rhs as NSNumber):
            return lhs == rhs
        case let (lhs as NSString, rhs as NSString):
            return lhs == rhs
        default:
            return false
        }
    }

    private static func captureAssistantResponseIfRequested(
        environment: [String: String],
        rawText: String,
        presentation: MacAssistantPresentation
    ) -> [String: Any] {
        guard let outputPath = environment["NOLA_MAC_RUNTIME_ACCEPTANCE_SCREENSHOT"],
              !outputPath.isEmpty else {
            return ["captured": false, "skipped": true]
        }

        let message = MacChatMessage(
            role: .assistant,
            text: rawText,
            createdAt: .now,
            presentation: presentation
        )
        let root = MacMessageView(
            message: message,
            transcriptWidth: 780,
            isStreaming: false,
            liveActivities: [],
            isSearchMatch: false,
            onSendTaskGuidance: { _, _ in },
            onRequestTaskCancellation: { _ in },
            showsAnimatedStatusLight: false
        )
        .frame(width: 780, alignment: .leading)
        .padding(24)
        .background(Color(red: 0.105, green: 0.105, blue: 0.115))
        .environment(\.colorScheme, .dark)

        let hosting = NSHostingView(rootView: root)
        hosting.frame = NSRect(x: 0, y: 0, width: 828, height: 900)
        hosting.layoutSubtreeIfNeeded()
        let measuredHeight = hosting.fittingSize.height
        let height = min(max(measuredHeight, 420), 4_000)
        hosting.frame = NSRect(x: 0, y: 0, width: 828, height: height)
        hosting.layoutSubtreeIfNeeded()

        guard let representation = hosting.bitmapImageRepForCachingDisplay(in: hosting.bounds) else {
            return ["captured": false, "error": "Could not allocate bitmap representation"]
        }
        hosting.cacheDisplay(in: hosting.bounds, to: representation)
        guard let data = representation.representation(using: .png, properties: [:]) else {
            return ["captured": false, "error": "Could not encode PNG"]
        }
        do {
            let outputURL = URL(fileURLWithPath: outputPath)
            try FileManager.default.createDirectory(
                at: outputURL.deletingLastPathComponent(),
                withIntermediateDirectories: true
            )
            try data.write(to: outputURL, options: .atomic)
            return [
                "captured": true,
                "artifact_kind": "assistant_component_snapshot",
                "observation_required": true,
                "observed": false,
                "path": outputPath,
                "bytes": data.count,
                "width": Int(hosting.bounds.width),
                "height": Int(hosting.bounds.height),
                "measured_height": measuredHeight,
            ]
        } catch {
            return ["captured": false, "error": error.localizedDescription]
        }
    }

    /// Capturing pixels is not visual validation. A human or image-capable
    /// reviewer must inspect the artifact and write a versioned receipt. This
    /// keeps CI from reporting a generated-but-never-observed PNG as a pass.
    private static func visualObservationResult(
        environment: [String: String],
        screenshot: [String: Any]
    ) -> [String: Any] {
        guard screenshot["captured"] as? Bool == true,
              let artifactPath = screenshot["path"] as? String,
              let artifactKind = screenshot["artifact_kind"] as? String else {
            return MacVisualAcceptanceContract.failure(
                reason: "No visual artifact"
            ).dictionary
        }
        let requiredScope = environment["NOLA_MAC_RUNTIME_ACCEPTANCE_VISUAL_SCOPE"]
            ?? "application_window_sequence"
        guard let receiptPath = environment["NOLA_MAC_RUNTIME_ACCEPTANCE_VISUAL_RECEIPT"],
              !receiptPath.isEmpty
        else {
            return MacVisualAcceptanceContract.failure(
                reason: "Visual artifact requires an observation receipt",
                artifactPath: artifactPath,
                artifactKind: artifactKind,
                requiredScope: requiredScope
            ).dictionary
        }
        return MacVisualAcceptanceContract.validate(
            receiptURL: URL(fileURLWithPath: receiptPath),
            artifactPath: artifactPath,
            artifactKind: artifactKind,
            requiredScope: requiredScope
        ).dictionary
    }

    private static func runAutomationChainAcceptance(
        environment: [String: String],
        source: MacWebSource
    ) async -> (success: Bool, result: [String: Any]) {
        let provider: MacAppleChatConfiguration
        do {
            guard let selected = try acceptanceProvider(environment: environment) else {
                return (false, ["matched": false, "error": "Acceptance provider is missing"])
            }
            provider = selected
        } catch {
            return (false, ["matched": false, "error": error.localizedDescription])
        }
        guard let outputPath = environment[
            "NOLA_MAC_RUNTIME_ACCEPTANCE_AUTOMATION_OUTPUT"
        ], !outputPath.isEmpty else {
            return (false, ["matched": false, "error": "Automation output path is missing"])
        }

        let sourceToken = environment[
            "NOLA_MAC_RUNTIME_ACCEPTANCE_AUTOMATION_SOURCE_TOKEN"
        ] ?? "web source refreshed"
        let fileManager = FileManager.default
        do {
            // Keep the production entry points but isolate their data and model
            // credential authority. Never replace the user's model/automation
            // configuration and then attempt to restore a stale snapshot.
            let fixture = try automationAcceptanceFixture(provider: provider)
            let workspace = fixture.environment.scratchWorkspaceURL
            let requestedName = URL(fileURLWithPath: outputPath).lastPathComponent
            let outputURL = workspace.appendingPathComponent(
                requestedName.isEmpty ? "automation-chain-output.json" : requestedName)
            let provider = fixture.provider
            let runtime = fixture.runtime
            let activityStore = MacActivityStore(baseDirectory:
                fixture.environment.supportRoot.appendingPathComponent("Activity", isDirectory: true))

            let orchestrationRequest = try MacLocalAgentRunRequest(
                conversationID: "mac-autonomous-orchestration-\(UUID().uuidString)",
                prompt: """
                AUTONOMOUS_ORCHESTRATION_E2E
                Create one enabled daily automation scheduled for 09:00.
                Its task must read \(source.url), load the data-analysis Skill, and write a JSON \
                receipt to \(outputURL.path).
                Use the web_sources context with that URL and the analyze-data Skill. \
                Do not request personal data or run-on-data-change triggers.
                The JSON receipt must contain keys status="validated", source="\(sourceToken)", \
                and skill="\(MacBuiltInSkillID.dataAnalysis.resourceDirectory)".
                The automation's final response must include exactly AUTOMATION_CHAIN_COMPLETE.
                """,
                projectURL: workspace,
                attachmentURLs: [],
                modelID: provider.model,
                modelProfileID: provider.profileID, modelProfileRevision: provider.profileRevision,
                endpoint: provider.baseURL.absoluteString,
                apiKey: provider.apiKey,
                modelExecutionCapabilities: provider.executionCapabilities,
                wireProtocol: provider.wireProtocol,
                databaseURLOverride: fixture.environment.databaseURL,
                appEnvironment: fixture.environment,
                applicationPort: fixture.port
            )
            let orchestrationDeadline = acceptanceDeadline(
                environment: environment,
                key: "NOLA_MAC_RUNTIME_ACCEPTANCE_AUTOMATION_TIMEOUT_SECONDS",
                defaultSeconds: 180,
                scenario: "automation_orchestration",
                conversationID: orchestrationRequest.conversationID,
                outputPath: environment["NOLA_MAC_RUNTIME_ACCEPTANCE"] ?? outputPath,
                runtime: runtime
            )
            defer { orchestrationDeadline.cancel() }
            for try await event in await runtime.run(orchestrationRequest) {
                if event.kind == .permissionRequested,
                   let request = event.permissionRequest,
                   request.toolName == MacAgentOrchestrationService.toolName {
                    // Approve the concrete production request only. The Host
                    // dispatcher owns executing it; applying it here would be
                    // a second mutation outside the canonical Run.
                    try await runtime.resolvePermission(
                        conversationID: orchestrationRequest.conversationID,
                        request: request,
                        allow: true
                    )
                }
            }
            orchestrationDeadline.cancel()
            let orchestrationInspection = try fixture.port.inspectRunRecovery(
                conversationID: orchestrationRequest.conversationID,
                expectedRunID: orchestrationRequest.submissionID,
                assistantMessageID: orchestrationRequest.assistantMessageID)
            let orchestrationSealed: Bool
            if case .terminal(let result, _, _) = orchestrationInspection.observation {
                orchestrationSealed = result.value.kind == .actionCompleted
            } else { orchestrationSealed = false }
            let orchestrationProjection = try MacCanonicalConversationRepository(store: fixture.port)
                .conversationProjection(conversationID: orchestrationRequest.conversationID)
            let orchestrationToolFinished = orchestrationProjection.messages.first(where: {
                $0.messageID == orchestrationRequest.assistantMessageID
            })?.parts.contains(where: { part in
                guard case .tool(let tool) = part else { return false }
                return tool.capabilityID == MacAgentOrchestrationService.toolName && tool.state == .completed
            }) == true
            let registry = MacDataSourceRegistry(storeURL: fixture.environment.supportRoot
                .appendingPathComponent("DataSources/source-registry.json"))
            let webSources = MacWebSourceStore(storeURL: fixture.environment.supportRoot
                .appendingPathComponent("WebSources/web-sources.json"), registry: registry)
            let automationStore = try MacAutomationStore(
                synchronizeSystemJobs: false, appEnvironment: fixture.environment,
                authorizationResolver: { MacAppEnvironment.contains($0, in: workspace) },
                activityStore: activityStore, completionNotifier: { _, _ in },
                dataSourceRegistry: registry, webSourceStore: webSources, runtime: runtime)
            guard automationStore.automations.count == 1,
                  let automation = automationStore.automations.first, automation.isEnabled else {
                return (false, [
                    "matched": false,
                    "error": "The Agent Host did not create an automation from natural language.",
                ])
            }
            let scheduledForMs = Int64(Date().timeIntervalSince1970 * 1_000)
            let occurrence = MacAutomationOccurrence(
                    automationID: automation.id,
                    occurrenceKey: "acceptance:\(UUID().uuidString.lowercased())",
                    trigger: "acceptance",
                    scheduledForMs: scheduledForMs
                )
            let runSucceeded = await automationStore.runScheduledAutomations(
                occurrences: [occurrence],
                providerOverride: provider
            )
            guard let executionReceipt = try automationStore.executionReceipt(for: occurrence) else {
                return (false, ["matched": false, "error": "Exact occurrence has no committed terminal seal",
                    "fixture_root": fixture.environment.supportRoot.path,
                    "occurrence_key": occurrence.occurrenceKey])
            }
            let logURL = automationStore.revealLog(automation)
            let output = (try? String(contentsOf: outputURL, encoding: .utf8)) ?? ""
            let artifact = (try? JSONSerialization.jsonObject(with: Data(output.utf8))) as? [String: Any]
            let activity = activityStore.entries.first {
                $0.kind == .automation && $0.sourceID == executionReceipt.notificationID
            }
            let identity = executionReceipt.terminal.identity
            guard identity.conversation_id == executionReceipt.binding.conversationID,
                  identity.run_id == executionReceipt.binding.runID else {
                throw MacApplicationServiceError.invalidResponse
            }
            let projection = try MacCanonicalConversationRepository(store: fixture.port)
                .conversationProjection(conversationID: identity.conversation_id)
            guard let assistant = projection.messages.first(where: {
                $0.messageID == identity.assistant_message_id
            }) else { throw MacRuntimeAcceptanceContractError.missingAssistantMessage(identity.assistant_message_id) }
            let committedText = try MacCanonicalConversationRepository.projectMessage(assistant).text
            let sourceRefreshed = webSources.sources.contains {
                $0.url == source.url && $0.cachedText.contains(sourceToken)
                    && $0.lastFetchedAt != nil && $0.lastError == nil
            }
            let sourceConsumed = (artifact?["source"] as? String) == sourceToken
            let skillLoaded = assistant.parts.contains { part in
                guard case .skill(let skill) = part else { return false }
                return skill.skillID == MacBuiltInSkillID.dataAnalysis.rawValue
                    && skill.state == .completed && skill.failure == nil
            }
            let outputWritten = fileManager.fileExists(atPath: outputURL.path)
                && (artifact?["status"] as? String) == "validated"
                && (artifact?["skill"] as? String) == MacBuiltInSkillID.dataAnalysis.resourceDirectory
            let activityRecorded = activity?.status == .actionCompleted
            let finalMarker = committedText.contains("AUTOMATION_CHAIN_COMPLETE")
            let matched = runSucceeded
                && executionReceipt.terminal.result.value.kind == .actionCompleted
                && sourceRefreshed
                && sourceConsumed
                && skillLoaded
                && outputWritten
                && activityRecorded
                && finalMarker
                && orchestrationToolFinished
                && orchestrationSealed

            return (matched, [
                "matched": matched,
                "run_succeeded": runSucceeded,
                "source_refreshed": sourceRefreshed,
                "source_consumed": sourceConsumed,
                "skill_loaded": skillLoaded,
                "output_written": outputWritten,
                "activity_recorded": activityRecorded,
                "orchestration_tool_finished": orchestrationToolFinished,
                "autonomous_orchestration_complete": orchestrationSealed && orchestrationToolFinished,
                "occurrence_id": executionReceipt.binding.occurrenceID,
                "occurrence_key": occurrence.occurrenceKey,
                "automation_id": automation.id.uuidString.lowercased(),
                "conversation_id": identity.conversation_id,
                "assistant_message_id": identity.assistant_message_id,
                "run_id": identity.run_id,
                "seal_digest": executionReceipt.terminal.seal_digest,
                "canonical_receipt": try JSONSerialization.jsonObject(with: JSONEncoder().encode(executionReceipt)),
                "canonical_database": fixture.environment.databaseURL.path,
                "fixture_root": fixture.environment.supportRoot.path,
                "final_marker": finalMarker ? "AUTOMATION_CHAIN_COMPLETE" : "",
                "output_path": outputURL.path,
                "log_path": logURL?.path ?? "",
            ])
        } catch {
            return (false, [
                "matched": false,
                "error": error.localizedDescription,
            ])
        }
    }

    private struct AutomationAcceptanceFixture {
        let environment: MacAppEnvironment
        let port: MacApplicationPort
        let provider: MacAppleChatConfiguration
        let runtime: MacAgentHostRuntime
    }

    /// An actual canonical profile plus an in-memory credential reader. The
    /// normal Host credential gate still validates route/protocol/secret before
    /// launch; no fake profile ID, key on disk, or production Keychain write.
    private static func automationAcceptanceFixture(
        provider original: MacAppleChatConfiguration
    ) throws -> AutomationAcceptanceFixture {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(
            "nola-mac-automation-acceptance-\(UUID().uuidString.lowercased())", isDirectory: true)
        try FileManager.default.createDirectory(at: root, withIntermediateDirectories: false,
                                               attributes: [.posixPermissions: 0o700])
        let appEnvironment = MacAppEnvironment(applicationSupportRoot: root,
            cachesRoot: root.appendingPathComponent("Caches", isDirectory: true))
        let port = try MacApplicationPort(databaseURL: appEnvironment.databaseURL)
        let profileID = "acceptance-model"
        let account = "acceptance-model-credential"
        let source: [String: Any] = [
            "schemaVersion": 2, "endpointID": "acceptance-endpoint",
            "endpointName": "Isolated acceptance", "baseURL": original.baseURL.absoluteString,
            "endpointEnabled": true, "endpointOrder": 0, "modelEnabled": true, "modelOrder": 0,
            "priority": 100, "capabilities": ["chat"],
            "wireProtocol": original.wireProtocol.rawValue, "credentialAccount": account,
            "executionCapabilities": try JSONSerialization.jsonObject(with: JSONEncoder().encode(original.executionCapabilities)),
        ]
        let now = Int64(Date().timeIntervalSince1970 * 1_000)
        try port.upsertModelProfile(MacCanonicalModelProfile(id: profileID,
            displayName: "Isolated acceptance", provider: "openai-compatible", modelName: original.model,
            runtime: "remote", sourceJSON: String(decoding: try JSONSerialization.data(withJSONObject: source), as: UTF8.self),
            parametersJSON: String(decoding: try JSONEncoder().encode(original.parameters), as: UTF8.self),
            selected: false, enabled: true, revision: 0, createdAtMs: now, updatedAtMs: now, deletedAtMs: nil))
        let secret = original.apiKey
        let credentials = MacAppleChatConfigurationStore(applicationPort: port,
            credentialReader: { requestedAccount in requestedAccount == account ? secret : nil })
        var provider = MacAppleChatConfiguration(profileID: profileID, endpointID: "acceptance-endpoint",
            endpointName: "Isolated acceptance", baseURL: original.baseURL, apiKey: secret,
            model: original.model, contextWindowTokens: original.contextWindowTokens,
            capabilities: original.capabilities)
        provider.profileRevision = try port.listModelProfiles().first(where: { $0.id == profileID })?.revision
        provider.wireProtocol = original.wireProtocol
        provider.parameters = original.parameters
        provider.reasoning = original.reasoning
        provider.executionCapabilities = original.executionCapabilities
        // Retain isolated Store/log/artifact evidence for final inspection.
        return .init(environment: appEnvironment, port: port, provider: provider,
                     runtime: MacAgentHostRuntime(credentialStore: credentials))
    }

    private static func acceptanceProvider(
        environment: [String: String]
    ) throws -> MacAppleChatConfiguration? {
        if environment["NOLA_MAC_RUNTIME_ACCEPTANCE_ENDPOINT"] != nil
            || environment["NOLA_MAC_RUNTIME_ACCEPTANCE_MODEL"] != nil
            || environment["NOLA_MAC_RUNTIME_ACCEPTANCE_MODEL_FIXTURE_JSON"] != nil {
            return try MacRuntimeAcceptanceModelFixture.provider(environment: environment)
        }
        return MacAppleChatConfigurationStore.shared.load()
    }

    private static func kindName(_ kind: MacLocalAgentEvent.Kind) -> String {
        switch kind {
        case .toolStarted: return "toolStarted"
        case .toolFinished: return "toolFinished"
        case .planUpdated: return "planUpdated"
        case .permissionRequested: return "permissionRequested"
        case .userInputRequested: return "userInputRequested"
        case .status: return "status"
        case .finished: return "finished"
        case .failed: return "failed"
        case .cancelled: return "cancelled"
        }
    }
}

/// DEBUG-only, read-only evidence from an owned existing temporary fixture.
/// This does not create permissions or replace production Core validation.
nonisolated struct MacRuntimeAcceptancePermissionFixture {
    let url: URL
    let relativePath: String
    let originalBytes: Data
    let originalSHA256: String

    init(projectURL: URL, relativePath: String) throws {
        let root = projectURL.standardizedFileURL.resolvingSymlinksInPath()
        let temporaryRoots = [URL(fileURLWithPath: "/tmp", isDirectory: true), FileManager.default.temporaryDirectory]
            .map { $0.standardizedFileURL.resolvingSymlinksInPath().path + "/" }
        guard temporaryRoots.contains(where: { root.path.hasPrefix($0) }),
              root.lastPathComponent.hasPrefix("nola-mac-"),
              !relativePath.isEmpty, !relativePath.hasPrefix("/"),
              !relativePath.split(separator: "/").contains("..") else {
            throw MacRuntimeAcceptanceContractError.invalidPermissionFixture
        }
        let requested = root.appendingPathComponent(relativePath).standardizedFileURL
        let target = requested.resolvingSymlinksInPath()
        let values = try requested.resourceValues(forKeys: [.isRegularFileKey, .isSymbolicLinkKey, .fileSizeKey])
        guard target.path == requested.path, target.path.hasPrefix(root.path + "/"),
              values.isRegularFile == true, values.isSymbolicLink != true,
              let size = values.fileSize, size > 0, size <= 65_536 else {
            throw MacRuntimeAcceptanceContractError.invalidPermissionFixture
        }
        self.url = target
        self.relativePath = relativePath
        let bytes = try Data(contentsOf: target)
        self.originalBytes = bytes
        self.originalSHA256 = Self.sha256(bytes)
    }

    static func sha256(_ bytes: Data) -> String {
        SHA256.hash(data: bytes).map { String(format: "%02x", $0) }.joined()
    }

    func audit(pending: MacCorePendingPermissionIdentity, challenge: MacRemotePermissionRequest,
               runID: String, currentBytes: Data) throws -> [String: Any] {
        guard pending.requestID == challenge.id, pending.callID == challenge.callID,
              pending.capabilityID == challenge.toolName, pending.turnID == runID,
              currentBytes == originalBytes,
              let encoded = pending.resource.data(using: .utf8),
              let subject = try JSONSerialization.jsonObject(with: encoded) as? [String: Any],
              subject["capability"] as? String == pending.capabilityID,
              subject["operation"] as? String == "filesystem",
              let resources = subject["resources"] as? [[String: Any]],
              resources.contains(where: { resource in
                  guard resource["kind"] as? String == "file", resource["access"] as? String == "write",
                        let components = resource["components"] as? [String] else { return false }
                  return "/" + components.joined(separator: "/") == url.path
              }) else { throw MacRuntimeAcceptanceContractError.invalidPermissionFixture }
        return [
            "request_id": pending.requestID, "call_id": pending.callID,
            "capability_id": pending.capabilityID, "run_id": runID,
            "resource": pending.resource, "canonical_pending_verified": true,
            "relative_path": relativePath, "fixture_sha256": Self.sha256(currentBytes),
            "fixture_bytes_base64": currentBytes.base64EncodedString(),
            "original_sha256": originalSHA256, "fixture_unchanged": true,
        ]
    }
}

nonisolated enum MacRuntimeAcceptanceContractError: LocalizedError, Equatable, Sendable {
    case missingAssistantMessage(String)
    case invalidModelFixture
    case invalidPermissionFixture

    var errorDescription: String? {
        switch self {
        case let .missingAssistantMessage(messageID):
            return "Canonical conversation projection is missing assistant message \(messageID)."
        case .invalidModelFixture:
            return "Real-test model route requires a valid explicit execution capability fixture."
        case .invalidPermissionFixture:
            return "Permission acceptance requires the exact canonical pending overwrite and unchanged owned temporary fixture. No permission decision was fabricated."
        }
    }
}

nonisolated enum MacRuntimeAcceptanceModelFixture {
    static func provider(environment: [String: String]) throws -> MacAppleChatConfiguration {
        guard let endpoint = environment["NOLA_MAC_RUNTIME_ACCEPTANCE_ENDPOINT"],
              let baseURL = URL(string: endpoint),
              let scheme = baseURL.scheme, ["http", "https"].contains(scheme),
              baseURL.host != nil,
              let model = environment["NOLA_MAC_RUNTIME_ACCEPTANCE_MODEL"], !model.isEmpty,
              let raw = environment["NOLA_MAC_RUNTIME_ACCEPTANCE_MODEL_FIXTURE_JSON"],
              let bytes = raw.data(using: .utf8),
              let value = try? JSONSerialization.jsonObject(with: bytes) as? [String: Any],
              Set(value.keys) == ["model", "wireProtocol", "executionCapabilities"],
              value["model"] as? String == model,
              let protocolValue = value["wireProtocol"] as? String,
              let wireProtocol = MacModelWireProtocol(rawValue: protocolValue),
              let execution = value["executionCapabilities"] as? [String: Any] else {
            throw MacRuntimeAcceptanceContractError.invalidModelFixture
        }
        let flags: Set<String> = ["toolCalls", "imageInput", "reasoning", "samplingParameters", "samplingRequiresNoReasoning"]
        guard flags.isSubset(of: Set(execution.keys)),
              Set(execution.keys).subtracting(flags).isSubset(of: ["chatOutputTokenField"]),
              flags.allSatisfy({ key in
                  guard let value = execution[key] as? NSNumber else { return false }
                  return CFGetTypeID(value) == CFBooleanGetTypeID()
              }),
              let encoded = try? JSONSerialization.data(withJSONObject: execution),
              let capabilities = try? JSONDecoder().decode(MacModelExecutionCapabilities.self, from: encoded),
              !capabilities.samplingRequiresNoReasoning || (capabilities.samplingParameters && capabilities.reasoning),
              environment["NOLA_MAC_RUNTIME_ACCEPTANCE_REQUIRE_TOOL"] != "1" || capabilities.toolCalls,
              environment["NOLA_MAC_RUNTIME_ACCEPTANCE_VISION_INPUT"] != "1" || capabilities.imageInput,
              environment["NOLA_MAC_RUNTIME_ACCEPTANCE_REQUIRE_THINKING"] != "1" || capabilities.reasoning else {
            throw MacRuntimeAcceptanceContractError.invalidModelFixture
        }
        var configuration = MacAppleChatConfiguration(baseURL: baseURL,
            apiKey: environment["NOLA_MAC_RUNTIME_ACCEPTANCE_API_KEY"] ?? "", model: model)
        configuration.wireProtocol = wireProtocol
        configuration.executionCapabilities = capabilities
        if let rawReasoning = environment["NOLA_MAC_RUNTIME_ACCEPTANCE_REASONING_JSON"], !rawReasoning.isEmpty {
            guard let data = rawReasoning.data(using: .utf8),
                  var reasoning = try? JSONDecoder().decode(MacReasoningConfiguration.self, from: data),
                  reasoning.isValid(for: wireProtocol),
                  ModelParameterLimits.outputTokens.contains(reasoning.outputLimit),
                  reasoning.control == .automatic || capabilities.reasoning else {
                throw MacRuntimeAcceptanceContractError.invalidModelFixture
            }
            reasoning.responses = wireProtocol.usesResponsesTransport
            configuration.reasoning = reasoning
        }
        return configuration
    }
}
#endif
