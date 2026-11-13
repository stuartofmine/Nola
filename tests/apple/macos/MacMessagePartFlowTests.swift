// SPDX-License-Identifier: Apache-2.0
import XCTest
@testable import Nola

final class MacMessagePartFlowTests: XCTestCase {
    private let identity = MacAssistantJournalIdentity(source_store_epoch: "epoch", conversation_id: "child",
        run_id: "run", assistant_message_id: "assistant")
    private func part(_ id: String, _ metadata: MacAssistantJournalMetadata) -> MacAssistantJournalPart {
        .init(identity: identity, part_id: id, generation: "1", revision: "1", metadata_revision: "1",
              order_key: .init(section: "transcript", position: id), metadata: metadata, bodies: [])
    }
    private func reasoning(_ id: String, state: String) -> MacAssistantJournalPart {
        part(id, .reasoning(.init(title: .inline("same text, distinct part"), state: state)))
    }
    private func tool(_ id: String, state: String = "completed") -> MacAssistantJournalPart {
        part(id, .tool(.init(execution_source: .model, tool_call_id: id, capability_id: "render_ui",
            title: .inline("Render"), summary: nil,
            origin: .init(kind: "local", provider_id: nil, provider_title: nil, namespace: nil),
            permission: nil, state: state, display_key: nil, affected_resources: .inline([]), artifacts: "0",
            full_output: nil, started_at_ms: nil, finished_at_ms: nil)))
    }
    func testOneExecutionDisclosurePreservesReasoningToolOrderAndPublicAnchors() {
        let surface = AppleUISurfaceIdentity(owner: .init(authority_id: "authority", conversation_id: "child",
            participant_id: "expert", context_id: "context", originating_run_id: "run", message_id: "assistant", part_id: "tool"),
            surface_id: "form", generation: 7)
        let parts = [reasoning("r1", state: "completed"), tool("tool"),
            part("surface", .uiSurface(.init(surface: .init(identity: surface, revision: 3), summary: "Form"))),
            reasoning("r2", state: "running"), part("answer", .text(.init(is_final: false, visibility: "public", review: nil)))]
        let flow = MacJournalPartFlow.segments(parts)
        XCTAssertEqual(flow.map { $0.parts.map(\.part_id) }, [["r1", "tool", "r2"], ["surface"], ["answer"]])
        XCTAssertEqual(flow.filter { $0.kind == .process }.count, 1)
        XCTAssertEqual(Set(flow.flatMap(\.parts).map(\.part_id)), Set(parts.map(\.part_id)))
        XCTAssertTrue(MacJournalPartFlow.hasLiveReasoning(flow[0].parts, delivery: .running))
        XCTAssertFalse(MacJournalPartFlow.hasLiveReasoning(flow[0].parts, delivery: .completed))
        XCTAssertEqual(MacJournalPartFlow.delivery(flow[0].parts), .running)
        let appended = MacJournalPartFlow.segments([parts[0], parts[1], parts[2], parts[3], tool("next-tool"), parts[4]])
        XCTAssertEqual(appended.map(\.id), flow.map(\.id), "Appending a Tool retains the message disclosure identity")
        XCTAssertEqual(appended[0].parts.map(\.part_id), ["r1", "tool", "r2", "next-tool"])
    }
    func testPublicHistoricalAndStepAnchorsStayOutsideSingleDisclosure() {
        let parts = [reasoning("r1", state: "completed"),
            part("history", .historical(.init(schema: "message_part_view", canonical_kind: "text", canonical_revision: "4"))),
            reasoning("r2", state: "completed"),
            part("step", .step(.init(title: .inline("Step"), state: "completed"))),
            reasoning("r3", state: "running"), reasoning("r4", state: "running")]
        let flow = MacJournalPartFlow.segments(parts)
        XCTAssertEqual(flow.map { $0.parts.map(\.part_id) }, [["r1", "r2", "r3", "r4"], ["history"], ["step"]])
        XCTAssertEqual(Set(flow.flatMap(\.parts).map(\.part_id)), Set(parts.map(\.part_id)))
    }
    func testExecutionAnchorAndStableIdentityAcrossAppend() {
        let intro = part("intro", .text(.init(is_final: false, visibility: "public", review: nil)))
        let end = part("end", .text(.init(is_final: true, visibility: "public", review: nil)))
        let before = MacJournalPartFlow.segments([intro, reasoning("r1", state: "running"), end])
        let after = MacJournalPartFlow.segments([intro, reasoning("r1", state: "completed"), end, tool("next")])
        XCTAssertEqual(after.map { $0.parts.map(\.part_id) }, [["intro"], ["r1", "next"], ["end"]])
        XCTAssertEqual(before[1].id, after[1].id)
        XCTAssertEqual(after.filter { $0.kind == .process }.count, 1)
        XCTAssertTrue(MacJournalPartFlow.segments([intro, end]).allSatisfy { $0.kind == .content })
        XCTAssertTrue(MacJournalPartFlow.segments([]).isEmpty)
    }

    @MainActor func testPerSegmentUserChoiceAndReadingKeepLiveDetailsVisibleAtTerminal() {
        let owner = MacJournalPresentationOwner()
        defer { owner.cancel() }
        let current = ["r2", "tool2"], earlier = ["r1"]
        XCTAssertTrue(AppleMessagePartFlow.isExpanded(choices: [Bool](), defaultExpanded: true))
        owner.retainProcessExpansion(partIDs: ["unrelated"], in: [])
        owner.retainProcessExpansion(partIDs: ["unrelated"], in: current)
        XCTAssertNil(owner.processChoices["unrelated"], "Only actual process children pin a segment")
        owner.retainProcessExpansion(partIDs: ["tool2"], in: current)
        XCTAssertTrue(AppleMessagePartFlow.isExpanded(choices: current.compactMap { owner.processChoices[$0] }, defaultExpanded: false))
        XCTAssertTrue(earlier.compactMap { owner.processChoices[$0] }.isEmpty)
        owner.processChoices["r2"] = false
        owner.retainProcessExpansion(partIDs: ["tool2"], in: current)
        XCTAssertFalse(AppleMessagePartFlow.isExpanded(choices: current.compactMap { owner.processChoices[$0] }, defaultExpanded: true))
        // A new leading Part changes segment identity but not the reader's choice.
        let prepended = ["new-leading"] + current
        XCTAssertFalse(AppleMessagePartFlow.isExpanded(choices: prepended.compactMap { owner.processChoices[$0] }, defaultExpanded: true))
        owner.processChoices["r2"] = true
        XCTAssertTrue(AppleMessagePartFlow.isExpanded(choices: prepended.compactMap { owner.processChoices[$0] }, defaultExpanded: false))

    }
    func testRunFailureRemainsSeparateFromEarlierCompletedProcessAndRealFailedTool() {
        let before = [reasoning("completed", state:"completed"), tool("ok")]
        let failed = [reasoning("later", state:"failed"), tool("failed",state:"failed")]
        XCTAssertEqual(MacJournalRunStatus.attentionState("failed"),"failed")
        XCTAssertEqual(MacJournalPartFlow.delivery(before),.completed)
        XCTAssertFalse(MacJournalProcessProjection(before).requiresAttention)
        XCTAssertFalse(MacJournalPartFlow.hasLiveReasoning(before,delivery:.failed))
        XCTAssertTrue(MacJournalProcessProjection(failed).requiresAttention)
        XCTAssertEqual(MacJournalPartFlow.delivery(failed),.failed)
    }

    func testNonJournalCanonicalMessagePreservesTypedFlowAndIsNotAnArchiveField() throws {
        typealias Part = MacCanonicalConversationProjectionV1.Part
        let surface = AppleUISurfaceIdentity(owner:.init(authority_id:"a",conversation_id:"child",participant_id:"expert",
            context_id:"context",originating_run_id:"run",message_id:"assistant",part_id:"tool"),surface_id:"form",generation:1)
        let toolJSON:[String:Any] = ["kind":"tool","execution_source":["kind":"model"],"part_id":"tool","revision":1,
            "tool_call_id":"call","capability_id":"render_ui","title":"Read","state":"completed","input":[:],
            "origin":["kind":"builtin"],"artifacts":[],"affected_resources":[]]
        let tool = try JSONDecoder().decode(Part.self,from:JSONSerialization.data(withJSONObject:toolJSON))
        let parts:[Part] = [.reasoning(.init(partID:"r1",revision:1,state:.completed,title:"R",summary:"One")),tool,
            .uiSurface(.init(partID:"surface",revision:1,surface:.init(identity:surface,revision:1),summary:"Form")),
            .reasoning(.init(partID:"r2",revision:1,state:.running,title:"R",summary:"Two")),
            .text(.init(partID:"text",revision:1,markdown:"Public",isFinal:false,review:.approved)),
            .error(.init(partID:"error",revision:1,code:"provider_failed",displayKey:"Error",recoverable:true))]
        let canonical = MacCanonicalConversationProjectionV1.Message(messageID:"assistant",role:.assistant,state:.running,
            revision:1,createdAtMs:1,parts:parts)
        let projected = try MacCanonicalConversationRepository.projectMessage(canonical)
        XCTAssertNil(projected.journalReference)
        XCTAssertEqual(projected.canonicalPresentation?.message,canonical)
        let flow = MacCanonicalPartFlow.segments(try XCTUnwrap(projected.canonicalPresentation).message.parts)
        XCTAssertEqual(flow.map { $0.parts.map(\.partID) }, [["r1","tool","r2"],["surface"],["text"],["error"]])
        let encoded = try XCTUnwrap(JSONSerialization.jsonObject(with:JSONEncoder().encode(projected)) as? [String:Any])
        XCTAssertNil(encoded["canonicalPresentation"])
        let restored = try JSONDecoder().decode(MacChatMessage.self,from:JSONEncoder().encode(projected))
        XCTAssertNil(restored.canonicalPresentation)
    }

    func testCurrentTypedSealedActivityIsNeutralAndUnknownActivityStillRejects() throws {
        let raw:[String:Any] = ["kind":"task","part_id":"task","revision":1,"task_id":"task-id",
            "display_key":"Delegated task","state":"sealed","cancel_requested":false]
        let part = try JSONDecoder().decode(MacCanonicalConversationProjectionV1.Part.self,
            from:JSONSerialization.data(withJSONObject:raw))
        guard case .task(let task) = part else { return XCTFail("Task DTO") }
        XCTAssertEqual(task.state,.sealed)
        XCTAssertNil(MacCanonicalPartFlow.delivery([.sealed]))
        XCTAssertNil(MacJournalPartFlow.delivery([reasoning("sealed",state:"sealed")]))
        let message = MacCanonicalConversationProjectionV1.Message(messageID:"sealed-message",role:.assistant,
            state:.completed,revision:1,createdAtMs:1,parts:[part])
        let projected = try MacCanonicalConversationRepository.projectMessage(message)
        XCTAssertEqual(projected.presentation?.traceItems.first?.state,.incomplete,
            "Sealed settles ownership; it does not prove successful task completion")
        var unknown=raw;unknown["state"]="future_unclassified"
        XCTAssertThrowsError(try JSONDecoder().decode(MacCanonicalConversationProjectionV1.Part.self,
            from:JSONSerialization.data(withJSONObject:unknown)))
    }

    @MainActor func testMessageReaderDismissAndOpenCallbacksCannotCrossPresentationEpoch() {
        let owner = MacJournalPresentationOwner()
        let selection = MacHistoricalPartSelection(owner: .init(conversationID:"A",messageID:"message",partID:"part"),
            canonicalRevision:1,windowSequence:1)
        let epochA = owner.readerEpoch
        let first = MacMessageReaderSelection.historical(.init(selections:[selection],reader:nil))
        owner.openReader(first,epoch:epochA)
        let second = MacMessageReaderSelection.historical(.init(selections:[selection],reader:nil))
        owner.openReader(second,epoch:epochA)
        owner.updateReader(nil,replacing:first.id,epoch:epochA)
        XCTAssertEqual(owner.readerSelection?.id,second.id,"A late dismiss cannot clear a newer sheet")
        owner.invalidateReaders(epoch:epochA)
        let returnedEpochA = owner.readerEpoch
        let returned = MacMessageReaderSelection.historical(.init(selections:[selection],reader:nil))
        owner.openReader(returned,epoch:returnedEpochA)
        owner.openReader(first,epoch:epochA)
        owner.updateReader(nil,replacing:returned.id,epoch:epochA)
        owner.invalidateReaders(epoch:epochA)
        XCTAssertEqual(owner.readerSelection?.id,returned.id,"A→B→A never revives old open/dismiss/disappear callbacks")
        owner.updateReader(nil,replacing:returned.id,epoch:returnedEpochA)
        XCTAssertNil(owner.readerSelection)
        let db=URL(fileURLWithPath:"/tmp/reader-source.db")
        let local=MacCanonicalReadingOwner(conversationID:"A",messageID:"message",
            source:.init(databaseURL:db,sourceOwner:.local))
        XCTAssertNotEqual(local,MacCanonicalReadingOwner(conversationID:"A",messageID:"message",
            source:.init(databaseURL:db,sourceOwner:.remote(hostID:"remote",storeEpoch:"epoch"))))
        XCTAssertNotEqual(local,MacCanonicalReadingOwner(conversationID:"A",messageID:"message",
            source:.init(databaseURL:URL(fileURLWithPath:"/tmp/other.db"),sourceOwner:.local)))
    }

}
