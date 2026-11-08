// SPDX-License-Identifier: Apache-2.0
//! Applies committed policy to undispatched work without forging user choices.
use super::*;

impl AgentRuntime {
    pub(super) fn apply_active_permission_policy(
        &mut self,
        policy: nola_domain::ActivePermissionPolicy,
        events: &mut Vec<RuntimeEvent>,
        effects: &mut Vec<AgentEffect>,
    ) -> Result<(), String> {
        policy.validate()?;
        let turn = self.session.execution.active_turn.as_ref()
            .filter(|turn| turn.turn_id.as_str() == policy.run_id && !turn.status.is_terminal())
            .ok_or("permission policy has no matching active Run")?;
        if self.session.execution.active_permission_policy.as_ref().is_some_and(|previous|
            previous.run_id == policy.run_id && previous.configuration_revision >= policy.configuration_revision) {
            return if self.session.execution.active_permission_policy.as_ref() == Some(&policy)
                || self.session.execution.active_permission_policy.as_ref().is_some_and(|previous|
                    previous.configuration_revision > policy.configuration_revision) {
                Ok(())
            } else { Err("permission policy revision is stale or conflicts".into()) };
        }
        let turn_id = turn.turn_id.clone();
        let input_origin = turn.context.input_origin;
        let cancelling = turn.status == AgentTurnStatus::Cancelling || turn.cancellation_reason.is_some();
        let mut rules = self.config.permission_rules.with_profile(policy.profile);
        rules.merge_checkpoint_conversation_scope(&self.session.execution.permission_rules);
        self.session.execution.permission_rules = rules;
        self.session.execution.active_permission_policy = Some(policy.clone());
        // Policy ACK is a permission projection change even when there were
        // no pending challenges. Selective subscribers must observe it.
        self.session.execution.permissions.revision = self.session.execution.permissions.revision.saturating_add(1);
        // Dispatched means Core generated an envelope, not that the Host has
        // crossed its physical boundary. ExecutionStarted changes the journal
        // to Accepted. Application atomically fences withdrawn leased outbox
        // deliveries, so an old worker cannot start after this transition.
        let undispatched = self.session.host.pending_effects.iter()
            .filter(|effect| !cancelling && effect.turn_id.as_deref() == Some(turn_id.as_str())
                && effect.state == PendingHostEffectState::Dispatched)
            .filter(|effect| effect.validated_control().ok().is_some_and(|control|
                matches!(control.payload.policy, nola_domain::host_control::HostResponsePolicy::ExecuteTool { .. })))
            .filter_map(|effect| effect.call_id.clone().map(|call_id|(effect.effect_id.clone(),call_id)))
            .collect::<Vec<_>>();
        for (effect_id, call_id) in &undispatched {
            self.session.host.pending_effects.complete(effect_id);
            if let Some(tool) = self.session.execution.active_turn.as_mut()
                .and_then(|turn|turn.tools.iter_mut().find(|tool|tool.call_id.as_str() == call_id))
                .filter(|tool|tool.status == ToolCallStatus::Running) {
                tool.status = ToolCallStatus::Pending;
                tool.permission_recheck = true;
            }
        }
        // Declared policy is not a new model prompt or a user authorization.
        // Completed, denied, cancelled and physically admitted calls are untouched.
        let releasable = self.session.execution.permissions.pending.iter()
            .filter(|_| !cancelling)
            .filter(|pending| pending.turn_id == turn_id)
            .filter(|pending| {
                let resolution = self.session.execution.active_turn.as_ref()
                    .and_then(|turn|turn.tools.iter().find(|tool|tool.call_id == pending.call_id))
                    .and_then(|tool|tool.permission_resolution.as_ref());
                let resolved = match resolution {
                    Some((request,facts)) => match facts.subject_for(request) {
                        Ok(subject) => Some(subject), Err(_) => return false,
                    },
                    None => None,
                };
                self.session.execution.permission_rules.evaluate_verified_resources_for_origin_with_explanation(
                    &super::super::permissions::PermissionSubject {
                        capability: pending.permission_domain.clone(), resource: pending.resource.clone(),
                    }, input_origin, true, resolved.as_ref()).decision == RuleDecision::Allow
            })
            .map(|pending| pending.request_id.clone()).collect::<Vec<_>>();
        for request_id in releasable {
            self.apply_permission_resolution(request_id, PermissionUserDecision::AllowOnce, None,
                nola_domain::permission_settlement::PermissionReceiptOutcome::PolicyApplied { policy: policy.clone() },
                events, effects)?;
        }
        if !undispatched.is_empty() {
            self.schedule_tools(&turn_id, events, effects)?;
        }
        let admitted = self.session.host.pending_effects.iter().filter(|effect|
            effect.turn_id.as_deref() == Some(turn_id.as_str()) && effect.state != PendingHostEffectState::Dispatched)
            .filter(|effect|effect.validated_control().ok().is_some_and(|control|
                matches!(control.payload.policy,nola_domain::host_control::HostResponsePolicy::ExecuteTool {..})))
            .map(|effect|effect.effect_id.clone()).collect::<Vec<_>>();
        events.push(RuntimeEvent::HostProgress {
            turn_id: Some(turn_id), effect_id: policy.command_id(),
            title: "permission_policy.applied".into(), detail: None,
            metadata: json!({"active_permission_policy":policy,"unaffected_admitted_effect_ids":admitted}),
        });
        Ok(())
    }
}
