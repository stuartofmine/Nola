// SPDX-License-Identifier: Apache-2.0
// Internal layer of the deterministic agent runtime. Public clients use AgentRuntime.

use super::*;

impl AgentRuntime {
    pub fn try_new(mut config: AgentCoreConfig) -> Result<Self, String> {
        Self::validate_config(&config)?;
        Self::validate_initial_items(&config.initial_items)?;
        config.context_window = config.context_window.normalized();
        let session_id = config
            .session_id
            .clone()
            .ok_or_else(|| "Agent Host must provide session_id".to_string())?;
        let mut session =
            AgentThreadState::new(session_id, AgentSessionConfiguration::from_runtime(&config));
        session.conversation.transcript = config.initial_items.clone();
        session.conversation.model_context = config.initial_items.clone();
        session.item_ownership = Self::infer_item_ownership(&config.initial_items);
        session
            .conversation
            .context_window
            .refresh_estimate(&session.conversation.model_context);
        session.execution.permission_rules = config.permission_rules.clone();
        if let Some(host_catalog) = config.host_catalog.clone() {
            session.host.catalog = host_catalog;
        }
        Ok(Self {
            session,
            config,
            observed_at_ms: 0,
        })
    }

    pub fn restore(
        mut config: AgentCoreConfig,
        checkpoint: CoreCheckpoint<AgentThreadState>,
    ) -> Result<Self, String> {
        Self::validate_config(&config)?;
        checkpoint.validate_state_identity()?;
        config.context_window = config.context_window.normalized();
        if let Some(expected_session_id) = config.session_id.as_ref() {
            if expected_session_id.as_str() != checkpoint.session_id {
                return Err("Agent checkpoint belongs to a different session".to_string());
            }
        }
        let expected_configuration = AgentSessionConfiguration::from_runtime(&config);
        let configuration_changed = !checkpoint.state.configuration.matches_runtime(&config);
        let mut replaceable_configuration = checkpoint.state.configuration.clone();
        replaceable_configuration.model = expected_configuration.model.clone();
        replaceable_configuration.max_model_iterations =
            expected_configuration.max_model_iterations;
        replaceable_configuration.tools = expected_configuration.tools.clone();
        replaceable_configuration.context_window = expected_configuration.context_window.clone();
        replaceable_configuration.turn_identity = expected_configuration.turn_identity.clone();
        let can_replace_idle_execution_configuration = configuration_changed
            && replaceable_configuration.matches_runtime(&config)
            && checkpoint.state.permits_new_turn_execution_configuration();
        if !checkpoint.state.configuration.matches_runtime(&config)
            && !can_replace_idle_execution_configuration
        {
            let expected = AgentSessionConfiguration::from_runtime(&config);
            let actual = &checkpoint.state.configuration;
            let mut differences = Vec::new();
            if actual.model != expected.model {
                differences.push("model");
            }
            if actual.max_model_iterations != expected.max_model_iterations {
                differences.push("max_model_iterations");
            }
            if actual.tools != expected.tools {
                differences.push("tools");
            }
            if serde_json::to_value(&actual.context_window).ok()
                != serde_json::to_value(&expected.context_window).ok()
            {
                differences.push("context window");
            }
            if actual.memory_policy != expected.memory_policy {
                differences.push("memory policy");
            }
            if actual.turn_identity != expected.turn_identity {
                differences.push("turn identity");
            }
            return Err(format!(
                "checkpoint session configuration mismatch: {}",
                differences.join(", ")
            ));
        }
        let mut session = checkpoint.state.into_checkpoint_projection();
        if can_replace_idle_execution_configuration {
            session.configuration.model = config.model.clone();
            session.configuration.max_model_iterations = config.max_model_iterations;
            session.configuration.tools = config.tools.clone();
            session.configuration.context_window = config.context_window.clone();
            session.configuration.turn_identity = config.turn_identity.clone();
        }
        session.conversation.context_window.config = config.context_window.clone();
        let checkpoint_permission_rules = session.execution.permission_rules.clone();
        session.execution.permission_rules = config.permission_rules.clone();
        session
            .execution
            .permission_rules
            .merge_checkpoint_conversation_scope(&checkpoint_permission_rules);
        if !session.permits_new_turn_execution_configuration() {
            if let Some(policy) = &session.execution.active_permission_policy {
                policy.validate()?;
                if session.execution.active_turn.as_ref().map(|turn| turn.turn_id.as_str()) != Some(policy.run_id.as_str()) {
                    return Err("active permission policy belongs to a different Run".into());
                }
                session.execution.permission_rules = session.execution.permission_rules.with_profile(policy.profile);
            }
        } else {
            session.execution.active_permission_policy = None;
        }
        if let Some(host_catalog) = config.host_catalog.clone() {
            if host_catalog.host_id != session.host.catalog.host_id
                && session.host.pending_effects.len() > 0
            {
                return Err("cannot restore pending effects onto a different Host".to_string());
            }
            let same_host_catalog_is_stale = host_catalog.host_id == session.host.catalog.host_id
                && (host_catalog.revision < session.host.catalog.revision
                    || host_catalog.availability_epoch < session.host.catalog.availability_epoch);
            if session.host.pending_effects.len() > 0
                && (host_catalog.host_id != session.host.catalog.host_id
                    || host_catalog.revision != session.host.catalog.revision
                    || host_catalog.availability_epoch != session.host.catalog.availability_epoch
                    || host_catalog.lease.as_ref().map(|lease| &lease.lease_id)
                        != session
                            .host
                            .catalog
                            .lease
                            .as_ref()
                            .map(|lease| &lease.lease_id))
            {
                return Err(
                    "pending Host effects require explicit recovery onto the new Host owner"
                        .to_string(),
                );
            }
            // A Host process may restart its in-memory catalog counter while
            // the session checkpoint already owns a newer committed catalog.
            // With no pending effects, retain that newer snapshot rather than
            // either regressing the catalog or making maintenance operations
            // such as compaction impossible. Pending effects remain protected
            // by the exact-owner check above.
            if !same_host_catalog_is_stale {
                session.host.catalog = host_catalog;
            }
        }
        let mut core = Self {
            session,
            config,
            observed_at_ms: 0,
        };
        // Recovery-decision effects are not automatically replayable, but their
        // frozen identity must still be valid before adopting this checkpoint.
        core.session.host.pending_effects.restore_controls()?;
        for effect in core.session.host.pending_effects.iter() {
            if effect.session_id != core.session.session_id.as_str() {
                return Err("pending Host effect belongs to a different session".into());
            }
            effect.validated_control()?;
        }
        Ok(core)
    }

    pub(super) fn validated_pending_request(
        &self,
        effect: &PendingHostEffect,
    ) -> Result<HostRequestEnvelope, String> {
        if effect.session_id != self.session.session_id.as_str() {
            return Err("pending Host effect belongs to a different session".into());
        }
        let mut request = effect.validated_request()?;
        request.resume_after_sequence = effect.last_response_sequence;
        Ok(request)
    }

    pub fn pending_host_requests(&self) -> Result<Vec<HostRequestEnvelope>, String> {
        self.session
            .host
            .pending_effects
            .iter()
            .filter(|effect| self.effect_can_replay_automatically(effect))
            .map(|effect| self.validated_pending_request(effect))
            .collect()
    }

    /// Returns the latest in-memory delivery cursor for one effect whose
    /// execution authority was already established by Application's durable
    /// outbox. Unlike `pending_host_requests`, this does not decide whether an
    /// effect is safe to replay automatically; it only overlays hot Core state
    /// onto an effect that the caller has independently proved is committed.
    pub fn committed_host_request_resume_snapshot(
        &self,
        effect_id: &str,
    ) -> Result<Option<HostRequestEnvelope>, String> {
        let Some(effect) = self.session.host.pending_effects.get(effect_id) else {
            return Ok(None);
        };
        self.validated_pending_request(effect).map(Some)
    }

    pub fn committed_host_effect_control(
        &self,
        effect_id: &str,
    ) -> Result<Option<nola_domain::host_control::HostEffectControl>, String> {
        let Some(effect) = self.session.host.pending_effects.get(effect_id) else {
            return Ok(None);
        };
        if effect.session_id != self.session.session_id.as_str() {
            return Err("pending Host effect belongs to a different session".into());
        }
        let mut control = effect.validated_control()?.clone();
        control.resume_after_sequence = effect.last_response_sequence;
        Ok(Some(control))
    }

    pub(super) fn infer_item_ownership(
        items: &[AgentItem],
    ) -> BTreeMap<String, AgentItemOwnership> {
        let mut ownership = BTreeMap::new();
        let mut active_turn_id: Option<String> = None;
        let mut active_assistant_message_id: Option<String> = None;
        for item in items {
            if item.is_user_input() {
                active_turn_id = Some(format!("history-turn:{}", item.id().as_str()));
                active_assistant_message_id = None;
            }
            let turn_id = active_turn_id
                .get_or_insert_with(|| format!("history-turn:{}", item.id().as_str()))
                .clone();
            let message_id = match item {
                AgentItem::UiAction { id, .. } | AgentItem::Message {
                    id, role: AgentRole::User, ..
                } => id.as_str().to_string(),
                AgentItem::Message {
                    id,
                    role: AgentRole::Assistant,
                    ..
                } => {
                    let message_id = id.as_str().to_string();
                    active_assistant_message_id = Some(message_id.clone());
                    message_id
                }
                AgentItem::Attachment { message_id, .. } => message_id.as_str().to_string(),
                _ => active_assistant_message_id
                    .clone()
                    .unwrap_or_else(|| format!("{turn_id}:assistant")),
            };
            ownership.insert(
                item.id().as_str().to_string(),
                AgentItemOwnership {
                    source: Default::default(),
                    turn_id,
                    message_id,
                },
            );
        }
        ownership
    }

    pub(super) fn validate_initial_items(items: &[AgentItem]) -> Result<(), String> {
        let mut item_ids = HashSet::new();
        let mut seen_call_ids = HashSet::new();
        let mut open_call_ids = HashSet::new();
        for item in items {
            let item_id = match item {
                AgentItem::UiAction { id, .. }
                | AgentItem::Message { id, .. }
                | AgentItem::Attachment { id, .. }
                | AgentItem::Reasoning { id, .. }
                | AgentItem::ToolCall { id, .. }
                | AgentItem::ToolResult { id, .. }
                | AgentItem::Compaction { id, .. } => id.as_str(),
            };
            if item_id.trim().is_empty() || !item_ids.insert(item_id.to_string()) {
                return Err(
                    "initial Agent history contains an empty or duplicate item id".to_string(),
                );
            }
            match item {
                AgentItem::UiAction { action, .. } => action.validate().map_err(|error| error.to_string())?,
                AgentItem::ToolCall { call_id, .. } => {
                    if call_id.as_str().trim().is_empty()
                        || !seen_call_ids.insert(call_id.as_str().to_string())
                    {
                        return Err(
                            "initial Agent history contains an empty or duplicate tool call id"
                                .to_string(),
                        );
                    }
                    open_call_ids.insert(call_id.as_str().to_string());
                }
                AgentItem::ToolResult { call_id, .. } => {
                    if !open_call_ids.remove(call_id.as_str()) {
                        return Err(
                            "initial Agent history contains an orphan tool result".to_string()
                        );
                    }
                }
                _ => {}
            }
        }
        if !open_call_ids.is_empty() {
            return Err("initial Agent history contains an unfinished tool call".to_string());
        }
        Ok(())
    }

    pub(super) fn validate_config(config: &AgentCoreConfig) -> Result<(), String> {
        config.turn_identity.validate_model_capability()?;
        if config
            .session_id
            .as_ref()
            .is_some_and(|session_id| session_id.as_str().trim().is_empty())
        {
            return Err("Agent session id cannot be empty".to_string());
        }
        if let Some(catalog) = config.host_catalog.as_ref() {
            if catalog.host_id.0.trim().is_empty() {
                return Err("Agent Host id cannot be empty".to_string());
            }
            if let Some(lease) = catalog.lease.as_ref() {
                if lease.lease_id.trim().is_empty()
                    || lease.last_heartbeat_ms < lease.issued_at_ms
                    || lease.expires_at_ms <= lease.last_heartbeat_ms
                {
                    return Err("Agent Host lease is invalid".to_string());
                }
            }
            for (capability_id, capability) in &catalog.capabilities {
                if capability_id.trim().is_empty() || capability_id != &capability.id {
                    return Err(
                        "Host capability catalog keys must match non-empty capability ids"
                            .to_string(),
                    );
                }
                if capability.output_policy.preview_char_limit == 0 {
                    return Err(format!(
                        "Host capability '{}' has an invalid output preview limit",
                        capability.id
                    ));
                }
            }
        }
        let mut tool_names = HashSet::new();
        for tool in &config.tools {
            let name = tool
                .pointer("/function/name")
                .and_then(Value::as_str)
                .or_else(|| tool.get("name").and_then(Value::as_str))
                .map(str::trim)
                .filter(|name| !name.is_empty())
                .ok_or_else(|| "Agent model tool is missing a function name".to_string())?;
            if !tool_names.insert(name.to_string()) {
                return Err(format!("Agent model tool '{name}' is duplicated"));
            }
        }
        Ok(())
    }

    /// Application supplies the exact admitted root before reducing a new Run.
    /// Internal continuations may deliberately reuse an earlier assistant root.
    pub fn bind_next_trace_root(&mut self, message_id: &str) -> Result<(), String> {
        if message_id.is_empty() || message_id.trim() != message_id {
            return Err("trace root requires an exact nonempty message identity".into());
        }
        if self.session.execution.active_turn.as_ref().is_some_and(|turn| turn.status != AgentTurnStatus::Idle && !turn.status.is_terminal()) {
            return Err("cannot replace trace root of an active Run".into());
        }
        self.session.journal.trace_context.set_next_message_id(Some(message_id.into()));
        Ok(())
    }

    pub fn turn_status(&self) -> AgentTurnStatus {
        self.session
            .execution
            .active_turn
            .as_ref()
            .map(|turn| turn.status)
            .unwrap_or(AgentTurnStatus::Idle)
    }

    pub fn host_effect_is_cancelling(&self, effect_id: &str) -> bool {
        self.session
            .host
            .pending_effects
            .get(effect_id)
            .is_some_and(|effect| {
                effect.state == super::super::recovery::PendingHostEffectState::Cancelling
            })
    }

    pub fn control_snapshot(&self) -> super::super::protocol::AgentControlSnapshot {
        let active_turn = self.session.execution.active_turn.as_ref();
        super::super::protocol::AgentControlSnapshot {
            active_permission_policy: self.session.execution.active_permission_policy.clone(),
            open_user_input_waits: self.session.execution.user_input_waits.values().filter(|wait| wait.is_open()).cloned().collect(),
            session_id: self.session.session_id.clone(),
            revision: self.session.journal.revision,
            last_event_cursor: EventCursor {
                sequence: self.session.journal.next_event_sequence.saturating_sub(1),
            },
            host_catalog_revision: self.session.host.catalog.revision,
            model: self.session.configuration.model.clone(),
            tool_count: self.session.configuration.tools.len(),
            active_turn_id: active_turn.map(|turn| turn.turn_id.clone()),
            turn_status: self.turn_status(),
            completion_outcome: active_turn.and_then(|turn| turn.completion_outcome),
            history_item_count: self.session.conversation.transcript.len(),
            model_context_item_count: self.session.conversation.model_context.len(),
            queued_input_count: self.session.execution.input_queue.len(),
            active_tool_count: active_turn
                .map(|turn| turn.active_tool_count())
                .unwrap_or_default(),
            context_window_tokens: self
                .session
                .conversation
                .context_window
                .config
                .context_window_tokens,
            context_used_tokens: self.session.conversation.context_window.used_tokens,
            context_usage_percent: self.session.conversation.context_window.usage_percent(),
            context_usage_is_estimated: self.session.conversation.context_window.usage_origin
                == UsageOrigin::Estimated,
            context_usage: self.session.conversation.context_window.usage_snapshot(),
            prompt_cache_diagnostic: self
                .session
                .conversation
                .context_window
                .cumulative_usage
                .prompt_cache_diagnostic
                .clone(),
            compaction_in_progress: self
                .session
                .conversation
                .context_window
                .compaction_in_progress,
            consecutive_compaction_failures: self
                .session
                .conversation
                .context_window
                .consecutive_compaction_failures,
            automatic_compaction_blocked: self
                .session
                .conversation
                .context_window
                .automatic_compaction_blocked,
            pending_host_effect_count: self.session.host.pending_effects.len(),
            pending_recovery_effect_count: self
                .session
                .host
                .pending_effects
                .iter()
                .filter(|effect| !self.effect_can_replay_automatically(effect))
                .count(),
            session_closing: self.session.journal.session_closing,
            session_closed: self.session.journal.session_closed,
            permission_projection_revision: self.session.execution.permissions.revision,
        }
    }

    pub fn snapshot(&self) -> AgentCoreSnapshot {
        let active_turn = self.session.execution.active_turn.as_ref();
        let context_usage = self.session.conversation.context_window.usage_snapshot();
        AgentCoreSnapshot {
            active_permission_policy: self.session.execution.active_permission_policy.clone(),
            open_user_input_waits: self.session.execution.user_input_waits.values().filter(|wait| wait.is_open()).cloned().collect(),
            session_id: self.session.session_id.clone(),
            revision: self.session.journal.revision,
            last_event_cursor: EventCursor {
                sequence: self.session.journal.next_event_sequence.saturating_sub(1),
            },
            host_catalog_revision: self.session.host.catalog.revision,
            model: self.session.configuration.model.clone(),
            tool_count: self.session.configuration.tools.len(),
            active_turn_id: active_turn.map(|turn| turn.turn_id.clone()),
            completion_outcome: active_turn.and_then(|turn| turn.completion_outcome),
            turn_status: self.turn_status(),
            history_item_count: self.session.conversation.transcript.len(),
            model_context_item_count: self.session.conversation.model_context.len(),
            queued_input_count: self.session.execution.input_queue.len(),
            active_tool_count: active_turn
                .map(|turn| turn.active_tool_count())
                .unwrap_or_default(),
            engineering: active_turn
                .map(|turn| turn.engineering.clone())
                .unwrap_or_default(),
            context_window_tokens: self
                .session
                .conversation
                .context_window
                .config
                .context_window_tokens,
            context_used_tokens: self.session.conversation.context_window.used_tokens,
            context_usage_percent: self.session.conversation.context_window.usage_percent(),
            context_usage_is_estimated: self.session.conversation.context_window.usage_origin
                == UsageOrigin::Estimated,
            context_usage,
            prompt_cache_diagnostic: self
                .session
                .conversation
                .context_window
                .cumulative_usage
                .prompt_cache_diagnostic
                .clone(),
            compaction_in_progress: self
                .session
                .conversation
                .context_window
                .compaction_in_progress,
            consecutive_compaction_failures: self
                .session
                .conversation
                .context_window
                .consecutive_compaction_failures,
            automatic_compaction_blocked: self
                .session
                .conversation
                .context_window
                .automatic_compaction_blocked,
            pending_host_effect_count: self.session.host.pending_effects.len(),
            pending_recovery_effect_count: self
                .session
                .host
                .pending_effects
                .iter()
                .filter(|effect| !self.effect_can_replay_automatically(effect))
                .count(),
            session_closing: self.session.journal.session_closing,
            session_closed: self.session.journal.session_closed,
            tasks: self.session.task_projection.clone(),
            goal: self.session.goal_projection.clone(),
            memories: self.session.memory_projection.clone(),
            memory_runtime: self.session.memory_runtime.clone(),
            permissions: self.session.execution.permissions.clone(),
            permission_rules: self.session.execution.permission_rules.clone(),
            skill_runtime: self.session.skill_runtime.clone(),
            projection: self.session.conversation.projection.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn committed_control_stays_small_and_matches_explicit_snapshot_scalars() {
        let private_body = "PRIVATE-HISTORICAL-BODY🙂".repeat(20_000);
        let mut core = AgentRuntime::try_new(AgentCoreConfig {
            turn_identity: crate::agent::model_capability::test_host_identity(),
            session_id: Some(SessionId::from_string("control-owner".into())),
            initial_items: vec![AgentItem::assistant_message(
                ItemId::from_string("historical-answer".into()),
                private_body.clone(),
            )],
            ..Default::default()
        })
        .unwrap();
        // Initial model context is not a canonical display import. Exercise
        // the real validated history rebase that materializes its Part graph.
        core.rebase_conversation(
            core.session.conversation.transcript.clone(),
            Default::default(),
        )
        .unwrap();
        let full = core.snapshot();
        let control = core.control_snapshot();
        let value = serde_json::to_value(&control).unwrap();
        assert_eq!(
            value,
            serde_json::to_value(super::super::super::protocol::AgentControlSnapshot::from(
                &full
            ))
            .unwrap()
        );
        let encoded = serde_json::to_string(&control).unwrap();
        assert!(encoded.len() < 4096);
        assert!(!encoded.contains("PRIVATE-HISTORICAL-BODY"));
        assert!(serde_json::to_string(&full)
            .unwrap()
            .contains(private_body.as_str()));
        for forbidden in [
            "projection",
            "tasks",
            "goal",
            "memories",
            "memory_runtime",
            "permissions",
            "permission_rules",
            "skill_runtime",
            "engineering",
        ] {
            assert!(
                value.get(forbidden).is_none(),
                "control carried {forbidden}"
            );
        }
        let mut foreign_shape = value;
        foreign_shape["projection"] = json!({"text":private_body});
        assert!(
            serde_json::from_value::<super::super::super::protocol::AgentControlSnapshot>(
                foreign_shape
            )
            .is_err()
        );
    }

    #[test]
    fn restore_rejects_inconsistent_checkpoint_envelopes() {
        let config = AgentCoreConfig {
            turn_identity: crate::agent::model_capability::test_host_identity(),
            session_id: Some(SessionId::from_string("restore-envelope".to_string())),
            ..AgentCoreConfig::default()
        };
        let runtime = AgentRuntime::try_new(config.clone()).unwrap();
        let checkpoint = runtime.checkpoint(CheckpointReason::Manual);
        assert!(AgentRuntime::restore(config.clone(), checkpoint.clone()).is_ok());

        for field in 0..6 {
            let mut invalid = checkpoint.clone();
            let expected = match field {
                0 => {
                    invalid.revision += 1;
                    "revision"
                }
                1 => {
                    invalid.state.journal.revision += 1;
                    "revision"
                }
                2 => {
                    invalid.last_event_cursor.sequence += 1;
                    "event cursor"
                }
                3 => {
                    invalid.state.journal.next_event_sequence = 0;
                    "event cursor"
                }
                4 => {
                    invalid.capability_catalog_revision += 1;
                    "Host catalog revision"
                }
                _ => {
                    invalid.state.host.catalog.revision += 1;
                    "Host catalog revision"
                }
            };
            let error = AgentRuntime::restore(config.clone(), invalid)
                .err()
                .expect("inconsistent checkpoint must fail before adoption");
            assert!(error.contains(expected), "{field}: {error}");
        }
    }

    #[test]
    fn idle_configuration_restore_uses_the_same_queued_input_admissibility() {
        let runtime = AgentRuntime::try_new(AgentCoreConfig {
            turn_identity: crate::agent::model_capability::test_host_identity(),
            session_id: Some(SessionId::from_string("idle-scope".into())),
            ..Default::default()
        })
        .unwrap();
        let fresh = AgentCoreConfig {
            tools: vec![
                serde_json::json!({"type":"function","function":{"name":"fresh.read","parameters":{"type":"object"}}}),
            ],
            ..runtime.config.clone()
        };
        let idle = runtime.checkpoint(CheckpointReason::Manual);
        assert!(AgentRuntime::restore(fresh.clone(), idle.clone()).is_ok());
        for delivery in [
            crate::agent::AgentInputDelivery::CurrentTurn,
            crate::agent::AgentInputDelivery::NextTurn,
        ] {
            let mut queued = idle.clone();
            queued
                .state
                .execution
                .input_queue
                .enqueue(
                    delivery,
                    [crate::agent::AgentItem::user_message(
                        crate::agent::ItemId::from_string("queued".into()),
                        "Queued request",
                    )],
                )
                .unwrap();
            assert!(!queued.state.permits_new_turn_execution_configuration());
            assert!(AgentRuntime::restore(fresh.clone(), queued).is_err());
        }
    }

    #[test]
    fn host_must_explicitly_declare_model_capabilities() {
        let config = AgentCoreConfig {
            session_id: Some(SessionId::from_string("stream-default".to_string())),
            model: Some("deepseek-v4-flash".to_string()),
            ..AgentCoreConfig::default()
        };
        assert!(AgentRuntime::try_new(config.clone()).err().unwrap().contains("model capability profile"));
        for id in ["", " ", " valid", "valid "] {
            let mut invalid = config.clone();
            invalid.turn_identity.model_capability.profile_id = id.into();
            invalid.turn_identity.model_capability.revision = 1;
            assert!(AgentRuntime::try_new(invalid).is_err());
        }
        let mut declared = config;
        declared.turn_identity.model_capability.profile_id = "text-only-host".into();
        assert!(AgentRuntime::try_new(declared.clone()).is_err(), "zero revision is not a declaration");
        declared.turn_identity.model_capability.revision = 1;
        let runtime = AgentRuntime::try_new(declared.clone()).unwrap();
        let profile = &runtime.session.configuration.turn_identity.model_capability;
        assert!(!profile.native_tool_calls);
        assert!(!profile.vision_input);
        assert!(!profile.stream_tool_calls);
        assert_eq!(profile, &declared.turn_identity.model_capability);
        let checkpoint = runtime.checkpoint(CheckpointReason::Manual);
        assert!(AgentRuntime::restore(declared.clone(), checkpoint.clone()).is_ok());
        let mut invalid = checkpoint;
        invalid.state.configuration.turn_identity.model_capability.profile_id.clear();
        assert!(AgentRuntime::restore(declared, invalid).is_err());
    }

    #[test]
    fn invalid_model_profile_patch_is_atomic() {
        let mut runtime = AgentRuntime::try_new(AgentCoreConfig {
            session_id: Some(SessionId::from_string("model-profile-patch".into())),
            turn_identity: crate::agent::model_capability::test_host_identity(),
            ..Default::default()
        }).unwrap();
        let before = serde_json::to_value(runtime.checkpoint(CheckpointReason::Manual)).unwrap();
        assert!(runtime.reduce(AgentSubmission::UpdateSessionConfiguration {
            patch: crate::agent::AgentSessionConfigurationPatch {
                turn_identity: Some(Default::default()),
                ..Default::default()
            },
        }, 0).is_err());
        assert_eq!(before, serde_json::to_value(runtime.checkpoint(CheckpointReason::Manual)).unwrap());
    }
}
