// SPDX-License-Identifier: Apache-2.0
//! Original provider IDs can repeat; accepted Part ownership selects sources.
use super::*;
use nola_agent_engine::agent::host::*;
use nola_agent_engine::agent::{AgentCoreConfig, AgentInputDelivery, AgentSubmission, ItemId, SessionId};
use nola_agent_engine::agent::parts::PartUpdate;
use nola_agent_engine::agent::recovery::CoreCheckpoint;
use nola_store::NolaStore;
use nola_store_port::assistant_journal::{AssistantJournalRepository,AssistantSourceCommitStamp};
use serde_json::json;
#[path = "../tests/support/model_identity.rs"] mod model_identity;
#[path = "../tests/support/admission_frontier.rs"] mod admission_frontier;
const CONVERSATION:&str="tool-source-scope";
const CALL:&str="provider-reused-call";
fn config()->AgentCoreConfig {
    let mut capabilities=std::collections::BTreeMap::new();
    for (name,route) in [("model.stream",HostCapabilityRoute::Model),("test.read",HostCapabilityRoute::Tool),("test.serial",HostCapabilityRoute::Tool)] {
        capabilities.insert(name.into(),HostCapabilityDescriptor{id:name.into(),title:name.into(),route,
            availability:CapabilityAvailability::Available,exposure:HostCapabilityExposure::Visible,permission_domain:None,
            input_schema:json!({"type":"object"}),input_normalization:HostInputNormalization{permission_contract:nola_domain::permission_resource_plan::PermissionOperationContract{risk:nola_domain::permission_resource_plan::PermissionOperationRisk::OrdinaryRead,..Default::default()},..Default::default()},
            output_policy:HostOutputPolicy{preview_char_limit:4000,persist_full_output:true,allow_artifacts:true},supports_streaming:true,supports_argument_streaming:false,parallelism:HostCapabilityParallelism::Serial,cancellation:HostCancellationMode::Cooperative,replay_safety:HostReplaySafety::Recoverable});
    }
    AgentCoreConfig{session_id:Some(SessionId::from_string(CONVERSATION.into())),turn_identity:model_identity::identity(),model:Some("test-model".into()),
        tools:["test.read","test.serial"].map(|name|json!({"type":"function","function":{"name":name,"parameters":{"type":"object"}}})).to_vec(),
        host_catalog:Some(HostCapabilityCatalog{host_id:HostId("scope-host".into()),revision:1,availability_epoch:1,lease:None,capabilities}),..Default::default()}
}
fn begin(store:&mut NolaStore,id:&str,now:i64)->crate::AcceptedAssistantTurn {
    let frontier=admission_frontier::read(store,CONVERSATION);
    crate::begin_assistant_turn(store,crate::BeginAssistantTurn{conversation_id:CONVERSATION.into(),expected_configuration_revision:frontier.0,expected_parent_message_id:frontier.1,
        conversation_title:"Source scope".into(),user_message_id:format!("{id}:user"),user_part_id:format!("{id}:user:text"),user_text:"Inspect".into(),user_content_json:None,
        assistant_message_id:format!("{id}:assistant"),context_checkpoint_id:None,core_config:config(),created_at_ms:now,
        submission:AgentSubmission::UserInput{items:vec![AgentItem::user_message(ItemId::from_string(format!("{id}:input")),"Inspect")],turn_context_items:vec![],turn_context_platform:Default::default(),turn_context_components:vec![],submission_id:Some(id.into()),submitted_at_epoch_ms:Some(now),delivery:AgentInputDelivery::CurrentTurn}}).unwrap()
}
fn claim(store:&mut NolaStore,turn:&crate::AcceptedAssistantTurn,now:i64)->HostRequestEnvelope {
    crate::claim_committed_host_requests(store,crate::ClaimCommittedHostRequests{run_id:turn.begin.run_id.clone(),worker_generation:"scope-worker".into(),lease_owner:"scope-owner".into(),claimed_at_ms:now,lease_expires_at_ms:now+60_000,limit:1}).unwrap().requests.remove(0)
}
fn accept(store:&mut NolaStore,turn:&crate::AcceptedAssistantTurn,request:&HostRequestEnvelope,sequence:u64,payload:HostResponse,now:i64){
    crate::reduce_and_accept_host_response(store,crate::ReduceAndAcceptHostResponse{conversation_id:CONVERSATION.into(),run_id:turn.begin.run_id.clone(),invocation_config_digest:turn.invocation_config_digest.clone(),received_at_ms:now,
        response:HostResponseEnvelope{request_id:request.request_id.clone(),attempt_id:request.attempt_id.clone(),delegated_invocation_id:request.delegated_invocation_id.clone(),effect_id:request.effect_id.clone(),host_id:request.host_id.clone(),lease_id:request.lease_id.clone(),session_id:request.session_id.clone(),turn_id:request.turn_id.clone(),catalog_revision:request.catalog_revision,availability_epoch:request.availability_epoch,delivery:request.delivery.clone(),sequence,payload}}).unwrap_or_else(|error|panic!("accept fixture response at {now}, sequence {sequence}: {error}"));
}
fn sample(store:&mut NolaStore,turn:&crate::AcceptedAssistantTurn,now:i64)->HostRequestEnvelope {
    let request=claim(store,turn,now);assert!(matches!(request.payload,HostRequest::ModelStream{..}));
    accept(store,turn,&request,1,HostResponse::ExecutionStarted,now+1);
    accept(store,turn,&request,2,HostResponse::ModelAttemptStarted{provider:"fixture-provider".into(),model:"test-model".into(),reason:"initial".into()},now+2);
    request
}
fn tool(store:&mut NolaStore,turn:&crate::AcceptedAssistantTurn,name:&str,now:i64){tool_with_delta(store,turn,name,now,false)}
fn tool_with_delta(store:&mut NolaStore,turn:&crate::AcceptedAssistantTurn,name:&str,now:i64,streamed:bool){
    let model=sample(store,turn,now);
    if streamed {
        accept(store,turn,&model,3,HostResponse::ModelToolCallDelta{part_id:format!("{}:tool:{CALL}",model.effect_id.0),call_id:CALL.into(),capability_id:name.into(),arguments_delta:"{}".into()},now+3);
    }
    accept(store,turn,&model,if streamed{4}else{3},HostResponse::ModelCompleted{items:vec![],tool_calls:vec![json!({"call_id":CALL,"name":name,"arguments":{}})],usage:json!({})},now+4);
    let request=claim(store,turn,now+5);assert!(matches!(&request.payload,HostRequest::ExecuteTool{capability_id,..} if capability_id==name));
    accept(store,turn,&request,1,HostResponse::ExecutionStarted,now+6);
    accept(store,turn,&request,2,HostResponse::ToolCompleted{call_id:CALL.into(),output:json!({"result":name}),outcome:HostToolOutcome::Succeeded,activation:None,full_output_ref:None,artifacts:vec![]},now+7);
}
fn finish(store:&mut NolaStore,turn:&crate::AcceptedAssistantTurn,now:i64){
    let request=sample(store,turn,now);let HostRequest::ModelStream{text_part_id,..}=&request.payload else{unreachable!()};
    accept(store,turn,&request,3,HostResponse::ModelDelta{channel:ModelDeltaChannel::Text,part_id:text_part_id.clone(),delta:"Done".into()},now+3);
    accept(store,turn,&request,4,HostResponse::ModelCompleted{items:vec![],tool_calls:vec![],usage:json!({})},now+4);
    assert!(store.local_assistant_run_terminal_seal(CONVERSATION,&turn.begin.run_id).unwrap().is_some());
}
fn state(store:&NolaStore)->AgentThreadState {
    let checkpoint=crate::load_current_checkpoint(store,CONVERSATION).unwrap().unwrap();
    serde_json::from_str::<CoreCheckpoint<AgentThreadState>>(&checkpoint.checkpoint_json).unwrap().state
}
fn count(path:&std::path::Path)->i64{rusqlite::Connection::open(path).unwrap().query_row("SELECT COUNT(*) FROM canonical_tool_sources",[],|row|row.get(0)).unwrap()}
#[test]
fn real_store_two_runs_reuse_upstream_call_id_with_different_capabilities_and_keep_exact_sources(){
    let dir=tempfile::tempdir().unwrap();let path=dir.path().join("scope.db");let mut store=NolaStore::open(&path).unwrap();
    let first=begin(&mut store,"first",1000);tool(&mut store,&first,"test.read",1100);finish(&mut store,&first,1200);
    drop(store);let mut store=NolaStore::open(&path).unwrap();
    let second=begin(&mut store,"second",2000);tool(&mut store,&second,"test.serial",2100);finish(&mut store,&second,2200);
    assert_eq!(count(&path),4);
    let connection=rusqlite::Connection::open(&path).unwrap();
    let mut statement=connection.prepare("SELECT json_extract(binding_json,'$.name'),run_id,message_id FROM canonical_tool_sources ORDER BY json_extract(binding_json,'$.name'),kind").unwrap();
    let rows=statement.query_map([],|r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?,r.get::<_,String>(2)?))).unwrap().collect::<Result<Vec<_>,_>>().unwrap();
    assert_eq!(rows.iter().filter(|row|row.0=="test.read"&&row.1==first.begin.run_id&&row.2=="first:assistant").count(),2);
    assert_eq!(rows.iter().filter(|row|row.0=="test.serial"&&row.1==second.begin.run_id&&row.2=="second:assistant").count(),2);
}
#[test]
fn current_source_owner_provenance_capability_duplicates_and_incomplete_pairs_still_reject_without_writes(){
    let dir=tempfile::tempdir().unwrap();let path=dir.path().join("invalid.db");let mut store=NolaStore::open(&path).unwrap();
    let turn=begin(&mut store,"current",1000);tool(&mut store,&turn,"test.read",1100);
    let original=state(&store);let part=original.conversation.projection.parts.values().find(|part|matches!(part,AgentPart::Tool(_))).unwrap().clone();
    let AgentPart::Tool(tool_part)=&part else{unreachable!()};let turn_id=tool_part.ownership.turn_id.clone();
    let before=count(&path);
    for mutation in ["owner","missing-owner","capability","provenance","duplicate","missing-call","missing-result","foreign-message"] {
        let mut candidate=original.clone();let mut changed=part.clone();
        let ids=candidate.conversation.transcript.iter().filter(|item|matches!(item,AgentItem::ToolCall{call_id,..}|AgentItem::ToolResult{call_id,..} if call_id.as_str()==CALL)).map(|item|item.id().as_str().to_owned()).collect::<Vec<_>>();
        let call=ids.iter().find(|id|candidate.conversation.transcript.iter().any(|item|item.id().as_str()==*id&&matches!(item,AgentItem::ToolCall{..}))).unwrap().clone();
        match mutation {
            "owner"=>{let AgentPart::Tool(part)=&mut changed else{unreachable!()};part.ownership.turn_id="foreign".into();},
            "missing-owner"=>{candidate.item_ownership.remove(&call);},
            "capability"=>{for item in &mut candidate.conversation.transcript {if item.id().as_str()==call {let AgentItem::ToolCall{name,..}=item else{unreachable!()};*name="foreign-capability".into();}}},
            "provenance"=>{let value=candidate.item_ownership.get_mut(&call).unwrap();value.source=nola_domain::code_mode::CodeItemSource::CodeCell{cell:nola_domain::code_mode::CodeCellIdentity{host_id:"foreign-host".into(),conversation_id:CONVERSATION.into(),turn_id:turn_id.clone(),parent_call_id:"foreign-parent".into(),cell_id:"foreign-cell".into(),generation:"foreign-generation".into()}};},
            "duplicate"=>{let mut item=candidate.conversation.transcript.iter().find(|item|item.id().as_str()==call).unwrap().clone();let AgentItem::ToolCall{id,..}=&mut item else{unreachable!()};*id=ItemId::from_string("duplicate-item".into());candidate.item_ownership.insert("duplicate-item".into(),candidate.item_ownership[&call].clone());candidate.conversation.transcript.push(item);},
            "missing-call"=>candidate.conversation.transcript.retain(|item|item.id().as_str()!=call),
            "missing-result"=>candidate.conversation.transcript.retain(|item|!matches!(item,AgentItem::ToolResult{call_id,..} if call_id.as_str()==CALL)),
            "foreign-message"=>{for id in &ids {candidate.item_ownership.get_mut(id).unwrap().message_id="foreign-message".into();}},
            _=>unreachable!(),
        }
        let update=PartUpdate::Created{part:changed.clone()};let changes=AssistantJournalChanges{changes:vec![AssistantJournalChange::Part{turn_id:&turn_id,update:&update,part:&changed}],attempt_predecessors:vec![]};
        store.begin_write_unit_of_work().unwrap();
        let error=bind_changed(&mut store,&AssistantSourceCommitStamp::Admission{run_id:turn.begin.run_id.clone()},CONVERSATION,&candidate,&changes).unwrap_err();
        assert!(error.contains("ownership")||error.contains("Part and Turn")||error.contains("capability identity")||error.contains("provenance")||error.contains("duplicate transcript")||error.contains("lacks its original"),"{mutation}: {error}");
        store.rollback_write_unit_of_work().unwrap();assert_eq!(count(&path),before,"{mutation}");
    }
}

#[test]
fn same_run_new_assistant_segment_keeps_reused_call_sources_separate(){
    let dir=tempfile::tempdir().unwrap();let path=dir.path().join("segments.db");let mut store=NolaStore::open(&path).unwrap();
    let turn=begin(&mut store,"segmented",1000);tool(&mut store,&turn,"test.read",1100);
    crate::reduce_and_commit_active_submission(&mut store,crate::ReduceAndCommitActiveSubmission{
        conversation_id:CONVERSATION.into(),run_id:turn.begin.run_id.clone(),transition_id:"new-input-boundary".into(),invocation_config_digest:turn.invocation_config_digest.clone(),committed_at_ms:1150,
        submission:AgentSubmission::UserInput{items:vec![AgentItem::user_message(ItemId::from_string("new-input".into()),"Continue in the same Run")],turn_context_items:vec![],turn_context_platform:Default::default(),turn_context_components:vec![],submission_id:Some("new-input".into()),submitted_at_epoch_ms:Some(1150),delivery:AgentInputDelivery::CurrentTurn}}).unwrap();
    let request=sample(&mut store,&turn,1200);let HostRequest::ModelStream{text_part_id,..}=&request.payload else{unreachable!()};
    accept(&mut store,&turn,&request,3,HostResponse::ModelDelta{channel:ModelDeltaChannel::Text,part_id:text_part_id.clone(),delta:"Before follow-up".into()},1203);
    accept(&mut store,&turn,&request,4,HostResponse::ModelCompleted{items:vec![],tool_calls:vec![],usage:json!({})},1204);
    tool(&mut store,&turn,"test.serial",1300);finish(&mut store,&turn,1400);
    let connection=rusqlite::Connection::open(&path).unwrap();
    let rows:Vec<(String,String,String)>=connection.prepare("SELECT json_extract(binding_json,'$.name'),run_id,message_id FROM canonical_tool_sources ORDER BY kind").unwrap().query_map([],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).unwrap().collect::<Result<_,_>>().unwrap();
    assert_eq!(rows.len(),4);assert!(rows.iter().all(|row|row.1==turn.begin.run_id));
    let first=&rows.iter().find(|row|row.0=="test.read").unwrap().2;
    let next=&rows.iter().find(|row|row.0=="test.serial").unwrap().2;
    assert_ne!(first,next,"new accepted input owns a new assistant segment in the same Run");
}
