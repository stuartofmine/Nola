// SPDX-License-Identifier: Apache-2.0
//! Store-boundary fixtures submit original current-protocol facts, never a
//! status-only completion or a fabricated canonical result DTO.
use super::NolaStore;
use nola_store_port::run_effect::{CheckpointWrite, CommitTransitionInput, RunEventInput};
use nola_store_port::assistant_journal::*;
use nola_domain::assistant_journal::*;
use nola_domain::assistant_journal_operations::*;
use serde_json::json;

// Included by both Store unit tests and integration fixtures; only the former
// use this complete-turn convenience function.
#[allow(dead_code)]
pub fn settle_fixture(store: &mut NolaStore, conversation: &str, run: &str, turn: &str, user: &str) {
    let revision = store.load_checkpoint(conversation).unwrap().unwrap().revision;
    let mut input = CommitTransitionInput {
        transition_id: format!("{run}-fixture-terminal"), run_id: run.into(),
        expected_checkpoint_revision: revision, checkpoint_revision: revision+1,
        checkpoint_event_cursor: 0, checkpoint: CheckpointWrite::Baseline { checkpoint_json: "{}".into() },
        submission_reduction_json: None, host_event: None, events: vec![], ensured_messages: vec![],
        assistant_turn_bindings: vec![], part_message_owners: Default::default(), part_mutations: vec![],
        outbox_effects: vec![], model_attempt_mutations: vec![], delegated_invocation_mutations: vec![],
        delegated_permission_mutations: vec![], effect_delivery_attempt_mutations: vec![],
        acknowledged_effect_ids: vec![], memory_usage_entries: vec![], run_status: None,
        assistant_status: Some("completed".into()), assistant_status_message_id: None, committed_at_ms: revision+1,
    };
    add_sources(store, &mut input, conversation, turn, user, false);
    store.begin_write_unit_of_work().unwrap();
    assert!(store.commit_transition(&input).unwrap().applied);
    publish_and_seal(store, &input, conversation);
    store.commit_write_unit_of_work().unwrap();
}

pub fn add_sources(store:&NolaStore, input:&mut CommitTransitionInput,
    conversation:&str, turn:&str, user:&str, cancelled:bool) {
    let run=input.run_id.clone();
    // These independent synthetic Core admissions do not replay a prior
    // Core input registry. Give each its own session instead of resetting
    // accepted_sequence to one inside a shared session.
    let core_session = format!("fixture-core:{run}");
    let next=store.active_run(conversation).unwrap().unwrap().next_event_sequence;
    let revision=input.checkpoint_revision as u64;
    let cursor_base = store.load_checkpoint(conversation).unwrap().unwrap().event_cursor as u64;
    let admission=format!("{run}:accepted");
    let scope=format!("{run}:scope");
    let requirement=format!("{run}:requirement");
    let fact=format!("{run}:cause");
    let cancellation=format!("{run}:cancel");
    let assistant=store.assistant_message_id_for_run(&run,conversation).unwrap().unwrap();
    let applied=json!([{"admission_id":admission,"turn_id":turn,"applied_sequence":"1"}]);
    let value=if cancelled {
        input.submission_reduction_json=Some(serde_json::to_string(&nola_store_port::run_effect::SubmissionReplayReduction {
            protocol_version:1,session_id:core_session.clone(),decision_time_ms:input.committed_at_ms,
            submission_json:json!({"type":"interrupt","cancellation_id":cancellation,"reason":"fixture user cancellation"}).to_string(),
            bindings:vec![],
        }).unwrap());
        json!({"kind":"cancelled","cancellation_id":cancellation,"observed_effect_refs":[],"unknown_effect_refs":[]})
    } else {
        json!({"kind":"incomplete","reason":{"code":"fixture.remaining","detail":"Fixture explicitly retains its unfinished requirement"},
            "remaining_requirement_refs":[requirement],"obligation_refs":[fact]})
    };
    let kind=if cancelled {json!({"kind":"cancellation","cancellation_id":cancellation,"reason":"fixture user cancellation"})}
        else {json!({"kind":"error","code":"fixture.unfinished","detail":"Further work remains"})};
    let payloads=[
        json!({"type":"turn_started","turn_id":turn,"assistant_message_id":assistant}),
        json!({"type":"input_accepted","admission":{"admission_id":admission,"submission_id":format!("{run}:submission"),
            "accepted_sequence":1,"origin":"interactive_user","delivery":"current_turn","anchor_turn_id":null,
            "payload_digest":"a".repeat(64),"item_ids":[user]}}),
        json!({"type":"input_applied","turn_id":turn,"applications":[{"admission_id":admission,"destination_turn_id":turn,"applied_sequence":1}]}),
        json!({"type":"requirement_scope_changed","turn_id":turn,"scope":{"scope_id":scope,"revision":"1",
            "applied_input_refs":applied,"source_input_refs":[user],"requirements":[{"requirement_id":requirement,
            "description":"Original accepted request","source_refs":[user],"dependency_ids":[],"goal_requirement_ref":null,"resource_scope":{"kind":"undeclared"}}],
            "source":{"kind":"runtime","code":"input_applied","source_refs":[admission]}}}),
        json!({"type":"runtime_decision_fact_observed","turn_id":turn,"fact":{"fact_id":fact,"kind":kind}}),
        json!({"type":"run_result_decided","turn_id":turn,"decision":{"result_id":format!("{run}:result"),
            "scope_id":scope,"requirement_revision":"1","applied_input_refs":applied,"evidence_frontier":(cursor_base+5).to_string(),
            "value":value,"source":{"kind":"runtime","code":"fixture_terminal","source_refs":[fact]}}}),
    ];
    assert!(input.events.is_empty());
    input.events=payloads.into_iter().enumerate().map(|(index,mut payload)|{
        let cursor=cursor_base+index as u64+1;
        payload["core_envelope"]=json!({"session_id":core_session,"turn_id":turn,"revision":revision,"cursor":{"sequence":cursor}});
        RunEventInput {event_id:format!("core:{run}:{cursor}"),sequence:next+index as i64,
            event_kind:payload["type"].as_str().unwrap().into(),payload_json:payload.to_string(),created_at_ms:input.committed_at_ms}
    }).collect();
    input.checkpoint_event_cursor=(cursor_base+6) as i64;
    input.checkpoint=CheckpointWrite::Baseline {checkpoint_json:json!({"session_id":core_session,"revision":revision,
        "last_event_cursor":{"sequence":cursor_base+6},"state":{"session_id":core_session,"task_projection":{"tasks":{}}}}).to_string()};
    input.assistant_turn_bindings.push(nola_domain::AssistantTurnBinding::TurnStarted {
        turn_id:turn.into(),assistant_message_id:assistant,
    });
    input.run_status=Some("sealed".into());
}

/// Publishes the fixture's actual completed tool Part under its distinct
/// assistant owner. It is neither a second answer Text nor a forged Run result.
#[allow(dead_code)]
pub fn publish_completed_tool(store:&mut NolaStore,input:&CommitTransitionInput,conversation:&str,
    assistant:&str,part_id:&str,call_id:&str,capability_id:&str) {
    let message=store.load_conversation(conversation).unwrap().unwrap().messages.into_iter()
        .find(|message|message.id==assistant).unwrap();
    assert_eq!(message.run_id.as_deref(),Some(input.run_id.as_str()));
    assert_eq!(message.status,"completed");
    assert!(message.completed_at_ms.is_some());
    let part=message.parts.into_iter().find(|part|part.id==part_id).unwrap();
    assert_eq!(part.kind,"tool");
    assert_eq!(part.state,"completed");
    let identity=AssistantJournalIdentity {source_store_epoch:store.assistant_source_epoch().unwrap(),
        conversation_id:conversation.into(),run_id:input.run_id.clone(),assistant_message_id:assistant.into()};
    let expected=store.assistant_journal_head(&identity).unwrap();
    assert_eq!(expected.sequence,0);
    // A terminal Tool cannot accept later Append operations. Prepare both
    // canonical bodies in the enclosing UOW, then publish its complete head.
    let output=store.prepare_assistant_content(&identity,part_id,
        &empty_journal_content(&identity,part_id,JournalContentField::ToolOutputText,JournalNumber(1)).unwrap(),
        &[AssistantTextChunkV1::new(part.content_text).unwrap()]).unwrap();
    let artifacts=store.prepare_assistant_content(&identity,part_id,
        &empty_journal_content(&identity,part_id,JournalContentField::ToolArtifacts,JournalNumber(1)).unwrap(),
        &[AssistantTextChunkV1::new(
            nola_domain::assistant_journal_content::encode_assistant_artifact_content(&[]).unwrap()
        ).unwrap()]).unwrap();
    store.commit_assistant_journal(&CommitAssistantJournal {
        stamp:AssistantSourceCommitStamp::Transition {transition_id:input.transition_id.clone()},expected,
        batch:AssistantOperationBatchV1 {protocol_version:1,identity:identity.clone(),operations:vec![
            AssistantJournalOperationV1::Declare(DeclareJournalPart {part:JournalPartHead {
                identity,part_id:part_id.into(),generation:JournalNumber(1),revision:JournalNumber(1),metadata_revision:JournalNumber(1),
                order_key:JournalOrderKey {section:JournalSection::Transcript,position:JournalNumber(0)},
                metadata:JournalPartMetadata::Tool(JournalToolMetadata {
                    execution_source: Default::default(),
                    tool_call_id:call_id.into(),capability_id:capability_id.into(),title:"Read source.rs".into(),summary:None,
                    origin:JournalToolOrigin {kind:nola_domain::HostCapabilityOriginKind::Builtin,
                        provider_id:None,provider_title:None,namespace:None},permission:None,
                    state:nola_domain::ActivityState::Completed,display_key:None,affected_resources:vec![].into(),
                    artifacts:JournalContentField::ToolArtifacts,full_output:None,
                    started_at_ms:Some(JournalNumber(part.created_at_ms as u64)),
                    finished_at_ms:Some(JournalNumber(part.updated_at_ms as u64)),
                }),bodies:vec![output,artifacts],
            }}),
        ]},supplied_chunks:vec![],
    }).unwrap();
}

pub fn publish_and_seal(store:&mut NolaStore,input:&CommitTransitionInput,conversation:&str) {
    let assistant=store.assistant_message_id_for_run(&input.run_id,conversation).unwrap().unwrap();
    let identity=AssistantJournalIdentity {source_store_epoch:store.assistant_source_epoch().unwrap(),
        conversation_id:conversation.into(),run_id:input.run_id.clone(),assistant_message_id:assistant.clone()};
    let expected=store.assistant_journal_head(&identity).unwrap();
    if expected.sequence==0 {
        let message=store.load_conversation(conversation).unwrap().unwrap().messages.into_iter()
            .find(|m|m.id==assistant).unwrap();
        let mut operations=Vec::new();
        let mut supplied_chunks=Vec::new();
        for (position,part) in message.parts.into_iter().filter(|p|p.kind=="text").enumerate() {
            let empty=empty_journal_content(&identity,&part.id,JournalContentField::Markdown,JournalNumber(1)).unwrap();
            let mut refs=Vec::new();
            // Keep chunks below the protocol limit without splitting UTF-8.
            let mut text=String::new();
            for ch in part.content_text.chars() {
                if text.len()+ch.len_utf8()>4096 {
                    let chunk=AssistantTextChunkV1::new(std::mem::take(&mut text)).unwrap();
                    refs.push(JournalChunkRef {digest:chunk.digest(),utf8_bytes:JournalNumber(chunk.byte_length() as u64)});
                    supplied_chunks.push(chunk);
                }
                text.push(ch);
            }
            if !text.is_empty() {
                let chunk=AssistantTextChunkV1::new(text).unwrap();
                refs.push(JournalChunkRef {digest:chunk.digest(),utf8_bytes:JournalNumber(chunk.byte_length() as u64)});
                supplied_chunks.push(chunk);
            }
            let part_id=part.id.clone();
            let appended = !refs.is_empty();
            operations.push(AssistantJournalOperationV1::Declare(DeclareJournalPart {part:JournalPartHead {
                identity:identity.clone(),part_id:part.id,generation:JournalNumber(1),revision:JournalNumber(1),metadata_revision:JournalNumber(1),
                order_key:JournalOrderKey {section:JournalSection::Transcript,position:JournalNumber(position as u64)},
                metadata:JournalPartMetadata::Text(JournalTextMetadata {is_final:!appended,visibility:JournalVisibility::Public,review:None}),
                bodies:vec![empty.clone()],
            }}));
            if appended {
                operations.push(AssistantJournalOperationV1::Append(AppendJournalText {
                    part_id:part_id.clone(),expected_generation:JournalNumber(1),expected_revision:JournalNumber(1),
                    resulting:append_journal_content(&empty,&refs).unwrap(),expected:empty,chunks:refs,
                }));
            }
            if appended { operations.push(AssistantJournalOperationV1::Update(UpdateJournalMetadata {
                part_id,expected_generation:JournalNumber(1),expected_revision:JournalNumber(2),
                metadata:JournalPartMetadata::Text(JournalTextMetadata {is_final:true,visibility:JournalVisibility::Public,review:None}),
            })); }
        }
        store.commit_assistant_journal(&CommitAssistantJournal {stamp:AssistantSourceCommitStamp::Transition {transition_id:input.transition_id.clone()},
            expected,batch:AssistantOperationBatchV1 {protocol_version:1,identity,operations},supplied_chunks}).unwrap();
    }
    store.seal_assistant_run_terminal(&input.transition_id).unwrap().unwrap();
}
