// SPDX-License-Identifier: Apache-2.0
//! Model-interpreted execution conditions, bound to admitted task inputs.
use super::*;
use nola_domain::core_requirements::{CoreAppliedInputRef, CoreDecisionSource, CoreRequirementScope};
use nola_domain::execution_constraints::*;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use crate::agent::protocol::AgentInputOrigin;

pub(super) const TOOL: &str = "execution_constraints_update";

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExecutionConstraintLedger {
    pub entries: Vec<ExecutionConstraintEntry>,
    #[serde(deserialize_with = "required_scope_binding")]
    pub active_scope_id: Option<String>,
    pub source_scopes: Vec<ExecutionConstraintSourceScope>,
}

fn required_scope_binding<'de, D: serde::Deserializer<'de>>(deserializer: D) -> Result<Option<String>, D::Error> {
    Option::<String>::deserialize(deserializer)
}

/// Frozen provenance only; this does not make historical inputs requirements of
/// a successor Run, nor carry any historical completion/evidence assessment.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExecutionConstraintSourceScope {
    scope_id: String,
    source_input_refs: Vec<String>,
    applied_input_refs: Vec<CoreAppliedInputRef>,
}

pub(super) struct ExecutionTargetAdmission {
    scope_id: Option<String>,
    bound_scope_id: Option<String>,
    current: Option<ExecutionConstraintEntry>,
    user_frontier: Vec<String>,
}

impl ExecutionTargetAdmission {
    pub(super) fn validate(&self, declaration: Option<&ExecutionTargetProjection>, input: &Value) -> Result<(), String> {
        let Some(declaration) = declaration else { return Ok(()); };
        let selected = declaration.resolve(input)?;
        if declaration.role == ExecutionTargetRole::Diagnostic { return Ok(()); }
        let entry = self.current.as_ref()
            .ok_or("execution_target_interpretation_required: call execution_constraints_update with the user's target or explicit unconstrained selection; this call did not execute")?;
        if self.scope_id.is_none() || self.scope_id != self.bound_scope_id {
            return Err("execution_target_scope_not_bound: no current admitted task scope; this call did not execute".into());
        }
        if !interpretation_current(entry, self.scope_id.as_deref().unwrap(), &self.user_frontier) {
            return Err("execution_target_new_user_input: interpret the newly applied user input once; the previous target remains recorded and this call did not execute".into());
        }
        if entry.domain != declaration.domain { return Err("execution target domain differs from the recorded constraint".into()); }
        if matches!(entry.selection, ExecutionTargetSelection::Required { target } if target != selected) {
            return Err(format!("execution_target_mismatch: action target differs from constraint {}; retain the requested target or obtain explicit new user direction; this call did not execute", entry.constraint_id));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExecutionConstraintEntry {
    pub constraint_id: String,
    pub scope_id: String,
    pub domain: ExecutionConstraintDomain,
    pub selection: ExecutionTargetSelection,
    pub source_input_refs: Vec<String>,
    pub interpreted_user_frontier: Vec<String>,
    pub interpreted_user_sequence: u64,
    pub rationale: String,
    pub replaces_constraint_id: Option<String>,
    pub source: CoreDecisionSource,
}

fn frontier_for_refs(refs: &[CoreAppliedInputRef], inputs: &crate::agent::input_queue::AgentInputQueue) -> (Vec<String>, u64) {
    let mut admissions = refs.iter().filter_map(|reference| inputs.applied_admission(reference))
        .filter(|admission| matches!(admission.origin, AgentInputOrigin::InteractiveUser | AgentInputOrigin::GoalInteractiveUser))
        .collect::<Vec<_>>();
    admissions.sort_by_key(|admission| admission.accepted_sequence);
    (admissions.iter().map(|admission| admission.admission_id.clone()).collect(),
        admissions.last().map_or(0, |admission| admission.accepted_sequence))
}

fn interpretation_current(entry: &ExecutionConstraintEntry, _scope_id: &str, frontier: &[String]) -> bool {
    entry.interpreted_user_frontier == frontier
}

fn is_task_demand(origin: AgentInputOrigin) -> bool {
    matches!(origin, AgentInputOrigin::InteractiveUser | AgentInputOrigin::GoalInteractiveUser
        | AgentInputOrigin::ScheduledAction | AgentInputOrigin::GoalContinuation | AgentInputOrigin::DelegatedAgent)
}

impl ExecutionConstraintLedger {
    pub(super) fn current(&self, domain: ExecutionConstraintDomain) -> Option<&ExecutionConstraintEntry> {
        self.entries.iter().rev().find(|entry| entry.domain == domain)
    }

    /// Preserve original identities before clearing requirement state. A
    /// suspended ledger is durable provenance, not authorization for an action.
    pub(super) fn suspend(&self, scope: Option<&CoreRequirementScope>, inputs: &crate::agent::input_queue::AgentInputQueue) -> Result<Self, String> {
        self.validate(scope, inputs)?;
        let mut ledger = self.clone();
        if let Some(scope) = scope.filter(|_| !self.entries.is_empty()) {
            let snapshot = ExecutionConstraintSourceScope { scope_id: scope.scope_id.clone(),
                source_input_refs: scope.source_input_refs.clone(), applied_input_refs: scope.applied_input_refs.clone() };
            if let Some(existing) = ledger.source_scopes.iter_mut().find(|entry| entry.scope_id == scope.scope_id) {
                *existing = snapshot;
            } else { ledger.source_scopes.push(snapshot); }
        }
        ledger.active_scope_id = None;
        ledger.validate(None, inputs)?;
        Ok(ledger)
    }

    /// Bind, never relabel, an inherited declaration after the successor scope
    /// has admitted its own inputs. Fail closed rather than discard provenance.
    pub(super) fn bind_scope(&mut self, scope: &CoreRequirementScope, inputs: &crate::agent::input_queue::AgentInputQueue) -> Result<(), String> {
        let old = self.active_scope_id.replace(scope.scope_id.clone());
        if let Err(error) = self.validate(Some(scope), inputs) {
            self.active_scope_id = old;
            return Err(error);
        }
        Ok(())
    }

    fn lineage_refs(&self, scope: Option<&CoreRequirementScope>) -> Vec<CoreAppliedInputRef> {
        let mut references = BTreeMap::new();
        for reference in self.source_scopes.iter().flat_map(|snapshot|snapshot.applied_input_refs.iter())
            .chain(scope.into_iter().flat_map(|scope|scope.applied_input_refs.iter())) {
            references.insert(reference.admission_id.clone(), reference.clone());
        }
        references.into_values().collect()
    }

    fn user_frontier(&self, scope: &CoreRequirementScope, inputs: &crate::agent::input_queue::AgentInputQueue) -> (Vec<String>, u64) {
        frontier_for_refs(&self.lineage_refs(Some(scope)), inputs)
    }

    pub(super) fn validate(&self, scope: Option<&CoreRequirementScope>, inputs: &crate::agent::input_queue::AgentInputQueue) -> Result<(), String> {
        if self.active_scope_id.as_deref() != scope.map(|scope| scope.scope_id.as_str())
            && !(self.entries.is_empty() && self.source_scopes.is_empty() && self.active_scope_id.is_none()) {
            return Err("execution constraints have an invalid active scope binding".into());
        }
        if self.source_scopes.len() > 128 || self.source_scopes.iter()
            .map(|snapshot| snapshot.source_input_refs.len().saturating_add(snapshot.applied_input_refs.len()))
            .sum::<usize>() > 4096
            || serde_json::to_vec(&self.source_scopes).map_err(|error|error.to_string())?.len() > 512 * 1024 {
            return Err("execution constraint source scope budget exhausted; target was not forgotten".into());
        }
        let mut scope_ids = BTreeSet::new();
        for snapshot in &self.source_scopes {
            if snapshot.scope_id.is_empty() || !scope_ids.insert(&snapshot.scope_id)
                || snapshot.applied_input_refs.iter().any(|reference|!inputs.matches_applied_reference(reference))
                || snapshot.source_input_refs.iter().any(|id| !snapshot.applied_input_refs.iter()
                    .filter_map(|reference|inputs.applied_admission(reference)).any(|admission|admission.item_ids.contains(id))) {
                return Err("execution constraint source scope has invalid admitted provenance".into());
            }
        }
        let mut ids = BTreeSet::new();
        let mut latest = BTreeMap::<ExecutionConstraintDomain, &ExecutionConstraintEntry>::new();
        let lineage_refs = self.lineage_refs(scope);
        let (_, max_sequence) = frontier_for_refs(&lineage_refs, inputs);
        if self.entries.len() > 128 { return Err("execution constraint ledger exceeds its task budget".into()); }
        for entry in &self.entries {
            let (source_refs, applied_refs) = if let Some(scope) = scope.filter(|scope|scope.scope_id == entry.scope_id) {
                (&scope.source_input_refs, &scope.applied_input_refs)
            } else {
                let snapshot = self.source_scopes.iter().find(|snapshot|snapshot.scope_id == entry.scope_id)
                    .ok_or("execution constraint lost its original scope provenance")?;
                (&snapshot.source_input_refs, &snapshot.applied_input_refs)
            };
            if entry.constraint_id.is_empty() || !ids.insert(&entry.constraint_id)
                || entry.rationale.trim().is_empty() || entry.rationale.chars().count() > 2000 || entry.source_input_refs.is_empty()
                || entry.source_input_refs.len() > 32
                || entry.source_input_refs.iter().collect::<BTreeSet<_>>().len() != entry.source_input_refs.len()
                || entry.source_input_refs.iter().any(|id| id.len() > 256 || !source_refs.contains(id)
                    || !applied_refs.iter().filter_map(|reference|inputs.applied_admission(reference))
                        .any(|admission|is_task_demand(admission.origin) && admission.item_ids.contains(id)))
                || entry.interpreted_user_sequence > max_sequence
                || entry.interpreted_user_frontier != lineage_refs.iter()
                    .filter_map(|reference|inputs.applied_admission(reference))
                    .filter(|admission|matches!(admission.origin,AgentInputOrigin::InteractiveUser|AgentInputOrigin::GoalInteractiveUser)
                        && admission.accepted_sequence <= entry.interpreted_user_sequence)
                    .map(|admission|(admission.accepted_sequence,admission.admission_id.clone()))
                    .collect::<BTreeMap<_,_>>().into_values().collect::<Vec<_>>()
                || (entry.interpreted_user_sequence != 0 && !lineage_refs.iter()
                    .filter_map(|reference|inputs.applied_admission(reference))
                    .any(|admission|admission.accepted_sequence == entry.interpreted_user_sequence
                        && matches!(admission.origin,AgentInputOrigin::InteractiveUser|AgentInputOrigin::GoalInteractiveUser)))
                || !matches!(&entry.source, CoreDecisionSource::Model { call_id: Some(call), .. } if !call.is_empty()) {
                return Err("execution constraint checkpoint has invalid scope or admitted source".into());
            }
            match latest.insert(entry.domain, entry) {
                None if entry.replaces_constraint_id.is_none() => {},
                Some(previous) if entry.replaces_constraint_id.as_ref() == Some(&previous.constraint_id)
                    && entry.interpreted_user_sequence > previous.interpreted_user_sequence
                    && applied_refs.iter().filter_map(|reference|inputs.applied_admission(reference))
                        .any(|admission|admission.accepted_sequence > previous.interpreted_user_sequence
                            && admission.accepted_sequence <= entry.interpreted_user_sequence
                            && matches!(admission.origin,AgentInputOrigin::InteractiveUser|AgentInputOrigin::GoalInteractiveUser)
                            && admission.item_ids.iter().any(|id|entry.source_input_refs.contains(id))) => {},
                _ => return Err("execution constraint replacement has no newer user input or exact prior owner".into()),
            }
        }
        Ok(())
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ConstraintUpdate {
    domain: ExecutionConstraintDomain,
    selection: ExecutionTargetSelection,
    source_input_refs: Vec<String>,
    rationale: String,
    replaces_constraint_id: Option<String>,
}

pub(super) fn tool_schema() -> Value {
    json!({"type":"function","function":{"name":TOOL,
        "description":"Record your interpretation of the user's explicit execution target before target-sensitive actions. Record once for the current applied user input frontier, including unconstrained when no target was requested. Preserve a recorded target after errors or recovery. A changed target requires a later explicit user instruction and replaces_constraint_id. May be the first call in the same batch as actions; no tool permission is granted.",
        "parameters":{"type":"object","additionalProperties":false,
            "required":["domain","selection","source_input_refs","rationale"],"properties":{
                "domain":{"enum":["browser_target"]},
                "selection":{"oneOf":[
                    {"type":"object","additionalProperties":false,"required":["kind","target"],"properties":{"kind":{"const":"required"},"target":{"enum":["attached","managed"]}}},
                    {"type":"object","additionalProperties":false,"required":["kind"],"properties":{"kind":{"const":"unconstrained"}}}
                ]},
                "source_input_refs":{"type":"array","minItems":1,"maxItems":32,"uniqueItems":true,"items":{"type":"string","maxLength":256},"description":"Exact source item IDs from execution_constraint_state; cite the actual user instruction, not a tool error or your own recovery plan."},
                "rationale":{"type":"string","minLength":1,"maxLength":2000},
                "replaces_constraint_id":{"type":"string","description":"Exact current ID when reaffirming after new user input or replacing by an explicit later user instruction."}
            }}}})
}

impl AgentRuntime {
    pub(super) fn apply_model_execution_constraint(&mut self, turn_id: &TurnId, call_id: &CallId, arguments: &Value) -> Result<Value, String> {
        let update: ConstraintUpdate = serde_json::from_value(arguments.clone()).map_err(|error| format!("invalid_execution_constraint: {error}"))?;
        let binding = self.require_turn(turn_id, AgentTurnStatus::AwaitingModel)?.last_model_requirement_binding.as_ref()
            .ok_or("execution constraint has no frozen model input binding")?;
        let state = &self.session.execution.requirements;
        let scope = state.scope.as_ref().ok_or("execution constraint has no current task scope")?;
        if state.execution_constraints.active_scope_id.as_ref() != Some(&scope.scope_id) {
            return Err("execution constraint ledger is not bound to the current admitted scope".into());
        }
        if binding.scope_id != scope.scope_id || binding.expected_input_frontier_token != state.frontier_token(self.session.session_id.as_str())? {
            return Err("execution constraint input frontier changed; interpret current admitted input".into());
        }
        if update.rationale.trim().is_empty() || update.rationale.chars().count() > 2000
            || update.source_input_refs.is_empty() || update.source_input_refs.len() > 32
            || update.source_input_refs.iter().collect::<BTreeSet<_>>().len() != update.source_input_refs.len()
            || update.source_input_refs.iter().any(|id| id.len() > 256 || !scope.source_input_refs.contains(id)
                || !scope.applied_input_refs.iter().filter_map(|reference|self.session.execution.input_queue.applied_admission(reference))
                    .any(|admission|is_task_demand(admission.origin) && admission.item_ids.contains(id))) {
            return Err("execution constraint requires bounded rationale and exact applied input sources".into());
        }
        let (frontier, max_sequence) = state.execution_constraints.user_frontier(scope, &self.session.execution.input_queue);
        let previous = state.execution_constraints.current(update.domain);
        if let Some(previous) = previous {
            if interpretation_current(previous, &scope.scope_id, &frontier) && previous.selection == update.selection {
                return Ok(json!({"constraint":previous,"unchanged":true,"permission_granted":false}));
            }
            if update.replaces_constraint_id.as_ref() != Some(&previous.constraint_id) || max_sequence <= previous.interpreted_user_sequence {
                return Err("execution_target_change_requires_new_user_input: retain the recorded target; a tool failure or model recovery is not a user instruction".into());
            }
            let cites_new_user = scope.applied_input_refs.iter().filter_map(|reference| self.session.execution.input_queue.applied_admission(reference))
                .any(|admission| admission.accepted_sequence > previous.interpreted_user_sequence
                    && matches!(admission.origin, AgentInputOrigin::InteractiveUser | AgentInputOrigin::GoalInteractiveUser)
                    && admission.item_ids.iter().any(|id| update.source_input_refs.contains(id)));
            if !cites_new_user { return Err("execution constraint replacement must cite the later applied user instruction".into()); }
        } else if update.replaces_constraint_id.is_some() {
            return Err("execution constraint has no current entry to replace".into());
        }
        if state.execution_constraints.entries.len() >= 128 { return Err("execution constraint task ledger is full; no action was authorized".into()); }
        let mut source = state.model_source.clone().ok_or("execution constraint is missing its model decision source")?;
        match &mut source {
            CoreDecisionSource::Model { call_id: source_call, .. } => *source_call = Some(call_id.as_str().into()),
            _ => return Err("execution constraint must be an explicit model interpretation".into()),
        }
        let scope_id = scope.scope_id.clone();
        let entry = ExecutionConstraintEntry { constraint_id: self.session.allocate_id("execution_constraint"), scope_id,
            domain: update.domain, selection: update.selection, source_input_refs: update.source_input_refs,
            interpreted_user_frontier: frontier, interpreted_user_sequence: max_sequence,
            rationale: update.rationale, replaces_constraint_id: update.replaces_constraint_id, source };
        self.session.execution.requirements.execution_constraints.entries.push(entry.clone());
        Ok(json!({"constraint":entry,"unchanged":false,"permission_granted":false}))
    }

    /// Call only after input schema validation, before permissions/dispatch.
    /// A diagnostic may inspect availability but cannot satisfy a target action.
    pub(super) fn execution_target_admission(&self) -> ExecutionTargetAdmission {
        let state = &self.session.execution.requirements;
        ExecutionTargetAdmission {
            scope_id: state.scope.as_ref().map(|scope|scope.scope_id.clone()),
            bound_scope_id: state.execution_constraints.active_scope_id.clone(),
            current: state.execution_constraints.current(ExecutionConstraintDomain::BrowserTarget).cloned(),
            user_frontier: state.scope.as_ref().map(|scope|state.execution_constraints.user_frontier(scope,&self.session.execution.input_queue).0).unwrap_or_default(),
        }
    }

    pub(super) fn execution_constraint_context(&self, turn_id: &TurnId) -> Result<Option<AgentItem>, String> {
        let state = &self.session.execution.requirements;
        let Some(scope) = &state.scope else { return Ok(None); };
        let admissions = scope.applied_input_refs.iter().filter_map(|reference|self.session.execution.input_queue.applied_admission(reference)).collect::<Vec<_>>();
        let selected = admissions.iter().rev().find(|admission|matches!(admission.origin,AgentInputOrigin::InteractiveUser|AgentInputOrigin::GoalInteractiveUser))
            .or_else(||admissions.iter().find(|admission|is_task_demand(admission.origin)));
        let source_ids = selected.iter().flat_map(|admission|admission.item_ids.iter()).filter(|id|id.len() <= 256).take(8).cloned().collect::<Vec<_>>();
        let current = state.execution_constraints.current(ExecutionConstraintDomain::BrowserTarget);
        let metadata = json!({"scope_id":scope.scope_id,
            "historical_source_scopes_omitted":state.execution_constraints.source_scopes.len(),
            "interpretation_sources":{"origin":selected.map(|admission|admission.origin),"source_input_refs":source_ids,
                "source_items_omitted":selected.map_or(0,|admission|admission.item_ids.len()).saturating_sub(source_ids.len()),
                "admissions_omitted":admissions.len().saturating_sub(if selected.is_some() {1} else {0})},
            "current":current.map(|entry|json!({"constraint_id":entry.constraint_id,"selection":entry.selection,
                "source_scope_id":entry.scope_id,"bound_scope_id":state.execution_constraints.active_scope_id,
                "source_input_refs":entry.source_input_refs.iter().take(4).collect::<Vec<_>>(),
                "source_items_omitted":entry.source_input_refs.len().saturating_sub(4),
                "rationale_excerpt":entry.rationale.chars().take(512).collect::<String>(),
                "rationale_truncated":entry.rationale.chars().count()>512})),
            "interpretation_current":current
                .is_some_and(|entry|state.execution_constraints.active_scope_id.as_ref() == Some(&scope.scope_id)
                    && interpretation_current(entry,&scope.scope_id,&state.execution_constraints.user_frontier(scope,&self.session.execution.input_queue).0))});
        Ok(Some(AgentItem::Message { id: ItemId::from_string(format!("{}:execution_constraints",turn_id.as_str())), role: AgentRole::User,
            content:format!("Core execution conditions, not new user instructions. Interpret the original admitted demand; do not infer a target from an ambiguous word. Before browser actions, explicitly record the required target or unconstrained once for the current user frontier. If a backend fails, preserve the target, diagnose, or ask for changed direction; successful work on another backend is not fulfillment. New user input requires one reaffirmation referencing the current constraint ID; recovery, compaction and tool errors do not. Diagnostic status can inspect another backend but does not fulfill the requested action. A constraint update may be first in the same tool batch as the action.\n<execution_constraint_state>{metadata}</execution_constraint_state>") }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn add_input(runtime: &mut AgentRuntime, turn_id: &TurnId, id: &str, text: &str, origin: AgentInputOrigin, new_scope: bool) {
        let items = vec![AgentItem::user_message(ItemId::from_string(id.into()),text)];
        let batch = runtime.session.execution.input_queue.admit(format!("admission-{id}"),None,origin,
            AgentInputDelivery::CurrentTurn,None,"a".repeat(64),items.clone()).unwrap();
        let applied = runtime.session.execution.input_queue.apply(&batch,turn_id.as_str()).unwrap();
        runtime.session.record_turn_items(turn_id,items.clone());
        runtime.apply_requirement_inputs(turn_id,&[applied],&items,new_scope,&mut vec![]).unwrap();
        let binding = runtime.requirement_binding().unwrap();
        runtime.session.execution.active_turn.as_mut().unwrap().last_model_requirement_binding = binding;
        runtime.session.execution.requirements.model_source = Some(CoreDecisionSource::Model {
            effect_id:format!("model-effect-{id}"),request_id:format!("model-request-{id}"),attempt_id:None,call_id:None });
    }

    fn fixture(text: &str) -> (AgentRuntime,TurnId) {
        let mut runtime = AgentRuntime::try_new(AgentCoreConfig {
            session_id:Some(SessionId::from_string("execution-constraint-test".into())),
            turn_identity:crate::agent::model_capability::test_host_identity(),..Default::default() }).unwrap();
        let turn_id = TurnId::from_string("target-turn".into());
        let context = AgentTurnContext { input_origin:AgentInputOrigin::InteractiveUser,
            configuration:runtime.session.configuration.clone(),platform:Default::default(),components:vec![],
            component_metrics:vec![],component_payload_digest:String::new(),items:vec![],
            host_id:runtime.session.host.catalog.host_id.0.clone(),host_catalog_revision:runtime.session.host.catalog.revision,
            host_availability_epoch:runtime.session.host.catalog.availability_epoch,
            capability_projection:Default::default(),runtime_snapshot:Default::default() };
        let mut turn = AgentTurnState::new(turn_id.clone(),context);
        turn.status = AgentTurnStatus::AwaitingModel;
        runtime.session.execution.active_turn = Some(Box::new(turn));
        add_input(&mut runtime,&turn_id,"original",text,AgentInputOrigin::InteractiveUser,true);
        (runtime,turn_id)
    }

    fn update(target: &str, source: &str, previous: Option<&str>) -> Value {
        json!({"domain":"browser_target","selection":{"kind":"required","target":target},
            "source_input_refs":[source],"rationale":"Explicit interpretation of admitted demand",
            "replaces_constraint_id":previous})
    }

    fn projection(role: ExecutionTargetRole) -> ExecutionTargetProjection {
        ExecutionTargetProjection {domain:ExecutionConstraintDomain::BrowserTarget,argument:"target".into(),
            default_target:BrowserExecutionTarget::Managed,role}
    }

    #[test]
    fn recovery_and_ambiguous_language_do_not_change_a_recorded_target() {
        for prompt in ["请用系统 Chrome", "Use my connected Chrome", "Search current page", "继续在这里处理"] {
            let (mut runtime,turn_id) = fixture(prompt);
            let action = projection(ExecutionTargetRole::Action);
            assert!(runtime.execution_target_admission().validate(Some(&action),&json!({"target":"attached"})).is_err());
            runtime.apply_model_execution_constraint(&turn_id,&CallId::from_string("declare".into()),&update("attached","original",None)).unwrap();
            assert!(runtime.execution_target_admission().validate(Some(&action),&json!({"target":"attached"})).is_ok());
            assert!(runtime.execution_target_admission().validate(Some(&action),&json!({"target":"managed"})).is_err());
            let before = serde_json::to_value(&runtime.session.execution.requirements.execution_constraints).unwrap();
            let previous = runtime.session.execution.requirements.execution_constraints.current(ExecutionConstraintDomain::BrowserTarget).unwrap().constraint_id.clone();
            assert!(runtime.apply_model_execution_constraint(&turn_id,&CallId::from_string("fallback".into()),
                &update("managed","original",Some(&previous))).is_err());
            add_input(&mut runtime,&turn_id,"recovery","Attached unavailable; try another approach",AgentInputOrigin::SystemRecovery,false);
            assert!(runtime.execution_target_admission().validate(Some(&action),&json!({"target":"attached"})).is_ok());
            assert!(runtime.apply_model_execution_constraint(&turn_id,&CallId::from_string("recovery-fallback".into()),
                &update("managed","recovery",Some(&previous))).is_err());
            assert_eq!(serde_json::to_value(&runtime.session.execution.requirements.execution_constraints).unwrap(),before);
        }
    }

    #[test]
    fn real_new_user_frontier_needs_one_reaffirmation_or_explicit_replacement() {
        let (mut runtime,turn_id) = fixture("Use connected Chrome");
        runtime.apply_model_execution_constraint(&turn_id,&CallId::from_string("declare".into()),&update("attached","original",None)).unwrap();
        let old = runtime.session.execution.requirements.execution_constraints.current(ExecutionConstraintDomain::BrowserTarget).unwrap().constraint_id.clone();
        add_input(&mut runtime,&turn_id,"follow-up","Continue / 继续; search the current page",AgentInputOrigin::InteractiveUser,false);
        let action = projection(ExecutionTargetRole::Action);
        assert!(runtime.execution_target_admission().validate(Some(&action),&json!({"target":"attached"})).is_err());
        assert!(runtime.apply_model_execution_constraint(&turn_id,&CallId::from_string("reuse-old-input".into()),
            &update("managed","original",Some(&old))).is_err());
        runtime.apply_model_execution_constraint(&turn_id,&CallId::from_string("reaffirm".into()),&update("attached","follow-up",Some(&old))).unwrap();
        assert!(runtime.execution_target_admission().validate(Some(&action),&json!({"target":"attached"})).is_ok());
        let current = runtime.session.execution.requirements.execution_constraints.current(ExecutionConstraintDomain::BrowserTarget).unwrap().constraint_id.clone();
        add_input(&mut runtime,&turn_id,"change","改用 Nola 托管浏览器 / use Nola managed instead",AgentInputOrigin::InteractiveUser,false);
        runtime.apply_model_execution_constraint(&turn_id,&CallId::from_string("change-target".into()),&update("managed","change",Some(&current))).unwrap();
        assert!(runtime.execution_target_admission().validate(Some(&action),&json!({"target":"managed"})).is_ok());
        assert_eq!(runtime.session.execution.requirements.execution_constraints.entries.len(),3);
        runtime.session.execution.requirements.validate(&runtime.session.execution.input_queue).unwrap();
    }

    #[test]
    fn cold_ledger_retains_constraints_and_only_status_is_exempt() {
        let (mut runtime,turn_id) = fixture("Use connected browser");
        let diagnostic = projection(ExecutionTargetRole::Diagnostic);
        assert!(runtime.execution_target_admission().validate(Some(&diagnostic),&json!({"target":"managed"})).is_ok());
        runtime.apply_model_execution_constraint(&turn_id,&CallId::from_string("declare".into()),&update("attached","original",None)).unwrap();
        let stored = serde_json::to_value(&runtime.session.execution.requirements).unwrap();
        runtime.session.execution.requirements = serde_json::from_value(stored.clone()).unwrap();
        runtime.session.execution.requirements.validate(&runtime.session.execution.input_queue).unwrap();
        assert!(runtime.execution_target_admission().validate(Some(&projection(ExecutionTargetRole::Action)),&json!({})).is_err(),"Default managed cannot bypass attached constraint");
        assert!(runtime.execution_target_admission().validate(Some(&diagnostic),&json!({"target":"managed"})).is_ok());
        assert!(runtime.execution_target_admission().validate(None,&json!({"word":"search current"})).is_ok(),"Nonconsumer tools are unaffected, not selected by language");
        let mut incompatible = stored;
        incompatible.as_object_mut().unwrap().remove("execution_constraints");
        assert!(serde_json::from_value::<super::super::requirements::RequirementExecutionState>(incompatible).is_err());
    }

    #[test]
    fn unconstrained_is_explicit_and_invalid_control_preserves_ledger_and_ids() {
        let (mut runtime,turn_id) = fixture("Inspect available browser content");
        let id_sequence = runtime.session.journal.next_id_sequence;
        let before = serde_json::to_value(&runtime.session.execution.requirements).unwrap();
        assert!(runtime.apply_model_execution_constraint(&turn_id,&CallId::from_string("invent".into()),&update("attached","tool-error",None)).is_err());
        assert_eq!(runtime.session.journal.next_id_sequence,id_sequence);
        assert_eq!(serde_json::to_value(&runtime.session.execution.requirements).unwrap(),before);
        let value = json!({"domain":"browser_target","selection":{"kind":"unconstrained"},
            "source_input_refs":["original"],"rationale":"No target condition in this admitted task"});
        runtime.apply_model_execution_constraint(&turn_id,&CallId::from_string("unconstrained".into()),&value).unwrap();
        let action = projection(ExecutionTargetRole::Action);
        for target in ["attached","managed"] { assert!(runtime.execution_target_admission().validate(Some(&action),&json!({"target":target})).is_ok()); }
        runtime.apply_model_execution_constraint(&turn_id,&CallId::from_string("repeat".into()),&value).unwrap();
        assert_eq!(runtime.session.execution.requirements.execution_constraints.entries.len(),1);
    }

    #[test]
    fn model_context_exposes_only_bounded_exact_current_sources() {
        let (mut runtime,turn_id) = fixture("Use connected browser");
        runtime.apply_model_execution_constraint(&turn_id,&CallId::from_string("declare".into()),&update("attached","original",None)).unwrap();
        let previous = runtime.session.execution.requirements.execution_constraints.current(ExecutionConstraintDomain::BrowserTarget).unwrap().constraint_id.clone();
        for index in 0..40 { add_input(&mut runtime,&turn_id,&format!("follow-up-{index:02}"),"Continue",AgentInputOrigin::InteractiveUser,false); }
        let Some(AgentItem::Message {content,..}) = runtime.execution_constraint_context(&turn_id).unwrap() else {panic!("constraint context")};
        assert!(content.len() < 8_000);
        let body = content.split("<execution_constraint_state>").nth(1).unwrap().split("</execution_constraint_state>").next().unwrap();
        let context: Value = serde_json::from_str(body).unwrap();
        assert_eq!(context["current"]["constraint_id"],previous);
        assert_eq!(context["current"]["source_input_refs"],json!(["original"]));
        assert_eq!(context["interpretation_sources"]["source_input_refs"],json!(["follow-up-39"]));
        assert_eq!(context["interpretation_sources"]["admissions_omitted"],40);
        assert_eq!(context["interpretation_current"],false);
    }

    #[test]
    fn goal_successor_scopes_retain_original_constraint_provenance_and_target() {
        let (mut runtime,turn_id) = fixture("Use connected Chrome");
        runtime.apply_model_execution_constraint(&turn_id,&CallId::from_string("declare".into()),&update("attached","original",None)).unwrap();
        let original = runtime.session.execution.requirements.execution_constraints.entries[0].clone();
        for index in 0..3 {
            let source = format!("continuation-{index}");
            add_input(&mut runtime,&turn_id,&source,"Continue the accepted goal",AgentInputOrigin::GoalContinuation,true);
            let state = &runtime.session.execution.requirements;
            assert_ne!(state.scope.as_ref().unwrap().scope_id,original.scope_id);
            assert!(!state.scope.as_ref().unwrap().source_input_refs.contains(&"original".to_owned()),"old Run input is not a new requirement source");
            assert_eq!(state.execution_constraints.entries,vec![original.clone()],"do not relabel original scope/input/model ownership");
            assert_eq!(state.execution_constraints.active_scope_id.as_ref(),Some(&state.scope.as_ref().unwrap().scope_id));
            assert!(runtime.execution_target_admission().validate(Some(&projection(ExecutionTargetRole::Action)),&json!({"target":"attached"})).is_ok());
            assert!(runtime.execution_target_admission().validate(Some(&projection(ExecutionTargetRole::Action)),&json!({"target":"managed"})).is_err());
            assert!(runtime.apply_model_execution_constraint(&turn_id,&CallId::from_string(format!("fallback-{index}")),&update("managed",&source,Some(&original.constraint_id))).is_err());
            runtime.session.execution.requirements.validate(&runtime.session.execution.input_queue).unwrap();
        }
        let stored = serde_json::to_value(&runtime.session.execution.requirements).unwrap();
        runtime.session.execution.requirements = serde_json::from_value(stored).unwrap();
        runtime.session.execution.requirements.validate(&runtime.session.execution.input_queue).unwrap();
        assert!(runtime.execution_target_admission().validate(Some(&projection(ExecutionTargetRole::Action)),&json!({"target":"attached"})).is_ok());
    }

    #[test]
    fn maintenance_suspends_authority_without_forgetting_the_goal_target() {
        let (mut runtime,turn_id) = fixture("Use connected Chrome");
        runtime.apply_model_execution_constraint(&turn_id,&CallId::from_string("declare".into()),&update("attached","original",None)).unwrap();
        let state = &runtime.session.execution.requirements;
        let suspended = state.execution_constraints.suspend(state.scope.as_ref(),&runtime.session.execution.input_queue).unwrap();
        runtime.session.execution.requirements = super::super::requirements::RequirementExecutionState {
            execution_constraints:suspended,..Default::default() };
        runtime.session.execution.requirements.validate(&runtime.session.execution.input_queue).unwrap();
        assert!(runtime.execution_target_admission().validate(Some(&projection(ExecutionTargetRole::Action)),&json!({"target":"attached"})).is_err());
        let stored = serde_json::to_value(&runtime.session.execution.requirements).unwrap();
        runtime.session.execution.requirements = serde_json::from_value(stored).unwrap();
        add_input(&mut runtime,&turn_id,"post-maintenance","Continue goal",AgentInputOrigin::GoalContinuation,true);
        assert_eq!(runtime.session.execution.requirements.execution_constraints.entries.len(),1);
        assert!(runtime.execution_target_admission().validate(Some(&projection(ExecutionTargetRole::Action)),&json!({"target":"attached"})).is_ok());
        runtime.session.execution.requirements.validate(&runtime.session.execution.input_queue).unwrap();
    }

    #[test]
    fn new_live_user_can_explicitly_replace_a_carried_target_but_nonlive_cannot() {
        let (mut runtime,turn_id) = fixture("Use connected Chrome");
        runtime.apply_model_execution_constraint(&turn_id,&CallId::from_string("declare".into()),&update("attached","original",None)).unwrap();
        let original = runtime.session.execution.requirements.execution_constraints.entries[0].clone();
        add_input(&mut runtime,&turn_id,"continuation","Continue goal",AgentInputOrigin::GoalContinuation,true);
        add_input(&mut runtime,&turn_id,"user-change","Use Nola managed browser instead",AgentInputOrigin::GoalInteractiveUser,true);
        assert!(runtime.execution_target_admission().validate(Some(&projection(ExecutionTargetRole::Action)),&json!({"target":"attached"})).is_err());
        assert!(runtime.apply_model_execution_constraint(&turn_id,&CallId::from_string("wrong-source".into()),&update("managed","original",Some(&original.constraint_id))).is_err());
        runtime.apply_model_execution_constraint(&turn_id,&CallId::from_string("explicit-change".into()),&update("managed","user-change",Some(&original.constraint_id))).unwrap();
        assert_eq!(runtime.session.execution.requirements.execution_constraints.entries[0],original);
        assert!(runtime.execution_target_admission().validate(Some(&projection(ExecutionTargetRole::Action)),&json!({"target":"managed"})).is_ok());
        runtime.session.execution.requirements.validate(&runtime.session.execution.input_queue).unwrap();
    }

    #[test]
    fn scope_rollover_does_not_hide_an_uninterpreted_live_user_frontier() {
        let (mut runtime,turn_id) = fixture("Use connected Chrome");
        runtime.apply_model_execution_constraint(&turn_id,&CallId::from_string("declare".into()),&update("attached","original",None)).unwrap();
        add_input(&mut runtime,&turn_id,"uninterpreted-user","Continue / search current",AgentInputOrigin::InteractiveUser,false);
        add_input(&mut runtime,&turn_id,"continuation","Continue goal",AgentInputOrigin::GoalContinuation,true);
        assert!(runtime.execution_target_admission().validate(Some(&projection(ExecutionTargetRole::Action)),&json!({"target":"attached"})).is_err(),"new scope cannot launder pending user interpretation");
        let id = runtime.session.execution.requirements.execution_constraints.entries[0].constraint_id.clone();
        assert!(runtime.apply_model_execution_constraint(&turn_id,&CallId::from_string("nonlive-reaffirm".into()),&update("attached","continuation",Some(&id))).is_err());
        add_input(&mut runtime,&turn_id,"fresh-user","Continue using the connected browser",AgentInputOrigin::GoalInteractiveUser,false);
        runtime.apply_model_execution_constraint(&turn_id,&CallId::from_string("reaffirm".into()),&update("attached","fresh-user",Some(&id))).unwrap();
        assert!(runtime.execution_target_admission().validate(Some(&projection(ExecutionTargetRole::Action)),&json!({"target":"attached"})).is_ok());
        runtime.session.execution.requirements.validate(&runtime.session.execution.input_queue).unwrap();
    }

    #[test]
    fn carried_checkpoint_rejects_lost_sources_wrong_binding_and_budget_overflow() {
        let (mut runtime,turn_id) = fixture("Use connected Chrome");
        runtime.apply_model_execution_constraint(&turn_id,&CallId::from_string("declare".into()),&update("attached","original",None)).unwrap();
        add_input(&mut runtime,&turn_id,"continuation","Continue goal",AgentInputOrigin::GoalContinuation,true);
        let valid = runtime.session.execution.requirements.clone();
        let mut missing = valid.clone();
        missing.execution_constraints.source_scopes.clear();
        assert!(missing.validate(&runtime.session.execution.input_queue).is_err());
        let mut wrong_binding = valid.clone();
        wrong_binding.execution_constraints.active_scope_id = Some("invented-scope".into());
        assert!(wrong_binding.validate(&runtime.session.execution.input_queue).is_err());
        let mut wrong_source = valid.clone();
        wrong_source.execution_constraints.source_scopes[0].applied_input_refs[0].turn_id = "different-owner".into();
        assert!(wrong_source.validate(&runtime.session.execution.input_queue).is_err());
        let mut excessive = valid;
        let snapshot = excessive.execution_constraints.source_scopes[0].clone();
        excessive.execution_constraints.source_scopes = vec![snapshot;129];
        assert!(excessive.execution_constraints.suspend(excessive.scope.as_ref(),&runtime.session.execution.input_queue).is_err());
        let mut incompatible = serde_json::to_value(&runtime.session.execution.requirements.execution_constraints).unwrap();
        incompatible.as_object_mut().unwrap().remove("source_scopes");
        assert!(serde_json::from_value::<ExecutionConstraintLedger>(incompatible).is_err());
        let mut incompatible = serde_json::to_value(&runtime.session.execution.requirements.execution_constraints).unwrap();
        incompatible.as_object_mut().unwrap().remove("active_scope_id");
        assert!(serde_json::from_value::<ExecutionConstraintLedger>(incompatible).is_err());
    }
}
