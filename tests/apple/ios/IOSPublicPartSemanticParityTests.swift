// SPDX-License-Identifier: Apache-2.0
import XCTest
import UIKit
import SwiftUI
@testable import Nola

@MainActor
final class IOSPublicPartSemanticParityTests: XCTestCase {
    func testEmptyDateAcceptanceMatchesVisibleConstrainedSelection() {
        let today = Date(timeIntervalSince1970: 1_000)
        let future = Date(timeIntervalSince1970: 2_000)
        let past = Date(timeIntervalSince1970: 500)
        XCTAssertEqual(AppleUIDateInput.emptySelectionDate(now: today, minimum: future, maximum: nil), future)
        XCTAssertEqual(AppleUIDateInput.emptySelectionDate(now: today, minimum: nil, maximum: past), past)
        XCTAssertEqual(AppleUIDateInput.emptySelectionDate(now: today, minimum: past, maximum: future), today)
        XCTAssertEqual(AppleUIDateInput.emptySelectionDate(now: today, minimum: nil, maximum: nil), today)
    }

    func testExplicitDraftRevalidationRebasesWithoutLosingInput() throws {
        let path = try XCTUnwrap(Bundle(for: Self.self).url(forResource: "form_interaction_v0_9_1", withExtension: "json"))
        let fixture = try XCTUnwrap(JSONSerialization.jsonObject(with: Data(contentsOf: path)) as? [String: Any])
        let snapshot = try JSONDecoder().decode(AppleUISnapshot<AgentCoreJSONValue>.self,
            from: JSONSerialization.data(withJSONObject: XCTUnwrap(fixture["edited_snapshot"])))
        let node = try XCTUnwrap(snapshot.nodes.values.first { !$0.bindings.isEmpty })
        let binding = try XCTUnwrap(node.bindings.keys.first)
        let value = try JSONDecoder().decode(AgentCoreJSONValue.self, from: Data(#""retained input""#.utf8))
        let state = AppleUIEditingState<AgentCoreJSONValue>()
        let key = AppleUIEditingState<AgentCoreJSONValue>.key(node.instance_id, binding)
        state.drafts = [key: value]
        state.draftRevision = snapshot.revision - 1
        state.failure = "stale revision"
        state.revalidate(against: snapshot)
        XCTAssertEqual(state.draftRevision, snapshot.revision)
        XCTAssertEqual(state.drafts[key], value)
        XCTAssertEqual(state.draftGeneration, 1)
        XCTAssertNil(state.failure)

        state.drafts["removed field"] = value
        state.draftRevision = snapshot.revision - 1
        state.revalidate(against: snapshot)
        XCTAssertEqual(state.draftRevision, snapshot.revision - 1)
        XCTAssertEqual(state.draftGeneration, 1)
        XCTAssertEqual(state.drafts.count, 2)
        XCTAssertNotNil(state.failure)

        state.drafts = [key: value]
        state.pending = true
        state.revalidate(against: snapshot)
        XCTAssertEqual(state.draftRevision, snapshot.revision - 1)
        XCTAssertEqual(state.draftGeneration, 1)
    }

    func testDraftPreviewRejectsStaleGenerationRevisionAndForeignOwner() throws {
        let path = try XCTUnwrap(Bundle(for: Self.self).url(forResource: "form_interaction_v0_9_1", withExtension: "json"))
        let fixture = try XCTUnwrap(JSONSerialization.jsonObject(with: Data(contentsOf: path)) as? [String: Any])
        let snapshot = try JSONDecoder().decode(AppleUISnapshot<AgentCoreJSONValue>.self,
            from: JSONSerialization.data(withJSONObject: XCTUnwrap(fixture["initial_snapshot"])))
        let preview = AppleUIPreviewResult(identity: snapshot.identity, base_revision: snapshot.revision,
            draft_generation: 7, snapshot: snapshot)
        XCTAssertTrue(preview.matches(identity: snapshot.identity, revision: snapshot.revision, generation: 7))
        XCTAssertFalse(preview.matches(identity: snapshot.identity, revision: snapshot.revision, generation: 8))
        XCTAssertFalse(preview.matches(identity: snapshot.identity, revision: snapshot.revision + 1, generation: 7))
        let other = AppleUISurfaceIdentity(owner: snapshot.identity.owner, surface_id: "other", generation: snapshot.identity.generation)
        XCTAssertFalse(preview.matches(identity: other, revision: snapshot.revision, generation: 7))
        let input = AppleUIPreviewInput<AgentCoreJSONValue>(identity: snapshot.identity,
            expected_surface_revision: snapshot.revision, edits: [], draft_generation: 7)
        let encoded = try XCTUnwrap(JSONSerialization.jsonObject(with: JSONEncoder().encode(AppleUIPreviewCommand(input: input))) as? [String: Any])
        XCTAssertEqual(encoded["type"] as? String, "preview_ui_surface")
        let body = try XCTUnwrap(encoded["input"] as? [String: Any])
        XCTAssertNil(body["action"])
        XCTAssertNil(body["submission_id"])
    }

    func testNumberInputPreservesStringAndRejectsTrailingGarbageAtCommit() {
        let formatter = AppleUINumberFormatter()
        for raw in ["", "-12", "42", "0"] {
            var object: AnyObject?
            var error: NSString?
            XCTAssertTrue(formatter.getObjectValue(&object, for: raw, errorDescription: &error), raw)
            XCTAssertEqual(object as? String, raw)
        }
        for raw in ["12abc", "--1", "hello"] {
            var object: AnyObject?
            var error: NSString?
            XCTAssertFalse(formatter.getObjectValue(&object, for: raw, errorDescription: &error), raw)
        }
        for composing in ["-", "中文", "にほんご", "한국어"] {
            XCTAssertTrue(formatter.isPartialStringValid(composing, newEditingString: nil, errorDescription: nil))
        }
    }

    func testNativeIconsHaveRealSymbolsAndSVGGeometry() throws {
        XCTAssertEqual(AppleUIIconSymbols.names.count, 59)
        for (name, symbol) in AppleUIIconSymbols.names { XCTAssertNotNil(UIImage(systemName: symbol), name) }
        let square = try AppleUISVGPath.parse("M2 2h20v20H2z")
        XCTAssertEqual(square.boundingRect, CGRect(x: 2, y: 2, width: 20, height: 20))
        let arc = try AppleUISVGPath.parse("M2 12 A10 10 0 0122 12 A10 10 0 012 12Z")
        XCTAssertEqual(arc.boundingRect.minX, 2, accuracy: 0.001)
        XCTAssertEqual(arc.boundingRect.maxX, 22, accuracy: 0.001)
        XCTAssertEqual(arc.boundingRect.height, 20, accuracy: 0.001)
        XCTAssertFalse(try AppleUISVGPath.parse("M1e0 2 C2 3 4 5 6 7 s2 3 4 5 Q1 2 3 4 t2 3 L4 5z").isEmpty)
        for invalid in ["", "L1 2", "M0", "MNaN 2", "M0 0 A1 1 0 2 0 2 2", "M0 0X2 2", "M0 0L1e999 2", "M1e308 0l1e308 0"] {
            XCTAssertThrowsError(try AppleUISVGPath.parse(invalid), invalid)
        }
    }

    func testSharedCoreGoldenSurfaceInteractionAndDelta() throws {
        let path = try XCTUnwrap(Bundle(for: Self.self).url(forResource: "form_interaction_v0_9_1", withExtension: "json"))
        let data = try Data(contentsOf: path)
        let fixture = try XCTUnwrap(JSONSerialization.jsonObject(with: data) as? [String: Any])
        XCTAssertEqual(fixture["fixture_version"] as? Int, 1)
        func decode<T: Decodable>(_ key: String, as: T.Type) throws -> T {
            try JSONDecoder().decode(T.self, from: JSONSerialization.data(withJSONObject: XCTUnwrap(fixture[key])))
        }
        let identity = try decode("identity", as: AppleUISurfaceIdentity.self)
        let initial = try decode("initial_snapshot", as: AppleUISnapshot<AgentCoreJSONValue>.self)
        let edited = try decode("edited_snapshot", as: AppleUISnapshot<AgentCoreJSONValue>.self)
        let delta = try decode("changes", as: AppleUIChangeSet<AgentCoreJSONValue>.self)
        let interaction = try decode("interaction", as: AppleUIInteraction<AgentCoreJSONValue>.self)
        let history = try decode("historical_snapshot", as: AppleUISnapshot<AgentCoreJSONValue>.self)
        XCTAssertEqual(initial.identity, identity)
        XCTAssertEqual(interaction.identity, identity)
        XCTAssertEqual(interaction.expected_surface_revision, initial.revision)
        XCTAssertFalse(interaction.edits.isEmpty)
        XCTAssertNotNil(interaction.action)
        XCTAssertEqual(try AppleUISurfaceUpdate.changes(delta).applying(to: initial, identity: identity), edited)
        XCTAssertEqual(history.identity, identity)
        XCTAssertEqual(history.phase, .readOnly)
        XCTAssertTrue(history.nodes.values.allSatisfy { $0.actions.isEmpty && $0.bindings.isEmpty })
        XCTAssertEqual(initial.identity.accessibilityIdentifier, edited.identity.accessibilityIdentifier)
    }

    func testSurfaceAccessibilityIdentityIsCompactAndIncludesEveryOwnerField() throws {
        let raw = #"{"owner":{"authority_id":"store","conversation_id":"conversation","participant_id":"assistant","context_id":"context","originating_run_id":"run","message_id":"message","part_id":"tool-part"},"surface_id":"form","generation":1}"#
        func identity(_ json: String) throws -> String {
            try JSONDecoder().decode(AppleUISurfaceIdentity.self, from: Data(json.utf8)).accessibilityIdentifier
        }
        let baseline = try identity(raw)
        XCTAssertEqual(baseline, try identity(raw))
        XCTAssertEqual(baseline.count, "a2ui.surface.".count + 64)
        for value in ["store", "conversation", "assistant", "context", "run", "message", "tool-part", "form"] {
            XCTAssertNotEqual(baseline, try identity(raw.replacingOccurrences(of: "\"" + value + "\"", with: "\"changed-" + value + "\"")))
        }
        XCTAssertNotEqual(baseline, try identity(raw.replacingOccurrences(of: "\"generation\":1", with: "\"generation\":2")))
    }

    func testQueuedUIActionDisplaysSummaryWithoutRewritingCorePayload() throws {
        let raw = #"{"type":"submission","submission":{"type":"user_input","submission_id":"command","items":[{"type":"ui_action","id":"user","action":{"summary":"Submitted form","envelope":{"name":"private-value"}}}]}}"#
        let decoded = try JSONDecoder().decode(AppleUIQueuedSubmission.self, from: Data(raw.utf8))
        XCTAssertEqual(try decoded.item(commandID: "command").action.summary, "Submitted form")
        XCTAssertEqual(try decoded.item(commandID: "command").id, "user")
        XCTAssertThrowsError(try decoded.item(commandID: "foreign"))
        let old = raw.replacingOccurrences(of: "ui_action", with: "message")
        XCTAssertThrowsError(try JSONDecoder().decode(AppleUIQueuedSubmission.self, from: Data(old.utf8)).item(commandID: "command"))
    }

    func testUIQueueDispatchRequiresOriginalOwnerAndCurrentStateShape() throws {
        let raw = #"{"head":{"command_id":"submission","kind":"user_input","user_message_id":"ui:submission:user","lease_owner":null,"lease_until_ms":null},"dispatch":{"protocol_version":1,"identity":{"source_host_id":"host","client_id":"client","conversation_id":"conversation","submission_id":"submission","user_message_id":"ui:submission:user","assistant_message_id":"assistant"},"request_digest":"digest","phase":"prepared","admission":null,"rejection_code":null}}"#
        func decode(_ value: String) throws -> WebAgentProvider.UIQueueState {
            try JSONDecoder().decode(WebAgentProvider.UIQueueState.self, from: Data(value.utf8))
        }
        try decode(raw).validate(hostID: "host", clientID: "client", conversationID: "conversation")
        XCTAssertThrowsError(try decode(raw).validate(hostID: "changed", clientID: "client", conversationID: "conversation"))
        XCTAssertThrowsError(try decode(raw).validate(hostID: "host", clientID: "changed", conversationID: "conversation"))
        XCTAssertThrowsError(try decode(raw.replacingOccurrences(of: "\"request_digest\":\"digest\"", with: "\"request_digest\":\"\"")).validate(hostID: "host", clientID: "client", conversationID: "conversation"))
        XCTAssertThrowsError(try decode("{}"))
        XCTAssertThrowsError(try decode("{\"command_id\":\"submission\"}"))
        try decode("{\"head\":null,\"dispatch\":null}").validate(hostID: "host", clientID: "client", conversationID: "conversation")
    }

    func testUIActionRetainsEnvelopeAsDataAndDisplaysOnlySummary() throws {
        let payload = #"{"kind":"ui_action","part_id":"action:submission","revision":9,"surface":{"identity":{"owner":{"authority_id":"store","conversation_id":"conversation","participant_id":"assistant","context_id":"context","originating_run_id":"run","message_id":"message","part_id":"tool-part"},"surface_id":"form","generation":1},"revision":3},"envelope":{"secret_form_value":"never-render-this-envelope"},"summary":"已提交表单"}"#
        let part = try JSONDecoder().decode(IOSConversationProjectionV1.Part.self, from: Data(payload.utf8))
        guard case .uiAction(let value) = part else { return XCTFail("Expected typed UI action") }
        XCTAssertEqual(value.partID, "action:submission")
        XCTAssertEqual(value.summary, "已提交表单")
        XCTAssertEqual(value.surface.revision, 3)
        XCTAssertFalse(value.summary.contains("never-render-this-envelope"))
        let messageJSON = "{\"message_id\":\"action-message\",\"role\":\"user\",\"state\":\"completed\",\"revision\":9,\"created_at_ms\":1,\"parts\":[" + payload + "]}"
        let message = try JSONDecoder().decode(IOSConversationProjectionV1.Message.self, from: Data(messageJSON.utf8))
        XCTAssertEqual(IOSConversationProjectionPresenter.message(from: message, activeRun: nil)?.content, "已提交表单")
    }

    func testUISurfacePartPreservesIndependentRevisionsAndToolProvenance() throws {
        let payload = #"{"kind":"ui_surface","part_id":"ui-surface:hash","revision":42,"surface":{"identity":{"owner":{"authority_id":"store","conversation_id":"conversation","participant_id":"assistant","context_id":"context","originating_run_id":"run","message_id":"message","part_id":"tool-part"},"surface_id":"form","generation":1},"revision":3},"summary":"Form"}"#
        let part = try JSONDecoder().decode(IOSConversationProjectionV1.Part.self, from: Data(payload.utf8))
        guard case .uiSurface(let value) = part else { return XCTFail("Expected typed Surface Part") }
        XCTAssertEqual(value.partID, "ui-surface:hash")
        XCTAssertEqual(value.revision, 42)
        XCTAssertEqual(value.surface.revision, 3)
        XCTAssertEqual(value.surface.identity.owner.part_id, "tool-part")
        XCTAssertThrowsError(try JSONDecoder().decode(IOSConversationProjectionV1.Part.self,
            from: Data(payload.replacingOccurrences(of: "\"revision\":3", with: "\"revision\":0").utf8)))
    }

    func testHistoricalSurfaceRequiresCompleteCanonicalSupportedReference() throws {
        let raw = #"{"owner":{},"canonical_revision":42,"schema":"message_part_view","canonical_kind":"ui_surface","binding":"canonical","status":{"state":"supported"},"fields":{"kind":{"availability":"complete","value":"ui_surface"},"part_id":{"availability":"complete","value":"ui-surface:hash"},"revision":{"availability":"complete","value":42},"surface":{"availability":"complete","value":{"identity":{"owner":{"authority_id":"store","conversation_id":"conversation","participant_id":"assistant","context_id":"context","originating_run_id":"run","message_id":"message","part_id":"tool"},"surface_id":"form","generation":1},"revision":3}},"summary":{"availability":"complete","value":"Form"}}}"#
        func decode(_ json: String) throws -> IOSConversationProjectionV1.HistoricalProjection {
            try JSONDecoder().decode(IOSConversationProjectionV1.HistoricalProjection.self, from: Data(json.utf8))
        }
        XCTAssertEqual(try decode(raw).uiSurfaceReference?.revision, 3)
        var incomplete = try XCTUnwrap(JSONSerialization.jsonObject(with: Data(raw.utf8)) as? [String: Any])
        let completeFields = try XCTUnwrap(incomplete["fields"] as? [String: Any])
        for key in ["kind", "part_id", "revision", "surface", "summary"] {
            var missingFields = completeFields
            missingFields.removeValue(forKey: key)
            incomplete["fields"] = missingFields
            let encoded = try JSONSerialization.data(withJSONObject: incomplete)
            XCTAssertNil(try decode(String(decoding: encoded, as: UTF8.self)).uiSurfaceReference)
        }
        for invalid in [
            raw.replacingOccurrences(of: "\"binding\":\"canonical\"", with: "\"binding\":\"untrusted\""),
            raw.replacingOccurrences(of: "\"state\":\"supported\"", with: "\"state\":\"deferred\""),
            raw.replacingOccurrences(of: "\"schema\":\"message_part_view\"", with: "\"schema\":\"agent_part\""),
            raw.replacingOccurrences(of: "\"availability\":\"complete\",\"value\":\"Form\"", with: "\"availability\":\"deferred\",\"locator\":{}"),
        ] {
            XCTAssertNil(try decode(invalid).uiSurfaceReference)
        }
    }

    func testSurfaceDeltaRequiresExactBaseAndRetainsUntouchedNodes() throws {
        let owner = AppleUISurfaceOwner(authority_id: "store", conversation_id: "conversation", participant_id: "agent",
            context_id: "context", originating_run_id: "run", message_id: "message", part_id: "tool")
        let identity = AppleUISurfaceIdentity(owner: owner, surface_id: "form", generation: 1)
        let old = AppleUINode<AgentCoreJSONValue>(instance_id: "old", component_id: "old", component: "Text", props: [:], slots: [:], bindings: [:], actions: [:], validation_errors: [])
        let retained = AppleUINode<AgentCoreJSONValue>(instance_id: "keep", component_id: "keep", component: "Text", props: [:], slots: [:], bindings: [:], actions: [:], validation_errors: [])
        let base = AppleUISnapshot(identity: identity, revision: 2, catalog_id: "catalog", catalog_digest: "digest",
            phase: .ready, root_instance_id: "keep", nodes: ["old": old, "keep": retained], summary: "Old")
        let change = AppleUIChangeSet<AgentCoreJSONValue>(identity: identity, base_revision: 2, revision: 4, catalog_digest: "digest",
            phase: .ready, root_instance_id: "keep", upserted_nodes: [], removed_instance_ids: ["old"], summary: "New")
        let delta = AppleUISurfaceUpdate<AgentCoreJSONValue>.changes(change)
        let result = try XCTUnwrap(delta.applying(to: base, identity: identity))
        XCTAssertEqual(result.revision, 4)
        XCTAssertEqual(result.nodes["keep"], retained)
        XCTAssertNil(result.nodes["old"])
        XCTAssertNotNil(base.nodes["old"])
        XCTAssertThrowsError(try delta.applying(to: nil, identity: identity))
        XCTAssertThrowsError(try delta.applying(to: result, identity: identity))
    }

    func testUIIntentRecoveryPreservesOriginalRequestAndSource() throws {
        var storage: [String: (json: String, revision: Int64)] = [:]
        func journal() -> AppleUIIntentJournal<AgentCoreJSONValue> {
            AppleUIIntentJournal(read: { storage[$0] }, write: { key, value, expected in
                guard storage[key]?.revision == expected else { throw NSError(domain: "CAS", code: 1) }
                storage[key] = (value, (expected ?? 0) + 1)
            })
        }
        let identity = AppleUISurfaceIdentity(owner: AppleUISurfaceOwner(authority_id: "store", conversation_id: "conversation",
            participant_id: "agent", context_id: "context", originating_run_id: "run", message_id: "message", part_id: "tool"),
            surface_id: "form", generation: 1)
        let source = AppleUIIntentSource(hostID: "host", address: "https://host.example", certificate: "certificate", clientID: "client")
        let input = AppleUISubmission<AgentCoreJSONValue>(interaction: AppleUIInteraction(identity: identity, expected_surface_revision: 3,
            edits: [], action: nil), submission_id: "original", submitted_at_ms: 123)
        try journal().stage(input, source: source)
        let recovered = try XCTUnwrap(journal().pending(identity: identity, source: source))
        XCTAssertEqual(recovered.submission_id, "original")
        XCTAssertEqual(recovered.submitted_at_ms, 123)
        XCTAssertEqual(recovered.interaction.expected_surface_revision, 3)
        let replacement = AppleUISubmission(interaction: input.interaction, submission_id: "replacement", submitted_at_ms: 456)
        XCTAssertThrowsError(try journal().stage(replacement, source: source))
        XCTAssertThrowsError(try journal().pending(identity: identity,
            source: AppleUIIntentSource(hostID: "other", address: source.address, certificate: source.certificate, clientID: source.clientID)))
        try journal().retire(input, source: source)
        XCTAssertNil(try journal().pending(identity: identity, source: source))
        try journal().stage(replacement, source: source)
        XCTAssertEqual(try journal().pending(identity: identity, source: source)?.submission_id, "replacement")
    }

    func testUIIntentRestoresFromColdCanonicalSettingsStore() throws {
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent("a2ui-intent-\(UUID())", isDirectory: true)
        defer { try? FileManager.default.removeItem(at: directory) }
        let database = directory.appendingPathComponent("state.db")
        func journal() throws -> AppleUIIntentJournal<AgentCoreJSONValue> {
            let runtime = try IOSApplicationRuntime(databaseURL: database)
            return AppleUIIntentJournal(read: { key in
                try runtime.productSetting(key: key).map { ($0.valueJSON, $0.revision) }
            }, write: { key, value, expected in
                _ = try runtime.putProductSettingChecked(key: key, valueJSON: value, expectedRevision: expected, updatedAtMs: 123)
            })
        }
        let identity = AppleUISurfaceIdentity(owner: AppleUISurfaceOwner(authority_id: "store", conversation_id: "conversation",
            participant_id: "agent", context_id: "context", originating_run_id: "run", message_id: "message", part_id: "tool"),
            surface_id: "form", generation: 1)
        let source = AppleUIIntentSource(hostID: "host", address: "https://host.example", certificate: "cert", clientID: "client")
        let input = AppleUISubmission<AgentCoreJSONValue>(interaction: AppleUIInteraction(identity: identity,
            expected_surface_revision: 3, edits: [.set(instanceID: "field", binding: "value", value: .string("unsent draft"))], action: nil),
            submission_id: "original", submitted_at_ms: 123)
        try journal().stage(input, source: source)
        let cold = try journal()
        let restored = try XCTUnwrap(cold.pending(identity: identity, source: source))
        let encoder = JSONEncoder(); encoder.outputFormatting = .sortedKeys
        XCTAssertEqual(try encoder.encode(restored), try encoder.encode(input))
        try cold.retire(input, source: source)
        XCTAssertNil(try journal().pending(identity: identity, source: source))
    }

    func testUIIntentStorageFailuresNeverLoseOriginalSubmission() throws {
        var storage: [String: (json: String, revision: Int64)] = [:]
        var failWrite = true
        let journal = AppleUIIntentJournal<AgentCoreJSONValue>(read: { storage[$0] }, write: { key, value, expected in
            guard !failWrite, storage[key]?.revision == expected else { throw NSError(domain: "CAS", code: 1) }
            storage[key] = (value, (expected ?? 0) + 1)
        })
        let identity = AppleUISurfaceIdentity(owner: AppleUISurfaceOwner(authority_id: "store", conversation_id: "conversation",
            participant_id: "agent", context_id: "context", originating_run_id: "run", message_id: "message", part_id: "tool"),
            surface_id: "form", generation: 1)
        let source = AppleUIIntentSource(hostID: "host", address: "https://host.example", certificate: "cert", clientID: "client")
        let input = AppleUISubmission<AgentCoreJSONValue>(interaction: AppleUIInteraction(identity: identity,
            expected_surface_revision: 3, edits: [], action: nil), submission_id: "original", submitted_at_ms: 123)
        XCTAssertThrowsError(try journal.stage(input, source: source))
        XCTAssertTrue(storage.isEmpty)
        failWrite = false
        try journal.stage(input, source: source)
        let saved = storage
        failWrite = true
        // Replaying the exact durable request needs no new write or identifier.
        XCTAssertNoThrow(try journal.stage(input, source: source))
        XCTAssertThrowsError(try journal.retire(input, source: source))
        XCTAssertEqual(try journal.pending(identity: identity, source: source)?.submission_id, "original")
        XCTAssertEqual(storage.values.first?.revision, saved.values.first?.revision)
        for changedSource in [
            AppleUIIntentSource(hostID: source.hostID, address: "https://other.example", certificate: source.certificate, clientID: source.clientID),
            AppleUIIntentSource(hostID: source.hostID, address: source.address, certificate: "new-cert", clientID: source.clientID),
            AppleUIIntentSource(hostID: source.hostID, address: source.address, certificate: source.certificate, clientID: "new-client"),
        ] {
            XCTAssertThrowsError(try journal.pending(identity: identity, source: changedSource))
        }
        failWrite = false
        try journal.retire(input, source: source)
        XCTAssertNil(try journal.pending(identity: identity, source: source))
    }

    func testCodeCellProvenanceRequiresExactOwnerAndRejectsUnknownFields() throws {
        let cell: [String: Any] = [
            "host_id": "host", "conversation_id": "conversation", "turn_id": "turn",
            "parent_call_id": "parent", "cell_id": "cell", "generation": "generation",
        ]
        func decode(_ value: [String: Any]) throws -> IOSConversationProjectionV1.ToolExecutionSource {
            try JSONDecoder().decode(IOSConversationProjectionV1.ToolExecutionSource.self,
                from: JSONSerialization.data(withJSONObject: value))
        }
        XCTAssertEqual(try decode(["kind": "model"]).kind, .model)
        let source = try decode(["kind": "code_cell", "cell": cell])
        XCTAssertEqual(source.cell?.conversationID, "conversation")
        XCTAssertEqual(source.cell?.parentCallID, "parent")
        XCTAssertEqual(source.cell?.generation, "generation")
        for invalid in [
            [:], ["kind": "unknown"], ["kind": "model", "cell": NSNull()],
            ["kind": "model", "permission": "allow"], ["kind": "code_cell"],
            ["kind": "code_cell", "cell": cell, "permission": "allow"],
        ] as [[String: Any]] {
            XCTAssertThrowsError(try decode(invalid))
        }
        var extra = cell
        extra["alias"] = "owner"
        XCTAssertThrowsError(try decode(["kind": "code_cell", "cell": extra]))
        for key in cell.keys {
            var missing = cell
            missing.removeValue(forKey: key)
            XCTAssertThrowsError(try decode(["kind": "code_cell", "cell": missing]))
            for invalid in ["", "   ", "owner\n", String(repeating: "界", count: 171)] {
                var malformed = cell
                malformed[key] = invalid
                XCTAssertThrowsError(try decode(["kind": "code_cell", "cell": malformed]))
            }
        }
    }

    private let conversation = "ios-rich-conversation"
    private let run = "ios-rich-run"
    private var assistant: String { "\(run):assistant" }

    private func remotePresentationFixture(epoch: String = "epoch", message: String = "assistant") throws
        -> (IOSAssistantJournalCursor, IOSRunMessageAdmissionCursor, IOSConversationProjectionWindowV1) {
        let cursor = IOSAssistantJournalCursor(identity: .init(source_store_epoch: epoch,
            conversation_id: conversation, run_id: run, assistant_message_id: message),
            sequence: "1", record_digest: String(repeating: "a", count: 64))
        let admission = IOSRunMessageAdmissionCursor(identity: .init(source_store_epoch: epoch,
            conversation_id: conversation, run_id: run, assistant_message_id: assistant),
            sequence: "1", record_digest: String(repeating: "b", count: 64))
        let fields: [String: Any] = ["contract_version": 1, "execution_ownership": ["kind": "user"],  "conversation_id": conversation,
            "revision": 1, "title": "Fixture", "messages": [["message_id": message, "role": "assistant",
                "state": "running", "revision": 1, "created_at_ms": 1, "parts": []]],
            "context_usage": ["used_tokens": 0, "context_window_tokens": 128000,
                "prompt_token_limit": 120000, "estimated": true, "measured_at_revision": 1]]
        let projection = try JSONDecoder().decode(IOSConversationProjectionV1.self,
            from: JSONSerialization.data(withJSONObject: fields))
        return (cursor, admission, IOSConversationProjectionWindowV1(projection: projection,
            eventSequence: 1, totalMessageCount: 1, beforeOrdinal: nil, hasMore: false,
            assistantJournalHeads: [cursor]))
    }

    func testRemoteJournalDeltasReuseAdmissionButNewOwnerAndTerminalRefreshMetadata() async throws {
        let (cursor, admission, window) = try remotePresentationFixture()
        let cache = IOSRemoteJournalPresentationCache()
        var reads = 0
        let initial = try await cache.loadIfNeeded(admission: admission, generation: 1, cursor: cursor,
            force: false, isCurrent: { true }, readWindow: { _ in reads += 1; return window })
        XCTAssertEqual(initial, window)
        for sequence in 2...100 {
            let delta = IOSAssistantJournalCursor(identity: cursor.identity, sequence: String(sequence),
                record_digest: String(repeating: "c", count: 64))
            let unchanged = try await cache.loadIfNeeded(admission: admission, generation: 1, cursor: delta,
                force: false, isCurrent: { true }, readWindow: { _ in reads += 1; return window })
            XCTAssertNil(unchanged)
        }
        XCTAssertEqual(reads, 1)
        let (second, _, secondWindow) = try remotePresentationFixture(message: "second")
        let newOwner = try await cache.loadIfNeeded(admission: admission, generation: 1, cursor: second,
            force: false, isCurrent: { true }, readWindow: { _ in reads += 1; return secondWindow })
        XCTAssertEqual(newOwner, secondWindow)
        let advancedAdmission = IOSRunMessageAdmissionCursor(identity: admission.identity, sequence: "2",
            record_digest: String(repeating: "d", count: 64))
        let admissionWindow = try await cache.loadIfNeeded(admission: advancedAdmission, generation: 1, cursor: second,
            force: false, isCurrent: { true }, readWindow: { _ in reads += 1; return secondWindow })
        XCTAssertEqual(admissionWindow, secondWindow)
        let terminal = try await cache.loadIfNeeded(admission: advancedAdmission, generation: 1, cursor: second,
            force: true, isCurrent: { true }, readWindow: { _ in reads += 1; return secondWindow })
        XCTAssertEqual(terminal, secondWindow)
        XCTAssertEqual(reads, 4)
    }

    func testRemoteJournalWindowFailureAndViewSwitchNeverCacheUnadmittedOwner() async throws {
        let (cursor, admission, window) = try remotePresentationFixture()
        let cache = IOSRemoteJournalPresentationCache()
        var reads = 0
        let failed = try await cache.loadIfNeeded(admission: admission, generation: 1, cursor: cursor,
            force: false, isCurrent: { true }, readWindow: { _ in
                reads += 1; throw IOSApplicationRuntimeError.applicationUnavailable
            }, sleep: { _ in })
        XCTAssertNil(failed)
        XCTAssertEqual(reads, 3)
        var current = true
        let switched = try await cache.loadIfNeeded(admission: admission, generation: 1, cursor: cursor,
            force: false, isCurrent: { current }, readWindow: { _ in
                reads += 1; current = false; return window
            })
        XCTAssertNil(switched)
        let retry = try await cache.loadIfNeeded(admission: admission, generation: 2, cursor: cursor,
            force: false, isCurrent: { true }, readWindow: { _ in reads += 1; return window })
        XCTAssertEqual(retry, window)
        XCTAssertEqual(reads, 5)
        let (newCursor, newAdmission, newWindow) = try remotePresentationFixture(epoch: "new-epoch")
        let newEpoch = try await cache.loadIfNeeded(admission: newAdmission, generation: 2, cursor: newCursor,
            force: false, isCurrent: { true }, readWindow: { _ in reads += 1; return newWindow })
        XCTAssertEqual(newEpoch, newWindow)
        do {
            _ = try await cache.loadIfNeeded(admission: newAdmission, generation: 2, cursor: cursor,
                force: false, isCurrent: { true }, readWindow: { _ in XCTFail("Foreign epoch must not read metadata"); return window })
            XCTFail("Foreign epoch was accepted")
        } catch {}
        do {
            _ = try await cache.loadIfNeeded(admission: admission, generation: 2, cursor: cursor,
                force: true, isCurrent: { true }, readWindow: { _ in XCTFail("Retired epoch must not return"); return window })
            XCTFail("Retired epoch was accepted")
        } catch {}
    }

    func testTaskCardsFollowExactJournalOwnerAfterStreamingAndColdLoad() throws {
        let snapshot = try committedSnapshot([], sequence: 100)
        let rendered = try XCTUnwrap(IOSConversationProjectionPresenter.message(from: snapshot.message, activeRun: nil))
        var messages = [rendered]
        func projection(_ revision: UInt64, _ state: String, owner: String? = nil) throws -> AgentCoreTaskProjection {
            let task = try JSONDecoder().decode(AgentCoreTaskHandle.self, from: JSONSerialization.data(withJSONObject: [
                "id": "task", "owner_session_id": "independent-core-session", "owner_turn_id": owner ?? run,
                "kind": "agent", "state": state, "cancel_requested": false
            ]))
            return .init(revision: revision, order: [task.id], tasks: [task.id: task])
        }
        let current = try projection(9, "cancelled")
        ChatMessageDraftReducer.applyTaskProjection(current, in: &messages, conversationID: conversation)
        XCTAssertNil(messages[0].taskProjection, "No guessed Run ownership before journal admission")
        let identity = IOSAssistantJournalIdentity(source_store_epoch: "epoch", conversation_id: conversation,
            run_id: run, assistant_message_id: assistant)
        let content = IOSJournalMessageContent(cursor: .init(identity: identity, sequence: "1",
            record_digest: String(repeating: "a", count: 64)), parts: [])
        XCTAssertTrue(ChatMessageDraftReducer.applyJournalContent(content, in: &messages, conversationID: conversation))
        ChatMessageDraftReducer.applyTaskProjection(current, in: &messages, conversationID: conversation)
        XCTAssertEqual(messages[0].taskProjection?.tasks["task"]?.state, .cancelled)
        ChatMessageDraftReducer.applyTaskProjection(try projection(8, "queued"), in: &messages, conversationID: conversation)
        ChatMessageDraftReducer.applyTaskProjection(try projection(10, "running", owner: "foreign-run"), in: &messages, conversationID: conversation)
        ChatMessageDraftReducer.applyTaskProjection(current, in: &messages, conversationID: UUID().uuidString)
        XCTAssertEqual(messages[0].taskProjection?.revision, 9)
        XCTAssertEqual(messages[0].taskProjection?.tasks["task"]?.state, .cancelled)
    }

    func testJournalContentSurvivesMetadataRefreshAndCannotBeResurrectedByDraft() throws {
        let snapshot = try committedSnapshot([], sequence: 100)
        let rendered = try XCTUnwrap(IOSConversationProjectionPresenter.message(from: snapshot.message, activeRun: nil))
        var messages = [rendered]
        let identity = IOSAssistantJournalIdentity(source_store_epoch: "epoch", conversation_id: conversation,
            run_id: run, assistant_message_id: assistant)
        func content(_ sequence: Int, _ text: String?) throws -> IOSJournalMessageContent {
            let parts = try text.map { value in
                try decodeMessage([["kind": "text", "part_id": "text", "revision": sequence,
                    "markdown": value, "is_final": false]]).parts
            } ?? []
            return IOSJournalMessageContent(cursor: .init(identity: identity, sequence: String(sequence),
                record_digest: String(repeating: "a", count: 64)), parts: parts)
        }
        XCTAssertTrue(ChatMessageDraftReducer.applyJournalContent(try content(1, "你好🙂"),
            in: &messages, conversationID: conversation))
        XCTAssertTrue(ChatMessageDraftReducer.applyCommitted(snapshot, in: &messages, conversationID: conversation))
        XCTAssertEqual(messages[0].content, "你好🙂")
        XCTAssertEqual(messages[0].canonicalJournalCursor?.sequence, "1")
        XCTAssertEqual(messages[0].canonicalEventSequence, 100)
        let update = IOSCommittedJournalPresentation(conversationID: conversation, generation: 1,
            eventSequence: 101, messages: [snapshot.message], activeRun: nil, surface: snapshot.surface)
        ChatMessageDraftReducer.applyJournalPresentation(update, in: &messages, conversationID: conversation)
        XCTAssertEqual(messages[0].content, "你好🙂")
        XCTAssertEqual(messages[0].canonicalEventSequence, 101)
        XCTAssertTrue(ChatMessageDraftReducer.applyJournalContent(try content(2, nil),
            in: &messages, conversationID: conversation))
        ChatMessageDraftReducer.upsertAssistant(in: &messages, assistantID: rendered.id,
            afterUserID: UUID(), content: "stale draft", thinking: "stale reasoning")
        XCTAssertEqual(messages[0].content, "")
        XCTAssertEqual(messages[0].thinking, "")
        XCTAssertTrue(messages[0].canonicalParts.isEmpty)
        XCTAssertFalse(ChatMessageDraftReducer.applyJournalContent(try content(1, "stale"),
            in: &messages, conversationID: conversation))
        XCTAssertFalse(ChatMessageDraftReducer.applyJournalContent(try content(3, "foreign"),
            in: &messages, conversationID: "other"))
        XCTAssertEqual(messages[0].content, "")
        XCTAssertEqual(messages[0].canonicalEventSequence, 101)
    }

    private func tool(output: Any? = nil) -> [String: Any] {
        var value: [String: Any] = ["kind": "tool", "execution_source": ["kind": "model"], "part_id": "rich-tool", "revision": 2,
            "tool_call_id": "rich-call", "capability_id": "write_file", "state": "completed", "title": "工具标题 α",
            "summary": "", "input": ["path": "/tmp/Report.txt"], "affected_resources": [] as [String],
            "origin": ["kind": "mcp", "provider_id": "provider", "provider_title": "Provider title", "namespace": "ns"],
            "permission": ["request_id": "approval", "permission_domain": "filesystem", "rationale": "Readonly rationale", "decision": "allow"],
            "output_preview": "Independent preview", "full_output": ["message_id": assistant, "part_id": "rich-tool", "tool_call_id": "rich-call"],
            "artifacts": [["name": "真实名.txt", "uri": "file:///tmp/Report.txt", "mime_type": "text/plain", "operation": "create"]],
            "started_at_ms": 10, "finished_at_ms": 20]
        if let output { value["output"] = output }
        return value
    }

    private func message(_ parts: [[String: Any]]) -> [String: Any] {
        ["message_id": assistant, "role": "assistant", "state": "running", "revision": 2, "created_at_ms": 1, "parts": parts]
    }

    private func decodeMessage(_ parts: [[String: Any]]) throws -> IOSConversationProjectionV1.Message {
        try JSONDecoder().decode(IOSConversationProjectionV1.Message.self,
            from: JSONSerialization.data(withJSONObject: message(parts)))
    }

    private func numericToolJSON(input: String, output: String, suffix: String = "scalar") -> String {
        #"""
        {"kind":"tool","execution_source":{"kind":"model"},"part_id":"numeric-tool-\#(suffix)","revision":2,"tool_call_id":"numeric-call-\#(suffix)",
         "capability_id":"write_file","state":"completed","title":"Numbers","origin":{"kind":"builtin"},
         "input":\#(input),"output":\#(output),"artifacts":[],"affected_resources":[]}
        """#
    }

    private func numericMessageJSON(
        parts: [String], state: IOSConversationProjectionV1.MessageState = .completed
    ) -> String {
        #"""
        {"message_id":"\#(assistant)","role":"assistant","state":"\#(state.rawValue)","revision":2,
         "created_at_ms":1,"parts":[\#(parts.joined(separator: ","))]}
        """#
    }

    private var nestedNumericJSON: String {
        #"{"positive":18446744073709551615,"negative":-9223372036854775808,"boundary":[9007199254740993,-9007199254740993,9007199254740991],"fraction":0.125}"#
    }

    private var nestedNumericDisplay: String {
        "{boundary: [9007199254740993, -9007199254740993, 9007199254740991], fraction: 0.125, negative: -9223372036854775808, positive: 18446744073709551615}"
    }

    func testRawPublicToolIntegersReachPresenterAndDetailsWithoutAnotherDoubleBridge() throws {
        for literal in ["18446744073709551615", "-9223372036854775808", "9223372036854775807",
                        "9223372036854775808", "9007199254740993", "-9007199254740993",
                        "9007199254740991", "0.125"] {
            let projected = try JSONDecoder().decode(IOSConversationProjectionV1.Message.self,
                from: Data(numericMessageJSON(parts: [numericToolJSON(input: literal, output: literal)]).utf8))
            let rendered = try XCTUnwrap(IOSConversationProjectionPresenter.message(from: projected, activeRun: nil))
            let tool = try XCTUnwrap(rendered.canonicalTools.first)
            let expected = try JSONDecoder().decode(AgentCoreJSONValue.self, from: Data(literal.utf8))
            XCTAssertEqual(tool.input, expected, literal)
            XCTAssertEqual(tool.output, expected, literal)
            XCTAssertEqual(tool.input.displayText, literal)
            XCTAssertEqual(tool.output?.displayText, literal)
            XCTAssertEqual(rendered.hostToolEvents.first?.argsPreview, literal)
            XCTAssertEqual(rendered.hostToolEvents.first?.text, literal)
            let details = ChatToolDetailProjection(event: try XCTUnwrap(rendered.hostToolEvents.first),
                activity: nil, canonicalTool: tool)
            XCTAssertEqual(details.arguments, literal)
            XCTAssertEqual(details.result, literal)
            XCTAssertEqual(String(decoding: try JSONEncoder().encode(tool.input), as: UTF8.self), literal)
        }
        let projected = try JSONDecoder().decode(IOSConversationProjectionV1.Message.self, from: Data(
            numericMessageJSON(parts: [numericToolJSON(input: nestedNumericJSON, output: nestedNumericJSON)]).utf8))
        let rendered = try XCTUnwrap(IOSConversationProjectionPresenter.message(from: projected, activeRun: nil))
        let tool = try XCTUnwrap(rendered.canonicalTools.first)
        XCTAssertEqual(tool.input.displayText, nestedNumericDisplay)
        XCTAssertEqual(rendered.hostToolEvents.first?.text, nestedNumericDisplay)
        let details = ChatToolDetailProjection(event: try XCTUnwrap(rendered.hostToolEvents.first),
            activity: nil, canonicalTool: tool)
        XCTAssertEqual(details.result, nestedNumericDisplay)
        guard case .object(let object)? = tool.output else { return XCTFail("Missing structured object") }
        XCTAssertEqual(object["positive"], .unsignedInteger(UInt64.max))
        XCTAssertEqual(object["negative"], .signedInteger(Int64.min))
        XCTAssertEqual(object["boundary"], .array([
            .signedInteger(9_007_199_254_740_993), .signedInteger(-9_007_199_254_740_993),
            .number(9_007_199_254_740_991)
        ]))
        XCTAssertEqual(object["fraction"], .number(0.125))
    }

    func testLiteralLargeIntegersSurviveLosslessWindowAndLivePresenterContract() throws {
        let literals = ["18446744073709551615", "-9223372036854775808", "9007199254740993",
            "-9007199254740993", "9007199254740991", "0.125"]
        var parts = literals.enumerated().map { index, literal in
            numericToolJSON(input: literal, output: literal, suffix: String(index))
        }
        parts.append(numericToolJSON(input: nestedNumericJSON, output: nestedNumericJSON, suffix: "nested"))
        let expectedNested = try JSONDecoder().decode(AgentCoreJSONValue.self, from: Data(nestedNumericJSON.utf8))
        let raw = #"""
        {"event_sequence":2,"total_message_count":1,"has_more":false,"assistant_journal_heads":[],"projection":{
        "contract_version":1, "execution_ownership": {"kind": "user"}, "conversation_id":"\#(conversation)","revision":2,"title":"Numbers",
        "messages":[\#(numericMessageJSON(parts: parts, state: .running))],
        "surface":{"revision":2,"committed_message_ids":[],"live_message_id":"\#(assistant)","archived_activity_part_ids":[],"show_assistant_placeholder":false},
        "context_usage":{"used_tokens":0,"context_window_tokens":65536,"prompt_token_limit":57344,"estimated":true,"measured_at_revision":2}}}
        """#
        // Decode twice to exercise a fresh consumer without a Foundation Any/Double bridge.
        for _ in 0..<2 {
            let window = try JSONDecoder().decode(IOSConversationProjectionWindowV1.self, from: Data(raw.utf8))
            let owner = IOSCommittedAssistantOwner(conversationID: conversation, assistantMessageID: assistant, generation: 1)
            let snapshot = try XCTUnwrap(IOSCommittedAssistantProjection(window: window, owner: owner))
            XCTAssertTrue(snapshot.belongs(to: owner))
            XCTAssertEqual(snapshot.message.role, .assistant)
            XCTAssertEqual(snapshot.message.state, .running)
            XCTAssertEqual(snapshot.message.revision, 2)
            XCTAssertEqual(snapshot.message.parts.count, parts.count)
            let rendered = try XCTUnwrap(IOSConversationProjectionPresenter.message(
                from: snapshot.message, activeRun: snapshot.activeRun, surface: snapshot.surface))
            var live = [ChatMessage(id: rendered.id, role: .assistant, content: "Draft remains")]
            XCTAssertTrue(ChatMessageDraftReducer.applyCommitted(snapshot, in: &live, conversationID: conversation))
            for (index, literal) in literals.enumerated() {
                let callID = "numeric-call-\(index)"
                let tool = try XCTUnwrap(live[0].canonicalTools.first { $0.toolCallID == callID })
                let expected = try JSONDecoder().decode(AgentCoreJSONValue.self, from: Data(literal.utf8))
                XCTAssertEqual(tool.input, expected, literal)
                XCTAssertEqual(tool.output, expected, literal)
                let event = try XCTUnwrap(live[0].hostToolEvents.first { $0.id == callID })
                XCTAssertEqual(event.argsPreview, literal)
                XCTAssertEqual(event.text, literal)
                XCTAssertEqual(ChatToolDetailProjection(event: event, activity: nil, canonicalTool: tool).result, literal)
            }
            let nested = try XCTUnwrap(live[0].canonicalTools.first { $0.toolCallID == "numeric-call-nested" })
            XCTAssertEqual(nested.input, expectedNested)
            XCTAssertEqual(nested.output, expectedNested)
            XCTAssertEqual(nested.output?.displayText, nestedNumericDisplay)
            XCTAssertEqual(live[0].content, "Draft remains")
        }
    }

    private func committedSnapshot(
        _ parts: [[String: Any]], sequence: UInt64, revision: UInt64 = 2
    ) throws -> IOSCommittedAssistantProjection {
        var assistantMessage = message(parts)
        assistantMessage["revision"] = revision
        let object: [String: Any] = ["event_sequence": sequence, "total_message_count": 1, "assistant_journal_heads": [],
            "has_more": false, "projection": ["contract_version": 1, "execution_ownership": ["kind": "user"],  "conversation_id": conversation,
                "revision": revision, "title": "Rich", "messages": [assistantMessage],
                "surface": ["revision": revision, "committed_message_ids": [] as [String], "live_message_id": assistant,
                    "archived_activity_part_ids": [] as [String], "show_assistant_placeholder": false],
                "context_usage": ["used_tokens": 0, "context_window_tokens": 65536, "prompt_token_limit": 57344,
                    "estimated": true, "measured_at_revision": revision]]]
        let window = try JSONDecoder().decode(IOSConversationProjectionWindowV1.self,
            from: JSONSerialization.data(withJSONObject: object))
        return try XCTUnwrap(IOSCommittedAssistantProjection(window: window, owner: .init(
            conversationID: conversation, assistantMessageID: assistant, generation: 1
        )))
    }

    func testJournalPresentationAppendsSteeringOwnersAndUpdatesOnlyExactBubble() throws {
        let root = try committedSnapshot([], sequence: 10)
        func row(_ id: String, _ role: String, _ text: String, revision: UInt64 = 1) throws -> IOSConversationProjectionV1.Message {
            try JSONDecoder().decode(IOSConversationProjectionV1.Message.self,
                from: JSONSerialization.data(withJSONObject: ["message_id": id, "role": role,
                    "state": role == "user" ? "completed" : "running", "revision": revision, "created_at_ms": 1,
                    "parts": [["kind": "text", "part_id": "\(id)-body", "revision": revision,
                               "markdown": text, "is_final": role == "user"]]]))
        }
        let original = ChatMessage(id: IOSCanonicalMessageIdentity.stableUUID(for: assistant), role: .assistant, content: "root reply")
        var messages = [original]
        let admitted = IOSCommittedJournalPresentation(conversationID: conversation, generation: 1, eventSequence: 11,
            messages: [try row("steering-user", "user", "follow-up"), try row("successor", "assistant", "first")],
            activeRun: root.activeRun, surface: root.surface)
        ChatMessageDraftReducer.applyJournalPresentation(admitted, in: &messages, conversationID: conversation)
        XCTAssertEqual(messages.map(\.content), ["root reply", "follow-up", "first"])
        let update = IOSCommittedJournalPresentation(conversationID: conversation, generation: 1, eventSequence: 12,
            messages: [try row("successor", "assistant", "first and second", revision: 2)],
            activeRun: root.activeRun, surface: root.surface)
        ChatMessageDraftReducer.applyJournalPresentation(update, in: &messages, conversationID: conversation)
        ChatMessageDraftReducer.applyJournalPresentation(admitted, in: &messages, conversationID: conversation)
        XCTAssertEqual(messages.map(\.content), ["root reply", "follow-up", "first and second"])
        XCTAssertEqual(messages[0].id, original.id)
        var foreign: [ChatMessage] = []
        ChatMessageDraftReducer.applyJournalPresentation(update, in: &foreign, conversationID: "another-conversation")
        XCTAssertTrue(foreign.isEmpty)
    }

    func testCommittedLiveOverlayPreservesAheadDraftAndRejectsForeignOrOlderFacts() throws {
        let userID = UUID()
        let assistantID = IOSCanonicalMessageIdentity.stableUUID(for: assistant)
        var messages = [ChatMessage(id: userID, role: .user, content: "Original"),
            ChatMessage(id: assistantID, role: .assistant, content: "Committed text plus newer draft")]
        let snapshot = try committedSnapshot([tool(output: NSNull()),
            ["kind": "text", "part_id": "text", "revision": 2, "markdown": "Committed text", "is_final": false],
            ["kind": "reasoning", "part_id": "reason", "revision": 2, "state": "running", "title": "实时标题", "summary": "Detail"]], sequence: 10)
        XCTAssertTrue(snapshot.belongs(to: IOSCommittedAssistantOwner(conversationID: conversation,
            assistantMessageID: assistant, generation: 1)))
        XCTAssertFalse(snapshot.belongs(to: IOSCommittedAssistantOwner(conversationID: conversation,
            assistantMessageID: assistant, generation: 2)))
        XCTAssertTrue(ChatMessageDraftReducer.applyCommitted(snapshot, in: &messages, conversationID: conversation))
        XCTAssertEqual(messages.map(\.id), [userID, assistantID])
        XCTAssertEqual(messages[1].content, "Committed text plus newer draft")
        XCTAssertEqual(messages[1].canonicalTools.first?.output, .null)
        XCTAssertEqual(messages[1].canonicalTools.first?.summary, "")
        XCTAssertEqual(messages[1].canonicalTools.first?.outputPreview, "Independent preview")
        XCTAssertEqual(messages[1].canonicalReasoningTitles, ["实时标题"])
        ChatMessageDraftReducer.upsertAssistant(in: &messages, assistantID: assistantID,
            afterUserID: userID, content: "Still newer draft", thinkingFinished: true)
        XCTAssertEqual(messages[1].canonicalTools.first?.output, .null)
        XCTAssertEqual(messages[1].canonicalTools.first?.artifacts.first?.operation, "create")
        XCTAssertEqual(messages[1].canonicalEventSequence, 10)
        XCTAssertFalse(ChatMessageDraftReducer.applyCommitted(snapshot, in: &messages, conversationID: "foreign"))
        XCTAssertFalse(ChatMessageDraftReducer.applyCommitted(
            try committedSnapshot([tool()], sequence: 9, revision: 3), in: &messages, conversationID: conversation))
        XCTAssertFalse(ChatMessageDraftReducer.applyCommitted(
            try committedSnapshot([tool()], sequence: 11, revision: 1), in: &messages, conversationID: conversation))
        XCTAssertEqual(messages[1].content, "Still newer draft")
        XCTAssertEqual(messages[1].canonicalTools.first?.output, .null)
        var noAssistant = [messages[0]]
        XCTAssertFalse(ChatMessageDraftReducer.applyCommitted(snapshot, in: &noAssistant, conversationID: conversation))
        XCTAssertEqual(noAssistant.count, 1)
    }

    func testCommittedReviewWithholdingCannotBeReintroducedByLateDraftUpdate() throws {
        let userID = UUID()
        let assistantID = IOSCanonicalMessageIdentity.stableUUID(for: assistant)
        for review in ["pending", "rejected", "interrupted"] {
            var messages = [ChatMessage(id: assistantID, role: .assistant, content: "PRIVATE")]
            let withheld = try committedSnapshot([["kind": "text", "part_id": "reviewed", "revision": 2,
                "markdown": "PRIVATE", "is_final": true, "review": review]], sequence: 10)
            XCTAssertTrue(ChatMessageDraftReducer.applyCommitted(withheld, in: &messages, conversationID: conversation))
            XCTAssertEqual(messages[0].content, "")
            ChatMessageDraftReducer.upsertAssistant(in: &messages, assistantID: assistantID,
                afterUserID: userID, content: "PRIVATE-LATE")
            XCTAssertEqual(messages[0].content, "")
            let approved = try committedSnapshot([["kind": "text", "part_id": "reviewed", "revision": 3,
                "markdown": "Approved answer", "is_final": true, "review": "approved"]], sequence: 11, revision: 3)
            XCTAssertTrue(ChatMessageDraftReducer.applyCommitted(approved, in: &messages, conversationID: conversation))
            XCTAssertEqual(messages[0].content, "Approved answer")
        }
    }

    func testCommittedToolFactsOverrideLateLegacyLifecycleAndTraceResult() throws {
        let assistantID = IOSCanonicalMessageIdentity.stableUUID(for: assistant)
        let late = HostAgentToolEvent(id: "rich-call", name: "write_file",
            argsPreview: "old args", status: .running, text: "old result")
        let activity = AgentTraceActivity(spanId: "rich-call", toolName: "write_file", status: .running,
            arguments: .string("old trace args"), result: .string("old trace result"), startedSequence: 1)
        let states: [(String, HostAgentToolEvent.Status)] = [
            ("completed", .succeeded), ("failed", .failed), ("cancelled", .cancelled)
        ]
        for (state, expectedStatus) in states {
            var canonical = tool(output: NSNull())
            canonical["state"] = state
            let snapshot = try committedSnapshot([canonical], sequence: 12)
            var messages = [ChatMessage(id: assistantID, role: .assistant, content: "Draft body",
                hostToolEvents: [late], traceActivities: [activity])]
            XCTAssertTrue(ChatMessageDraftReducer.applyCommitted(snapshot, in: &messages, conversationID: conversation))
            XCTAssertEqual(messages[0].hostToolEvents.first?.status, expectedStatus)
            XCTAssertEqual(messages[0].hostToolEvents.first?.text, "null")
            ChatMessageDraftReducer.upsertAssistant(in: &messages, assistantID: assistantID,
                afterUserID: UUID(), content: "Still newer body", hostToolEvents: [late])
            XCTAssertEqual(messages[0].hostToolEvents.first?.status, expectedStatus)
            XCTAssertEqual(messages[0].hostToolEvents.first?.text, "null")
            XCTAssertEqual(messages[0].content, "Still newer body")
            let typedTool = try XCTUnwrap(messages[0].canonicalTools.first)
            let details = ChatToolDetailProjection(event: late, activity: activity, canonicalTool: typedTool)
            XCTAssertEqual(details.result, "null")
            XCTAssertTrue(details.arguments.contains("/tmp/Report.txt"))
            XCTAssertFalse(details.arguments.contains("old trace"))
            XCTAssertTrue(details.resources.isEmpty)
            canonical.removeValue(forKey: "output")
            let withoutOutput = try committedSnapshot([canonical], sequence: 13, revision: 3)
            XCTAssertTrue(ChatMessageDraftReducer.applyCommitted(withoutOutput, in: &messages, conversationID: conversation))
            XCTAssertEqual(messages[0].hostToolEvents.first?.text, "Independent preview")
            canonical.removeValue(forKey: "output_preview")
            XCTAssertTrue(ChatMessageDraftReducer.applyCommitted(
                try committedSnapshot([canonical], sequence: 14, revision: 4), in: &messages, conversationID: conversation))
            XCTAssertEqual(messages[0].hostToolEvents.first?.text, "")
            XCTAssertEqual(ChatToolDetailProjection(event: late, activity: activity,
                canonicalTool: messages[0].canonicalTools.first).result, "")
        }
    }

    func testRecoveryObservationFenceRejectsGenerationConversationOrCoordinatorChange() {
        let conversationID = UUID()
        let fence = IOSRecoveryObservationFence(conversationID: conversationID, generation: 7)
        XCTAssertTrue(fence.accepts(conversationID: conversationID, generation: 7, coordinatorIsAvailable: true))
        XCTAssertFalse(fence.accepts(conversationID: conversationID, generation: 8, coordinatorIsAvailable: true))
        XCTAssertFalse(fence.accepts(conversationID: UUID(), generation: 7, coordinatorIsAvailable: true))
        XCTAssertFalse(fence.accepts(conversationID: nil, generation: 7, coordinatorIsAvailable: true))
        XCTAssertFalse(fence.accepts(conversationID: conversationID, generation: 7, coordinatorIsAvailable: false))
    }

    func testPresenterConsumesIndependentRichSlotsAndRejectsForeignOutputOwner() throws {
        let projected = try decodeMessage([tool(output: NSNull()),
            ["kind": "reasoning", "part_id": "reasoning", "revision": 2, "state": "completed", "title": "推理标题", "summary": "推理正文"],
            ["kind": "attachment", "part_id": "artifact", "revision": 2, "attachment_id": "attachment", "name": "Actual.pdf",
             "uri": "file:///tmp/Actual.pdf", "mime_type": "application/pdf", "operation": "create"]])
        let rendered = try XCTUnwrap(IOSConversationProjectionPresenter.message(from: projected, activeRun: nil))
        let metadata = try XCTUnwrap(rendered.canonicalTools.first)
        XCTAssertEqual(metadata.summary, "")
        XCTAssertEqual(metadata.output, .null)
        XCTAssertEqual(metadata.outputPreview, "Independent preview")
        XCTAssertEqual(metadata.origin.providerTitle, "Provider title")
        XCTAssertEqual(metadata.permission?.rationale, "Readonly rationale")
        XCTAssertEqual(metadata.artifacts.first?.name, "真实名.txt")
        XCTAssertEqual(metadata.artifacts.first?.operation, "create")
        XCTAssertEqual(metadata.startedAtMs, 10)
        XCTAssertEqual(metadata.finishedAtMs, 20)
        XCTAssertEqual(metadata.fullOutput?.messageID, assistant)
        XCTAssertEqual(rendered.hostToolEvents.first?.display, "工具标题 α")
        XCTAssertEqual(rendered.hostToolEvents.first?.text, "null")
        XCTAssertNil(rendered.hostToolEvents.first?.finalURL)
        XCTAssertEqual(rendered.canonicalReasoningTitles, ["推理标题"])
        XCTAssertEqual(rendered.thinking, "推理正文")
        XCTAssertEqual(rendered.canonicalAttachments.first?.operation, "create")
        guard case .tool(let absent) = try XCTUnwrap(decodeMessage([tool()]).parts.first) else { return XCTFail("Expected tool") }
        XCTAssertNil(absent.output)
        for key in ["message_id", "part_id", "tool_call_id", "path"] {
            var bad = tool(output: NSNull())
            var owner = try XCTUnwrap(bad["full_output"] as? [String: Any])
            owner[key] = "/private/foreign"
            bad["full_output"] = owner
            XCTAssertThrowsError(try decodeMessage([bad]), key)
        }
        for key in ["title", "origin", "artifacts"] {
            var missing = tool()
            missing.removeValue(forKey: key)
            XCTAssertThrowsError(try decodeMessage([missing]), key)
        }
        var citation: [String: Any] = ["kind": "citation", "part_id": "citation", "revision": 2,
            "citation_id": "actual", "source_id": "source", "title": "", "uri": "https://example.com",
            "origin_tool_call_id": "rich-call", "location": [:]]
        guard case .citation(let emptyLocation) = try XCTUnwrap(decodeMessage([citation]).parts.first) else { return XCTFail("Expected citation") }
        XCTAssertNotNil(emptyLocation.location)
        citation["section"] = "inconsistent"
        XCTAssertThrowsError(try decodeMessage([citation]))
        citation.removeValue(forKey: "section")
        citation["range_start"] = 7
        XCTAssertThrowsError(try decodeMessage([citation]))
        for review in ["pending", "rejected", "interrupted"] {
            let reviewed = try decodeMessage([["kind": "text", "part_id": "withheld", "revision": 2,
                "markdown": "PRIVATE-\(review)", "is_final": true, "review": review]])
            XCTAssertEqual(IOSConversationProjectionPresenter.message(from: reviewed, activeRun: nil)?.content, "")
        }
    }

    func testJournalReaderKeepsStructuredNullReviewAndPrivateOutputBoundary() async throws {
        var value = tool(output: NSNull())
        value["started_at_ms"] = 11; value["finished_at_ms"] = 22
        let content = try await readJournalPresentation([value], conversation: conversation, run: run, assistant: assistant)
        guard case .tool(let part)? = content.parts.first else { return XCTFail("Missing tool") }
        XCTAssertEqual(part.output, .null)
        XCTAssertEqual(part.startedAtMs, 11); XCTAssertEqual(part.finishedAtMs, 22)
        let event = IOSConversationProjectionPresenter.toolEvent(from: part)
        XCTAssertEqual(event.text, "null"); XCTAssertNil(event.finalURL)
        value.removeValue(forKey:"output")
        let absent = try await readJournalPresentation([value], conversation: conversation, run: run, assistant: assistant)
        guard case .tool(let absentPart)? = absent.parts.first else { return XCTFail("Missing tool") }
        XCTAssertNil(absentPart.output)
        for review in ["pending","rejected","interrupted"] {
            let withheld = try await readJournalPresentation([["kind":"text","part_id":"withheld","revision":1,
                "markdown":"PRIVATE","is_final":true,"review":review]])
            XCTAssertEqual(withheld.text,"")
        }
        do {
            _ = try await readJournalPresentation([["kind":"text","part_id":"withheld","revision":1,
                "markdown":"PRIVATE","is_final":true,"review":"unknown"]])
            XCTFail("Unknown review must fail")
        } catch {}
        let maximum = try JSONDecoder().decode(ChatSourceLocation.self,
            from: Data(#"{"page":18446744073709551615,"line_start":18446744073709551615}"#.utf8))
        XCTAssertEqual(maximum.page, UInt64.max)
        XCTAssertTrue(maximum.displayText?.contains(String(UInt64.max)) == true)
        var foreign = tool()
        foreign["full_output"] = ["message_id":"foreign","part_id":"rich-tool","tool_call_id":"rich-call"]
        do { _ = try await readJournalPresentation([foreign], conversation:conversation, run:run, assistant:assistant)
            XCTFail("Foreign full output owner must fail") } catch {}
    }

    func testJournalStructuredResultPreservesPresenceForEveryJSONShape() async throws {
        let values: [(String, Any?, String)] = [
            ("missing",nil,"Independent preview"), ("null",NSNull(),"null"),
            ("empty string","",""), ("string","actual result","actual result"),
            ("bool",true,"true"), ("number",42,"42"),
            ("empty object",[String:Any](),"{}"), ("empty array",[Any](),"[]"),
            ("object",["z":2,"a":1],"{a: 1, z: 2}")
        ]
        for (label,value,expected) in values {
            var source = tool(output:value)
            if let value { source["input"] = value }
            let content = try await readJournalPresentation([source], conversation:conversation,run:run,assistant:assistant)
            guard case .tool(let part)? = content.parts.first else { return XCTFail("Missing tool") }
            XCTAssertEqual(part.output == nil,value == nil,label)
            let event = IOSConversationProjectionPresenter.toolEvent(from:part)
            XCTAssertEqual(event.text,expected,label)
            XCTAssertEqual(event.argsPreview,part.input.displayText,label)
            let original = try JSONDecoder().decode(AgentCoreJSONValue.self,
                from:JSONSerialization.data(withJSONObject:source["input"]!, options:[.fragmentsAllowed]))
            XCTAssertEqual(part.input,original,label)
        }
    }
    func testJournalRichPartsSurviveMetadataRefreshAndExactOwnerOverlay() async throws {
        let usage: [String: Any] = ["record_id": "memory", "component_id": "memory-component",
            "scope": "global", "namespace": ["kind": "workspace"], "source_role": "tool",
            "range_start": "a", "range_end": "b", "requires_current_state_verification": true, "truncated": true,
            "supersedes": ["older"], "usage_count": 7, "last_used_at_epoch_ms": 31]
        var parts: [[String: Any]] = [tool(output: NSNull()),
            ["kind": "reasoning", "part_id": "reasoning", "revision": 2, "state": "completed", "title": "推理标题 α", "summary": "正文"],
            ["kind": "source", "part_id": "source-part", "revision": 2, "source_id": "source", "title": "Evidence", "uri": "https://example.com",
             "evidence": ["kind": "memory", "usage": usage]],
            ["kind": "citation", "part_id": "citation-part", "revision": 2, "citation_id": "actual-citation", "source_id": "source",
             "title": "", "uri": "https://example.com", "mime_type": "application/pdf", "origin_tool_call_id": "rich-call",
             "location": ["section": "方法 α", "page": 3], "section": "方法 α", "page": 3, "range_start": 7, "range_end": 12],
            ["kind": "citation", "part_id": "empty-location", "revision": 2, "citation_id": "empty-location", "source_id": "source", "title": "",
             "uri": "https://example.com", "origin_tool_call_id": "rich-call", "location": [:]],
            ["kind": "attachment", "part_id": "attachment", "revision": 2, "attachment_id": "attachment", "name": "Actual.pdf",
             "uri": "file:///tmp/Actual.pdf", "mime_type": "application/pdf", "operation": "create"],
            ["kind": "text", "part_id": "text", "revision": 2, "markdown": "Visible answer", "is_final": true, "review": "approved"]]
        for review in ["pending", "rejected", "interrupted"] {
            parts.append(["kind": "text", "part_id": "text-\(review)", "revision": 2, "markdown": "", "is_final": true, "review": review])
        }
        for _ in 0..<2 {
            // Independently read the fixed journal again, as on a cold reopen.
            // Metadata contains no body; only the reader can supply rich Parts.
            let content = try await readJournalPresentation(parts, conversation: conversation,
                run: run, assistant: assistant, sequence: 2)
            let committed = try committedSnapshot([], sequence: 2)
            let shell = try XCTUnwrap(IOSConversationProjectionPresenter.message(
                from: committed.message, activeRun: committed.activeRun, surface: committed.surface))
            var live = [shell]
            XCTAssertTrue(ChatMessageDraftReducer.applyJournalContent(content, in: &live, conversationID: conversation))
            let rendered = live[0]
            XCTAssertEqual(rendered.content, "Visible answer")
            XCTAssertEqual(rendered.canonicalTools.first?.output, .null)
            XCTAssertEqual(rendered.canonicalTools.first?.summary, "")
            XCTAssertEqual(rendered.canonicalTools.first?.outputPreview, "Independent preview")
            XCTAssertEqual(rendered.canonicalReasoningTitles, ["推理标题 α"])
            let reference = try XCTUnwrap(rendered.sourceReferences.first { $0.citationID == "actual-citation" })
            XCTAssertEqual(reference.id, "actual-citation")
            XCTAssertEqual(reference.title, "")
            XCTAssertEqual(reference.location?.section, "方法 α")
            XCTAssertEqual(reference.rangeStart, 7)
            XCTAssertEqual(reference.rangeEnd, 12)
            XCTAssertEqual(reference.mimeType, "application/pdf")
            XCTAssertEqual(reference.memoryUsage?.usageCount, 7)
            XCTAssertEqual(reference.memoryUsage?.sourceRole, "tool")
            XCTAssertEqual(reference.memoryUsage?.rangeEnd, "b")
            XCTAssertNotNil(rendered.sourceReferences.first { $0.citationID == "empty-location" }?.location)
            XCTAssertEqual(rendered.canonicalAttachments.first?.operation, "create")
            let owner = IOSCommittedAssistantOwner(conversationID: conversation,
                assistantMessageID: assistant, generation: 1)
            XCTAssertTrue(committed.belongs(to: owner))
            let refreshed = try committedSnapshot([], sequence: 3, revision: 3)
            XCTAssertTrue(ChatMessageDraftReducer.applyCommitted(refreshed, in: &live, conversationID: conversation))
            XCTAssertEqual(live[0].canonicalTools.first?.output, .null)
            XCTAssertEqual(live[0].canonicalTools.first?.summary, "")
            XCTAssertEqual(live[0].canonicalTools.first?.outputPreview, "Independent preview")
            XCTAssertEqual(live[0].canonicalReasoningTitles, ["推理标题 α"])
            XCTAssertEqual(live[0].sourceReferences.first { $0.citationID == "actual-citation" }?.location?.section, "方法 α")
            XCTAssertEqual(live[0].canonicalAttachments.first?.operation, "create")
            XCTAssertEqual(live[0].canonicalEventSequence, refreshed.eventSequence)
            XCTAssertEqual(live[0].canonicalJournalCursor, content.cursor)
            XCTAssertEqual(live[0].content, "Visible answer")
            XCTAssertFalse(ChatMessageDraftReducer.applyJournalContent(content, in: &live, conversationID: "foreign-conversation"))
            XCTAssertFalse(committed.belongs(to: .init(
                conversationID: conversation, assistantMessageID: "foreign-assistant", generation: 1)))
        }
    }
}
