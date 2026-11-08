// SPDX-License-Identifier: Apache-2.0
//! Canonical binding and sealing from original Core facts in the same transaction.
use crate::{params, Connection, OptionalExtension, StoreError, StoreResult};
use nola_domain::assistant_journal_operations::JournalNumber;
use nola_domain::core_requirements::*;
use nola_domain::run_requirements::*;
use nola_domain::run_result::{RunResult, RunResultValue};
use nola_domain::core_event::CoreEventSource;
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;

fn invalid(message: &str) -> StoreError {
    StoreError::Invariant(format!("Run requirement provenance: {message}"))
}
struct Source {
    run: String, conversation: String, kind: String, payload: Value,
    envelope: CoreEventSource, digest: String,
    transition_id: Option<String>, host_event_id: Option<String>, host_digest: Option<String>,
}
impl Source {
    fn decision(&self) -> RequirementDecisionRef {
        RequirementDecisionRef { core_session_id: self.envelope.session_id.clone(),
            turn_id: self.envelope.turn_id.clone().expect("validated Turn"),
            revision: JournalNumber(self.envelope.revision),
            event_cursor: JournalNumber(self.envelope.cursor.sequence) }
    }
}
fn source(c: &Connection, id: &str) -> StoreResult<Source> {
    let (run, conversation, kind, raw): (String, String, String, String) = c.query_row(
        "SELECT e.run_id,r.conversation_id,e.event_kind,e.payload_json FROM run_events e
         JOIN runs r ON r.id=e.run_id WHERE event_id=?1", [id],
        |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?)))?;
    let payload: Value = serde_json::from_str(&raw)?;
    let envelope: CoreEventSource = serde_json::from_value(payload["core_envelope"].clone())?;
    if payload["type"] != kind || envelope.revision == 0 || envelope.cursor.sequence == 0
        || envelope.revision > i64::MAX as u64 || envelope.cursor.sequence > i64::MAX as u64
        || envelope.turn_id.as_deref() != payload["turn_id"].as_str()
        || envelope.turn_id.is_none() || id != format!("core:{run}:{}", envelope.cursor.sequence) {
        return Err(invalid("original event identity differs"));
    }
    let owned: bool = c.query_row(
        "SELECT EXISTS(SELECT 1 FROM assistant_turn_message_bindings
         WHERE conversation_id=?1 AND run_id=?2 AND turn_id=?3)",
        params![conversation,run,envelope.turn_id], |r| r.get(0))?;
    if !owned { return Err(invalid("event Turn has no exact Run owner")); }
    let digest = format!("{:x}", Sha256::digest(serde_json::to_vec(&payload)?));
    let transition_id:Option<String>=c.query_row("SELECT t.transition_id FROM committed_transitions t
        JOIN run_events e ON e.event_id=?1 AND e.run_id=t.run_id
        WHERE t.next_event_sequence>e.sequence ORDER BY t.checkpoint_revision LIMIT 1",
        [id],|r|r.get(0)).optional()?;
    let indexed:Option<(Option<String>,Option<String>,Option<String>)>=c.query_row(
        "SELECT source_transition_id,source_host_event_id,source_host_digest FROM run_requirement_sources WHERE source_event_id=?1",
        [id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).optional()?;
    let (transition_id,host_event_id,host_digest)=match indexed {
        Some((original,host,digest)) => {
            // Admission facts precede every transition. A later first commit
            // must never retroactively claim them as its own source.
            if original.is_some() && original!=transition_id {
                return Err(invalid("original source transition boundary differs"));
            }
            (original,host,digest)
        }
        None => (transition_id,None,None),
    };
    Ok(Source { run,conversation,kind,payload,envelope,digest,transition_id,host_event_id,host_digest })
}
fn host_response(c:&Connection,s:&Source)->StoreResult<Value> {
    let id=s.host_event_id.as_deref().ok_or_else(||invalid("decision has no same-transition Host source"))?;
    let (run,raw):(String,String)=c.query_row("SELECT run_id,reduction_json FROM host_event_inbox WHERE event_id=?1",
        [id],|r|Ok((r.get(0)?,r.get(1)?)))?;
    let digest=format!("{:x}",Sha256::digest(raw.as_bytes()));
    if run!=s.run || s.transition_id.is_none() || s.host_digest.as_deref()!=Some(digest.as_str()) {
        return Err(invalid("original Host input owner or digest differs"));
    }
    let reduction:nola_store_port::host_replay::HostReplayReduction=serde_json::from_str(&raw)?;
    Ok(serde_json::from_str(reduction.normalized_response().get())?)
}
fn audit(c: &Connection, s: &Source, refs: &[CoreAppliedInputRef])
    -> StoreResult<crate::run_input_frontier::AuditedRunInput> {
    let input = crate::run_input_frontier::audit_for_decision(c, &s.conversation, &s.run,
        &s.envelope.session_id, s.envelope.revision, s.envelope.cursor.sequence)?;
    if input.applied != refs { return Err(invalid("incomplete or substituted applied input set")); }
    Ok(input)
}
fn accepts_model_terminal_source(kind: &str, has_control_call: bool, incomplete_result: bool) -> bool {
    kind == "model_completed"
        || (kind == "model_output_limited" && !has_control_call && incomplete_result)
}
/// The command acknowledges only this same-transition durable Task promotion.
/// Physical child completion is deliberately not part of this result contract.
fn validate_delegation_acceptance(c: &Connection, s: &Source, task_id: &str, parent_call_id: Option<&str>) -> StoreResult<()> {
    let response = host_response(c, s)?;
    if response["payload"]["type"] != "task_promoted" || response["payload"]["kind"] != "agent"
        || response["payload"]["task_id"] != task_id
        || response["session_id"] != s.envelope.session_id
        || response["turn_id"].as_str() != s.envelope.turn_id.as_deref() {
        return Err(invalid("delegation result has no exact same-transition TaskPromoted source"));
    }
    let mut query = c.prepare("SELECT source_event_id,host_event_id,evidence_id FROM execution_evidence_sources
        WHERE run_id=?1 AND host_event_id=?2 AND core_revision=?3 AND core_cursor<?4")?;
    let sources = query.query_map(params![s.run,s.host_event_id,s.envelope.revision,s.envelope.cursor.sequence],
        |row| Ok((row.get::<_,String>(0)?, row.get::<_,String>(1)?, row.get::<_,String>(2)?)))?.collect::<Result<Vec<_>,_>>()?;
    if sources.len() != 1 { return Err(invalid("delegation result has no unique original tool handoff")); }
    // Indexing already validated the exact dispatch before pruning. Revalidate
    // immutable source digests, not a disposable outbox row, on cold reads.
    validate_evidence(c, s, &sources[0].2, s.envelope.cursor.sequence)?;
    let (evidence, _, _) = crate::execution_evidence::read_source(c, &sources[0].0, &sources[0].1, false)?;
    if evidence.run_id != s.run || evidence.conversation_id != s.conversation
        || evidence.observation.tool_name != "spawn_agent"
        || evidence.observation.outcome != nola_domain::host_effect::HostToolOutcome::Deferred
        || parent_call_id.is_some_and(|call| call != evidence.observation.call_id) {
        return Err(invalid("delegation result differs from its exact parent tool admission"));
    }
    Ok(())
}

fn decision_source(c: &Connection, s: &Source, decision: &CoreDecisionSource, incomplete_result: bool) -> StoreResult<()> {
    match decision {
        CoreDecisionSource::Model { effect_id, request_id, attempt_id, call_id } => {
                let response = host_response(c,s)?;
                if response["session_id"] != s.envelope.session_id
                    || response["effect_id"].as_str()!=Some(effect_id.as_str())
                    || response["request_id"].as_str()!=Some(request_id.as_str())
                    || response["turn_id"].as_str() != s.envelope.turn_id.as_deref()
                    || response["attempt_id"].as_str() != attempt_id.as_deref() {
                    return Err(invalid(&format!("decision source identity differs from original model response: effect={}, request={}, turn={}, attempt={}", response["effect_id"] == effect_id.as_str(), response["request_id"] == request_id.as_str(), response["turn_id"].as_str() == s.envelope.turn_id.as_deref(), response["attempt_id"].as_str() == attempt_id.as_deref())));
                }
                if response["payload"]["type"] == "delegated_agent_event" {
                    // Delegated completion is reduced by Core as ModelCompleted,
                    // but its original Host receipt must remain untouched. Only
                    // the Completed variant can witness a model decision, never
                    // progress, failure or a cancellation confirmation.
                    let event: nola_domain::host_effect::DelegatedAgentEventV1 =
                        serde_json::from_value(response["payload"]["event"].clone())?;
                    event.validate().map_err(invalid)?;
                    if !matches!(event, nola_domain::host_effect::DelegatedAgentEventV1::Completed { .. })
                        || call_id.is_some() {
                        return Err(invalid("delegated decision requires a completed event without a control call"));
                    }
                    let stored = crate::outbox_payload::read(c,effect_id)?
                        .ok_or_else(||invalid("delegated decision original request missing"))?;
                    let request: Value = serde_json::from_str(&stored.payload_json)?;
                    if stored.run_id != s.run || stored.request_id != *request_id
                        || request["payload"]["type"] != "delegated_agent"
                        || request["session_id"] != response["session_id"]
                        || request["turn_id"] != response["turn_id"]
                        || request["attempt_id"] != response["attempt_id"]
                        || request["delegated_invocation_id"] != response["delegated_invocation_id"]
                        || !request["delegated_invocation_id"].as_str().is_some_and(|id| !id.is_empty()) {
                        return Err(invalid("delegated decision differs from original invocation"));
                    }
                } else if !accepts_model_terminal_source(
                    response["payload"]["type"].as_str().unwrap_or(""), call_id.is_some(), incomplete_result) {
                    return Err(invalid(&format!("model response cannot witness this decision: type={}, control={}, incomplete={}", response["payload"]["type"], call_id.is_some(), incomplete_result)));
                }
                if let Some(call) = call_id {
                    if !response["payload"]["tool_calls"].as_array().is_some_and(|calls|
                        calls.iter().any(|entry| entry["call_id"].as_str() == Some(call))) {
                        return Err(invalid("decision call absent from original model response"));
                    }
                }
                Ok(())
        }
        CoreDecisionSource::Runtime { code, source_refs } => {
            crate::require_id("runtime decision code", code)?;
            if code == "delegation_accepted" {
                if source_refs.len() != 1 { return Err(invalid("delegation result requires its one Task source")); }
                validate_delegation_acceptance(c, s, &source_refs[0], None)?;
            } else if code == "input_applied" {
                if source_refs.is_empty() { return Err(invalid("scope has no applied input origin")); }
                for admission in source_refs {
                    let exists: bool = c.query_row("SELECT EXISTS(SELECT 1 FROM input_turn_applications
                        WHERE core_session_id=?1 AND source_run_id=?2 AND admission_id=?3 AND core_cursor<?4)",
                        params![s.envelope.session_id,s.run,admission,s.envelope.cursor.sequence], |r|r.get(0))?;
                    if !exists { return Err(invalid("runtime input reference is not applied")); }
                }
            } else {
                for reference in source_refs { validate_fact_reference(c,s,reference,None)?; }
                if source_refs.is_empty() { return Err(invalid("runtime result has no committed cause")); }
            }
            Ok(())
        }
    }
}
fn scope(c: &Connection, s: &Source) -> StoreResult<(CoreRequirementScope, RequirementSetRevision)> {
    let (semantic,canonical)=scope_snapshot(c,s)?;
    validate_scope_change(c,s,&semantic)?;
    Ok((semantic,canonical))
}
fn scope_snapshot(c: &Connection, s: &Source) -> StoreResult<(CoreRequirementScope, RequirementSetRevision)> {
    let semantic: CoreRequirementScope = serde_json::from_value(s.payload["scope"].clone())?;
    crate::require_id("requirement scope", &semantic.scope_id)?;
    let input = audit(c,s,&semantic.applied_input_refs)?;
    let refs: BTreeSet<_> = semantic.source_input_refs.iter().cloned().collect();
    if refs.len() != semantic.source_input_refs.len() || refs != input.item_ids {
        return Err(invalid("requirement scope omits or substitutes original input items"));
    }
    for requirement in &semantic.requirements {
        if requirement.source_refs.iter().any(|reference| !refs.contains(reference)) {
            return Err(invalid("requirement refers to unapplied input"));
        }
        if let RequirementResourceScope::Declared { interpreted_user_frontier, exclusions, .. } = &requirement.resource_scope {
            for reference in interpreted_user_frontier.iter().chain(exclusions.iter()
                .filter(|entry| entry.provenance == RequirementScopeInterpretationSource::UserInputInterpretation)
                .flat_map(|entry| &entry.source_refs)) {
                let user: bool = c.query_row("SELECT EXISTS(SELECT 1 FROM messages WHERE id=?1 AND conversation_id=?2 AND role='user')",
                    params![reference,s.conversation], |r|r.get(0))?;
                if !user { return Err(invalid("resource user interpretation references non-user source")); }
            }
        }
    }
    decision_source(c,s,&semantic.source,false)?;
    let canonical = RequirementSetRevision {
        conversation_id:s.conversation.clone(),run_id:s.run.clone(),revision:semantic.revision,
        input:input.commitment,source_input_refs:semantic.source_input_refs.clone(),
        source_decision_ref:s.decision(),requirements:semantic.requirements.clone(),
    };
    canonical.validate().map_err(|e| invalid(&e))?;
    Ok((semantic,canonical))
}
fn validate_scope_change(c:&Connection,s:&Source,next:&CoreRequirementScope)->StoreResult<()> {
    if next.revision.0==1 {
        if !matches!(&next.source,CoreDecisionSource::Runtime {code,..} if code=="input_applied") {
            return Err(invalid("initial scope must originate in applied input"));
        }
        return Ok(());
    }
    let previous_source=scope_source(c,s,&next.scope_id,JournalNumber(next.revision.0-1))?;
    let (previous_semantic,previous)=scope_snapshot(c,&previous_source)?;
    let mut user_frontier = Vec::new();
    for id in &next.source_input_refs {
        let user: bool = c.query_row("SELECT EXISTS(SELECT 1 FROM messages WHERE id=?1 AND conversation_id=?2 AND role='user')",
            params![id,s.conversation], |r|r.get(0))?;
        if user { user_frontier.push(id.clone()); }
    }
    validate_resource_scope_revision(&previous.requirements, &next.requirements, &user_frontier).map_err(|e|invalid(&e))?;
    match &next.source {
        CoreDecisionSource::Runtime {code,..} if code=="input_applied" => {
            if !next.requirements.starts_with(&previous.requirements)
                || next.requirements.len()<=previous.requirements.len() {
                return Err(invalid("new input silently retires or rewrites an old requirement"));
            }
        }
        CoreDecisionSource::Model {call_id:Some(call_id),..} => {
            let response=host_response(c,s)?;
            let call=response["payload"]["tool_calls"].as_array().and_then(|calls|
                calls.iter().find(|call|call["call_id"].as_str()==Some(call_id.as_str())
                    && call["name"]=="task_scope_update"))
                .ok_or_else(||invalid("scope change has no original task_scope_update call"))?;
            let bound = {
                let binding = frozen_requirement_request(c, s, &next.source)?;
                let expected_requirements = previous.requirements.iter().enumerate()
                    .map(|(i,r)|(format!("r{}",i+1),r.requirement_id.clone())).collect();
                if binding.scope_id != next.scope_id || binding.expected_requirement_revision != previous.revision
                    || binding.requirements != expected_requirements {
                    return Err(invalid("scope aliases differ from original request scope"));
                }
                let declared: BTreeSet<_> = binding.sources.values().cloned().collect();
                let actual: BTreeSet<_> = previous.source_input_refs.iter().cloned().collect();
                if declared != actual { return Err(invalid("scope source aliases differ from applied inputs")); }
                let mut users = BTreeSet::new();
                for id in &previous.source_input_refs {
                    let user: bool = c.query_row("SELECT EXISTS(SELECT 1 FROM messages WHERE id=?1 AND conversation_id=?2 AND role='user')",
                        params![id,s.conversation], |r|r.get(0))?;
                    if user { users.insert(id.clone()); }
                }
                if binding.user_sources.values().cloned().collect::<BTreeSet<_>>() != users {
                    return Err(invalid("scope user aliases differ from input roles"));
                }
                nola_domain::requirement_control::bind_model_task_scope(&call["arguments"], &binding).map_err(|e| invalid(&e))?
            };
            let args = &bound;
            let expected:JournalNumber=serde_json::from_value(args["expected_requirement_revision"].clone())?;
            if args["scope_id"]!=next.scope_id || expected!=previous.revision {
                return Err(invalid("scope control expected source version differs"));
            }
            let token=format!("{:x}",Sha256::digest(serde_json::to_vec(&(
                &s.envelope.session_id,&next.scope_id,previous.revision,&previous_semantic.applied_input_refs))?));
            if args["expected_input_frontier_token"].as_str()!=Some(token.as_str()) {
                return Err(invalid("scope control input frontier token differs"));
            }
            let operations=args["operations"].as_array().ok_or_else(||invalid("scope control operations missing"))?;
            let revisions:Vec<_>=operations.iter().filter(|op|op["kind"]=="revise_scope").collect();
            if revisions.len()!=1 { return Err(invalid("scope control must contain one explicit revision")); }
            let op=revisions[0];
            let added=op["requirements"].as_array().ok_or_else(||invalid("new requirements missing"))?;
            let dispositions=op["dispositions"].as_array().ok_or_else(||invalid("old dispositions missing"))?;
            if added.len()>next.requirements.len() { return Err(invalid("new requirements absent from scope")); }
            let mut keys=std::collections::BTreeMap::new();
            for (planned,actual) in added.iter().zip(&next.requirements) {
                let key=planned["key"].as_str().filter(|key|!key.is_empty()).ok_or_else(||invalid("new requirement key missing"))?;
                // Compare the current typed contract, not JSON spelling:
                // optional version/predicate may be omitted or explicit null.
                let resource_scope: RequirementResourceScope =
                    serde_json::from_value(planned["resource_scope"].clone())?;
                if keys.insert(key,actual.requirement_id.as_str()).is_some()
                    || planned["description"]!=actual.description
                    || planned["source_refs"]!=serde_json::to_value(&actual.source_refs)?
                    || resource_scope != actual.resource_scope
                    || actual.goal_requirement_ref.is_some()
                    || previous.requirements.iter().any(|old|old.requirement_id==actual.requirement_id) {
                    return Err(invalid("new requirement differs from original control"));
                }
            }
            let resolve=|reference:&Value|->StoreResult<String> {
                match reference["kind"].as_str() {
                    Some("existing_id") => reference["id"].as_str().map(str::to_owned)
                        .ok_or_else(||invalid("existing requirement reference missing")),
                    Some("local_key") => reference["key"].as_str().and_then(|key|keys.get(key))
                        .map(|id|(*id).to_owned()).ok_or_else(||invalid("local requirement reference missing")),
                    _ => Err(invalid("unknown requirement reference kind")),
                }
            };
            for (planned,actual) in added.iter().zip(&next.requirements) {
                let dependencies=planned["dependencies"].as_array().ok_or_else(||invalid("dependencies missing"))?
                    .iter().map(&resolve).collect::<StoreResult<Vec<_>>>()?;
                if dependencies!=actual.dependency_ids { return Err(invalid("requirement dependency binding differs")); }
            }
            let mut seen=BTreeSet::new();
            let mut retained=Vec::new();
            for disposition in dispositions {
                let id=disposition["requirement_id"].as_str().ok_or_else(||invalid("disposition identity missing"))?;
                let old=previous.requirements.iter().find(|r|r.requirement_id==id)
                    .ok_or_else(||invalid("disposition refers to nonexistent prior requirement"))?;
                if !seen.insert(id) { return Err(invalid("duplicate requirement disposition")); }
                match disposition["kind"].as_str() {
                    Some("retain") => retained.push(old.clone()),
                    Some("replace")|Some("withdraw") => {
                        if disposition["reason"].as_str().is_none_or(|s|s.trim().is_empty()) {
                            return Err(invalid("requirement retirement has no explicit reason"));
                        }
                        let refs=disposition["user_input_refs"].as_array().filter(|refs|!refs.is_empty())
                            .ok_or_else(||invalid("requirement retirement has no original user input"))?;
                        let user_refs = refs.iter().map(|value| value.as_str().map(str::to_owned)
                            .ok_or_else(||invalid("user input identity missing"))).collect::<StoreResult<Vec<_>>>()?;
                        validate_resource_scope_retirement(old, &next.requirements, &user_refs).map_err(|e|invalid(&e))?;
                        for reference in refs {
                            let id=reference.as_str().ok_or_else(||invalid("user input identity missing"))?;
                            let user:bool=c.query_row("SELECT EXISTS(SELECT 1 FROM messages WHERE id=?1 AND conversation_id=?2 AND role='user')",
                                params![id,s.conversation],|r|r.get(0))?;
                            if !user || !previous.source_input_refs.iter().any(|r|r==id) {
                                return Err(invalid("retirement user input was not applied to this scope"));
                            }
                        }
                        if disposition["kind"]=="replace" {
                            let successors=disposition["successors"].as_array().filter(|refs|!refs.is_empty())
                                .ok_or_else(||invalid("replacement requirements missing"))?;
                            for reference in successors {
                                let id=resolve(reference)?;
                                if !next.requirements.iter().any(|r|r.requirement_id==id) {
                                    return Err(invalid("replacement is absent from final scope"));
                                }
                            }
                        }
                    }
                    _ => return Err(invalid("unknown requirement disposition")),
                }
            }
            if seen.len()!=previous.requirements.len() || next.requirements[added.len()..]!=retained {
                return Err(invalid("scope omits or mutates retained requirements"));
            }
        }
        _ => return Err(invalid("scope revision has no authorized source operation")),
    }
    Ok(())
}
fn indexed_source(c: &Connection, id: &str) -> StoreResult<Source> {
    let s = source(c,id)?;
    let indexed: (String,String,String,String,u64,u64) = c.query_row(
        "SELECT run_id,conversation_id,core_session_id,source_digest,core_revision,core_cursor
         FROM run_requirement_sources WHERE source_event_id=?1", [id],
        |r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?)))?;
    if indexed != (s.run.clone(),s.conversation.clone(),s.envelope.session_id.clone(),
        s.digest.clone(),s.envelope.revision,s.envelope.cursor.sequence) {
        return Err(invalid("source index differs from original event"));
    }
    let (kind,entity,scope_id,revision,turn):(String,String,String,u64,String)=c.query_row(
        "SELECT kind,entity_id,scope_id,requirement_revision,turn_id FROM run_requirement_sources WHERE source_event_id=?1",
        [id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?)))?;
    let expected=semantic_identity(&s)?;
    if kind!=s.kind || (entity,scope_id,revision)!=expected
        || Some(turn.as_str())!=s.envelope.turn_id.as_deref() {
        return Err(invalid("source lookup keys differ from original semantic identity"));
    }
    let transition:Option<String>=c.query_row("SELECT source_transition_id FROM run_requirement_sources
        WHERE source_event_id=?1",[id],|r|r.get(0))?;
    if transition!=s.transition_id { return Err(invalid("source transition differs from original event boundary")); }
    if s.host_event_id.is_some() { host_response(c,&s)?; }
    Ok(s)
}
fn semantic_identity(s:&Source)->StoreResult<(String,String,u64)> {
    Ok(match s.kind.as_str() {
        "requirement_scope_changed" => {
            let v:CoreRequirementScope=serde_json::from_value(s.payload["scope"].clone())?;
            (format!("{}:{}",v.scope_id,v.revision.0),v.scope_id,v.revision.0)
        }
        "requirement_assessed" => {
            let v:CoreRequirementAssessment=serde_json::from_value(s.payload["assessment"].clone())?;
            (v.assessment_id,v.scope_id,v.requirement_set_revision.0)
        }
        "run_result_decided" => {
            let v:CoreRunResultDecision=serde_json::from_value(s.payload["decision"].clone())?;
            (v.result_id,v.scope_id,v.requirement_revision.0)
        }
        "runtime_decision_fact_observed" => {
            let v:CoreRuntimeDecisionFact=serde_json::from_value(s.payload["fact"].clone())?;
            (v.fact_id,String::new(),0)
        }
        _ => return Err(invalid("unknown control source kind")),
    })
}
fn exact_scope(c: &Connection, s: &Source, scope_id: &str, revision: JournalNumber)
    -> StoreResult<RequirementSetRevision> {
    let original=scope_source(c,s,scope_id,revision)?;
    let (_, canonical) = scope(c,&original)?;
    Ok(canonical)
}
fn scope_source(c:&Connection,s:&Source,scope_id:&str,revision:JournalNumber)->StoreResult<Source> {
    let id: String = c.query_row("SELECT source_event_id FROM run_requirement_sources
        WHERE run_id=?1 AND kind='requirement_scope_changed' AND scope_id=?2 AND requirement_revision=?3
        AND core_cursor<=?4",params![s.run,scope_id,revision.0,s.envelope.cursor.sequence],|r|r.get(0))?;
    indexed_source(c,&id)
}
fn assessment(c: &Connection, s: &Source) -> StoreResult<RequirementAssessment> {
    let semantic: CoreRequirementAssessment = serde_json::from_value(s.payload["assessment"].clone())?;
    let input = audit(c,s,&semantic.applied_input_refs)?;
    decision_source(c,s,&semantic.source,false)?;
    if !matches!(semantic.source, CoreDecisionSource::Model { .. }) {
        return Err(invalid("model judgment must originate in an accepted model response"));
    }
    if !matches!(semantic.method, RequirementAssessmentMethod::ModelJudgment {}) {
        return Err(invalid("unimplemented inspection/user acceptance authority"));
    }
    if semantic.evidence_frontier.0 > s.envelope.cursor.sequence {
        return Err(invalid("assessment evidence frontier is future"));
    }
    for reference in &semantic.evidence_refs {
        validate_evidence(c,s,reference,semantic.evidence_frontier.0)?;
    }
    let requirements = exact_scope(c,s,&semantic.scope_id,semantic.requirement_set_revision)?;
    let CoreDecisionSource::Model {call_id:Some(call_id),..}=&semantic.source else {
        return Err(invalid("assessment requires an explicit original control call"));
    };
    let response=host_response(c,s)?;
    let control=response["payload"]["tool_calls"].as_array().and_then(|calls|calls.iter().find(|call|
        call["call_id"].as_str()==Some(call_id.as_str()) && call["name"]=="requirements_update"))
        .ok_or_else(||invalid("assessment source is not requirements_update"))?;
    let bound = {
        // Re-audit the retained, digest-checked outbound request on cold reads too.
        // The semantic event alone must not substitute for the frozen model input.
        let binding = frozen_assessment_request(c, s, &semantic)?;
        let (original_scope, _) = scope_snapshot(c, &scope_source(c, s,
            &semantic.scope_id, semantic.requirement_set_revision)?)?;
        let token = format!("{:x}", Sha256::digest(serde_json::to_vec(&(
            &s.envelope.session_id, &semantic.scope_id, semantic.requirement_set_revision,
            &original_scope.applied_input_refs))?));
        let expected_requirements = requirements.requirements.iter().enumerate()
            .map(|(i, requirement)| (format!("r{}", i + 1), requirement.requirement_id.clone()))
            .collect();
        if binding.scope_id != semantic.scope_id
            || binding.expected_requirement_revision != semantic.requirement_set_revision
            || binding.expected_input_frontier_token != token
            || binding.requirements != expected_requirements
        {
            return Err(invalid("compact assessment request scope differs"));
        }
        nola_domain::requirement_control::bind_model_task_result(
            &control["arguments"], &binding).map_err(|error| invalid(&error))?
    };
    let arguments = &bound;
    let operations=arguments["operations"].as_array()
        .ok_or_else(||invalid("assessment control operations missing"))?;
    let mut matched=false;
    for op in operations.iter().filter(|op|op["kind"]=="assess") {
        let reference=&op["requirement"];
        let id=match reference["kind"].as_str() {
            Some("existing_id") => reference["id"].as_str(),
            _ => None,
        };
        if id==Some(semantic.requirement_id.as_str())
            && op["verdict"]==serde_json::to_value(semantic.verdict)?
            && op["rationale"]==semantic.rationale
            && op["evidence_refs"]==serde_json::to_value(&semantic.evidence_refs)? {
            matched=true;
        }
    }
    if !matched { return Err(invalid("assessment differs from original model control operation")); }
    let canonical = RequirementAssessment {
        assessment_id:semantic.assessment_id,run_id:s.run.clone(),
        requirement_id:semantic.requirement_id,requirement_set_revision:semantic.requirement_set_revision,
        assessed_input:input.commitment,evidence_frontier:semantic.evidence_frontier,
        source_decision_ref:s.decision(),method:semantic.method,evidence_refs:semantic.evidence_refs,
        verdict:semantic.verdict,rationale:semantic.rationale,
    };
    canonical.validate_against(&requirements).map_err(|e|invalid(&e))?;
    Ok(canonical)
}

/// Authenticate aliases against the retained, digest-checked dispatched request
/// both at acceptance and during subsequent cold audits.
fn validate_assessment_request(c: &Connection, s: &Source,
    semantic: &CoreRequirementAssessment) -> StoreResult<()>
{
    if semantic.request_binding.is_some() { frozen_assessment_request(c, s, semantic)?; }
    Ok(())
}
fn frozen_assessment_request(c: &Connection, s: &Source,
    semantic: &CoreRequirementAssessment)
    -> StoreResult<nola_domain::requirement_control::RequirementRequestBinding>
{
    let binding = frozen_requirement_request(c, s, &semantic.source)?;
    if semantic.request_binding.as_ref().is_some_and(|recorded| recorded != &binding) {
        return Err(invalid("assessment aliases differ from the original model request"));
    }
    Ok(binding)
}
fn frozen_requirement_request(c: &Connection, s: &Source, source: &CoreDecisionSource)
    -> StoreResult<nola_domain::requirement_control::RequirementRequestBinding>
{
    let CoreDecisionSource::Model { effect_id, request_id, .. } = source else {
        return Err(invalid("assessment binding has no model request"));
    };
    let outbox = crate::outbox_payload::read(c, effect_id)?
        .ok_or_else(|| invalid("assessment binding has no dispatched request"))?;
    let request: Value = serde_json::from_str(&outbox.payload_json)?;
    let frozen = match request["payload"]["type"].as_str() {
        Some("model_stream") => &request["payload"]["settings"]["requirement_binding"],
        Some("delegated_agent") => &request["payload"]["request"]["settings"]["requirement_binding"],
        _ => return Err(invalid("assessment binding source is not a model request")),
    };
    if outbox.run_id != s.run || outbox.request_id != *request_id
        || request["session_id"] != s.envelope.session_id
        || request["turn_id"].as_str() != s.envelope.turn_id.as_deref()
    {
        return Err(invalid("assessment frozen request identity differs"));
    }
    let binding = serde_json::from_value(frozen.clone())?;
    Ok(binding)
}
fn validate_evidence(c: &Connection, s: &Source, reference: &str, frontier: u64) -> StoreResult<()> {
    let (id,host,digest,host_digest,cursor):(String,String,String,String,u64) = c.query_row(
        "SELECT source_event_id,host_event_id,source_digest,host_reduction_digest,core_cursor
         FROM execution_evidence_sources WHERE run_id=?1 AND evidence_id=?2",
        params![s.run,reference],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?)))?;
    let (original,actual_digest,actual_host_digest) =
        crate::execution_evidence::read_source(c,&id,&host,false)?;
    if cursor > frontier || original.core_event_cursor != cursor
        || original.run_id != s.run || original.conversation_id != s.conversation
        || original.core_session_id != s.envelope.session_id
        || original.observation.evidence_id != reference
        || actual_digest != digest || actual_host_digest != host_digest {
        return Err(invalid("evidence reference differs from original receipt"));
    }
    Ok(())
}
pub(crate) fn activity_error_text(c: &Connection, conversation: &str, run: &str, error_ref: &str) -> StoreResult<Option<String>> {
    let event: Option<String> = c.query_row("SELECT s.source_event_id FROM run_requirement_sources s
        JOIN runs r ON r.id=s.run_id WHERE r.conversation_id=?1 AND s.run_id=?2
        AND s.kind='runtime_decision_fact_observed' AND s.entity_id=?3",
        params![conversation,run,error_ref], |r| r.get(0)).optional()?;
    let Some(event) = event else { return Ok(None); };
    let original = indexed_source(c, &event)?;
    let fact: CoreRuntimeDecisionFact = serde_json::from_value(original.payload["fact"].clone())?;
    match fact.kind {
        CoreRuntimeDecisionFactKind::Error { detail, .. } => Ok(Some(detail)),
        _ => Err(invalid("activity failure reference is not an error fact")),
    }
}
fn validate_fact_reference(c: &Connection,s: &Source,id: &str,kind: Option<&str>) -> StoreResult<CoreRuntimeDecisionFact> {
    let event: String = c.query_row("SELECT source_event_id FROM run_requirement_sources
        WHERE run_id=?1 AND kind='runtime_decision_fact_observed' AND entity_id=?2 AND core_cursor<?3",
        params![s.run,id,s.envelope.cursor.sequence],|r|r.get(0))?;
    let original=indexed_source(c,&event)?;
    let fact:CoreRuntimeDecisionFact=serde_json::from_value(original.payload["fact"].clone())?;
    if fact.fact_id!=id || kind.is_some_and(|kind| original.payload["fact"]["kind"]["kind"]!=kind) {
        return Err(invalid("runtime fact differs from result reference"));
    }
    if matches!(&fact.kind, CoreRuntimeDecisionFactKind::RecoveryStopped { .. })
        && (original.transition_id.is_none() || original.transition_id != s.transition_id)
    {
        return Err(invalid("recovery result borrowed a stop from another transition"));
    }
    validate_runtime_fact(c,&original,&fact)?;
    Ok(fact)
}
/// Called only after the original event/ownership/input facts have been committed
/// inside the still-open admission/transition transaction.
pub(crate) fn index_event(c: &Connection,id: &str,host:Option<&nola_store_port::run_effect::HostEventInput>) -> StoreResult<()> {
    let kind:String=c.query_row("SELECT event_kind FROM run_events WHERE event_id=?1",[id],|r|r.get(0))?;
    if !matches!(kind.as_str(),"requirement_scope_changed"|"requirement_assessed"|"run_result_decided"|"runtime_decision_fact_observed") { return Ok(()); }
    if c.is_autocommit() { return Err(invalid("source indexing requires transition transaction")); }
    let mut s=source(c,id)?;
    if let Some(host)=host {
        s.host_event_id=Some(host.event_id.clone());
        s.host_digest=Some(format!("{:x}",Sha256::digest(host.reduction_json.as_deref()
            .ok_or_else(||invalid("Host decision lacks normalized reducer input"))?.as_bytes())));
        host_response(c,&s)?;
        if kind == "runtime_decision_fact_observed"
            && s.payload["fact"]["kind"]["kind"] == "recovery_stopped"
            && s.payload["fact"]["kind"]["cause"]["kind"] == "budget_exhausted"
        {
            // An ordered flush can reduce a previously buffered tool response
            // in a later Core revision than the Host input that flushed it.
            // Bind the stop to that revision's validated original observation,
            // never substitute the flush trigger for the actual tool source.
            let mut statement = c.prepare("SELECT evidence_id,host_event_id FROM execution_evidence_sources
                WHERE run_id=?1 AND core_revision=?2 AND core_cursor<?3")?;
            let sources = statement.query_map(params![s.run,s.envelope.revision,s.envelope.cursor.sequence],
                |row| Ok((row.get::<_,String>(0)?,row.get::<_,String>(1)?)))?
                .collect::<Result<Vec<_>,_>>()?;
            if sources.len() > 1 { return Err(invalid("recovery stop has ambiguous tool sources")); }
            if let Some((reference, source_host)) = sources.first() {
                validate_evidence(c, &s, reference, s.envelope.cursor.sequence)?;
                let raw: String = c.query_row("SELECT reduction_json FROM host_event_inbox WHERE event_id=?1 AND run_id=?2",
                    params![source_host,s.run], |row| row.get(0))?;
                s.host_event_id = Some(source_host.clone());
                s.host_digest = Some(format!("{:x}",Sha256::digest(raw.as_bytes())));
                host_response(c, &s)?;
            }
        }
    }
    let sealed:bool=c.query_row("SELECT EXISTS(SELECT 1 FROM assistant_run_terminal_seals WHERE run_id=?1)",
        [&s.run],|r|r.get(0))?;
    if sealed { return Err(invalid("sealed Run cannot acquire new control facts")); }
    let valid:bool=c.query_row("SELECT EXISTS(SELECT 1 FROM core_delivery_witnesses
        WHERE conversation_id=?1 AND json_extract(witness_json,'$.session_id')=?2
        AND event_cursor>=?3 AND json_extract(witness_json,'$.core_revision')>=?4)",
        params![s.conversation,s.envelope.session_id,s.envelope.cursor.sequence,s.envelope.revision],|r|r.get(0))?;
    if !valid { return Err(invalid("source has no current Core commit witness")); }
    let old:Option<String>=c.query_row("SELECT source_digest FROM run_requirement_sources WHERE source_event_id=?1",
        [id],|r|r.get(0)).optional()?;
    if let Some(old)=old {
        let original=indexed_source(c,id)?;
        return if old==s.digest && original.host_event_id==s.host_event_id
            && original.host_digest==s.host_digest && original.transition_id==s.transition_id {
            Ok(())
        } else { Err(invalid("source replay differs")) };
    }
    let (entity,scope_id,revision)=match kind.as_str() {
        "requirement_scope_changed" => {
            let (semantic,_)=scope(c,&s)?;
            let prior:Option<(String,u64)>=c.query_row("SELECT scope_id,requirement_revision FROM run_requirement_sources
                WHERE run_id=?1 AND kind='requirement_scope_changed' ORDER BY core_cursor DESC LIMIT 1",
                [&s.run],|r|Ok((r.get(0)?,r.get(1)?))).optional()?;
            if prior.as_ref().is_some_and(|(id,rev)| id!=&semantic.scope_id || rev.checked_add(1)!=Some(semantic.revision.0))
                || (prior.is_none() && semantic.revision.0!=1) { return Err(invalid("scope revision is not contiguous")); }
            let cross:bool=c.query_row("SELECT EXISTS(SELECT 1 FROM run_requirement_sources
                WHERE core_session_id=?1 AND scope_id=?2 AND run_id<>?3)",
                params![s.envelope.session_id,semantic.scope_id,s.run],|r|r.get(0))?;
            if cross { return Err(invalid("scope is owned by another Run")); }
            (format!("{}:{}",semantic.scope_id,semantic.revision.0),semantic.scope_id,semantic.revision.0)
        }
        "requirement_assessed" => {
            let semantic:CoreRequirementAssessment=serde_json::from_value(s.payload["assessment"].clone())?;
            validate_assessment_request(c, &s, &semantic)?;
            assessment(c,&s)?;
            (semantic.assessment_id,semantic.scope_id,semantic.requirement_set_revision.0)
        }
        "run_result_decided" => {
            let semantic:CoreRunResultDecision=serde_json::from_value(s.payload["decision"].clone())?;
            bind_result(c,&s)?;
            (semantic.result_id,semantic.scope_id,semantic.requirement_revision.0)
        }
        _ => {
            let fact:CoreRuntimeDecisionFact=serde_json::from_value(s.payload["fact"].clone())?;
            crate::require_id("runtime fact",&fact.fact_id)?;
            validate_runtime_fact(c,&s,&fact)?;
            (fact.fact_id,String::new(),0)
        }
    };
    c.execute("INSERT INTO run_requirement_sources VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15)",
        params![id,s.run,s.conversation,s.envelope.session_id,s.envelope.turn_id,kind,entity,scope_id,revision,
            s.envelope.revision,s.envelope.cursor.sequence,s.digest,s.transition_id,s.host_event_id,s.host_digest])?;
    Ok(())
}
fn validate_runtime_fact(c:&Connection,s:&Source,fact:&CoreRuntimeDecisionFact)->StoreResult<()> {
    match &fact.kind {
        CoreRuntimeDecisionFactKind::RecoveryStopped { cause, detail, consumed_model_requests } => {
            crate::require_id("recovery stop detail", detail)?;
            let response = host_response(c, s)?;
            if response["session_id"] != s.envelope.session_id
                || response["turn_id"].as_str() != s.envelope.turn_id.as_deref()
            {
                return Err(invalid("recovery stop has a foreign Host transition"));
            }
            let raw: Option<String> = c.query_row(
                "SELECT payload_json FROM run_events WHERE run_id=?1 AND event_kind='engineering_loop_updated'
                 AND json_extract(payload_json,'$.core_envelope.revision')=?2
                 AND json_extract(payload_json,'$.core_envelope.cursor.sequence')<?3 ORDER BY sequence DESC LIMIT 1",
                params![s.run, s.envelope.revision, s.envelope.cursor.sequence], |row| row.get(0)).optional()?;
            let witness: Value = serde_json::from_str(&raw.ok_or_else(|| invalid("recovery stop lacks same-transition accounting witness"))?)?;
            if witness["turn_id"].as_str() != s.envelope.turn_id.as_deref()
                || witness["state"]["no_progress_recovery"]["state"] != "pending"
                || witness["state"]["strategy_recovery"]["model_requests"] != serde_json::to_value(consumed_model_requests)?
                || consumed_model_requests.iter().any(|number| *number == 0)
                || consumed_model_requests.windows(2).any(|pair| pair[0] >= pair[1])
            {
                return Err(invalid("recovery stop differs from original accounting witness"));
            }
            match cause {
                RecoveryStopCause::BudgetExhausted { limit } => {
                    // A final tool handoff also calls request_model and may
                    // exhaust an already pending recovery budget. Accept only
                    // its exact, previously indexed Deferred promotion source.
                    let trigger_valid = match response["payload"]["type"].as_str() {
                        Some("model_completed" | "model_output_limited" | "tool_completed") => true,
                        Some("task_promoted") => {
                            let mut statement = c.prepare("SELECT evidence_id FROM execution_evidence_sources
                                WHERE run_id=?1 AND host_event_id=?2 AND core_revision=?3 AND core_cursor<?4")?;
                            let ids = statement.query_map(params![s.run,s.host_event_id,s.envelope.revision,s.envelope.cursor.sequence],
                                |row| row.get::<_,String>(0))?.collect::<Result<Vec<_>,_>>()?;
                            if ids.len() != 1 { return Err(invalid("recovery promotion has no unique original handoff")); }
                            validate_evidence(c, s, &ids[0], s.envelope.cursor.sequence)?;
                            true
                        }
                        _ => false,
                    };
                    let (config, digest): (String, String) = c.query_row(
                        "SELECT p.config_json,r.invocation_config_digest FROM run_invocation_payloads p JOIN runs r ON r.id=p.run_id WHERE p.run_id=?1",
                        [&s.run], |row| Ok((row.get(0)?, row.get(1)?)))
                        .optional()?.ok_or_else(|| invalid("recovery exhaustion lacks frozen invocation payload"))?;
                    let frozen: Value = serde_json::from_str(&config)?;
                    if format!("{:x}", Sha256::digest(config.as_bytes())) != digest
                        || frozen["max_strategy_recovery_attempts"].as_u64() != Some(u64::from(*limit))
                        || consumed_model_requests.len() != *limit as usize
                        || !trigger_valid
                    {
                        return Err(invalid("recovery exhaustion differs from frozen budget or trigger"));
                    }
                }
                RecoveryStopCause::EmptyModelContinuation => {
                    if !matches!(response["payload"]["type"].as_str(), Some("model_completed" | "model_output_limited"))
                        || !response["payload"]["tool_calls"].as_array().is_some_and(Vec::is_empty)
                        || response["payload"]["items"].as_array().is_some_and(|items| items.iter().any(|item|
                            item["role"] == "assistant" && item["content"].as_str().is_some_and(|text| !text.trim().is_empty())))
                    {
                        return Err(invalid("empty recovery stop differs from original model response"));
                    }
                }
                RecoveryStopCause::UnfinishedRequirementControl { call_id } => {
                    if !matches!(response["payload"]["type"].as_str(), Some("model_completed" | "model_output_limited"))
                        || !response["payload"]["tool_calls"].as_array().is_some_and(|calls| calls.iter().any(|call|
                            call["call_id"].as_str() == Some(call_id.as_str()) && call["name"] == "requirements_update"))
                    {
                        return Err(invalid("unfinished recovery control is absent from original model response"));
                    }
                }
            }
        }
        CoreRuntimeDecisionFactKind::UnknownEffectRecovery { decision } => {
            use nola_domain::unknown_effect_recovery::UnknownEffectRecoveryDisposition;
            crate::require_id("recovery resolution",&decision.resolution_id)?;
            crate::require_id("recovery reason",&decision.reason)?;
            if s.transition_id.as_deref()!=Some(decision.resolution_id.as_str()) || decision.targets.is_empty() {
                return Err(invalid("recovery fact lacks exact original decision transition"));
            }
            let raw:String=c.query_row("SELECT submission_json FROM host_event_replay_order WHERE run_id=?1 AND submission_id=?2 AND submission_json IS NOT NULL",
                params![s.run,decision.resolution_id],|row|row.get(0))?;
            let reduction:Value=serde_json::from_str(&raw)?;
            let submission:Value=serde_json::from_str(reduction["submission_json"].as_str().ok_or_else(||invalid("recovery original submission missing"))?)?;
            if submission["type"]!="resolve_unknown_effects" || submission["decision"]!=serde_json::to_value(decision)? {
                return Err(invalid("recovery fact differs from original explicit user decision"));
            }
            for target in &decision.targets {
                let raw:String=c.query_row("SELECT record_json FROM effect_delivery_attempts WHERE id=?1",[&target.attempt_id],|row|row.get(0))?;
                let attempt:nola_domain::EffectDeliveryAttempt=serde_json::from_str(&raw)?;
                let original=attempt.revision==target.expected_attempt_revision
                    && (attempt.state==nola_domain::EffectDeliveryAttemptState::OutcomeUnknown
                        || decision.disposition==UnknownEffectRecoveryDisposition::KeepUnknownBlocked);
                let authorized=decision.disposition==UnknownEffectRecoveryDisposition::AuthorizeRiskRetry
                    && target.expected_attempt_revision.checked_add(1)==Some(attempt.revision)
                    && matches!(&attempt.manual_resolution,Some(nola_domain::EffectDeliveryManualResolution::RiskRetryAuthorized{evidence,..})
                        if serde_json::from_str::<nola_domain::unknown_effect_recovery::UnknownEffectRecoveryDecision>(evidence).ok().as_ref()==Some(decision));
                if (!original && !authorized) || attempt.run_id!=s.run || attempt.effect_id!=target.effect_id || attempt.request_id!=target.request_id
                    || attempt.worker_generation.as_deref()!=Some(target.worker_generation.as_str())
                    || attempt.lease_owner.as_deref()!=Some(target.lease_owner.as_str()) || attempt.lease_token.as_deref()!=Some(target.lease_token.as_str()) {
                    return Err(invalid("recovery decision has no exact original unknown physical attempt"));
                }
            }
        }
        CoreRuntimeDecisionFactKind::HostCancellation { cancellation_id,request_id,effect_id,host_id,response_sequence } => {
            if cancellation_id != &format!("host_response:{request_id}:{}",response_sequence.0) {
                return Err(invalid("host cancellation identity differs from original response"));
            }
            let response=host_response(c,s)?;
            if response["session_id"] != s.envelope.session_id
                || response["turn_id"].as_str()!=s.envelope.turn_id.as_deref()
                || response["host_id"].as_str()!=Some(host_id.as_str())
                || response["effect_id"].as_str()!=Some(effect_id.as_str())
                || response["request_id"].as_str()!=Some(request_id.as_str())
                || response["sequence"].as_u64()!=Some(response_sequence.0)
                || response["payload"]["type"]!="permission_resolved"
                || response["payload"]["resolution"]["type"]!="intervention"
                || response["payload"]["resolution"]["intervention"]["type"]!="return_to_composer" {
                return Err(invalid("host cancellation has no exact original return-to-composer response"));
            }
        }
        CoreRuntimeDecisionFactKind::Cancellation {cancellation_id,reason} => {
            crate::require_id("cancellation identity",cancellation_id)?;
            crate::require_id("cancellation reason",reason)?;
            let mut q=c.prepare("SELECT submission_json FROM host_event_replay_order
                WHERE run_id=?1 AND submission_id=?2 AND submission_json IS NOT NULL")?;
            let rows=q.query_map(params![s.run,s.transition_id],|r|r.get::<_,String>(0))?.collect::<Result<Vec<_>,_>>()?;
            let mut matched=false;
            for row in rows {
                let reduction:Value=serde_json::from_str(&row)?;
                let submission:Value=serde_json::from_str(reduction["submission_json"].as_str()
                    .ok_or_else(||invalid("submission reduction lacks typed original command"))?)?;
                if submission["type"]=="interrupt" && submission["cancellation_id"].as_str()==Some(cancellation_id) {
                    matched=true;
                }
            }
            if !matched { return Err(invalid("cancellation fact has no original committed interrupt")); }
        }
        CoreRuntimeDecisionFactKind::Error {code,detail} => {
            crate::require_id("runtime failure code",code)?;
            crate::require_id("runtime failure detail",detail)?;
        }
        CoreRuntimeDecisionFactKind::Blocker {code,detail,source_refs} => {
            crate::require_id("runtime blocker code",code)?;
            crate::require_id("runtime blocker detail",detail)?;
            for reference in source_refs {
                let evidence:bool=c.query_row("SELECT EXISTS(SELECT 1 FROM execution_evidence_sources
                    WHERE run_id=?1 AND evidence_id=?2)",params![s.run,reference],|r|r.get(0))?;
                if evidence { validate_evidence(c,s,reference,s.envelope.cursor.sequence)?; }
                else {
                    let effect:bool=c.query_row("SELECT EXISTS(SELECT 1 FROM effect_outbox WHERE run_id=?1 AND effect_id=?2)",
                        params![s.run,reference],|r|r.get(0))?;
                    if !effect { return Err(invalid("blocker reference has no original evidence or effect")); }
                }
            }
        }
    }
    Ok(())
}
fn validate_recovery_result_binding(
    source: &CoreDecisionSource, value: &RunResultValue, has_recovery_stop: bool,
) -> StoreResult<()> {
    let recovery_source = matches!(source, CoreDecisionSource::Runtime { code, .. }
        if code == "execution.no_progress");
    let recovery_result = matches!(value, RunResultValue::Incomplete { reason, .. }
        if reason.code == "execution.no_progress");
    if (has_recovery_stop || recovery_source || recovery_result)
        && !(has_recovery_stop && recovery_source && recovery_result)
    {
        return Err(invalid("recovery stop requires its runtime source and incomplete execution.no_progress result"));
    }
    Ok(())
}

fn bind_result(c:&Connection,s:&Source)->StoreResult<RunResult> {
    let semantic:CoreRunResultDecision=serde_json::from_value(s.payload["decision"].clone())?;
    let input=audit(c,s,&semantic.applied_input_refs)?;
    if matches!(&semantic.source, CoreDecisionSource::Runtime {code,..} if code == "delegation_accepted")
        != matches!(&semantic.value, RunResultValue::DelegationAccepted {..}) {
        return Err(invalid("delegation source and result kind must match"));
    }
    // A limited response is evidence only for an incomplete result, never
    // an answer, successful action, assessment, or a model control call.
    decision_source(c,s,&semantic.source,matches!(&semantic.value, RunResultValue::Incomplete { .. }))?;
    let mut has_recovery_stop = false;
    if let CoreDecisionSource::Runtime { code, source_refs } = &semantic.source {
        for reference in source_refs.iter().filter(|_| code != "delegation_accepted") {
            has_recovery_stop |= matches!(validate_fact_reference(c, s, reference, None)?.kind,
                CoreRuntimeDecisionFactKind::RecoveryStopped { .. });
        }
    }
    validate_recovery_result_binding(&semantic.source, &semantic.value, has_recovery_stop)?;
    let requirements=exact_scope(c,s,&semantic.scope_id,semantic.requirement_revision)?;
    if requirements.input!=input.commitment || semantic.evidence_frontier.0>s.envelope.cursor.sequence {
        return Err(invalid("result scope/input/frontier differs"));
    }
    let latest:u64=c.query_row("SELECT MAX(requirement_revision) FROM run_requirement_sources
        WHERE run_id=?1 AND kind='requirement_scope_changed' AND core_cursor<=?2",
        params![s.run,s.envelope.cursor.sequence],|r|r.get(0))?;
    if latest!=semantic.requirement_revision.0 { return Err(invalid("result uses superseded scope")); }
    let result=RunResult {
        result_id:semantic.result_id,run_id:s.run.clone(),conversation_id:s.conversation.clone(),
        originating_submission_id:input.originating_submission_id,input:input.commitment,
        decided_at_revision:JournalNumber(s.envelope.revision),
        decision_event_cursor:JournalNumber(s.envelope.cursor.sequence),
        requirement_revision:semantic.requirement_revision,evidence_frontier:semantic.evidence_frontier,
        value:semantic.value,
    };
    result.validate().map_err(|e|invalid(&e))?;
    if let RunResultValue::Failed {observed_effect_refs,unknown_effect_refs,..}
        | RunResultValue::Cancelled {observed_effect_refs,unknown_effect_refs,..} = &result.value {
        for id in observed_effect_refs {
            let mut q=c.prepare("SELECT i.evidence_id FROM execution_evidence_sources i
                JOIN run_events e ON e.event_id=i.source_event_id WHERE i.run_id=?1 AND i.core_cursor<=?2
                AND json_extract(e.payload_json,'$.evidence.source.effect_id')=?3")?;
            let evidence=q.query_map(params![s.run,result.evidence_frontier.0,id],|r|r.get::<_,String>(0))?
                .collect::<Result<Vec<_>,_>>()?;
            if evidence.is_empty() { return Err(invalid("observed effect has no committed execution observation")); }
            for reference in evidence { validate_evidence(c,s,&reference,result.evidence_frontier.0)?; }
        }
        for id in unknown_effect_refs {
            let dispatched: bool = c.query_row("SELECT EXISTS(SELECT 1 FROM effect_delivery_attempts
                WHERE run_id=?1 AND effect_id=?2
                AND json_extract(record_json,'$.execution_started_at_ms') IS NOT NULL)",params![s.run,id],|r|r.get(0))?;
            if !dispatched { return Err(invalid("unknown effect has no real delivery attempt")); }
        }
    }
    match &result.value {
        RunResultValue::DelegationAccepted { task_id, parent_call_id } => {
            if !matches!(&semantic.source, CoreDecisionSource::Runtime {code,source_refs}
                if code == "delegation_accepted" && source_refs == &vec![task_id.clone()]) {
                return Err(invalid("delegation acknowledgement requires its runtime promotion source"));
            }
            validate_delegation_acceptance(c, s, task_id, Some(parent_call_id))?;
        }
        RunResultValue::ActionCompleted { .. } => {
            let mut q=c.prepare("SELECT source_event_id FROM run_requirement_sources WHERE run_id=?1
                AND kind='requirement_assessed' AND requirement_revision=?2 AND core_cursor<=?3 ORDER BY core_cursor")?;
            let ids=q.query_map(params![s.run,result.requirement_revision.0,s.envelope.cursor.sequence],|r|r.get::<_,String>(0))?
                .collect::<Result<Vec<_>,_>>()?;
            let mut assessments=Vec::new();
            for id in ids { assessments.push(assessment(c,&indexed_source(c,&id)?)?); }
            result.validate_action_requirement_coverage(&requirements,&assessments).map_err(|e|invalid(&e))?;
        }
        RunResultValue::Answered { answer_part_refs } => {
            for part in answer_part_refs {
                // Internal continuations may reuse a visible assistant message.
                // Its current Run is mutable; the accepted Part/Turn ownership
                // remains the exact source for both historical and new results.
                let message:Option<String>=c.query_row("SELECT m.id FROM message_parts p
                    JOIN messages m ON m.id=p.message_id
                    JOIN assistant_turn_part_bindings b ON b.part_id=p.id AND b.message_id=m.id
                    JOIN assistant_turn_message_bindings o ON o.run_id=b.run_id AND o.turn_id=b.turn_id
                        AND o.message_id=b.message_id AND o.conversation_id=b.conversation_id
                    WHERE p.id=?1 AND b.run_id=?2 AND b.conversation_id=?3 AND b.turn_id=?4
                        AND m.conversation_id=?3 AND m.role='assistant' AND p.kind='text'",
                    params![part,s.run,s.conversation,s.envelope.turn_id],|r|r.get(0)).optional()?;
                let valid = match message {
                    Some(message) => crate::canonical_message_ownership::matches_local_read_owner(
                        c, &s.conversation, &message, &s.run)?,
                    None => false,
                };
                if !valid { return Err(invalid("answer Part does not belong to this Run")); }
            }
        }
        RunResultValue::Incomplete {remaining_requirement_refs,obligation_refs,..} => {
            for id in remaining_requirement_refs {
                if !requirements.requirements.iter().any(|r| &r.requirement_id==id) {
                    return Err(invalid("remaining requirement does not exist in current scope"));
                }
            }
            for id in obligation_refs { validate_fact_reference(c,s,id,None)?; }
        }
        RunResultValue::Blocked {blocking_fact_refs,..} => {
            for id in blocking_fact_refs { validate_fact_reference(c,s,id,Some("blocker"))?; }
        }
        RunResultValue::Failed {error_ref,..} => { validate_fact_reference(c,s,error_ref,Some("error"))?; }
        RunResultValue::Cancelled {cancellation_id,..} => {
            let mut q=c.prepare("SELECT source_event_id FROM run_requirement_sources WHERE run_id=?1
                AND kind='runtime_decision_fact_observed' AND core_cursor<?2")?;
            let ids=q.query_map(params![s.run,s.envelope.cursor.sequence],|r|r.get::<_,String>(0))?
                .collect::<Result<Vec<_>,_>>()?;
            let mut found=false;
            for id in ids {
                let original=indexed_source(c,&id)?;
                let fact:CoreRuntimeDecisionFact=serde_json::from_value(original.payload["fact"].clone())?;
                if matches!(&fact.kind,CoreRuntimeDecisionFactKind::Cancellation {cancellation_id:id,..}
                    | CoreRuntimeDecisionFactKind::HostCancellation {cancellation_id:id,..} if id==cancellation_id) {
                    validate_runtime_fact(c,&original,&fact)?;
                    found=true;
                }
            }
            if !found { return Err(invalid("cancelled result has no original cancellation fact")); }
        }
    }
    Ok(result)
}
pub(crate) fn validate_result_event(c:&Connection,id:&str)->StoreResult<RunResult> {
    bind_result(c,&indexed_source(c,id)?)
}
/// Freeze recomputes from immutable original events, not caller-provided result JSON.
pub(crate) fn seal_result(c:&Connection,conversation:&str,run:&str)->StoreResult<RunResult> {
    let id:String=c.query_row("SELECT source_event_id FROM run_requirement_sources
        WHERE run_id=?1 AND kind='run_result_decided'",[run],|r|r.get(0))
        .optional()?.ok_or_else(|| invalid("terminal seal requires a committed Run result decision"))?;
    let s=indexed_source(c,&id)?;
    if s.conversation!=conversation { return Err(invalid("result conversation differs")); }
    // Every raw control fact must be represented, including newer counterexamples.
    let missing:bool=c.query_row("SELECT EXISTS(SELECT 1 FROM run_events e LEFT JOIN run_requirement_sources i
        ON i.source_event_id=e.event_id WHERE e.run_id=?1
        AND e.event_kind IN ('requirement_scope_changed','requirement_assessed','run_result_decided','runtime_decision_fact_observed')
        AND i.source_event_id IS NULL)",[run],|r|r.get(0))?;
    if missing { return Err(invalid("original control event is absent from source index")); }
    let mut q=c.prepare("SELECT source_event_id FROM run_requirement_sources WHERE run_id=?1 ORDER BY core_cursor")?;
    let ids=q.query_map([run],|r|r.get::<_,String>(0))?.collect::<Result<Vec<_>,_>>()?;
    for id in ids {
        let original=indexed_source(c,&id)?;
        if original.envelope.cursor.sequence>s.envelope.cursor.sequence {
            return Err(invalid("control frontier advanced beyond frozen result decision"));
        }
        match original.kind.as_str() {
            "requirement_scope_changed" => { scope(c,&original)?; }
            "requirement_assessed" => { assessment(c,&original)?; }
            "runtime_decision_fact_observed" => {
                let fact=serde_json::from_value(original.payload["fact"].clone())?;
                validate_runtime_fact(c,&original,&fact)?;
            }
            _ => {}
        }
    }
    let late_input:bool=c.query_row("SELECT EXISTS(SELECT 1 FROM run_events WHERE run_id=?1
        AND event_kind='input_applied' AND json_extract(payload_json,'$.core_envelope.cursor.sequence')>?2)",
        params![run,s.envelope.cursor.sequence],|r|r.get(0))?;
    if late_input { return Err(invalid("new input was applied after the result decision")); }
    let result=bind_result(c,&s)?;
    validate_seal_effects(c,&s,&result)?;
    crate::user_input_wait::ensure_not_sealed_with_open_waits(c,run)?;
    let json=String::from_utf8(result.canonical_bytes().map_err(|e|invalid(&e))?).map_err(|e|invalid(&e.to_string()))?;
    c.execute("INSERT INTO run_results(run_id,source_event_id,result_json,result_digest) VALUES(?1,?2,?3,?4)",
        params![run,id,json,result.computed_digest().map_err(|e|invalid(&e))?.to_hex()])?;
    Ok(result)
}

/// Unknown effects remain unknown, never synthetically acknowledged. Only a
/// durable non-live outcome-unknown attempt explicitly retained by the result
/// may outlive the Run seal. Every other pending effect blocks publication.
fn validate_seal_effects(c:&Connection,s:&Source,result:&RunResult)->StoreResult<()> {
    let mut retained=BTreeSet::new();
    match &result.value {
        RunResultValue::Cancelled {unknown_effect_refs,..}
        | RunResultValue::Failed {unknown_effect_refs,..} => retained.extend(unknown_effect_refs.iter().cloned()),
        RunResultValue::Blocked {blocking_fact_refs,..} => {
            for id in blocking_fact_refs {
                let fact=validate_fact_reference(c,s,id,Some("blocker"))?;
                if let CoreRuntimeDecisionFactKind::Blocker {source_refs,..}=fact.kind {
                    retained.extend(source_refs);
                }
            }
        }
        _ => {}
    }
    let mut q=c.prepare("SELECT effect_id FROM effect_outbox WHERE run_id=?1 AND status<>'acknowledged'")?;
    let pending=q.query_map([&s.run],|r|r.get::<_,String>(0))?.collect::<Result<Vec<_>,_>>()?;
    for effect in pending {
        if !retained.contains(&effect) { return Err(invalid("Run still owns an unaccounted unsettled effect")); }
        let row:Option<(String,String,String,String,u32)>=c.query_row(
            "SELECT a.record_json,a.state,a.id,e.request_id,a.attempt_number FROM effect_delivery_attempts a
             JOIN effect_outbox e ON e.run_id=a.run_id AND e.effect_id=a.effect_id
             WHERE a.run_id=?1 AND a.effect_id=?2 ORDER BY a.attempt_number DESC LIMIT 1",
            params![s.run,effect],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?))).optional()?;
        let (raw,state,id,request,number)=row.ok_or_else(||invalid("unknown effect has no delivery attempt"))?;
        let attempt:nola_domain::EffectDeliveryAttempt=serde_json::from_str(&raw)?;
        nola_kernel::validate_effect_delivery_attempt(&attempt).map_err(StoreError::Invariant)?;
        if attempt.run_id!=s.run || attempt.effect_id!=effect
            || state!="outcome_unknown" || attempt.id!=id || attempt.request_id!=request || attempt.attempt_number!=number
            || attempt.state!=nola_domain::EffectDeliveryAttemptState::OutcomeUnknown
            || attempt.execution_started_at_ms.is_none() || attempt.finished_at_ms.is_none()
            || attempt.settlement.is_some() {
            return Err(invalid("unknown effect is still live or has no exact uncertain outcome"));
        }
    }
    Ok(())
}

#[cfg(test)]
pub(crate) mod tests {
    #[test]
    fn recovery_stop_result_binding_rejects_mismatched_source_fact_or_outcome() {
        use super::*;
        use nola_domain::run_result::RunResultReason;
        for source_code in ["execution.no_progress", "terminal_boundary"] {
            for result_code in ["execution.no_progress", "requirements.unassessed"] {
                for has_stop in [false, true] {
                    let source = CoreDecisionSource::Runtime {
                        code: source_code.into(), source_refs: vec!["fact".into()],
                    };
                    let value = RunResultValue::Incomplete {
                        reason: RunResultReason { code: result_code.into(), detail: None },
                        remaining_requirement_refs: vec![], obligation_refs: vec![],
                    };
                    let expected = (source_code == "execution.no_progress"
                        && result_code == "execution.no_progress" && has_stop)
                        || (source_code != "execution.no_progress"
                            && result_code != "execution.no_progress" && !has_stop);
                    assert_eq!(validate_recovery_result_binding(&source, &value, has_stop).is_ok(), expected);
                }
            }
        }
        let source = CoreDecisionSource::Runtime {
            code: "execution.no_progress".into(), source_refs: vec!["fact".into()],
        };
        assert!(validate_recovery_result_binding(&source,
            &RunResultValue::Answered { answer_part_refs: vec![] }, true).is_err());
        let model = CoreDecisionSource::Model {
            effect_id: "effect".into(), request_id: "request".into(),
            attempt_id: None, call_id: None,
        };
        let incomplete = RunResultValue::Incomplete {
            reason: RunResultReason { code: "execution.no_progress".into(), detail: None },
            remaining_requirement_refs: vec![], obligation_refs: vec![],
        };
        assert!(validate_recovery_result_binding(&model, &incomplete, false).is_err());
    }
    use super::*;
    use nola_store_port::run_creation::InitialCoreRunEvent;
    use serde_json::json;

    pub(crate) fn events() -> Vec<InitialCoreRunEvent> {
        let applied=json!([{"admission_id":"a","turn_id":"run","applied_sequence":"1"}]);
        let source=json!({"kind":"runtime","code":"input_applied","source_refs":["a"]});
        [
            json!({"type":"input_accepted","admission":{"admission_id":"a","submission_id":"submission",
                "accepted_sequence":1,"origin":"interactive_user","delivery":"current_turn","anchor_turn_id":null,
                "payload_digest":"a".repeat(64),"item_ids":["user"]}}),
            json!({"type":"input_applied","turn_id":"run","applications":[
                {"admission_id":"a","destination_turn_id":"run","applied_sequence":1}]}),
            json!({"type":"requirement_scope_changed","turn_id":"run","scope":{
                "scope_id":"scope","revision":"1","applied_input_refs":applied,"source_input_refs":["user"],
                "requirements":[{"requirement_id":"requirement","description":"Original user request",
                    "source_refs":["user"],"dependency_ids":[],"goal_requirement_ref":null,"resource_scope":{"kind":"undeclared"}}],"source":source}}),
            json!({"type":"runtime_decision_fact_observed","turn_id":"run","fact":{
                "fact_id":"error","kind":{"kind":"error","code":"provider_failed","detail":"Provider did not return a response"}}}),
            json!({"type":"run_result_decided","turn_id":"run","decision":{
                "result_id":"result","scope_id":"scope","requirement_revision":"1","applied_input_refs":applied,
                "evidence_frontier":"4","value":{"kind":"failed","error_ref":"error","observed_effect_refs":[],"unknown_effect_refs":[]},
                "source":{"kind":"runtime","code":"provider_failed","source_refs":["error"]}}}),
        ].into_iter().enumerate().map(|(index,payload)| InitialCoreRunEvent {
            core_session_id:"core".into(),core_turn_id:Some("run".into()),core_revision:10,
            core_cursor:index as u64+1,event_kind:payload["type"].as_str().unwrap().into(),
            payload_json:payload.to_string(),
        }).collect()
    }

    #[test]
    fn delegation_result_cannot_be_invented_without_same_transition_promotion() {
        for decision_source in [
            json!({"kind":"runtime","code":"delegation_accepted","source_refs":["task"]}),
            json!({"kind":"runtime","code":"terminal_boundary","source_refs":["error"]}),
        ] {
            let mut inputs = protocol_recovery_events();
            let result = inputs.iter_mut().find(|event| event.event_kind == "run_result_decided").unwrap();
            let mut payload: Value = serde_json::from_str(&result.payload_json).unwrap();
            payload["decision"]["value"] = json!({"kind":"delegation_accepted","task_id":"task","parent_call_id":"spawn"});
            payload["decision"]["source"] = decision_source;
            result.payload_json = payload.to_string();
            let mut store = crate::NolaStore::open(":memory:").unwrap();
            assert!(store.begin_turn(&crate::input_facts::tests::input(inputs)).is_err());
            let results: i64 = store.connection.query_row("SELECT count(*) FROM run_results", [], |row| row.get(0)).unwrap();
            assert_eq!(results, 0, "rejected delegation results must roll back atomically");
        }
    }

    #[test]
    fn initial_scope_rejects_resource_claims_with_unapplied_sources() {
        let mut inputs = events();
        let event = inputs.iter_mut().find(|event| event.event_kind == "requirement_scope_changed").unwrap();
        let mut value: Value = serde_json::from_str(&event.payload_json).unwrap();
        value["scope"]["requirements"][0]["resource_scope"] = json!({"kind":"declared",
            "required":[{"resource_uri":"file:///requested","operation":"read","version":null,"predicate":null,"source_refs":["assistant-suggestion"]}],
            "exclusions":[],"interpreted_user_frontier":["user"]});
        event.payload_json=value.to_string();
        let mut store=crate::NolaStore::open(":memory:").unwrap();
        assert!(store.begin_turn(&crate::input_facts::tests::input(inputs)).is_err());
    }

    #[test]
    fn protocol_recovery_incomplete_seals_from_committed_runtime_error() {
        let mut inputs = protocol_recovery_events();
        let mut store = crate::NolaStore::open(":memory:").unwrap();
        store.begin_turn(&crate::input_facts::tests::input(inputs.clone())).unwrap();
        assert_eq!(activity_error_text(&store.connection, "conversation", "run", "error").unwrap(),
            Some("A skipped tool call still requires protocol correction.".into()));
        assert!(activity_error_text(&store.connection, "foreign", "run", "error").unwrap().is_none());
        let source = indexed_source(&store.connection, "core:run:5").unwrap();
        let result = seal_result(&store.connection, &source.conversation, &source.run).unwrap();
        assert!(matches!(result.value, RunResultValue::Incomplete { ref reason, .. }
            if reason.code == "protocol.recovery_pending"));
        let stored: String = store.connection.query_row(
            "SELECT result_json FROM run_results WHERE run_id='run'", [], |row| row.get(0),
        ).unwrap();
        assert_eq!(serde_json::from_str::<RunResult>(&stored).unwrap(), result);

        // No relaxation to accept a naked runtime conclusion or a model source
        // from another transition: both still roll back the admission.
        for source in [
            json!({"kind":"runtime","code":"terminal_boundary","source_refs":[]}),
            json!({"kind":"runtime","code":"terminal_boundary","source_refs":["missing"]}),
            json!({"kind":"model","effect_id":"old-effect","request_id":"old-request",
                "attempt_id":null,"call_id":null}),
        ] {
            let mut payload: Value = serde_json::from_str(&inputs[4].payload_json).unwrap();
            payload["decision"]["source"] = source;
            inputs[4].payload_json = payload.to_string();
            let mut rejected = crate::NolaStore::open(":memory:").unwrap();
            assert!(rejected.begin_turn(&crate::input_facts::tests::input(inputs.clone())).is_err());
            let count: i64 = rejected.connection.query_row(
                "SELECT COUNT(*) FROM runs", [], |row| row.get(0),
            ).unwrap();
            assert_eq!(count, 0);
        }
    }

    fn protocol_recovery_events() -> Vec<InitialCoreRunEvent> {
        let mut inputs = events();
        let mut fact: Value = serde_json::from_str(&inputs[3].payload_json).unwrap();
        fact["fact"]["kind"]["code"] = json!("protocol.recovery_pending");
        fact["fact"]["kind"]["detail"] = json!("A skipped tool call still requires protocol correction.");
        inputs[3].payload_json = fact.to_string();
        let mut result: Value = serde_json::from_str(&inputs[4].payload_json).unwrap();
        result["decision"]["value"] = json!({"kind":"incomplete",
            "reason":{"code":"protocol.recovery_pending","detail":null},
            "remaining_requirement_refs":["requirement"],"obligation_refs":[]});
        result["decision"]["source"]["code"] = json!("terminal_boundary");
        inputs[4].payload_json = result.to_string();
        inputs
    }

    #[test]
    fn recovery_stop_cannot_be_invented_at_admission_without_a_host_transition() {
        for cause in [
            json!({"kind":"budget_exhausted","limit":0}),
            json!({"kind":"empty_model_continuation"}),
            json!({"kind":"unfinished_requirement_control","call_id":"invented"}),
        ] {
            let mut inputs = protocol_recovery_events();
            let mut fact: Value = serde_json::from_str(&inputs[3].payload_json).unwrap();
            fact["fact"]["kind"] = json!({"kind":"recovery_stopped","cause":cause,
                "detail":"claimed policy stop","consumed_model_requests":[]});
            inputs[3].payload_json = fact.to_string();
            let mut store = crate::NolaStore::open(":memory:").unwrap();
            assert!(store.begin_turn(&crate::input_facts::tests::input(inputs)).is_err());
            let runs: i64 = store.connection.query_row("SELECT COUNT(*) FROM runs", [], |row| row.get(0)).unwrap();
            assert_eq!(runs, 0, "invalid recovery fact must roll back the complete admission");
        }
    }

    #[test]
    fn immutable_requirement_sources_follow_their_owning_run_lifecycle() {
        let mut store = crate::NolaStore::open(":memory:").unwrap();
        store.begin_turn(&crate::input_facts::tests::input(events())).unwrap();
        let error = store.connection.execute(
            "DELETE FROM run_requirement_sources WHERE run_id='run'", [],
        ).unwrap_err();
        assert!(error.to_string().contains("requirement source is immutable"));
        let count: i64 = store.connection.query_row(
            "SELECT COUNT(*) FROM run_requirement_sources WHERE run_id='run'",
            [], |row| row.get(0),
        ).unwrap();
        assert!(count > 0);

        // Archive replacement removes an entire Run, not one immutable fact.
        // The conversation survives; child facts must cascade with their owner.
        store.connection.execute("DELETE FROM runs WHERE id='run'", []).unwrap();
        let remaining: i64 = store.connection.query_row(
            "SELECT COUNT(*) FROM run_requirement_sources WHERE run_id='run'",
            [], |row| row.get(0),
        ).unwrap();
        assert_eq!(remaining, 0);
        let conversations: i64 = store.connection.query_row(
            "SELECT COUNT(*) FROM conversations", [], |row| row.get(0),
        ).unwrap();
        assert_eq!(conversations, 1);
    }

    #[test]
    fn source_binding_survives_reopen_without_canonical_owner_claims() {
        let directory=tempfile::tempdir().unwrap();
        let path=directory.path().join("requirements.sqlite");
        let mut store=crate::NolaStore::open(&path).unwrap();
        store.begin_turn(&crate::input_facts::tests::input(events())).unwrap();
        let expected=bind_result(&store.connection,&indexed_source(&store.connection,"core:run:5").unwrap()).unwrap();
        assert_eq!(expected.originating_submission_id,"submission");
        assert_eq!(expected.input.watermark.0,1);
        assert_eq!(expected.decided_at_revision.0,10);
        assert_eq!(expected.decision_event_cursor.0,5);
        drop(store);
        let store=crate::NolaStore::open(&path).unwrap();
        assert_eq!(bind_result(&store.connection,&indexed_source(&store.connection,"core:run:5").unwrap()).unwrap(),expected);
    }

    #[test]
    fn invalid_semantics_or_missing_input_reference_roll_back_admission() {
        for damage in ["input","scope","future","missing_fact","foreign_turn","unknown_field","model_without_host","unknown_effect"] {
            let mut inputs=events();
            let index=if damage=="input" {2} else {4};
            let mut payload:Value=serde_json::from_str(&inputs[index].payload_json).unwrap();
            match damage {
                "input" => payload["scope"]["applied_input_refs"]=json!([]),
                "scope" => payload["decision"]["requirement_revision"]=json!("2"),
                "future" => payload["decision"]["evidence_frontier"]=json!("99"),
                "missing_fact" => payload["decision"]["value"]["error_ref"]=json!("absent"),
                "foreign_turn" => payload["turn_id"]=json!("foreign"),
                "model_without_host" => payload["decision"]["source"]=json!({"kind":"model",
                    "effect_id":"old-effect","request_id":"old-request","attempt_id":null,"call_id":null}),
                "unknown_effect" => payload["decision"]["value"]["unknown_effect_refs"]=json!(["never-dispatched"]),
                _ => payload["decision"]["run_id"]=json!("claimed-owner"),
            }
            inputs[index].payload_json=payload.to_string();
            let mut store=crate::NolaStore::open(":memory:").unwrap();
            assert!(store.begin_turn(&crate::input_facts::tests::input(inputs)).is_err(),"{damage}");
            for table in ["runs","run_requirement_sources","run_input_applications"] {
                let count:i64=store.connection.query_row(&format!("SELECT COUNT(*) FROM {table}"),[],|r|r.get(0)).unwrap();
                assert_eq!(count,0,"{damage}: {table}");
            }
        }
    }
}

#[cfg(test)]
mod limited_model_source_tests {
    #[test]
    fn truncated_output_can_only_witness_incomplete_without_control_authority() {
        use super::accepts_model_terminal_source;
        assert!(accepts_model_terminal_source("model_output_limited", false, true));
        assert!(!accepts_model_terminal_source("model_output_limited", false, false));
        assert!(!accepts_model_terminal_source("model_output_limited", true, true));
        assert!(!accepts_model_terminal_source("model_delta", false, true));
        assert!(!accepts_model_terminal_source("failed", false, true));
        assert!(accepts_model_terminal_source("model_completed", false, false));
    }
}
