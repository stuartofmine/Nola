// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Implements ProcessingAgent logic in the iOS service layer.
// - Interfaces:
//   - ProcessingAgent
// - APIs:
//   - none
//
//
//
//  ProcessingAgent.swift
//  Nola
//
//  Agent that takes preprocessed events, calls the LLM for judgment/formatting,
//  decides whether to persist a daily summary and/or push notifications, and
//  executes the chosen actions using existing services.
//

import Foundation

@MainActor
public final class ProcessingAgent {
    private let invocationOwnerID = "local-account"
    public init() {}

    private func appendURLHistoryIfPossible(
        container: ServiceContainer,
        event: ProcessedEvent,
        dedupeKey: String,
        notificationText: String
    ) {
        guard event.source == .url else { return }
        guard let fetcher = container.urlFetcher else { return }

        let sid: String = {
            if !event.sourceId.isEmpty { return event.sourceId }
            if let v = event.metadata["sourceId"], !v.isEmpty { return v }
            return ""
        }()

        guard let source = fetcher.sources.first(where: { $0.id.uuidString == sid }) else {
            container.logger.log(.warn, "ProcessingAgent", NSLocalizedString("autogen.ProcessingAgent.c0d24c8a", comment: "AUTOGEN"), metadata: [
                "dedupeKey": dedupeKey,
                "eventId": event.id,
                "sourceId": sid
            ])
            return
        }

        let title = String(notificationText.prefix(240))
        let details: String? = {
            if let n = event.notes?.trimmingCharacters(in: .whitespacesAndNewlines), !n.isEmpty {
                return String(n.prefix(800))
            }
            let t = event.title.trimmingCharacters(in: .whitespacesAndNewlines)
            return t.isEmpty ? nil : String(t.prefix(800))
        }()

        let entry = URLDataSource.HistoryEntry(
            dedupeKey: dedupeKey,
            title: title,
            details: details,
            eventId: event.id,
            priority: event.priority
        )
        fetcher.appendHistoryEntry(forURL: source.url, entry: entry)
    }

    /// Process a batch of processed events: call LLM, decide store/push, execute actions.
    /// - Parameters:
    ///   - events: processed events (already preprocessed and scored)
    ///   - container: service container providing the model, storage, and notification services.
    ///   - timeout: optional LLM timeout in seconds
    public func process(_ events: [ProcessedEvent], container: ServiceContainer, timeout: TimeInterval? = 30.0) async {
        guard !events.isEmpty else {
            container.logger.log(.debug, "ProcessingAgent", "No events to process")
            return
        }

        // Build model input
        let iso = ISO8601DateFormatter()
        let now = Date()
        let start = Calendar.current.date(byAdding: .day, value: -7, to: now) ?? now
        let end = Calendar.current.date(byAdding: .day, value: 7, to: now) ?? now
        let dateRange = ModelInput.DateRange(start: iso.string(from: start), end: iso.string(from: end))

        let candidates: [ModelInput.Candidate] = events.map { ev in
            ModelInput.Candidate(
                type: ev.source.rawValue,
                title: ev.title,
                name: nil,
                date: ev.date.map { iso.string(from: $0) },
                start: ev.start,
                due: ev.due,
                relation: ev.metadata["relation"],
                notesTag: ev.tags?.first,
                durationMin: nil
            )
        }

        // Build user preferences including per-source prompts (if URL sources provided a userPrompt)
        var userPrefs: [String: String] = [:]
        if let fetcher = container.urlFetcher {
            var collected: [String] = []
            for src in fetcher.sources {
                if let p = src.userPrompt, !p.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
                    // key by source id and source url to make prompts discoverable by provider
                    userPrefs["prompt_for_source:\(src.id.uuidString)"] = p
                    userPrefs["prompt_for_url:\(src.url)"] = p
                    collected.append("Source: \(src.name) (\(src.url))\n\(p)")
                }
            }
            if !collected.isEmpty {
                userPrefs["combined_prompts"] = collected.joined(separator: "\n\n---\n\n")
            }
        }

        let input = ModelInput(dateRange: dateRange, candidates: candidates, userPrefs: userPrefs)

        container.logger.log(.info, "ProcessingAgent", "Calling LLM for summary (candidates: \(candidates.count))")

        do {
            let client = container.llmClient
            let invocation = (client as? ModelService)?.summaryInvocation(ownerID: invocationOwnerID, scenario: .eventProcessing)
                ?? LLMSummaryInvocation(ownerID: invocationOwnerID, provider: client, scenario: .eventProcessing)
            container.logger.log(.debug, "ProcessingAgent", "Summary invocation admitted", metadata: [
                "owner": invocation.ownerID, "operation": invocation.operationID.uuidString,
                "scenario": invocation.scenario.rawValue, "configurationRevision": String(invocation.configurationRevision)
            ])
            let output = try await invocation.generate(input: input, timeout: timeout)

            guard isValid(output: output) else {
                container.logger.log(.error, "ProcessingAgent", "LLM output invalid; skipping save and push")
                return
            }
            // Core is the sole owner of event transformation and action decisions.
            let encoder = JSONEncoder()
            encoder.dateEncodingStrategy = .iso8601
            guard let modelData = try? encoder.encode(output) else {
                container.logger.log(.error, "ProcessingAgent", "Failed to encode model output for Rust decision engine")
                return
            }

            guard let transformedEvents = RustInterop.transformModelOutput(
                modelData,
                sourceId: "processing_agent",
                sourceType: "manual"
            ) else {
                container.logger.log(.error, "ProcessingAgent", "Core event transformation unavailable")
                return
            }

            guard let processedData = try? encoder.encode(transformedEvents) else {
                container.logger.log(.error, "ProcessingAgent", "Failed to encode processed events for Rust decision engine")
                return
            }

            guard let decision = RustInterop.decideActions(
                processed: processedData,
                modelOutput: modelData
            ) else {
                container.logger.log(.error, "ProcessingAgent", "Core action decision unavailable")
                return
            }
            container.logger.log(.debug, "ProcessingAgent", "Rust decision: saveSummary=\(decision.saveSummary), candidates=\(decision.candidates.count)")
                container.logger.log(.info, "ProcessingAgent", "Rust decision: saveSummary=\(decision.saveSummary), candidates=\(decision.candidates.count)")
                if decision.saveSummary {
                    let today = DateFormatter.dayFormatter.string(from: Date())
                    let daily = DailySummary(date: today, items: output.items, explain: output.explain, generatedAt: Date(), lastRefresh: Date())
                    do {
                        try container.summaryStorage.saveSummary(daily)
                        container.logger.log(.info, "ProcessingAgent", "Saved daily summary (Rust)", metadata: ["items": "\(output.items.count)"])
                    } catch {
                        container.logger.log(.error, "ProcessingAgent", "Failed to save summary (Rust)", metadata: ["error": error.localizedDescription])
                    }
                }

                // Use candidates to push notifications
                for nc in decision.candidates {
                    if container.rulesEngine.wasRecentlySent(nc.dedupeKey) {
                        container.logger.log(.debug, "ProcessingAgent", "Skip URL/history push due to throttle window (Rust)", metadata: ["key": nc.dedupeKey])
                        continue
                    }

                    let action: ActionType = {
                        switch nc.event.source {
                        case .contact: return .openCalendar
                        case .calendar: return .openCalendar
                        case .reminder: return .openReminder
                        case .health: return .openHealth
                        case .url: return .openCalendar
                        }
                    }()

                    let candidate = NotificationCandidate(event: nc.event, dedupeKey: nc.dedupeKey, text: nc.text, actionType: action)

                    self.appendURLHistoryIfPossible(container: container, event: nc.event, dedupeKey: nc.dedupeKey, notificationText: nc.text)
                        // Persist via NotificationManager when available
                        if let nm = container.notificationManager {
                            do {
                                let ne = try nm.post(candidate: candidate)
                                try? container.notificationStore?.markSent(id: ne.id) // optimistic
                            } catch {
                                container.logger.log(.warn, "ProcessingAgent", "Failed to persist notification candidate", metadata: ["error": error.localizedDescription])
                            }
                        }
                    _ = await container.unifiedNotificationService.scheduleNotification(for: candidate.event, at: nil)
                    container.rulesEngine.markSent(nc.dedupeKey)
                    container.logger.log(.info, "ProcessingAgent", "Pushed notification for item (Rust)", metadata: ["key": nc.dedupeKey, "priority": "\(nc.event.priority)"])
                }

                // If Decision provided a summaryEvent to persist, persist it as a ProcessedEvent
                if let summaryEvent = decision.summaryEvent {
                    do {
                        try container.unifiedStorage?.saveProcessedEvent(summaryEvent)
                        container.logger.log(.info, "ProcessingAgent", "Saved summary event (Rust)", metadata: ["key": summaryEvent.dedupeKey])
                    } catch {
                        container.logger.log(.error, "ProcessingAgent", "Failed to save summary event (Rust)", metadata: ["error": error.localizedDescription])
                    }
                }
            // If LLM returned no items but provided an explain string, create a single summary notification
            if output.items.isEmpty, let explain = output.explain, !explain.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
                container.logger.log(.info, "ProcessingAgent", "LLM returned explain-only summary; creating summary notification")

                // Build a deterministic dedupe key for the day's summary to avoid duplicate pushes
                let day = DateFormatter.dayFormatter.string(from: Date())
                let dedupeKey = "summary:\(day):\(abs(explain.hashValue))"

                // Create a lightweight ProcessedEvent to attach to the notification candidate
                let summaryEvent = ProcessedEvent(
                    id: UUID().uuidString,
                    source: events.first?.source ?? .reminder,
                    sourceType: .manual,
                    sourceId: "processing_agent_summary",
                    title: String(explain.prefix(240)),
                    date: nil,
                    start: nil,
                    due: nil,
                    priority: 0.5,
                    tags: nil,
                    dedupeKey: dedupeKey,
                    provenance: ["explain": .string(explain)],
                    metadata: ["generatedBy": "processing_agent"],
                    notes: nil,
                    hash: explain.hashValue
                )

                let candidate = NotificationCandidate(event: summaryEvent, dedupeKey: dedupeKey, text: explain, actionType: .openCalendar)
                if let nm = container.notificationManager {
                        do {
                            let ne = try nm.post(candidate: candidate)
                            try? container.notificationStore?.markSent(id: ne.id)
                        } catch {
                            container.logger.log(.warn, "ProcessingAgent", "Failed to persist summary candidate", metadata: ["error": error.localizedDescription])
                        }
                    }
                _ = await container.unifiedNotificationService.scheduleNotification(for: candidate.event, at: nil)
                container.rulesEngine.markSent(dedupeKey)
                container.logger.log(.info, "ProcessingAgent", "Pushed summary notification", metadata: ["key": dedupeKey])
            }

        } catch let pending as LLMRemoteAuxiliaryScopeBlocked {
            container.logger.log(.warn, "ProcessingAgent", "Prior auxiliary source still owns the worker slot; no new model request", metadata: [
                "sourceHost": pending.sourceHostID, "operation": pending.operationID, "attempt": pending.attemptID
            ])
        } catch let pending as LLMRemoteAuxiliaryOutcomeUnknown {
            container.logger.log(.warn, "ProcessingAgent", "Auxiliary source outcome remains pending; inspect the original operation without automatic retry", metadata: [
                "sourceHost": pending.sourceHostID, "operation": pending.operationID, "attempt": pending.attemptID
            ])
        } catch {
            container.logger.log(.error, "ProcessingAgent", "LLM call failed", metadata: ["error": error.localizedDescription])
        }
    }

    // Validate ModelOutput shape and values
    private func isValid(output: ModelOutput) -> Bool {
        // Must be decodable into non-empty items
        guard !output.items.isEmpty else { return true /* empty is allowed */ }

        for item in output.items {
            if item.text.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty { return false }
            if item.dedupeKey.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty { return false }
            if item.priority.isNaN || item.priority < 0.0 || item.priority > 1.0 { return false }
        }
        return true
    }
}
