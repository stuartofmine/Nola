// SPDX-License-Identifier: Apache-2.0
//! Common Task worker proof from an actual expert tool effect, not forged metadata.
use super::*;
use nola_application::{ApplicationCommand,claim_committed_host_requests,ClaimCommittedHostRequests,reduce_and_accept_host_response,ReduceAndAcceptHostResponse};
use nola_application::expert_peer_task::{PreparePeerTask,PreparedPeerTask};
use nola_host_protocol::delegation::*;

fn accept(store:&mut NolaStore,accepted:&AcceptedAssistantTurn,request:&HostRequestEnvelope,sequence:u64,payload:HostResponse,now:i64) {
    reduce_and_accept_host_response(store,ReduceAndAcceptHostResponse{conversation_id:accepted.begin.conversation_id.clone(),run_id:accepted.begin.run_id.clone(),invocation_config_digest:accepted.invocation_config_digest.clone(),received_at_ms:now,
        response:HostResponseEnvelope{request_id:request.request_id.clone(),attempt_id:request.attempt_id.clone(),delegated_invocation_id:request.delegated_invocation_id.clone(),effect_id:request.effect_id.clone(),host_id:request.host_id.clone(),lease_id:request.lease_id.clone(),session_id:request.session_id.clone(),turn_id:request.turn_id.clone(),catalog_revision:request.catalog_revision,availability_epoch:request.availability_epoch,delivery:request.delivery.clone(),sequence,payload}}).unwrap();
}
fn claim(store:&mut NolaStore,run:&str,now:i64)->Vec<HostRequestEnvelope>{claim_committed_host_requests(store,ClaimCommittedHostRequests{run_id:run.into(),worker_generation:"bridge-test".into(),lease_owner:"bridge-owner".into(),claimed_at_ms:now,lease_expires_at_ms:100_000,limit:64}).unwrap().requests}
fn fixture(path:&std::path::Path)->(NolaStore,PeerMaterial,HostRequestEnvelope,AcceptedAssistantTurn) {
    let (mut store,initial)=setup_at(path);
    let target=initial.selected_participants[1].clone();
    let run=uuid::Uuid::new_v4().to_string();
    let mut cfg=config("parent");
    let schema=nola_domain::expert_task_target::tool_schema();cfg.tools.push(schema.clone());
    let mut desc=cfg.host_catalog.as_ref().unwrap().capabilities["model.stream"].clone();
    desc.id="delegate_expert".into();desc.route=HostCapabilityRoute::Tool;desc.exposure=HostCapabilityExposure::Visible;desc.input_schema=schema["function"]["parameters"].clone();desc.supports_streaming=false;desc.input_normalization.permission_contract.risk=nola_domain::permission_resource_plan::PermissionOperationRisk::OrdinaryRead;
    cfg.host_catalog.as_mut().unwrap().capabilities.insert("delegate_expert".into(),desc);
    let accepted=begin_real_child(&mut store,"parent",&run,"Review this together",cfg,vec![]).unwrap();
    let model=claim(&mut store,&accepted.begin.run_id,20).into_iter().find(|r|matches!(r.payload,HostRequest::ModelStream{..})).unwrap();
    let args=json!({"action":"request","target_participant_id":target.participant_id,"text":"Check this exact implementation"});
    accept(&mut store,&accepted,&model,1,HostResponse::ExecutionStarted,21);
    accept(&mut store,&accepted,&model,2,HostResponse::ModelAttemptStarted{provider:"fixture".into(),model:"original-model".into(),reason:"initial".into()},22);
    accept(&mut store,&accepted,&model,3,HostResponse::ModelCompleted{items:vec![],tool_calls:vec![json!({"call_id":"peer-task-call","name":"delegate_expert","arguments":args})],usage:json!({})},23);
    let tool=claim(&mut store,&accepted.begin.run_id,24).into_iter().find(|r|matches!(r.payload,HostRequest::ExecuteTool{..})).expect("actual expert tool effect");
    accept(&mut store,&accepted,&tool,1,HostResponse::ExecutionStarted,25);
    let input=PeerExchangeInput{source_context_id:"parent".into(),source_run_id:run,call_id:"peer-task-call".into(),target_participant_id:target.participant_id,kind:"request".into(),text:"Check this exact implementation".into(),in_reply_to:None};
    let material=peer_admit_at(&mut store,input,26).unwrap();(store,material,tool,accepted)
}
fn prepare(store:&mut NolaStore,material:&PeerMaterial,tool:&HostRequestEnvelope)->PreparedPeerTask { prepare_at(store,material,tool,26) }
fn prepare_at(store:&mut NolaStore,material:&PeerMaterial,tool:&HostRequestEnvelope,now:i64)->PreparedPeerTask {
    let cfg=config(&material.view.target.context_conversation_id);
    let snapshot=DelegatedInvocationSnapshot{process_sandbox:None,bounded_file_writes:false,project_repository_identity:None,worktree:None,sandbox_requirement:None,owner_session_id:material.view.source.context_conversation_id().to_owned(),execution_backend:"nola".into(),external_execution_model:None,external_execution_model_provider:None,
        profile:nola_domain::delegation::effective_agent_profiles(&Default::default())["general"].clone(),model:DelegatedModel{endpoint:"https://fixture.invalid/v1".into(),model:"original-model".into(),swe_mode:false},runtime:DelegatedRuntimeConfiguration{selected_profile:Some(nola_host_protocol::HostModelProfileReference{profile_id:"model".into(),admitted_revision:1}),..Default::default()},credential:None,fallback_credentials:vec![]};
    nola_application::expert_peer_task::prepare(store,PreparePeerTask{request:Box::new(tool.clone()),exchange_id:material.view.exchange_id.clone(),target_configuration:Box::new(cfg),target_invocation:Some(Box::new(snapshot)),project_root:"/target-own-workspace".into(),authority:authority()},now).unwrap()
}
#[test]
fn actual_expert_effect_promotes_common_task_and_worker_uses_persistent_target() {
    actual_promotion();
}
fn actual_promotion() {

    let dir=tempfile::tempdir().unwrap();let path=dir.path().join("bridge.db");
    let(mut store,material,tool,source)=fixture(&path);
    let prepared=prepare(&mut store,&material,&tool);
    assert!(!prepared.prepared.source_admitted);
    assert!(store.validated_expert_task_target(&material.view.exchange_id,&material.view.target.context_conversation_id,true).is_err());
    let recovered=nola_application::expert_peer::load_for_effect(&mut store,&tool,&authority(),27).unwrap().unwrap();assert_eq!(recovered.view,material.view);
    accept(&mut store,&source,&tool,2,*prepared.response,28);drop(store);
    let mut runtime=support::open_runtime(&path).unwrap();
    let wake=wake(&mut runtime,29).unwrap();

    assert_eq!(wake["candidates"].as_array().unwrap().len(),1,"expert worker wake does not require a source spawn-agent snapshot");
    let cfg:AgentCoreConfig=load_config(&mut runtime,&material.view.exchange_id);

    let mut forged=cfg.clone();forged.tools.push(json!({"type":"function","function":{"name":"foreign_tool","parameters":{"type":"object"}}}));
    assert!(worker_admit(&mut runtime,worker_input(&material,forged,30,1000,"forged")).is_err());

    let admitted:nola_application::mobile_delegation::AdmittedMobileTaskChild=serde_json::from_value(worker_admit(&mut runtime,worker_input(&material,cfg.clone(),30,1000,"bridge-generation-1")).unwrap()).unwrap();

    assert_eq!(admitted.expert_target.as_ref().unwrap().exchange_id,material.view.exchange_id);
    assert_eq!(admitted.invocation.run_id,material.view.exchange_id);
    assert_eq!(admitted.invocation.conversation_id,material.view.target.context_conversation_id);
    let mut observer=NolaStore::open(&path).unwrap();
    observer.begin_write_unit_of_work().unwrap();
    assert!(nola_application::expert_peer::clarify(&mut observer,PeerClarificationInput{source_context_id:material.view.target.context_conversation_id.clone(),source_run_id:material.view.exchange_id.clone(),call_id:"forbidden".into(),exchange_id:material.view.exchange_id.clone(),text:"Missing input".into()},&authority(),31).is_err());
    observer.commit_write_unit_of_work().unwrap();
    assert!(observer.validated_expert_task_target(&material.view.exchange_id,&material.view.target.context_conversation_id,true).unwrap().is_some());
    drop(observer);
    assert!(worker_admit(&mut runtime,worker_input(&material,cfg.clone(),31,1000,"live-thief")).unwrap().is_null());
    let recovered:nola_application::mobile_delegation::AdmittedMobileTaskChild=serde_json::from_value(worker_admit(&mut runtime,worker_input(&material,cfg,1001,2000,"recovered-worker")).unwrap()).unwrap();
    assert_eq!(recovered.invocation.run_id,admitted.invocation.run_id);
    assert_ne!(recovered.attempt.id,admitted.attempt.id);
    drop(runtime);
    let mut store=NolaStore::open(&path).unwrap();
    assert!(store.validated_expert_task_target(&material.view.exchange_id,&material.view.target.context_conversation_id,true).unwrap().is_some());
    assert_eq!(store.active_run(&material.view.target.context_conversation_id).unwrap().unwrap().run_id,material.view.exchange_id);
    let model=claim(&mut store,&material.view.exchange_id,1010).into_iter().find(|r|matches!(r.payload,HostRequest::ModelStream{..})).unwrap();
    let request_text=serde_json::to_string(&model.payload).unwrap();
    assert!(request_text.contains("main agent assigned this work"));assert!(request_text.contains(&material.view.exchange_id));
    for (i,payload) in [HostResponse::ExecutionStarted,HostResponse::ModelAttemptStarted{provider:"fixture".into(),model:"original-model".into(),reason:"initial".into()},HostResponse::ModelCompleted{items:vec![serde_json::to_value(AgentItem::assistant_message(ItemId::from_string("task-answer".into()),"Verified own target" )).unwrap()],tool_calls:vec![],usage:json!({})}].into_iter().enumerate(){
        accept_invocation(&mut store,&recovered.invocation,&model,i as u64+1,payload,1020+i as i64);
    }
    store.begin_write_unit_of_work().unwrap();
    store.reconcile_runtime_task_bound_run_terminal(&material.view.exchange_id,&material.view.exchange_id,1030).unwrap();
    store.commit_write_unit_of_work().unwrap();
    assert!(store.runtime_task_terminal_receipt(&material.view.exchange_id).unwrap().is_some());
    assert!(store.active_run(&material.view.target.context_conversation_id).unwrap().is_none());
    assert!(!store.is_runtime_task_child_session(&material.view.target.context_conversation_id).unwrap(),"persistent expert context stays an expert history after Task completion");
    let receipt=store.runtime_task_terminal_receipt(&material.view.exchange_id).unwrap().unwrap();drop(store);
    deliver_terminal(&path,&material,receipt.receipt_id);
    let mut store=NolaStore::open(&path).unwrap();
    let next_model=claim(&mut store,&source.begin.run_id,1040).into_iter().find(|r|matches!(r.payload,HostRequest::ModelStream{..})).unwrap();
    let args=json!({"action":"request","target_participant_id":material.view.target.participant_id,"text":"Preserve cancellation ownership", "in_reply_to":material.view.exchange_id});
    accept(&mut store,&source,&next_model,1,HostResponse::ExecutionStarted,1041);
    accept(&mut store,&source,&next_model,2,HostResponse::ModelAttemptStarted{provider:"fixture".into(),model:"original-model".into(),reason:"initial".into()},1042);
    accept(&mut store,&source,&next_model,3,HostResponse::ModelCompleted{items:vec![],tool_calls:vec![json!({"call_id":"clarification-continuation","name":"delegate_expert","arguments":args})],usage:json!({})},1043);
    let next_tool=claim(&mut store,&source.begin.run_id,1044).into_iter().find(|r|matches!(r.payload,HostRequest::ExecuteTool{..})).unwrap();
    accept(&mut store,&source,&next_tool,1,HostResponse::ExecutionStarted,1045);
    let continuation=peer_admit_at(&mut store,PeerExchangeInput{source_context_id:material.view.source.context_conversation_id().to_owned(),source_run_id:source.begin.run_id.clone(),call_id:"clarification-continuation".into(),target_participant_id:material.view.target.participant_id.clone(),kind:"request".into(),text:"Preserve cancellation ownership".into(),in_reply_to:Some(material.view.exchange_id.clone())},1046).unwrap();
    let next_prepared=prepare_at(&mut store,&continuation,&next_tool,1046);
    accept(&mut store,&source,&next_tool,2,*next_prepared.response,1047);drop(store);
    let mut runtime=support::open_runtime(&path).unwrap();
    let cfg:AgentCoreConfig=load_config(&mut runtime,&continuation.view.exchange_id);
    let next:nola_application::mobile_delegation::AdmittedMobileTaskChild=serde_json::from_value(worker_admit(&mut runtime,worker_input(&continuation,cfg,1050,2000,"continuation-worker")).unwrap()).unwrap();
    assert_eq!(next.invocation.conversation_id,admitted.invocation.conversation_id);
    assert_ne!(next.invocation.run_id,admitted.invocation.run_id);
    assert_eq!(next.expert_target.unwrap().exchange_id,continuation.view.exchange_id);
}

fn worker_input(material:&PeerMaterial,cfg:AgentCoreConfig,now:i64,until:i64,nonce:&str)->Box<nola_application::mobile_delegation::AdmitMobileTaskChild> {
    Box::new(nola_application::mobile_delegation::AdmitMobileTaskChild{task_id:material.view.exchange_id.clone(),parent_conversation_id:material.view.source.context_conversation_id().to_owned(),worker:nola_host_protocol::HostWorkerIdentity::Unobservable{nonce:nonce.into()},now_ms:now,lease_until_ms:until,current_tools:cfg.tools.clone(),candidate_config:cfg})
}
fn accept_invocation(store:&mut NolaStore,invocation:&nola_application::ExactRunInvocation,request:&HostRequestEnvelope,sequence:u64,payload:HostResponse,now:i64){
    reduce_and_accept_host_response(store,ReduceAndAcceptHostResponse{conversation_id:invocation.conversation_id.clone(),run_id:invocation.run_id.clone(),invocation_config_digest:invocation.invocation_config_digest.clone(),received_at_ms:now,response:HostResponseEnvelope{request_id:request.request_id.clone(),attempt_id:request.attempt_id.clone(),delegated_invocation_id:request.delegated_invocation_id.clone(),effect_id:request.effect_id.clone(),host_id:request.host_id.clone(),lease_id:request.lease_id.clone(),session_id:request.session_id.clone(),turn_id:request.turn_id.clone(),catalog_revision:request.catalog_revision,availability_epoch:request.availability_epoch,delivery:request.delivery.clone(),sequence,payload}}).unwrap();
}
#[test]
fn expert_task_cancel_uses_shared_worker_and_never_reports_a_requested_stop_as_completion(){
    {
        let dir=tempfile::tempdir().unwrap();let path=dir.path().join("cancel.db");
        let(mut store,material,tool,source)=fixture(&path);let prepared=prepare(&mut store,&material,&tool);
        accept(&mut store,&source,&tool,2,*prepared.response,28);drop(store);
        let mut runtime=support::open_runtime(&path).unwrap();
        let cfg:AgentCoreConfig=load_config(&mut runtime,&material.view.exchange_id);
        let admitted:nola_application::mobile_delegation::AdmittedMobileTaskChild=serde_json::from_value(worker_admit(&mut runtime,worker_input(&material,cfg.clone(),30,1000,"cancel-worker")).unwrap()).unwrap();drop(runtime);
        let mut store=NolaStore::open(&path).unwrap();
        let model=claim(&mut store,&material.view.exchange_id,40).into_iter().find(|r|matches!(r.payload,HostRequest::ModelStream{..})).unwrap();
        accept_invocation(&mut store,&admitted.invocation,&model,1,HostResponse::ExecutionStarted,41);
        accept_invocation(&mut store,&admitted.invocation,&model,2,HostResponse::ModelAttemptStarted{provider:"fixture".into(),model:"original-model".into(),reason:"initial".into()},42);
        let source_model=claim(&mut store,&source.begin.run_id,43).into_iter().find(|r|matches!(r.payload,HostRequest::ModelStream{..})).expect("request returns control to source model");
        accept(&mut store,&source,&source_model,1,HostResponse::ExecutionStarted,44);
        accept(&mut store,&source,&source_model,2,HostResponse::ModelAttemptStarted{provider:"fixture".into(),model:"original-model".into(),reason:"initial".into()},45);
        accept(&mut store,&source,&source_model,3,HostResponse::ModelCompleted{items:vec![],tool_calls:vec![json!({"call_id":"cancel-peer-call","name":"delegate_expert","arguments":{"action":"cancel","exchange_id":material.view.exchange_id}})],usage:json!({})},46);
        let cancel=claim(&mut store,&source.begin.run_id,47).into_iter().find(|r|matches!(r.payload,HostRequest::ExecuteTool{..})).unwrap();
        accept(&mut store,&source,&cancel,1,HostResponse::ExecutionStarted,48);
        let result=nola_application::expert_peer_task::action(&mut store,nola_application::expert_peer_task::PeerTaskAction{request:Box::new(cancel.clone()),authority:authority()},49).unwrap();
        assert_eq!(result["terminal"],false);assert_eq!(result["response"]["output"]["completion_confirmed"],false);
        accept(&mut store,&source,&cancel,2,serde_json::from_value(result["response"].clone()).unwrap(),50);
        assert!(store.runtime_task_terminal_receipt(&material.view.exchange_id).unwrap().is_none(),"cancel request cannot fake a completed provider stop");drop(store);
        let mut runtime=support::open_runtime(&path).unwrap();
        assert!(worker_admit(&mut runtime,worker_input(&material,cfg.clone(),51,1000,"cancel-live")).unwrap().is_null());
        let value=worker_admit(&mut runtime,worker_input(&material,cfg,1001,2000,"cancel-recovery")).unwrap();
        if !value.is_null(){let recovered:nola_application::mobile_delegation::AdmittedMobileTaskChild=serde_json::from_value(value).unwrap();assert!(recovered.cancellation_only);assert_eq!(recovered.invocation.run_id,admitted.invocation.run_id);assert_eq!(recovered.attempt.id,admitted.attempt.id);}
        drop(runtime);let store=NolaStore::open(&path).unwrap();
        assert!(store.runtime_task(&material.view.exchange_id).unwrap().unwrap().cancel_requested);
    }
}

#[test]
fn failed_target_preparation_releases_only_unstarted_slot_and_never_cancels_a_lost_ack_task(){
    {
        let dir=tempfile::tempdir().unwrap();let path=dir.path().join("abort.db");
        let(mut store,material,tool,_)=fixture(&path);
        use nola_application::host_expert_template_operation::HostExpertTemplateOperation as Op;
        assert!(Op::AbortPeerTaskPreparation{request:Box::new(tool.clone()),exchange_id:material.view.exchange_id.clone(),authority:authority()}.mutates_state());
        assert!(!Op::LoadPeerForEffect{request:Box::new(tool.clone()),authority:authority()}.mutates_state());
        for (action,expected) in [("directory",false),("read",false),("wait",false),("clarify",true),("cancel",true),("request",true),("unsupported",true)] {
            let mut request=tool.clone();if let HostRequest::ExecuteTool{input,..}=&mut request.payload{input["action"]=json!(action);}
            assert_eq!(Op::PeerTaskAction{request:Box::new(request),authority:authority()}.mutates_state(),expected);
        }
        let mut forged=tool.clone();forged.session_id="other-source".into();
        assert!(nola_application::expert_peer_task::abort_preparation(&mut store,&forged,&material.view.exchange_id,&authority(),26).is_err());
        let aborted=nola_application::expert_peer_task::abort_preparation(&mut store,&tool,&material.view.exchange_id,&authority(),26).unwrap();
        assert_eq!(aborted["preparation_aborted"],true);
        assert_eq!(nola_application::expert_peer_task::abort_preparation(&mut store,&tool,&material.view.exchange_id,&authority(),27).unwrap()["preparation_aborted"],true);
        assert!(store.runtime_task(&material.view.exchange_id).unwrap().is_none());
        let receipt=rooms::submission(&store.product_setting_repository(),&material.view.room_id,&material.view.exchange_id).unwrap().unwrap();assert_eq!(receipt.state,rooms::RoomSubmissionState::Cancelled);
        let path=dir.path().join("lost-ack.db");let(mut store,material,tool,_)=fixture(&path);let _=prepare(&mut store,&material,&tool);
        let retained=nola_application::expert_peer_task::abort_preparation(&mut store,&tool,&material.view.exchange_id,&authority(),27).unwrap();
        assert_eq!(retained["preparation_aborted"],false);assert_eq!(retained["task_may_exist"],true);
        assert!(!store.runtime_task(&material.view.exchange_id).unwrap().unwrap().cancel_requested);
        let replay=prepare(&mut store,&material,&tool);assert!(replay.prepared.replayed);assert!(!replay.prepared.source_admitted);
    }
}

#[test]
fn native_target_snapshot_is_captured_from_target_configuration_without_source_fallback(){
    {
        let dir=tempfile::tempdir().unwrap();let path=dir.path().join("native-snapshot.db");
        let(mut store,material,tool,_)=fixture(&path);
        let mut target_config=config(&material.view.target.context_conversation_id);target_config.max_model_iterations=0;
        let cfg=nola_application::mobile_delegation_configuration::capture_parent(target_config,nola_application::mobile_delegation_configuration::MobileDelegationSettings{expert_leaf:true,
            project_root:"target-device-workspace".into(),independent_model_profiles:true,selected_profile:Some(nola_host_protocol::HostModelProfileReference{profile_id:"model".into(),admitted_revision:1}),agents:Default::default()}).unwrap();
        let original_binding=serde_json::to_value(&cfg.turn_identity.host_invocation).unwrap();
        let result=nola_application::expert_peer_task::prepare(&mut store,PreparePeerTask{request:Box::new(tool),exchange_id:material.view.exchange_id.clone(),target_configuration:Box::new(cfg),target_invocation:None,project_root:"target-device-workspace".into(),authority:authority()},26).unwrap();
        assert_eq!(result.prepared.snapshot.model.model,"original-model");
        assert_eq!(result.prepared.snapshot.runtime.selected_profile.unwrap().profile_id,"model");
        let metadata:serde_json::Value=serde_json::from_str(&result.prepared.task.metadata_json).unwrap();
        assert_eq!(metadata["expert_target_configuration"]["turn_identity"]["host_invocation"],original_binding);
        assert_eq!(metadata["max_turns"],0,"unbounded expert budget must not become one turn");
    }
}

#[inline(never)]
fn load_config(runtime:&mut support::ApplicationRuntime,id:&str)->AgentCoreConfig {
    serde_json::from_value(runtime.execute(ApplicationCommand::LoadMobileTaskChildConfiguration{task_id:id.into()}).unwrap()).unwrap()
}
#[inline(never)]
fn worker_admit(runtime:&mut support::ApplicationRuntime,input:Box<nola_application::mobile_delegation::AdmitMobileTaskChild>)->Result<serde_json::Value,String>{
    runtime.execute(ApplicationCommand::AdmitMobileTaskChild{input:*input})
}
#[inline(never)]
fn wake(runtime:&mut support::ApplicationRuntime,now_ms:i64)->Result<serde_json::Value,String>{runtime.execute(ApplicationCommand::LoadMobileTaskWakePlan{now_ms})}

fn peer_admit_at(store:&mut NolaStore,input:PeerExchangeInput,at:i64)->Result<PeerMaterial,String>{
    let context=peer_context(&nola_application::expert_peer::preview(store,&input,&authority())?);
    fixture_command(store,|store|nola_application::expert_peer::admit(store,input,&context,at))
}

#[inline(never)]
fn deliver_terminal(path:&std::path::Path,material:&PeerMaterial,receipt:String){
    let mut runtime=support::open_runtime(path).unwrap();
    runtime.execute(ApplicationCommand::EnqueueTaskTerminalReconciliation{task_id:material.view.exchange_id.clone(),expected_receipt_id:receipt}).unwrap();
    assert!(!runtime.execute(ApplicationCommand::ApplyPendingTaskNotice{conversation_id:material.view.source.context_conversation_id().to_owned(),consumer_id:"expert-test-source".into(),now_ms:1035}).unwrap().is_null());
}

#[test]
fn failed_unstarted_expert_task_releases_only_its_business_slot_after_canonical_receipt(){
    let dir=tempfile::tempdir().unwrap();let path=dir.path().join("failed-slot.db");
    let(mut store,material,tool,source)=fixture(&path);let prepared=prepare(&mut store,&material,&tool);
    accept(&mut store,&source,&tool,2,*prepared.response,28);
    let (run,cursor)=store.source_agent_task_admission_location(material.view.source.context_conversation_id(),&material.view.exchange_id).unwrap().unwrap();drop(store);
    import_local_admission(&path,&material,run,cursor);
    let mut store=NolaStore::open(&path).unwrap();
    let attempt=store.claim_runtime_task(&material.view.exchange_id,"failure-worker","failure-generation","failure-lease",30,1000).unwrap().unwrap();
    assert!(!store.expert_task_is_terminal_without_run(&material.view.target.context_conversation_id,&material.view.exchange_id,31).unwrap());
    assert!(store.runtime_task_run_binding(&material.view.exchange_id).unwrap().is_none());
    store.begin_write_unit_of_work().unwrap();
    assert!(store.finish_runtime_task_detailed_with_event(&nola_application::RuntimeTaskLeaseWitness::from(&attempt),"failed",Some(26),None,None,Some("target configuration unavailable"),None,true,"failed","failed",None,"{}",None,40).unwrap());
    store.commit_write_unit_of_work().unwrap();
    assert!(store.expert_task_is_terminal_without_run(&material.view.target.context_conversation_id,&material.view.exchange_id,41).unwrap());
    assert!(store.expert_task_is_terminal_without_run("foreign",&material.view.exchange_id,41).is_err());
    let receipt=store.runtime_task_terminal_receipt(&material.view.exchange_id).unwrap().unwrap();drop(store);
    deliver_terminal(&path,&material,receipt.receipt_id);
    let mut store=NolaStore::open(&path).unwrap();
    let args=json!({"action":"request","target_participant_id":material.view.target.participant_id,"text":"Retry after the configuration is available"});
    let next=next_request(&mut store,&source,args,"retry-after-terminal-failure",1040);
    let retry=peer_admit_at(&mut store,PeerExchangeInput{source_context_id:material.view.source.context_conversation_id().to_owned(),source_run_id:source.begin.run_id.clone(),call_id:"retry-after-terminal-failure".into(),target_participant_id:material.view.target.participant_id.clone(),kind:"request".into(),text:"Retry after the configuration is available".into(),in_reply_to:None},1046).unwrap();
    let retry_prepared=prepare_at(&mut store,&retry,&next,1046);
    accept(&mut store,&source,&next,2,*retry_prepared.response,1047);
    assert_eq!(store.runtime_task(&material.view.exchange_id).unwrap().unwrap().status,"failed");
    assert!(store.runtime_task_terminal_receipt(&material.view.exchange_id).unwrap().is_some());
    assert!(store.runtime_task_run_binding(&material.view.exchange_id).unwrap().is_none());
    assert_eq!(rooms::submission(&store.product_setting_repository(),&material.view.room_id,&material.view.exchange_id).unwrap().unwrap().state,rooms::RoomSubmissionState::Cancelled);
    assert!(store.runtime_task(&retry.view.exchange_id).unwrap().is_some());
}
#[inline(never)]
fn import_local_admission(path:&std::path::Path,material:&PeerMaterial,run:String,cursor:u64){
    let mut runtime=support::open_runtime(path).unwrap();
    let admission=serde_json::from_value(runtime.execute(ApplicationCommand::ExportAgentTaskAdmission{conversation_id:material.view.source.context_conversation_id().to_owned(),run_id:run,task_id:material.view.exchange_id.clone(),event_cursor:cursor}).unwrap()).unwrap();
    runtime.execute(ApplicationCommand::ImportHostAgentTaskAdmission{admission}).unwrap();
}
fn next_request(store:&mut NolaStore,source:&AcceptedAssistantTurn,args:serde_json::Value,call:&str,now:i64)->HostRequestEnvelope{
    let model=claim(store,&source.begin.run_id,now).into_iter().find(|r|matches!(r.payload,HostRequest::ModelStream{..})).unwrap();
    accept(store,source,&model,1,HostResponse::ExecutionStarted,now+1);
    accept(store,source,&model,2,HostResponse::ModelAttemptStarted{provider:"fixture".into(),model:"original-model".into(),reason:"initial".into()},now+2);
    accept(store,source,&model,3,HostResponse::ModelCompleted{items:vec![],tool_calls:vec![json!({"call_id":call,"name":"delegate_expert","arguments":args})],usage:json!({})},now+3);
    let tool=claim(store,&source.begin.run_id,now+4).into_iter().find(|r|matches!(r.payload,HostRequest::ExecuteTool{..})).unwrap();
    accept(store,source,&tool,1,HostResponse::ExecutionStarted,now+5);tool
}

#[test]
fn expert_actual_spawn_effect_is_rejected_by_core_even_if_a_host_advertises_it(){
    let dir=tempfile::tempdir().unwrap();let path=dir.path().join("leaf-enforcement.db");
    let(mut store,material,_,_)=fixture(&path);
    let target=&material.view.target.context_conversation_id;
    let mut cfg=config(target);
    let schema=json!({"type":"function","function":{"name":"spawn_agent","parameters":{"type":"object","properties":{"agent_type":{"type":"string"},"prompt":{"type":"string"}},"required":["prompt"]}}});
    cfg.tools.push(schema.clone());let mut descriptor=cfg.host_catalog.as_ref().unwrap().capabilities["model.stream"].clone();
    descriptor.id="spawn_agent".into();descriptor.route=HostCapabilityRoute::Tool;descriptor.exposure=HostCapabilityExposure::Visible;descriptor.input_schema=schema["function"]["parameters"].clone();
    descriptor.supports_streaming=false;descriptor.input_normalization.permission_contract.risk=nola_domain::permission_resource_plan::PermissionOperationRisk::OrdinaryRead;
    cfg.host_catalog.as_mut().unwrap().capabilities.insert("spawn_agent".into(),descriptor);
    let accepted=begin_real_child(&mut store,target,&material.view.exchange_id,&material.view.text,cfg,vec![]).unwrap();
    let model=claim(&mut store,&accepted.begin.run_id,30).into_iter().find(|r|matches!(r.payload,HostRequest::ModelStream{..})).unwrap();
    accept(&mut store,&accepted,&model,1,HostResponse::ExecutionStarted,31);
    accept(&mut store,&accepted,&model,2,HostResponse::ModelAttemptStarted{provider:"fixture".into(),model:"original-model".into(),reason:"initial".into()},32);
    accept(&mut store,&accepted,&model,3,HostResponse::ModelCompleted{items:vec![],tool_calls:vec![json!({"call_id":"forbidden-child","name":"spawn_agent","arguments":{"agent_type":"general","prompt":"Do this recursively"}})],usage:json!({})},33);
    let tool=claim(&mut store,&accepted.begin.run_id,34).into_iter().find(|r|matches!(r.payload,HostRequest::ExecuteTool{..})).unwrap();
    accept(&mut store,&accepted,&tool,1,HostResponse::ExecutionStarted,35);drop(store);
    let mut runtime=support::open_runtime(&path).unwrap();
    let error=runtime.execute(ApplicationCommand::PrepareMobileDelegatedTask{input:nola_application::mobile_delegation::PrepareMobileDelegatedTask{request:tool,now_ms:36}}).unwrap_err();
    assert!(error.contains("experts_cannot_spawn_agents"),"{error}");
}

#[test]
fn later_main_turn_can_cancel_its_previously_dispatched_task_without_spoofing_source_run(){
    let dir=tempfile::tempdir().unwrap();let path=dir.path().join("main-followup-control.db");
    let(mut store,material,tool,source)=fixture(&path);let prepared=prepare(&mut store,&material,&tool);
    accept(&mut store,&source,&tool,2,*prepared.response,28);
    let config_json=store.load_run_invocation_config(&source.begin.run_id).unwrap().unwrap().invocation_config_json;
    finish_target(&mut store,"parent",source.clone());
    let run=uuid::Uuid::new_v4().to_string();
    let cfg:AgentCoreConfig=serde_json::from_str(&config_json).unwrap();
    let next=begin_real_child(&mut store,"parent",&run,"Cancel the previous expert task",cfg,vec![]).unwrap();
    let model=claim(&mut store,&next.begin.run_id,2010).into_iter().find(|r|matches!(r.payload,HostRequest::ModelStream{..})).unwrap();
    accept(&mut store,&next,&model,1,HostResponse::ExecutionStarted,2011);
    accept(&mut store,&next,&model,2,HostResponse::ModelAttemptStarted{provider:"fixture".into(),model:"original-model".into(),reason:"initial".into()},2012);
    accept(&mut store,&next,&model,3,HostResponse::ModelCompleted{items:vec![],tool_calls:vec![json!({"call_id":"later-main-cancel","name":"delegate_expert","arguments":{"action":"cancel","exchange_id":material.view.exchange_id}})],usage:json!({})},2013);
    let cancel=claim(&mut store,&next.begin.run_id,2014).into_iter().find(|r|matches!(r.payload,HostRequest::ExecuteTool{..})).unwrap();
    accept(&mut store,&next,&cancel,1,HostResponse::ExecutionStarted,2015);
    assert_ne!(next.begin.run_id,material.view.source_run_id);
    let result=nola_application::expert_peer_task::action(&mut store,nola_application::expert_peer_task::PeerTaskAction{request:Box::new(cancel),authority:authority()},2016).unwrap();
    assert_eq!(result["response"]["output"]["exchange"]["source"]["kind"],"main_agent");
    assert!(store.runtime_task(&material.view.exchange_id).unwrap().unwrap().cancel_requested);
}

#[test]
fn leaf_expert_forged_human_question_recovers_without_creating_a_user_input_wait(){
    use nola_store_port::user_input_wait::UserInputWaitRepository;
    let root=tempfile::tempdir().unwrap();let path=root.path().join("leaf-question.db");
    let(mut store,material,tool,source)=fixture(&path);
    let mut raw=config(&material.view.target.context_conversation_id);
    let schema=json!({"type":"function","function":{"name":"request_user_input","parameters":{"type":"object"}}});
    raw.tools.push(schema.clone());let mut question=raw.host_catalog.as_ref().unwrap().capabilities["model.stream"].clone();
    question.id="request_user_input".into();question.route=HostCapabilityRoute::Tool;question.exposure=HostCapabilityExposure::Visible;question.supports_streaming=false;
    question.input_schema=schema["function"]["parameters"].clone();raw.host_catalog.as_mut().unwrap().capabilities.insert("request_user_input".into(),question);
    let rejected=nola_application::expert_peer_task::prepare(&mut store,PreparePeerTask{request:Box::new(tool.clone()),exchange_id:material.view.exchange_id.clone(),target_configuration:Box::new(raw.clone()),target_invocation:None,project_root:"leaf-target".into(),authority:authority()},26).unwrap_err();
    assert!(rejected.contains("expert_task_configuration_must_be_leaf"));assert!(store.runtime_task(&material.view.exchange_id).unwrap().is_none());
    let captured=nola_application::mobile_delegation_configuration::capture_parent(raw,nola_application::mobile_delegation_configuration::MobileDelegationSettings{
        expert_leaf:true,project_root:"leaf-target".into(),independent_model_profiles:true,
        selected_profile:Some(nola_host_protocol::HostModelProfileReference{profile_id:"model".into(),admitted_revision:1}),agents:Default::default()}).unwrap();
    assert!(!captured.host_catalog.as_ref().unwrap().capabilities.contains_key("request_user_input"));
    let prepared=nola_application::expert_peer_task::prepare(&mut store,PreparePeerTask{request:Box::new(tool.clone()),exchange_id:material.view.exchange_id.clone(),target_configuration:Box::new(captured),target_invocation:None,project_root:"leaf-target".into(),authority:authority()},26).unwrap();
    accept(&mut store,&source,&tool,2,*prepared.response,28);drop(store);
    let mut runtime=support::open_runtime(&path).unwrap();let cfg=load_config(&mut runtime,&material.view.exchange_id);
    let admitted:nola_application::mobile_delegation::AdmittedMobileTaskChild=serde_json::from_value(worker_admit(&mut runtime,worker_input(&material,cfg,30,1000,"leaf-question-worker")).unwrap()).unwrap();drop(runtime);
    let mut store=NolaStore::open(&path).unwrap();
    let model=claim(&mut store,&material.view.exchange_id,40).into_iter().find(|r|matches!(r.payload,HostRequest::ModelStream{..})).unwrap();
    accept_invocation(&mut store,&admitted.invocation,&model,1,HostResponse::ExecutionStarted,41);
    accept_invocation(&mut store,&admitted.invocation,&model,2,HostResponse::ModelAttemptStarted{provider:"fixture".into(),model:"original-model".into(),reason:"initial".into()},42);
    let question_arguments=json!({"questions":[{"id":"scope","prompt":"Which scope?","options":[{"id":"small","label":"Small","description":null}],"allow_freeform":true,"required":true,"max_text_chars":128}]});
    let legitimate:nola_domain::user_input_wait::QuestionSet=serde_json::from_value(question_arguments.clone()).unwrap();legitimate.validate().unwrap();
    accept_invocation(&mut store,&admitted.invocation,&model,3,HostResponse::ModelCompleted{items:vec![],tool_calls:vec![json!({"call_id":"forged-human-question","name":"request_user_input","arguments":question_arguments})],usage:json!({})},43);
    assert!(store.open_user_input_waits(&material.view.target.context_conversation_id,None).unwrap().is_empty());
    assert!(store.owned_task_user_input_waits("parent",Some(&material.view.exchange_id)).unwrap().is_empty());
    let next=claim(&mut store,&material.view.exchange_id,44).into_iter().find(|r|matches!(r.payload,HostRequest::ModelStream{..})).expect("forged unavailable question is a recoverable tool failure, not a human wait");
    let request=serde_json::to_string(&next.payload).unwrap();assert!(request.contains("forged-human-question"));
    accept_invocation(&mut store,&admitted.invocation,&next,1,HostResponse::ExecutionStarted,45);
    accept_invocation(&mut store,&admitted.invocation,&next,2,HostResponse::ModelAttemptStarted{provider:"fixture".into(),model:"original-model".into(),reason:"initial".into()},46);
    accept_invocation(&mut store,&admitted.invocation,&next,3,HostResponse::ModelCompleted{items:vec![serde_json::to_value(AgentItem::assistant_message(ItemId::from_string("missing-info-result".into()),"Main needs to obtain the missing scope from the user")).unwrap()],tool_calls:vec![],usage:json!({})},47);
    store.begin_write_unit_of_work().unwrap();store.reconcile_runtime_task_bound_run_terminal(&material.view.exchange_id,&material.view.exchange_id,48).unwrap();store.commit_write_unit_of_work().unwrap();
    assert!(store.runtime_task_terminal_receipt(&material.view.exchange_id).unwrap().is_some());
    assert!(store.open_user_input_waits(&material.view.target.context_conversation_id,None).unwrap().is_empty());
    assert_eq!(store.runtime_task(&material.view.exchange_id).unwrap().unwrap().status,"sealed","the expert returns a result after the refused question; it is not a failed/blocked Task");
    assert_eq!(store.load_conversation("parent").unwrap().unwrap().messages.iter().filter(|m|m.role=="user").count(),1);
}

// Archived-WIP fixture: convert the two matching private configuration copies,
// never fabricate source admission, TaskPromoted, leases or execution bindings.
fn archive_question_configuration(path:&std::path::Path,material:&PeerMaterial)->AgentCoreConfig{
    let connection=rusqlite::Connection::open(path).unwrap();
    let raw:String=connection.query_row("SELECT metadata_json FROM durable_tasks WHERE id=?1",[&material.view.exchange_id],|row|row.get(0)).unwrap();
    let mut metadata:serde_json::Value=serde_json::from_str(&raw).unwrap();
    let mut config:AgentCoreConfig=serde_json::from_value(metadata["expert_target_configuration"].clone()).unwrap();
    let schema=json!({"type":"function","function":{"name":"request_user_input","parameters":{"type":"object"}}});config.tools.push(schema.clone());
    let mut descriptor=config.host_catalog.as_ref().unwrap().capabilities["model.stream"].clone();descriptor.id="request_user_input".into();descriptor.route=HostCapabilityRoute::Tool;
    descriptor.exposure=HostCapabilityExposure::Visible;descriptor.supports_streaming=false;descriptor.input_schema=schema["function"]["parameters"].clone();
    config.host_catalog.as_mut().unwrap().capabilities.insert("request_user_input".into(),descriptor);
    metadata["expert_target_configuration"]=serde_json::to_value(&config).unwrap();
    let key=format!("expert_peer_private/v1/exchange/{}",material.view.exchange_id);
    let raw:String=connection.query_row("SELECT value_json FROM product_settings WHERE key=?1",[&key],|row|row.get(0)).unwrap();
    let mut record:serde_json::Value=serde_json::from_str(&raw).unwrap();record["task_configuration"]=metadata["expert_target_configuration"].clone();
    connection.execute("UPDATE durable_tasks SET metadata_json=?1 WHERE id=?2",rusqlite::params![metadata.to_string(),material.view.exchange_id]).unwrap();
    connection.execute("UPDATE product_settings SET value_json=?1 WHERE key=?2",rusqlite::params![record.to_string(),key]).unwrap();config
}

#[test]
fn archived_expert_question_config_is_readable_and_stoppable_but_never_reexecuted(){
    use nola_store_port::user_input_wait::UserInputWaitRepository;
    for bound in [false,true]{
        let root=tempfile::tempdir().unwrap();let path=root.path().join("archived-expert.db");
        let(mut store,material,tool,source)=fixture(&path);let prepared=prepare(&mut store,&material,&tool);accept(&mut store,&source,&tool,2,*prepared.response,28);drop(store);
        let mut runtime=support::open_runtime(&path).unwrap();let original=load_config(&mut runtime,&material.view.exchange_id);
        let admitted=if bound {Some(serde_json::from_value::<nola_application::mobile_delegation::AdmittedMobileTaskChild>(worker_admit(&mut runtime,worker_input(&material,original,30,1000,"original-worker")).unwrap()).unwrap())} else {
            let observer=NolaStore::open(&path).unwrap();let(run,cursor)=observer.source_agent_task_admission_location("parent",&material.view.exchange_id).unwrap().unwrap();drop(observer);drop(runtime);import_local_admission(&path,&material,run,cursor);runtime=support::open_runtime(&path).unwrap();None
        };
        drop(runtime);
        let started_model=if let Some(admitted)=&admitted {
            let mut store=NolaStore::open(&path).unwrap();let model=claim(&mut store,&material.view.exchange_id,40).into_iter().find(|r|matches!(r.payload,HostRequest::ModelStream{..})).unwrap();
            accept_invocation(&mut store,&admitted.invocation,&model,1,HostResponse::ExecutionStarted,41);
            accept_invocation(&mut store,&admitted.invocation,&model,2,HostResponse::ModelAttemptStarted{provider:"fixture".into(),model:"original-model".into(),reason:"initial".into()},42);
            Some(model)
        }else{None};
        let archived=archive_question_configuration(&path,&material);let before=NolaStore::open(&path).unwrap().runtime_task(&material.view.exchange_id).unwrap().unwrap().metadata_json;
        let mut runtime=support::open_runtime(&path).unwrap();
        let loaded:AgentCoreConfig=load_config(&mut runtime,&material.view.exchange_id);assert_eq!(serde_json::to_value(&loaded).unwrap(),serde_json::to_value(&archived).unwrap(),"read preparation does not rewrite archived policy");
        let error=worker_admit(&mut runtime,worker_input(&material,loaded.clone(),1100,2100,"refused-worker")).unwrap_err();
        assert!(error.contains("expert_task_frozen_leaf_configuration_unsupported")&&error.contains("Stop the old Task"),"{error}");
        if !bound {
            let mut observer=NolaStore::open(&path).unwrap();
            let lease=observer.claim_runtime_task(&material.view.exchange_id,"archived-cli-worker","archived-cli-generation","archived-cli-lease",35,1000).unwrap().unwrap();
            let frontier=support::admission_frontier::read(&observer,&material.view.target.context_conversation_id);drop(observer);
            let input=nola_application::BeginAssistantTurn{conversation_id:material.view.target.context_conversation_id.clone(),expected_configuration_revision:frontier.0,expected_parent_message_id:frontier.1,
                conversation_title:"Archived expert".into(),user_message_id:format!("{}:user",material.view.exchange_id),user_part_id:format!("{}:user:text",material.view.exchange_id),user_text:material.view.text.clone(),user_content_json:None,
                assistant_message_id:format!("{}:assistant",material.view.exchange_id),context_checkpoint_id:None,core_config:loaded.clone(),created_at_ms:36,
                submission:resend_submission(&material.view.exchange_id,&material.view.text,vec![])};
            assert!(runtime.execute(ApplicationCommand::BeginTaskAssistantTurn{lease:nola_application::RuntimeTaskLeaseWitness::from(&lease),input}).unwrap_err().contains("expert_task_frozen_leaf_configuration_unsupported"),"The normal CLI Task admission shares the same frozen-policy refusal");
        }
        if let Some(admitted)=&admitted {
            for mobile in [false,true] {
                let current=admitted.current.clone();let command=if mobile {ApplicationCommand::ValidateMobileRunRecoveryAuthority{conversation_id:admitted.invocation.conversation_id.clone(),expected_run_id:admitted.invocation.run_id.clone(),assistant_message_id:admitted.invocation.assistant_message_id.clone(),expected_invocation_config_digest:admitted.invocation.invocation_config_digest.clone(),current}}
                    else {ApplicationCommand::ValidateRunRecoveryAuthority{conversation_id:admitted.invocation.conversation_id.clone(),expected_run_id:admitted.invocation.run_id.clone(),assistant_message_id:admitted.invocation.assistant_message_id.clone(),expected_invocation_config_digest:admitted.invocation.invocation_config_digest.clone(),current}};
                assert!(runtime.execute(command).unwrap_err().contains("expert_task_frozen_leaf_configuration_unsupported"));
            }
            let request=started_model.as_ref().unwrap();
            let response=HostResponseEnvelope{request_id:request.request_id.clone(),attempt_id:request.attempt_id.clone(),effect_id:request.effect_id.clone(),delegated_invocation_id:request.delegated_invocation_id.clone(),host_id:request.host_id.clone(),lease_id:request.lease_id.clone(),session_id:request.session_id.clone(),turn_id:request.turn_id.clone(),catalog_revision:request.catalog_revision,availability_epoch:request.availability_epoch,delivery:request.delivery.clone(),sequence:3,
                payload:HostResponse::ModelCompleted{items:vec![],tool_calls:vec![json!({"call_id":"old-inflight-question","name":"request_user_input","arguments":{"questions":[{"id":"scope","prompt":"Which scope?","options":[{"id":"small","label":"Small","description":null}],"allow_freeform":true,"required":true,"max_text_chars":128}]}})],usage:json!({})}};
            assert!(runtime.execute(ApplicationCommand::ReduceAndAcceptHostResponse{input:nola_application::ReduceAndAcceptHostResponse{conversation_id:admitted.invocation.conversation_id.clone(),run_id:admitted.invocation.run_id.clone(),invocation_config_digest:admitted.invocation.invocation_config_digest.clone(),response,received_at_ms:1100}}).unwrap_err().contains("expert_task_frozen_leaf_configuration_unsupported"),"An old in-flight result cannot bypass the cold recovery gate to open a question");
            let answer=nola_application::RespondToConversationWait{conversation_id:admitted.invocation.conversation_id.clone(),run_id:admitted.invocation.run_id.clone(),committed_at_ms:1100,
                response:nola_domain::user_input_wait::RespondToUserInputWait{command_id:"old-question-answer".into(),wait_id:"old-question".into(),expected_revision:1,response_schema_digest:"0".repeat(64),response:nola_domain::user_input_wait::UserInputWaitResponse::Decline{reason:None}}};
            assert!(runtime.execute(ApplicationCommand::RespondToConversationWait{input:answer}).unwrap_err().contains("expert_task_frozen_leaf_configuration_unsupported"),"Answers cannot restart this archived non-leaf expert either");
            assert!(runtime.execute(ApplicationCommand::LoadExactRunInvocation{conversation_id:admitted.invocation.conversation_id.clone(),expected_run_id:admitted.invocation.run_id.clone(),assistant_message_id:admitted.invocation.assistant_message_id.clone()}).is_ok(),"historical exact witness remains readable");
            assert!(runtime.execute(ApplicationCommand::AgentRoom{command:rooms::AgentRoomCommand::ChildHistory{conversation_id:"parent".into(),child_conversation_id:admitted.invocation.conversation_id.clone(),before_ordinal:None,limit:20,content_byte_limit:262144}}).is_ok(),"child history remains readable");
        }
        drop(runtime);let mut store=NolaStore::open(&path).unwrap();assert_eq!(store.runtime_task(&material.view.exchange_id).unwrap().unwrap().metadata_json,before);
        assert!(store.open_user_input_waits(&material.view.target.context_conversation_id,None).unwrap().is_empty());
        store.request_runtime_task_cancel(&material.view.exchange_id,1101).unwrap();drop(store);
        let mut runtime=support::open_runtime(&path).unwrap();let cancelled=worker_admit(&mut runtime,worker_input(&material,loaded,1102,2100,"cancel-only-worker")).unwrap();
        if bound {let cancelled:nola_application::mobile_delegation::AdmittedMobileTaskChild=serde_json::from_value(cancelled).unwrap();assert!(cancelled.cancellation_only);assert_eq!(cancelled.invocation.run_id,material.view.exchange_id);}else{assert!(cancelled.is_null());}
        if let Some(admitted)=&admitted {
            runtime.execute(ApplicationCommand::RequestRunCancellation{input:nola_application::RequestRunCancellation{
                conversation_id:admitted.invocation.conversation_id.clone(),expected_run_id:admitted.invocation.run_id.clone(),assistant_message_id:admitted.invocation.assistant_message_id.clone(),
                cancellation_id:"stop-archived-expert".into(),reason:None,requested_at_ms:1103}}).unwrap();
            drop(runtime);
            let mut store=NolaStore::open(&path).unwrap();
            // Even an obsolete capture must accept the exact late completion as
            // cancellation evidence without executing its question payload.
            accept_invocation(&mut store,&admitted.invocation,started_model.as_ref().unwrap(),3,
                HostResponse::ModelCompleted{items:vec![],tool_calls:vec![json!({"call_id":"late-question","function":{"name":"request_user_input","arguments":"{}"}})],usage:json!({})},1104);
            let control=claim(&mut store,&material.view.exchange_id,1105).into_iter().find(|r|matches!(r.payload,HostRequest::Cancel{..})).unwrap();
            accept_invocation(&mut store,&admitted.invocation,&control,1,HostResponse::ExecutionStarted,1106);
            accept_invocation(&mut store,&admitted.invocation,&control,2,HostResponse::CancellationConfirmed{target_effect_id:started_model.as_ref().unwrap().effect_id.clone()},1107);
            assert!(store.open_user_input_waits(&material.view.target.context_conversation_id,None).unwrap().is_empty());
            assert_eq!(store.runtime_task(&material.view.exchange_id).unwrap().unwrap().metadata_json,before);
        } else {
            drop(runtime);let store=NolaStore::open(&path).unwrap();assert_eq!(store.runtime_task(&material.view.exchange_id).unwrap().unwrap().metadata_json,before);
        }
    }
}
