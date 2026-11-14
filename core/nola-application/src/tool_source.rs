// SPDX-License-Identifier: Apache-2.0
//! Bind only changed live Tool Parts; hydrate history without execution authority.
use nola_agent_engine::agent::{parts::AgentPart, protocol::AgentItem, AgentThreadState};
use nola_store_port::tool_source::*;
use crate::assistant_journal_changes::{AssistantJournalChange, AssistantJournalChanges};

pub(crate) fn bind_changed<S: crate::repositories::AssistantPublicationRepositories + ?Sized>(
    store: &mut S,
    stamp: &nola_store_port::assistant_journal::AssistantSourceCommitStamp,
    conversation: &str,
    state: &AgentThreadState,
    changes: &AssistantJournalChanges<'_>,
) -> Result<(), String> {
    let epoch=store.assistant_source_epoch().map_err(|e|e.to_string())?;
    let mut seen=std::collections::BTreeSet::new();
    // Use the final changed snapshot for each Part in this atomic transition.
    for change in changes.changes.iter().rev() {
        let AssistantJournalChange::Part { turn_id, part:AgentPart::Tool(part), .. }=change else { continue; };
        if !seen.insert(part.id.as_str()) { continue; }
        let owner=store.assistant_owner_for_core_turn(conversation,turn_id).map_err(|e|e.to_string())?
            .ok_or("tool source lacks accepted Core Turn owner")?;
        let bound=store.assistant_owner_for_part(conversation,&owner.run_id,&part.id).map_err(|e|e.to_string())?
            .ok_or("tool source lacks accepted Part owner")?;
        if bound.owner.conversation_id != owner.conversation_id
            || bound.owner.run_id != owner.run_id
            || bound.turn_id.as_deref() != Some(*turn_id)
            || part.ownership.turn_id != *turn_id
        { return Err("tool source Part and Turn ownership differ".into()); }
        // An accepted input can start a new assistant Message within this same
        // Turn. Publish sources under the Part's exact persisted segment.
        let owner = &bound.owner;
        let identity=nola_domain::assistant_journal::AssistantJournalIdentity {
            conversation_id:owner.conversation_id.clone(),run_id:owner.run_id.clone(),
            assistant_message_id:owner.assistant_message_id.clone(),source_store_epoch:epoch.clone(),
        };
        let mut selected=Vec::new();
        for item in &state.conversation.transcript {
            let (kind,call,name)=match item {
                AgentItem::ToolCall { call_id,name,.. } => (ToolSourceKind::ToolCall,call_id,name),
                AgentItem::ToolResult { call_id,name,.. } => (ToolSourceKind::ToolResult,call_id,name),
                _ => continue,
            };
            if call.as_str() != part.call_id { continue; }
            let item_owner=state.item_ownership.get(item.id().as_str()).ok_or("tool source item lacks accepted transcript ownership")?;
            // Provider call IDs are scoped to their accepted Turn/Message, not
            // the retained transcript. A prior Run or assistant segment may
            // legitimately reuse the same upstream ID, even for another tool.
            if item_owner.turn_id != *turn_id || item_owner.message_id != part.ownership.message_id {
                continue;
            }
            if name != &part.capability_id { return Err("tool source call has conflicting capability identity".into()); }
            if item_owner.source != part.execution_source {
                return Err("tool source transcript and Part execution provenance differ".into());
            }
            if selected.iter().any(|(old_kind,_)| old_kind == &kind) {
                return Err("tool source call has duplicate transcript items".into());
            }
            selected.push((kind,item));
        }
        // A pending Part may precede transcript call materialization. Never
        // synthesize an item from its Part ID, preview or arguments.
        let has_result=selected.iter().any(|(kind,_)|*kind==ToolSourceKind::ToolResult);
        if part.state==nola_agent_engine::agent::parts::ToolPartState::Completed
            || (part.state==nola_agent_engine::agent::parts::ToolPartState::Failed && part.structured_output.is_some())
        {
            if !has_result || !selected.iter().any(|(kind,_)|*kind==ToolSourceKind::ToolCall) {
                return Err("completed tool Part lacks its original transcript call/result sources".into());
            }
        }
        if selected.is_empty() { continue; }
        selected.sort_by_key(|(kind,_)| match kind { ToolSourceKind::ToolCall=>0,ToolSourceKind::ToolResult=>1 });
        for (kind,item) in selected {
            store.bind_tool_source(stamp,&identity,&ToolSourceInput {
                source_item_id:item.id().as_str().to_owned(),kind,call_id:part.call_id.clone(),
                name:part.capability_id.clone(),owner:bound.clone(),
                body_json:serde_json::to_string(item).map_err(|e|e.to_string())?,
            }).map_err(|e|e.to_string())?;
        }
    }
    Ok(())
}

/// Load a complete small source pair only. Large original objects remain range
/// readable; a partial JSON prefix is never promoted to a complete model item.
pub(crate) fn historical_items<S: ToolSourceRepository + ?Sized>(
    store:&S,owner:&nola_domain::CanonicalPartOwner,revision:i64,remaining:&mut usize,
) -> Result<Vec<AgentItem>,String> {
    let bindings=store.tool_sources_for_part(owner,revision)?;
    if bindings.is_empty() { return Ok(Vec::new()); }
    if bindings.len()!=2 || bindings[0].kind!=ToolSourceKind::ToolCall || bindings[1].kind!=ToolSourceKind::ToolResult
        || bindings[0].call_id!=bindings[1].call_id || bindings[0].name!=bindings[1].name
        || bindings[0].owner!=bindings[1].owner
    { return Ok(Vec::new()); } // Explicit caller note: incomplete historical pair is not executable.
    let total=bindings.iter().try_fold(0usize,|n,b|n.checked_add(usize::try_from(b.body.total_utf8_bytes()).ok()?));
    let Some(total)=total.filter(|n|*n<=(*remaining).min(8192)) else { return Ok(Vec::new()); };
    let mut result=Vec::new();
    for binding in bindings {
        let raw=read_complete(store,&binding,revision,8192)?;
        let item:AgentItem=serde_json::from_str(&raw).map_err(|e|e.to_string())?;
        if item.id().as_str()!=binding.source_item_id { return Err("historical tool source identity differs".into()); }
        // Historical call IDs remain in a separate, non-executable namespace.
        let mapped=crate::historical_context_references::mapped_source_id(owner,"history",&binding.call_id);
        let item=match item {
            AgentItem::ToolCall { id,call_id,name,arguments } if binding.kind==ToolSourceKind::ToolCall && call_id.as_str()==binding.call_id && name==binding.name =>
                AgentItem::ToolCall { id,call_id:crate::CallId::from_string(mapped),name,
                    arguments:nola_domain::tool_evidence::ModelToolEvidence::from_host_output(&arguments).into_value() },
            AgentItem::ToolResult { id,call_id,name,output,is_error,.. } if binding.kind==ToolSourceKind::ToolResult && call_id.as_str()==binding.call_id && name==binding.name =>
                AgentItem::ToolResult { id,call_id:crate::CallId::from_string(mapped),name,
                    output:nola_domain::tool_evidence::ModelToolEvidence::from_host_output(&output).into_value(),
                    is_error,full_output_ref:None,artifacts:Vec::new() },
            _=>return Err("historical tool source kind or call differs".into()),
        };
        result.push(item);
    }
    *remaining-=total;
    Ok(result)
}

pub(crate) fn read_complete<S:ToolSourceRepository+?Sized>(store:&S,binding:&ToolSourceBinding,revision:i64,limit:usize)->Result<String,String> {
    use sha2::{Digest,Sha256};
    if binding.body.total_utf8_bytes()>limit as u64 { return Err("tool source exceeds complete-read budget".into()); }
    let mut raw=String::new();
    let mut offset=0;
    loop {
        let page=store.read_tool_source_range(binding,revision,offset,32768)?;
        if page.offset_bytes!=offset || page.next_offset_bytes!=offset+page.text.len() as u64
            || page.next_offset_bytes>binding.body.total_utf8_bytes()
            || (!page.end_of_content && page.next_offset_bytes<=offset)
        { return Err("tool source range is not contiguous".into()); }
        raw.push_str(&page.text);
        offset=page.next_offset_bytes;
        if page.end_of_content { break; }
    }
    if offset!=binding.body.total_utf8_bytes() || format!("{:x}",Sha256::digest(raw.as_bytes()))!=binding.body_sha256 {
        return Err("tool source original bytes or digest differ".into());
    }
    Ok(raw)
}

#[cfg(test)]
mod tests {
    use super::*;
    use nola_domain::{CanonicalBodyReference,CanonicalPartOwner,PartBodyField};
    use nola_store_port::assistant_ownership::{AssistantOwner,AssistantPartOwner};
    use serde_json::json;
    use sha2::{Digest,Sha256};

    struct Sources { values:Vec<(ToolSourceBinding,String)> }
    impl Sources {
        fn pair(name:&str,output:serde_json::Value)->Self {
            let owner=AssistantPartOwner { part_id:"part".into(),turn_id:Some("source-turn".into()),
                owner:AssistantOwner { conversation_id:"conversation".into(),run_id:"source-run".into(),assistant_message_id:"message".into() } };
            let items=[AgentItem::ToolCall { id:crate::ItemId::from_string("original-call-item".into()),call_id:crate::CallId::from_string("actual-call".into()),name:name.into(),arguments:json!({"path":"原文"}) },
                AgentItem::ToolResult { id:crate::ItemId::from_string("original-result-item".into()),call_id:crate::CallId::from_string("actual-call".into()),name:name.into(),output,is_error:false,full_output_ref:None,artifacts:vec![] }];
            Self { values:items.into_iter().enumerate().map(|(index,item)| {
                let raw=serde_json::to_string(&item).unwrap();
                let binding=ToolSourceBinding { source_item_id:item.id().as_str().into(),kind:if index==0 {ToolSourceKind::ToolCall}else{ToolSourceKind::ToolResult},
                    call_id:"actual-call".into(),name:name.into(),owner:owner.clone(),provenance:ToolSourceProvenance::AcceptedRun,body_sha256:format!("{:x}",Sha256::digest(raw.as_bytes())),
                    body:CanonicalBodyReference::for_field(format!("body-{index}"),"conversation".into(),"message".into(),"part".into(),PartBodyField::ContentJson,raw.len() as u64,1).unwrap(),created_revision:2 };
                (binding,raw)
            }).collect() }
        }
    }
    impl ToolSourceRepository for Sources {
        fn tool_sources_for_part(&self,owner:&CanonicalPartOwner,revision:i64)->Result<Vec<ToolSourceBinding>,String> {
            if revision!=2 || owner!=&CanonicalPartOwner::new("conversation".into(),"message".into(),"part".into()).unwrap() { return Err("foreign source owner/revision".into()); }
            Ok(self.values.iter().map(|(binding,_)|binding.clone()).collect())
        }
        fn tool_source_by_id(&self,_:&str,_:&str,_:i64)->Result<Option<ToolSourceBinding>,String> { unreachable!() }
        fn read_tool_source_range(&self,binding:&ToolSourceBinding,revision:i64,offset:u64,limit:usize)->Result<ToolSourceRange,String> {
            if revision!=2 { return Err("stale source".into()); }
            let (_,raw)=self.values.iter().find(|(value,_)|value==binding).ok_or("unknown binding")?;
            let mut end=(offset as usize+limit).min(raw.len());
            while !raw.is_char_boundary(end) {end-=1;}
            Ok(ToolSourceRange {offset_bytes:offset,next_offset_bytes:end as u64,text:raw[offset as usize..end].into(),end_of_content:end==raw.len()})
        }
    }
    fn owner()->CanonicalPartOwner {CanonicalPartOwner::new("conversation".into(),"message".into(),"part".into()).unwrap()}

    #[test]
    fn ordinary_and_control_hydration_preserve_original_ids_without_restoring_call_authority() {
        for name in ["file.read","requirements_update"] {
            let sources=Sources::pair(name,json!({"ok":true,"message":"原始结果"}));
            let mut budget=32768;
            let items=historical_items(&sources,&owner(),2,&mut budget).unwrap();
            assert_eq!(items.len(),2);
            assert_eq!(items[0].id().as_str(),"original-call-item");
            assert_eq!(items[1].id().as_str(),"original-result-item");
            let [AgentItem::ToolCall {call_id,name:call_name,arguments,..},AgentItem::ToolResult {call_id:result_call,name:result_name,output,..}]=items.as_slice() else { panic!("incomplete pair"); };
            assert_eq!(call_id,result_call);
            assert_eq!(call_name,result_name);
            assert!(call_id.as_str().starts_with("history:"));
            assert_ne!(call_id.as_str(),"actual-call");
            assert_eq!(arguments,&json!({"path":"原文"}));
            assert_eq!(output,&json!({"ok":true,"message":"原始结果"}));
            assert!(budget<32768);
        }
    }
    #[test]
    fn historical_tool_sources_keep_model_control_projection_and_explicit_body_budget() {
        let sources=Sources::pair("file.read",json!({"ok":true,"preview_url":"http://127.0.0.1:3000/p?token=secret-fixture", "_nola_control":{"token":"secret-fixture"}}));
        let mut budget=32768;
        let items=historical_items(&sources,&owner(),2,&mut budget).unwrap();
        assert!(!serde_json::to_string(&items).unwrap().contains("secret-fixture"));
        assert!(sources.values[1].1.contains("secret-fixture"),"canonical source original remains untouched");
        let mut budget=1;
        assert!(historical_items(&sources,&owner(),2,&mut budget).unwrap().is_empty());
        assert_eq!(budget,1);
    }
    #[test]
    fn missing_partial_foreign_and_digest_corrupt_sources_never_create_a_pair() {
        let mut sources=Sources::pair("requirements_update",json!({"ok":true}));
        let mut budget=32768;
        assert!(historical_items(&sources,&owner(),3,&mut budget).is_err());
        sources.values[1].0.body_sha256="a".repeat(64);
        assert!(historical_items(&sources,&owner(),2,&mut budget).is_err());
        sources.values.pop();
        assert!(historical_items(&sources,&owner(),2,&mut budget).unwrap().is_empty());
        sources.values.clear();
        assert!(historical_items(&sources,&owner(),2,&mut budget).unwrap().is_empty());
    }

    #[test]
    fn complete_source_read_rejects_broken_range_contracts() {
        struct BrokenRange(ToolSourceRange);
        impl ToolSourceRepository for BrokenRange {
            fn tool_sources_for_part(&self, _: &CanonicalPartOwner, _: i64)
                -> Result<Vec<ToolSourceBinding>, String> { unreachable!() }
            fn tool_source_by_id(&self, _: &str, _: &str, _: i64)
                -> Result<Option<ToolSourceBinding>, String> { unreachable!() }
            fn read_tool_source_range(&self, _: &ToolSourceBinding, _: i64, _: u64, _: usize)
                -> Result<ToolSourceRange, String> { Ok(self.0.clone()) }
        }
        let sources = Sources::pair("file.read", json!({"ok": true}));
        let binding = &sources.values[0].0;
        for range in [
            ToolSourceRange { offset_bytes: 1, next_offset_bytes: 2, text: "x".into(), end_of_content: true },
            ToolSourceRange { offset_bytes: 0, next_offset_bytes: 2, text: "x".into(), end_of_content: true },
            ToolSourceRange { offset_bytes: 0, next_offset_bytes: 0, text: String::new(), end_of_content: false },
            ToolSourceRange { offset_bytes: 0, next_offset_bytes: 1, text: "x".into(), end_of_content: true },
        ] {
            assert!(read_complete(&BrokenRange(range), binding, 2, 8192).is_err());
        }
        // The budget is checked before calling the range repository.
        assert!(read_complete(&sources, binding, 2, 0).is_err());
    }
}

#[cfg(test)]
#[path = "tool_source_scope_tests.rs"]
mod scope_tests;
