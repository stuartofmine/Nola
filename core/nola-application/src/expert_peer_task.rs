// SPDX-License-Identifier: Apache-2.0
//! Main-agent expert delegation adapts onto the ordinary durable Agent Task worker.
use crate::{expert_peer::{self,PeerMaterial},expert_template_capture::HostExpertTemplateAuthority,
    repositories::ApplicationCommandRepositories};
use nola_agent_engine::agent::{host::*,AgentCoreConfig,AgentSubmission,AgentItem,AgentRole,SessionId,CallId,TaskKind,TaskState,TaskLineage};
use nola_host_protocol::delegation::DelegatedInvocationSnapshot;
use nola_domain::expert_task_target::{ExpertTaskTargetV1,EXPERT_TASK_TARGET_METADATA_KEY};
use nola_store_port::{runtime_task::RuntimeTaskRepository,model::ModelRepository};
use serde::{Serialize,Deserialize};
use serde_json::{json,Value};

#[derive(Debug,Clone,Serialize,Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PreparePeerTask {
    pub request:Box<HostRequestEnvelope>,
    pub exchange_id:String,
    pub target_configuration:Box<AgentCoreConfig>,
    #[serde(default)]
    pub target_invocation:Option<Box<DelegatedInvocationSnapshot>>,
    pub project_root:String,
    pub authority:HostExpertTemplateAuthority,
}
#[derive(Debug,Clone,Serialize,Deserialize)]
pub struct PreparedPeerTask {
    pub prepared:Box<crate::delegation_preparation::PreparedDelegatedTask>,
    pub response:Box<HostResponse>,
    pub view:Box<expert_peer::PeerView>,
}
#[derive(Debug,Clone,Serialize,Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PeerTaskAction {pub request:Box<HostRequestEnvelope>,pub authority:HostExpertTemplateAuthority}

fn exact_request<S:ApplicationCommandRepositories>(store:&mut S,request:&HostRequestEnvelope,now:i64)->Result<(String,String,CallId,serde_json::Map<String,Value>),String> {
    let (run,owner,_)=crate::started_host_effect::validate(store,request,"execute_tool",now)?;
    let HostRequest::ExecuteTool{capability_id,call_id,input,..}=&request.payload else {return Err("expert_task_requires_tool_effect".into())};
    if !matches!(capability_id.as_str(),"delegate_expert"|"expert_exchange") {return Err("expert_task_requires_exact_tool".into());}
    let arguments=input.as_object().cloned().ok_or("expert_task_arguments_must_be_object")?;
    let action=arguments.get("action").and_then(Value::as_str).ok_or("expert_task_action_missing")?;
    let allowed:&[&str]=match action {
        "directory"=>&["action"], "request"=>&["action","target_participant_id","text","in_reply_to"],
        "read"|"cancel"=>&["action","exchange_id"],"wait"=>&["action","exchange_id","timeout_ms"],
        _=>return Err("expert_task_unknown_action".into())};
    if arguments.keys().any(|key|!allowed.contains(&key.as_str())) {return Err("expert_task_unknown_or_source_argument".into());}
    Ok((run,owner,CallId::from_string(call_id.clone()),arguments))
}
fn response(task:&nola_store_port::runtime_task::StoredRuntimeTask,admitted:bool,call:CallId)->Result<HostResponse,String> {
    if admitted {
        return Ok(HostResponse::ToolCompleted{call_id:call.0,output:json!({"exchange_id":task.id,"task_id":task.id,"status":task.status,"replayed":true}),
            outcome:nola_domain::host_effect::HostToolOutcome::Succeeded,activation:None,full_output_ref:None,artifacts:vec![]});
    }
    Ok(HostResponse::TaskPromoted{task_id:task.id.clone(),input_watermark:task.input_watermark,kind:TaskKind::Agent,state:TaskState::Queued,
        progress:None,current_step:None,lineage:TaskLineage{parent_task_id:task.parent_task_id.clone(),resumed_from_task_id:None,
            child_session_id:Some(SessionId::from_string(task.child_session_id.clone().ok_or("expert_task_child_missing")?)),
            parent_call_id:Some(call),permission_scope_id:Some(format!("task:{}",task.id)),cancellation_scope_id:Some(format!("task:{}",task.id)),..Default::default()}})
}
fn target_snapshot<S:ApplicationCommandRepositories>(store:&mut S,material:&PeerMaterial,config:&mut AgentCoreConfig,
    supplied:Option<DelegatedInvocationSnapshot>,project:&str)->Result<DelegatedInvocationSnapshot,String> {
    let child=&material.view.target.context_conversation_id;
    use nola_domain::expert_task_target::{expert_leaf_control,schema_tool_name};
    if config.tools.iter().any(|tool|schema_tool_name(tool).is_some_and(expert_leaf_control))
        || config.host_catalog.as_ref().is_some_and(|catalog|catalog.capabilities.keys().any(|name|expert_leaf_control(name))) {
        return Err("expert_task_configuration_must_be_leaf".into());
    }
    if project.trim().is_empty() || project.len()>4096 {return Err("expert_task_target_workspace_missing".into());}
    if config.session_id.as_ref().is_some_and(|id|id.0!=*child) {return Err("expert_task_target_config_owner_changed".into());}
    if config.initial_items.iter().any(|item|!matches!(item,AgentItem::Message{role:AgentRole::System,..})) {
        return Err("expert_task_cannot_copy_caller_history".into());
    }
    config.session_id=Some(SessionId::from_string(child.clone()));
    let binding=config.turn_identity.host_invocation.as_ref().ok_or("expert_task_target_invocation_missing")?;
    binding.validate().map_err(str::to_owned)?;
    let profile=store.model_repository().model_profile(&material.entry.model_declaration.profile_id).map_err(|e|e.to_string())?.ok_or("expert_task_target_profile_missing")?;
    if profile.revision!=material.entry.model_declaration.profile_revision || !profile.enabled || profile.deleted_at_ms.is_some() {return Err("expert_task_target_profile_changed".into());}
    let source:Value=serde_json::from_str(&profile.source_json).map_err(|_|"expert_task_target_source_invalid")?;
    nola_domain::expert_message::validate_expert_model_route(&material.entry.model_declaration,&source,&material.entry.defaults,
        &serde_json::to_value(&binding.primary).map_err(|e|e.to_string())?)?;
    let mut snapshot=if let Some(value)=supplied {value} else {
        let mut parent=config.turn_identity.delegation.as_ref().ok_or("expert_task_target_snapshot_unavailable")?.snapshot()?;
        let profile=parent.profiles.get_mut("general").ok_or("expert_task_general_profile_missing")?;
        profile.model=None;profile.executor=None;profile.prompt.clear();
        if parent.project_root!=project {return Err("expert_task_target_workspace_changed".into());}
        crate::delegation_preparation::capture_delegated_invocation(store,&parent,material.view.source.context_conversation_id(),"general")?
    };
    if snapshot.owner_session_id!=material.view.source.context_conversation_id() || snapshot.execution_backend!="nola"
        || snapshot.runtime.selected_profile.as_ref().is_none_or(|pin|pin.profile_id!=material.entry.model_declaration.profile_id || pin.admitted_revision!=material.entry.model_declaration.profile_revision)
        || !snapshot.runtime.llm_fallbacks.is_empty() || !binding.fallbacks.is_empty() {
        return Err("expert_task_target_snapshot_or_fallback_changed".into());
    }
    if let nola_host_protocol::HostModelRouteBinding::Http(route)=&binding.primary {
        if snapshot.model.model!=route.model || snapshot.model.endpoint.trim_end_matches('/')!=route.endpoint.trim_end_matches('/') {
            return Err("expert_task_snapshot_model_route_changed".into());
        }
    }
    snapshot.profile.name="general".into();snapshot.profile.prompt.clear();
    snapshot.profile.model=None;snapshot.profile.executor=None;
    snapshot.runtime.core_permission_rules=config.permission_rules.rules.clone();
    snapshot.runtime.reasoning_selection=binding.generation.reasoning_selection.clone();
    Ok(snapshot)
}

pub fn prepare<S:ApplicationCommandRepositories>(store:&mut S,input:PreparePeerTask,now:i64)->Result<PreparedPeerTask,String> {
    let (run,owner,call,args)=exact_request(store,&input.request,now)?;
    if args["action"]!="request" {return Err("expert_task_prepare_requires_request".into());}
    let prior=expert_peer::load_for_effect(store,&input.request,&input.authority,now)?;
    let material=if let Some(material)=prior {material} else {
        let directory=expert_peer::directory(store,&owner,&run,&input.authority)?;
        expert_peer::load(store,&directory.room_id,&input.exchange_id,&input.authority)?
    };
    if material.view.exchange_id!=input.exchange_id || material.view.source.context_conversation_id()!=owner || material.view.source_run_id!=run || material.view.source_call_id!=call.0
        || args["target_participant_id"]!=material.view.target.participant_id || args["text"]!=material.view.text
        || args.get("in_reply_to").and_then(Value::as_str)!=material.view.in_reply_to.as_deref() {return Err("expert_task_original_tool_intent_changed".into());}
    let origin=json!({"parent_conversation_id":owner,"parent_run_id":run,"parent_effect_id":input.request.effect_id.0,"parent_call_id":call.0,"arguments":args});
    if let Some(task)=store.runtime_task(&input.exchange_id).map_err(|e|e.to_string())? {
        let metadata:Value=serde_json::from_str(&task.metadata_json).map_err(|_|"expert_task_metadata_invalid")?;
        if metadata["delegation_origin"]!=origin {return Err("expert_task_source_effect_changed".into());}
        store.read_validated_expert_task_target(&task.id,&material.view.target.context_conversation_id,false).map_err(|e|e.to_string())?.ok_or("expert_task_target_missing")?;
        let snapshot=serde_json::from_value(metadata["delegated_invocation"].clone()).map_err(|_|"expert_task_snapshot_invalid")?;
        let admitted=store.source_agent_task_admission_location(&owner,&task.id).map_err(|e|e.to_string())?.is_some();
        return Ok(PreparedPeerTask{response:Box::new(response(&task,admitted,call)?),prepared:Box::new(crate::delegation_preparation::PreparedDelegatedTask{task:crate::runtime_task::runtime_task(task),snapshot,replayed:true,source_admitted:admitted}),view:Box::new(material.view)});
    }
    if !matches!(&input.request.payload,HostRequest::ExecuteTool{capability_id,..} if capability_id=="delegate_expert") || !material.view.source.is_main_agent() {return Err("only_main_agent_may_delegate_experts".into());}
    expert_peer::source_binding(store,&owner,&run,&input.authority)?;
    let receipt=crate::agent_room::submission(&store.product_setting_repository(),&material.view.room_id,&input.exchange_id)?.ok_or("expert_task_pending_receipt_missing")?;
    if !matches!(receipt.state,crate::agent_room::RoomSubmissionState::Accepted|crate::agent_room::RoomSubmissionState::Selected) {return Err("expert_task_preparation_is_not_pending".into());}
    let mut config=*input.target_configuration;
    let snapshot=target_snapshot(store,&material,&mut config,input.target_invocation.map(|value|*value),&input.project_root)?;
    let pin=ExpertTaskTargetV1{version:1,task_id:input.exchange_id.clone(),exchange_id:input.exchange_id.clone(),parent_conversation_id:material.view.parent_conversation_id.clone(),
        room_id:material.view.room_id.clone(),source:material.view.source.clone(),target:material.view.target.clone(),source_run_id:run.clone(),source_call_id:call.0.clone()};
    pin.validate()?;
    let metadata=json!({EXPERT_TASK_TARGET_METADATA_KEY:pin,"expert_target_configuration":config,"expert_target_prompt_definitions":material.prompt_definitions,"expert_exchange_context":material.view,
        "delegated_invocation":snapshot,"delegation_origin":origin,"max_turns":config.max_model_iterations,"execution_backend":"nola","model_notified":false});
    let source_task=store.runtime_task_binding_for_run(pin.source.context_conversation_id(),&pin.source_run_id).map_err(|e|e.to_string())?;
    let task=crate::CreateRuntimeTask{id:input.exchange_id.clone(),kind:"agent".into(),title:format!("{} → {}",material.view.source_display_name,material.view.target_display_name),
        project_root:input.project_root,owner_session_id:Some(owner),parent_task_id:source_task.map(|binding|binding.task_id),parent_tool_call_id:Some(call.0.clone()),child_session_id:Some(pin.target.context_conversation_id.clone()),
        agent_type:Some("general".into()),prompt:Some(material.view.text.clone()),initial_progress_summary:None,initial_activity:None,output_path:None,initially_unread:false,
        input_watermark:Some(now),metadata,scheduling:crate::RuntimeTaskScheduling{priority:Default::default(),queued_at_ms:now,resource_budget:Default::default(),resource_usage:Default::default(),output_bytes:0,budget_exhaustion:None},created_at_ms:now};
    crate::assistant_publication_transaction::with_publication_transaction(store,|store| {
        expert_peer::freeze_task_configuration(store,&input.exchange_id,&config,&material.prompt_definitions,&input.authority,now)?;
        let task=crate::runtime_task::stored_task(task.into_task()?);
        store.create_runtime_task_with_event(&task).map_err(|e|e.to_string())?;
        store.read_validated_expert_task_target(&task.id,&pin.target.context_conversation_id,false).map_err(|e|e.to_string())?.ok_or("expert_task_target_missing")?;
        Ok(PreparedPeerTask{response:Box::new(response(&task,false,call)?),prepared:Box::new(crate::delegation_preparation::PreparedDelegatedTask{task:crate::runtime_task::runtime_task(task),snapshot,replayed:false,source_admitted:false}),view:Box::new(material.view)})
    })
}

fn security(config:&AgentCoreConfig)->Result<Value,String> {
    let binding=config.turn_identity.host_invocation.as_ref().ok_or("expert_task_actual_invocation_missing")?;
    Ok(json!({"primary":binding.primary,"fallbacks":binding.fallbacks,"generation":binding.generation,"output_admission":binding.output_admission,
        "catalog":config.host_catalog,"tools":config.tools,"permission_rules":config.permission_rules,"context_window":config.context_window,
        "model_capability":config.turn_identity.model_capability,"host_dependencies":config.turn_identity.host_dependencies,"memory_policy":config.memory_policy}))
}
/// A previous unpublished expert capture may contain now-forbidden controls.
/// Never rewrite that immutable capture: reads/cancellation retain it, execution
/// must stop and be redispatched by Main using a new valid expert configuration.
pub(crate) fn validate_saved_expert_leaf(config:&AgentCoreConfig)->Result<(),String>{
    use nola_domain::expert_task_target::{expert_leaf_control,schema_tool_name};
    if config.tools.iter().any(|tool|schema_tool_name(tool).is_some_and(expert_leaf_control))
        || config.host_catalog.as_ref().is_some_and(|catalog|catalog.capabilities.keys().any(|name|expert_leaf_control(name))) {
        return Err("expert_task_frozen_leaf_configuration_unsupported: This saved expert Task contains controls reserved for Main, including direct user questions. Its history is preserved. Stop the old Task, update the expert if needed, and ask Main to dispatch it again.".into());
    }
    Ok(())
}

/// Task IDs equal the bound expert Run IDs by the validated v1 target contract.
/// The ID lookup alone grants nothing; private target and actual binding must
/// both prove this exact expert context before the latest leaf policy applies.
pub(crate) fn validate_expert_question_execution<R:nola_store_port::runtime_task::TaskAuthorityReadRepository+?Sized>(store:&R,conversation:&str,run:&str)->Result<(),String>{
    let Some(task)=store.read_runtime_task(run).map_err(|e|e.to_string())? else {return Ok(())};
    let Some(pin)=store.read_validated_expert_task_target(&task.id,conversation,true).map_err(|e|e.to_string())? else {return Ok(())};
    let binding=store.read_runtime_task_run_binding(&task.id).map_err(|e|e.to_string())?.ok_or("expert_task_bound_run_missing")?;
    if pin.exchange_id!=run || binding.run_id!=run || binding.child_session_id!=conversation{return Err("expert_task_question_owner_changed".into());}
    let metadata:Value=serde_json::from_str(&task.metadata_json).map_err(|_|"expert_task_metadata_invalid")?;
    let frozen:AgentCoreConfig=serde_json::from_value(metadata["expert_target_configuration"].clone()).map_err(|_|"expert_task_configuration_invalid")?;
    validate_saved_expert_leaf(&frozen)
}

pub(crate) fn validate_expert_run_recovery<R:RuntimeTaskRepository+?Sized>(store:&R,conversation:&str,run:&str)->Result<(),String>{
    let Some(binding)=store.runtime_task_binding_for_run(conversation,run).map_err(|e|e.to_string())? else{return Ok(())};
    if store.validated_expert_task_target(&binding.task_id,conversation,true).map_err(|e|e.to_string())?.is_none(){return Ok(())}
    let task=store.runtime_task(&binding.task_id).map_err(|e|e.to_string())?.ok_or("expert_task_missing")?;
    let metadata:Value=serde_json::from_str(&task.metadata_json).map_err(|_|"expert_task_metadata_invalid")?;
    let frozen:AgentCoreConfig=serde_json::from_value(metadata["expert_target_configuration"].clone()).map_err(|_|"expert_task_configuration_invalid")?;
    validate_saved_expert_leaf(&frozen)
}

/// Both CLI BeginTaskAssistantTurn and embedded workers enter this same gate.
pub(crate) fn decorate_task_admission<R:RuntimeTaskRepository+?Sized>(store:&R,lease:&crate::RuntimeTaskLeaseWitness,command:&mut crate::BeginAssistantTurn)->Result<(),String> {
    let Some(pin)=store.validated_expert_task_target(&lease.task_id,&command.conversation_id,true).map_err(|e|e.to_string())? else {return Ok(())};
    let task=store.runtime_task(&lease.task_id).map_err(|e|e.to_string())?.ok_or("expert_task_missing")?;
    let metadata:Value=serde_json::from_str(&task.metadata_json).map_err(|_|"expert_task_metadata_invalid")?;
    let expected:AgentCoreConfig=serde_json::from_value(metadata["expert_target_configuration"].clone()).map_err(|_|"expert_task_configuration_invalid")?;
    validate_saved_expert_leaf(&expected)?;
    if security(&command.core_config)?!=security(&expected)? {return Err("expert_task_actual_target_policy_or_model_changed".into());}
    if command.user_message_id!=format!("{}:user",pin.exchange_id) || command.assistant_message_id!=format!("{}:assistant",pin.exchange_id) {
        return Err("expert_task_actual_message_identity_changed".into());
    }
    let definitions:Vec<nola_domain::expert_message::ExpertPromptDefinition>=serde_json::from_value(metadata["expert_target_prompt_definitions"].clone()).map_err(|_|"expert_task_prompt_definitions_missing")?;
    let AgentSubmission::UserInput{turn_context_components,submission_id,..}=&mut command.submission else {return Err("expert_task_requires_user_input_submission".into())};
    if submission_id.as_deref()!=Some(pin.exchange_id.as_str()) {return Err("expert_task_actual_run_identity_changed".into());}
    let provided:Vec<_>=turn_context_components.iter().filter(|c|c.kind==nola_agent_engine::context_engine::ContextComponentKind::PromptTemplate).collect();
    if !provided.is_empty() && (provided.len()!=definitions.len() || definitions.iter().any(|d|
        provided.iter().filter(|c|c.id==d.component_id&&c.content.as_str()==Some(d.content.as_str())).count()!=1)) {
        return Err("expert_task_actual_prompt_components_changed".into());
    }
    if provided.is_empty() {
        turn_context_components.extend(definitions.into_iter().map(|d|nola_agent_engine::context_engine::ContextComponent{
            kind:nola_agent_engine::context_engine::ContextComponentKind::PromptTemplate,id:d.component_id,content:json!(d.content),source:None}));
    }
    let view:expert_peer::PeerView=serde_json::from_value(metadata["expert_exchange_context"].clone()).map_err(|_|"expert_task_exchange_context_missing")?;
    let context_id=format!("nola.expert-exchange/{}",pin.exchange_id);
    if turn_context_components.iter().any(|c|c.id==context_id){return Err("expert_task_exchange_context_is_core_owned".into());}
    turn_context_components.push(nola_agent_engine::context_engine::ContextComponent{
        kind:nola_agent_engine::context_engine::ContextComponentKind::System,id:context_id,source:None,
        content:json!({"version":1,"exchange_id":pin.exchange_id,"task_id":pin.task_id,"parent_conversation_id":pin.parent_conversation_id,
            "source":pin.source,"target":pin.target,"source_display_name":view.source_display_name,"target_display_name":view.target_display_name,
            "instructions":"The main agent assigned this work. Complete it with your own tools and return a clear result through this Task. If information is missing, state exactly what is needed in your result; the main agent handles the user conversation and any follow-up. Do not dispatch experts or spawn child agents. These receipt references grant no tools or permissions."})});
    Ok(())
}

pub fn action<S:ApplicationCommandRepositories>(store:&mut S,input:PeerTaskAction,now:i64)->Result<Value,String> {
    let (run,owner,call,args)=exact_request(store,&input.request,now)?;
    let operation=args["action"].as_str().ok_or("expert_task_action_missing")?;
    let output=if operation=="directory" {
        serde_json::to_value(expert_peer::directory(store,&owner,&run,&input.authority)?).map_err(|e|e.to_string())?
    } else {
        let directory=if matches!(&input.request.payload,HostRequest::ExecuteTool{capability_id,..} if capability_id=="expert_exchange") {expert_peer::recovery_directory(store,&owner,&run,&input.authority)?} else {expert_peer::directory(store,&owner,&run,&input.authority)?};
        let id=args.get("exchange_id").and_then(Value::as_str).ok_or("expert_task_exchange_id_missing")?;
        let material=expert_peer::load(store,&directory.room_id,id,&input.authority)?;
            if material.view.source!=directory.source {return Err("expert_task_action_requires_original_requester".into());}
            let task=store.runtime_task(id).map_err(|e|e.to_string())?;
            if let Some(task)=task {
                if task.owner_session_id.as_deref()!=Some(owner.as_str()) || task.child_session_id.as_deref()!=Some(material.view.target.context_conversation_id.as_str()) {
                    return Err("expert_task_action_owner_changed".into());
                }
                if operation=="cancel" {
                    if !material.view.source.is_main_agent() && material.view.source_run_id!=run {return Err("expert_task_cancel_source_run_changed".into());}
                    store.request_runtime_task_cancel(id,now).map_err(|e|e.to_string())?;
                } else if !matches!(operation,"read"|"wait") {return Err("expert_task_use_prepare_for_request".into());}
                let current=store.runtime_task(id).map_err(|e|e.to_string())?.ok_or("expert_task_missing")?;
                let receipt=store.runtime_task_terminal_receipt(id).map_err(|e|e.to_string())?;
                json!({"exchange":material.view,"task":{"id":current.id,"status":current.status,"cancel_requested":current.cancel_requested,"error":current.error,"last_error":current.last_error},
                    "completion_confirmed":receipt.is_some(),"terminal_confirmed":receipt.is_some(),"result_available":receipt.as_ref().is_some_and(|r|r.journal_result.is_some()),"run_result":receipt.as_ref().and_then(|r|r.journal_result.as_ref()).map(|r|&r.seal.result),
                    "terminal_receipt_id":receipt.as_ref().map(|r|&r.receipt_id),"result_summary":if receipt.is_some(){current.result_summary}else{None}})
            } else if operation=="cancel" {
                if !material.view.source.is_main_agent() && material.view.source_run_id!=run {return Err("expert_task_cancel_source_run_changed".into());}
                crate::assistant_publication_transaction::with_publication_transaction(store,|store| {
                    let pending=crate::agent_room::submission(&store.product_setting_repository(),&material.view.room_id,id)?.ok_or("expert_task_pending_receipt_missing")?;
                    let cancelled=crate::agent_room::cancel_pending(&mut store.product_setting_repository(),&material.view.room_id,id,pending.revision,now)?;
                    Ok(json!({"exchange":material.view,"status":cancelled.state,"execution_started":false,"completion_confirmed":false}))
                })?
            } else if matches!(operation,"read"|"wait") {
                let receipt=crate::agent_room::submission(&store.product_setting_repository(),&material.view.room_id,id)?.ok_or("expert_task_pending_receipt_missing")?;
                let aborted=receipt.state==crate::agent_room::RoomSubmissionState::Cancelled;
                json!({"exchange":material.view,"status":if aborted {"preparation_aborted"}else{"awaiting_task_preparation"},"preparation_aborted":aborted,"observation_terminal":aborted,"completion_confirmed":false})
            }
            else {return Err("expert_task_use_prepare_for_request".into());}
    };
    let terminal=output.get("completion_confirmed").and_then(Value::as_bool).unwrap_or(false)||output.get("observation_terminal").and_then(Value::as_bool).unwrap_or(false);
    let wait_ms=if operation=="wait"&&!terminal {args.get("timeout_ms").map(|v|v.as_u64().filter(|v|(1..=30_000).contains(v)).ok_or("expert_task_invalid_timeout")).transpose()?.unwrap_or(30_000)}else{0};
    Ok(json!({"terminal":terminal,"wait_ms":wait_ms,"response":HostResponse::ToolCompleted{call_id:call.0,output,
        outcome:nola_domain::host_effect::HostToolOutcome::Succeeded,activation:None,full_output_ref:None,artifacts:vec![]}}))
}

pub fn abort_preparation<S:ApplicationCommandRepositories>(store:&mut S,request:&HostRequestEnvelope,id:&str,authority:&HostExpertTemplateAuthority,now:i64)->Result<Value,String>{
    let (_,_,_,args)=exact_request(store,request,now)?;
    if args["action"]!="request" {return Err("expert_task_abort_requires_request".into());}
    let material=expert_peer::load_for_effect(store,request,authority,now)?.ok_or("expert_task_original_request_missing")?;
    if material.view.exchange_id!=id {return Err("expert_task_abort_exchange_changed".into());}
    crate::assistant_publication_transaction::with_publication_transaction(store,|store|{
        if store.runtime_task(id).map_err(|e|e.to_string())?.is_some() || store.inspect_exact_run(id).map_err(|e|e.to_string())?.is_some(){return Ok(json!({"exchange_id":id,"preparation_aborted":false,"task_may_exist":true}));}
        let receipt=crate::agent_room::submission(&store.product_setting_repository(),&material.view.room_id,id)?.ok_or("expert_task_pending_receipt_missing")?;
        if matches!(receipt.state,crate::agent_room::RoomSubmissionState::Accepted|crate::agent_room::RoomSubmissionState::Selected){
            crate::agent_room::cancel_pending(&mut store.product_setting_repository(),&material.view.room_id,id,receipt.revision,now)?;
        } else if receipt.state!=crate::agent_room::RoomSubmissionState::Cancelled {return Err("expert_task_preparation_cannot_abort_execution".into());}
        Ok(json!({"exchange_id":id,"preparation_aborted":true,"execution_started":false}))
    })
}

#[cfg(test)]
#[test]
fn expert_private_payload_stack_footprint() {
    assert!(std::mem::size_of::<crate::host_expert_template_operation::HostExpertTemplateOperation>()<=512);
    assert!(std::mem::size_of::<PreparePeerTask>()<=256);
    assert!(std::mem::size_of::<PreparedPeerTask>()<=128);
    assert!(std::mem::size_of::<crate::ApplicationCommand>()<=std::mem::size_of::<crate::BeginAssistantTurn>()+std::mem::size_of::<crate::RuntimeTaskLeaseWitness>()+16,
        "New private expert payloads must not enlarge the existing turn-admission command baseline");
    eprintln!("ApplicationCommand={} HostExpertTemplateOperation={} PreparePeerTask={} AgentCoreConfig={} HostRequestEnvelope={} DelegatedInvocationSnapshot={}",
        std::mem::size_of::<crate::ApplicationCommand>(),std::mem::size_of::<crate::host_expert_template_operation::HostExpertTemplateOperation>(),std::mem::size_of::<PreparePeerTask>(),
        std::mem::size_of::<AgentCoreConfig>(),std::mem::size_of::<HostRequestEnvelope>(),std::mem::size_of::<DelegatedInvocationSnapshot>());
}
