// SPDX-License-Identifier: Apache-2.0
//! Atomic local, internal, maintenance, and remote turn admission transactions.

use super::*;

const INTERNAL_ASSISTANT_TARGET_SQL: &str = "SELECT user_message.id, user_message.ordinal,
            assistant_message.id, assistant_message.ordinal, assistant_message.run_id
     FROM runs AS owner_run
     JOIN messages AS user_message ON user_message.id=owner_run.user_message_id
     JOIN messages AS assistant_message ON assistant_message.id=owner_run.assistant_message_id
     WHERE owner_run.conversation_id=?1
       AND user_message.conversation_id=?1 AND user_message.role='user'
       AND assistant_message.conversation_id=?1 AND assistant_message.role='assistant'
     ORDER BY user_message.ordinal DESC, owner_run.created_at_ms ASC
     LIMIT 1";

impl NolaStore {
    pub fn internal_assistant_target(
        &self,
        conversation_id: &str,
    ) -> StoreResult<Option<nola_store_port::assistant_ownership::AssistantOwner>> {
        require_id("conversation_id", conversation_id)?;
        Ok(self
            .connection
            .query_row(INTERNAL_ASSISTANT_TARGET_SQL, [conversation_id], |row| {
                Ok(nola_store_port::assistant_ownership::AssistantOwner {
                    conversation_id: conversation_id.to_owned(),
                    assistant_message_id: row.get(2)?,
                    run_id: row.get(4)?,
                })
            })
            .optional()?)
    }

    pub fn begin_turn(&mut self, input: &BeginTurnInput) -> StoreResult<BeginTurnResult> {
        self.begin_turn_with_successor(input, None)
    }

    pub fn begin_successor_turn(&mut self, input: &BeginSuccessorTurnInput) -> StoreResult<BeginTurnResult> {
        self.begin_turn_with_successor(&input.turn, Some(input))
    }

    fn begin_turn_with_successor(&mut self, input: &BeginTurnInput, successor: Option<&BeginSuccessorTurnInput>) -> StoreResult<BeginTurnResult> {
        require_id("conversation_id", &input.conversation_id)?;
        require_id("user_message_id", &input.user_message_id)?;
        require_id("assistant_message_id", &input.assistant_message_id)?;
        require_id("run_id", &input.run_id)?;
        require_id("user_part_id", &input.user_part_id)?;
        require_id("assistant_part_id", &input.assistant_part_id)?;
        if input.user_text.trim().is_empty() {
            return Err(StoreError::Invariant(
                "user_text must not be empty".to_string(),
            ));
        }
        if let Some(content_json) = &input.user_content_json {
            validate_json("user_content_json", content_json)?;
        }
        match input.user_part_kind.as_str() {
            "text" => {},
            "ui_action" => {
                let raw = input.user_content_json.as_deref().ok_or_else(|| StoreError::Invariant("UI action body is required".into()))?;
                let part: nola_domain::MessagePartView = serde_json::from_str(raw)?;
                let nola_domain::MessagePartView::UiAction { part_id, surface, summary, .. } = &part else {
                    return Err(StoreError::Invariant("UI action Part kind mismatch".into()));
                };
                if part_id.0 != input.user_part_id || surface.identity.owner.conversation_id != input.conversation_id || summary != &input.user_text {
                    return Err(StoreError::Invariant("UI action Part owner or summary mismatch".into()));
                }
                part.validate_public_semantics(&nola_domain::MessageId(input.user_message_id.clone()))
                    .map_err(|error| StoreError::Invariant(error.to_string()))?;
            },
            _ => return Err(StoreError::Invariant("Unsupported admitted user Part kind".into())),
        }
        for part in &input.additional_user_parts {
            require_id("additional_user_part.id", &part.id)?;
            if let Some(content_json) = &part.content_json {
                validate_json("additional_user_part.content_json", content_json)?;
            }
        }
        validate_json("checkpoint_json", &input.checkpoint_json)?;
        validate_json("invocation_config_json", &input.invocation_config_json)?;
        require_id("invocation_config_digest", &input.invocation_config_digest)?;

        let transaction = self.write_transaction()?;
        crate::agent_room_admission::require_active_context(&transaction, &input.conversation_id)?;
        crate::edited_resend::validate_prepared_admission(&transaction, input)?;
        if successor.is_none() {
            let configuration_revision: Option<i64> = transaction.query_row(
                "SELECT configuration_revision FROM conversations WHERE id=?1", [&input.conversation_id], |row| row.get(0)
            ).optional()?;
            if configuration_revision != input.expected_configuration_revision {
                return Err(StoreError::Conflict("conversation configuration changed before turn admission".into()));
            }
            let parent: Option<String> = transaction.query_row(
                "SELECT m.id FROM messages m WHERE m.conversation_id=?1
                 AND COALESCE((SELECT e.visible FROM message_projection_events e JOIN conversation_projection_operations o ON o.operation_id=e.operation_id
                    WHERE e.conversation_id=m.conversation_id AND e.message_id=m.id ORDER BY o.resulting_revision DESC LIMIT 1),1)=1
                 ORDER BY m.ordinal DESC LIMIT 1", [&input.conversation_id], |row| row.get(0)
            ).optional()?;
            if parent != input.expected_parent_message_id {
                return Err(StoreError::Conflict("conversation parent message changed before turn admission".into()));
            }
        }
        let automation_bound: bool = transaction.query_row(
            "SELECT EXISTS(SELECT 1 FROM automation_execution_bindings WHERE conversation_id=?1)",
            [&input.conversation_id], |row|row.get(0))?;
        if automation_bound {
            return Err(StoreError::Conflict("automation occurrence owns exactly one Run; use a new occurrence for another execution".into()));
        }
        if let Some(successor) = successor {
            if let Some(result) = crate::successor_admission::validate(&transaction, successor)? {
                transaction.commit()?;
                return Ok(result);
            }
        }
        // A queued input's independent Run settles the same authenticated
        // submission receipt as an immediate Run. Local-only submissions have
        // no receipt; existing receipts must match before any admission write.
        crate::remote_submission::gate_source(&transaction, input)?;
        transaction.execute(
            "INSERT INTO conversations(id, title, revision, created_at_ms, updated_at_ms)
             VALUES (?1, ?2, 1, ?3, ?3)
             ON CONFLICT(id) DO UPDATE SET
               title = CASE
                   WHEN conversations.title = '' OR conversations.title = conversations.id
                   THEN excluded.title ELSE conversations.title END,
               revision = conversations.revision + 1,
               updated_at_ms = excluded.updated_at_ms",
            params![
                input.conversation_id,
                input.conversation_title,
                input.created_at_ms
            ],
        )?;

        let mut new_successor_messages = Vec::new();
        let next_ordinal: i64 = if let Some(successor) = successor {
            new_successor_messages = crate::successor_admission::ensure_users(&transaction, successor)?;
            transaction.query_row("SELECT ordinal FROM messages WHERE id=?1 AND conversation_id=?2",
                params![input.user_message_id,input.conversation_id], |row| row.get(0))?
        } else { transaction.query_row(
            "SELECT COALESCE(MAX(ordinal), 0) + 1 FROM messages WHERE conversation_id = ?1",
            params![input.conversation_id],
            |row| row.get(0),
        )? };
        let assistant_ordinal = if successor.is_some() {
            transaction.query_row("SELECT COALESCE(MAX(ordinal),0)+1 FROM messages WHERE conversation_id=?1",
                [&input.conversation_id], |row| row.get(0))?
        } else { next_ordinal + 1 };

        if successor.is_none() {
        transaction.execute(
            "INSERT INTO messages(
                id, conversation_id, run_id, role, status, ordinal, created_at_ms
             ) VALUES (?1, ?2, ?3, 'user', 'completed', ?4, ?5)",
            params![
                input.user_message_id,
                input.conversation_id,
                input.run_id,
                next_ordinal,
                input.created_at_ms
            ],
        )?;
        transaction.execute(
            "INSERT INTO message_parts(
                id, message_id, kind, ordinal, state, content_text, content_json,
                created_at_ms, updated_at_ms
             ) VALUES (?1, ?2, ?6, 1, 'completed', ?3, ?4, ?5, ?5)",
            params![
                input.user_part_id,
                input.user_message_id,
                input.user_text,
                input.user_content_json,
                input.created_at_ms,
                input.user_part_kind
            ],
        )?;
        for part in &input.additional_user_parts {
            transaction.execute(
                "INSERT INTO message_parts(
                    id, message_id, kind, ordinal, state, content_text, content_json,
                    content_digest, created_at_ms, updated_at_ms
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
                params![
                    part.id,
                    input.user_message_id,
                    part.kind,
                    part.ordinal,
                    part.state,
                    part.content_text,
                    part.content_json,
                    part.content_digest,
                    part.created_at_ms,
                    part.updated_at_ms
                ],
            )?;
        }
        }
        transaction.execute(
            "INSERT INTO messages(
                id, conversation_id, run_id, role, status, ordinal, created_at_ms
             ) VALUES (?1, ?2, ?3, 'assistant', 'queued', ?4, ?5)",
            params![
                input.assistant_message_id,
                input.conversation_id,
                input.run_id,
                assistant_ordinal,
                input.created_at_ms
            ],
        )?;
        transaction.execute(
            "INSERT INTO message_parts(
                id, message_id, kind, ordinal, state, content_text, created_at_ms, updated_at_ms
             ) VALUES (?1, ?2, 'text', 1, 'open', '', ?3, ?3)",
            params![
                input.assistant_part_id,
                input.assistant_message_id,
                input.created_at_ms
            ],
        )?;
        transaction.execute(
            "INSERT INTO runs(
                id, conversation_id, kind, user_message_id, assistant_message_id,
                context_checkpoint_id, invocation_config_digest,
                status, next_event_sequence, created_at_ms, updated_at_ms
             ) VALUES (?1, ?2, 'conversation', ?3, ?4, ?5, ?6, 'queued', 1, ?7, ?7)",
            params![
                input.run_id,
                input.conversation_id,
                input.user_message_id,
                input.assistant_message_id,
                input.context_checkpoint_id,
                input.invocation_config_digest,
                input.created_at_ms
            ],
        )?;
        transaction.execute(
            "INSERT INTO run_invocation_payloads(run_id,config_json) VALUES(?1,?2)",
            params![input.run_id,input.invocation_config_json],
        )?;
        if successor.is_none() {
            crate::edited_resend::record_plain_send(&transaction, input)?;
            crate::expert_message::record_child_send(&transaction, input)?;
            crate::edited_resend::mark_prepared_admitted(&transaction, input)?;
        }
        transaction.execute(
            "INSERT INTO canonical_run_initial_parts(run_id,part_id) VALUES(?1,?2)",
            params![input.run_id, input.assistant_part_id],
        )?;
        checkpoint_store::write_baseline(&transaction, &input.conversation_id,
            input.checkpoint_revision, input.checkpoint_event_cursor,
            &input.checkpoint_json, input.created_at_ms)?;
        checkpoint_store::bind_initial_run_baseline(&transaction, &input.conversation_id,
            &input.run_id, input.checkpoint_revision)?;
        apply_initial_assistant_parts(
            &transaction,
            &input.conversation_id,
            &input.run_id,
            &input.assistant_message_id,
            &input.initial_assistant_part_mutations,
            &input.assistant_turn_bindings,
        )?;
        let foundational_bindings: Vec<_> = input.assistant_turn_bindings.iter()
            .filter(|binding| matches!(binding, nola_domain::AssistantTurnBinding::Admission { .. }
                | nola_domain::AssistantTurnBinding::TurnStarted { .. } | nola_domain::AssistantTurnBinding::Part { .. }))
            .cloned().collect();
        assistant_turn_ownership::apply(&transaction, &input.conversation_id, &input.run_id,
            Some(&input.assistant_message_id), true, &foundational_bindings)?;
        apply_memory_usage_entries(&transaction, &input.memory_usage_entries)?;
        model_attempts::apply(&transaction, &input.run_id, &input.model_attempt_mutations)?;
        delegated_invocations::apply(
            &transaction,
            &input.run_id,
            &input.delegated_invocation_mutations,
        )?;
        delegated_permissions::apply(
            &transaction,
            &input.run_id,
            &input.delegated_permission_mutations,
        )?;
        model_attempts::append_initial_events(
            &transaction,
            &input.run_id,
            &input.model_attempt_mutations,
            input.created_at_ms,
        )?;
        delegated_invocations::append_initial_events(
            &transaction,
            &input.run_id,
            &input.delegated_invocation_mutations,
            input.created_at_ms,
        )?;
        delegated_permissions::append_initial_events(
            &transaction,
            &input.run_id,
            &input.delegated_permission_mutations,
            input.created_at_ms,
        )?;
        crate::initial_run_events::append(&transaction, &input.run_id, &input.initial_events, input.created_at_ms, input.checkpoint_event_cursor, input.checkpoint_revision)?;
        assistant_turn_ownership::apply(
            &transaction,
            &input.conversation_id,
            &input.run_id,
            Some(&input.assistant_message_id),
            true,
            &input.assistant_turn_bindings,
        )?;

        insert_outbox_effects(
            &transaction,
            &input.run_id,
            &input.outbox_effects,
            input.created_at_ms,
        )?;
        let conversation_revision: i64 = transaction.query_row(
            "SELECT revision FROM conversations WHERE id=?1",
            params![input.conversation_id],
            |row| row.get(0),
        )?;
        if successor.is_some() {
            new_successor_messages.push(input.assistant_message_id.clone());
            conversation_transition_sync::enqueue_admitted_messages(&transaction, &input.conversation_id,
                conversation_revision, &new_successor_messages, input.created_at_ms)?;
        } else if conversation_append_sync::needs_initial_turn_publication(
            &transaction,
            &input.conversation_id,
            next_ordinal,
        )? {
            // First publication establishes the base and contains only this turn.
            publish_conversation_content(
                &transaction,
                &input.conversation_id,
                input.created_at_ms,
            )?;
        } else {
            conversation_append_sync::enqueue_turn(
                &transaction,
                input,
                conversation_revision,
                next_ordinal,
            )?;
        }
        crate::run_message_admission::publish(&transaction, &input.conversation_id, &input.run_id)?;
        crate::remote_submission::commit_source_admission(&transaction, input)?;
        let result = BeginTurnResult {
            conversation_id: input.conversation_id.clone(),
            user_message_id: input.user_message_id.clone(),
            assistant_message_id: input.assistant_message_id.clone(),
            run_id: input.run_id.clone(),
            user_ordinal: next_ordinal,
            assistant_ordinal,
        };
        if let Some(successor) = successor {
            crate::successor_admission::record(&transaction, successor, &result)?;
        }
        transaction.commit()?;

        if self.has_write_unit_of_work() {
            self.applied_admission_stamps
                .insert(input.run_id.clone(), input.checkpoint_revision);
        }
        Ok(result)
    }

    pub fn begin_internal_turn(
        &mut self,
        input: &BeginInternalTurnInput,
    ) -> StoreResult<BeginTurnResult> {
        require_id("conversation_id", &input.conversation_id)?;
        require_id("assistant_message_id", &input.assistant_message_id)?;
        require_id("assistant_part_id", &input.assistant_part_id)?;
        require_id("run_id", &input.run_id)?;
        require_id("expected_previous_run_id", &input.expected_previous_run_id)?;
        validate_json("checkpoint_json", &input.checkpoint_json)?;
        validate_json("invocation_config_json", &input.invocation_config_json)?;
        require_id("invocation_config_digest", &input.invocation_config_digest)?;

        let transaction = self.write_transaction()?;
        let predecessor_ready: bool = transaction.query_row(
            "SELECT EXISTS(SELECT 1 FROM runs r
                WHERE r.id=?2 AND r.conversation_id=?1 AND r.kind='conversation'
                  AND r.status='sealed' AND r.completed_at_ms IS NOT NULL
                  AND (EXISTS(SELECT 1 FROM assistant_run_terminal_seals s WHERE s.run_id=r.id)
                    OR EXISTS(SELECT 1 FROM remote_assistant_run_terminal_stages s
                        WHERE s.run_id=r.id AND s.committed=1)))
              AND NOT EXISTS(SELECT 1 FROM runs WHERE conversation_id=?1 AND status<>'sealed')",
            params![input.conversation_id, input.expected_previous_run_id],
            |row| row.get(0),
        )?;
        if !predecessor_ready {
            return Err(StoreError::Conflict(
                "internal successor requires its exact sealed predecessor and no active Run".into(),
            ));
        }
        let (
            user_message_id,
            user_ordinal,
            assistant_message_id,
            assistant_ordinal,
            previous_run_id,
        ): (String, i64, String, i64, String) = transaction
            .query_row(
                INTERNAL_ASSISTANT_TARGET_SQL,
                params![input.conversation_id],
                |row| {
                    Ok((
                        row.get(0)?,
                        row.get(1)?,
                        row.get(2)?,
                        row.get(3)?,
                        row.get(4)?,
                    ))
                },
            )
            .map_err(|error| {
                StoreError::Invariant(format!(
                    "internal conversation Turn requires an existing visible Turn: {error}"
                ))
            })?;
        let actual_checkpoint_revision: Option<i64> = transaction
            .query_row(
                "SELECT revision FROM core_checkpoints WHERE conversation_id=?1",
                [&input.conversation_id],
                |row| row.get(0),
            )
            .optional()?;
        if assistant_message_id != input.assistant_message_id
            || previous_run_id != input.expected_previous_run_id
            || actual_checkpoint_revision != input.expected_checkpoint_revision
        {
            return Err(StoreError::Conflict(
                "internal conversation Turn target or checkpoint changed before admission".into(),
            ));
        }
        transaction.execute(
            "UPDATE conversations
             SET revision=revision+1, updated_at_ms=?2 WHERE id=?1",
            params![input.conversation_id, input.created_at_ms],
        )?;
        crate::canonical_message_ownership::record_resumption(
            &transaction,
            &input.conversation_id,
            &assistant_message_id,
            &previous_run_id,
            &input.run_id,
        )?;
        let updated_assistant = transaction.execute(
            "UPDATE messages
             SET run_id=?1, status='queued', completed_at_ms=NULL
             WHERE id=?2 AND conversation_id=?3 AND role='assistant'",
            params![input.run_id, assistant_message_id, input.conversation_id],
        )?;
        if updated_assistant != 1 {
            return Err(StoreError::Invariant(
                "internal conversation Turn lost its assistant owner".to_string(),
            ));
        }
        transaction.execute(
            "INSERT INTO runs(
                id, conversation_id, kind, user_message_id, assistant_message_id,
                context_checkpoint_id, invocation_config_digest,
                status, next_event_sequence, created_at_ms, updated_at_ms
             ) VALUES (?1, ?2, 'conversation', ?3, ?4, ?5, ?6, 'queued', 1, ?7, ?7)",
            params![
                input.run_id,
                input.conversation_id,
                user_message_id,
                assistant_message_id,
                input.context_checkpoint_id,
                input.invocation_config_digest,
                input.created_at_ms
            ],
        )?;
        transaction.execute(
            "INSERT INTO run_invocation_payloads(run_id,config_json) VALUES(?1,?2)",
            params![input.run_id,input.invocation_config_json],
        )?;
        checkpoint_store::write_baseline(&transaction, &input.conversation_id,
            input.checkpoint_revision, input.checkpoint_event_cursor,
            &input.checkpoint_json, input.created_at_ms)?;
        checkpoint_store::bind_initial_run_baseline(&transaction, &input.conversation_id,
            &input.run_id, input.checkpoint_revision)?;
        apply_initial_assistant_parts(
            &transaction,
            &input.conversation_id,
            &input.run_id,
            &assistant_message_id,
            &input.initial_assistant_part_mutations,
            &input.assistant_turn_bindings,
        )?;
        let foundational_bindings: Vec<_> = input.assistant_turn_bindings.iter()
            .filter(|binding| matches!(binding, nola_domain::AssistantTurnBinding::Admission { .. }
                | nola_domain::AssistantTurnBinding::TurnStarted { .. } | nola_domain::AssistantTurnBinding::Part { .. }))
            .cloned().collect();
        assistant_turn_ownership::apply(&transaction, &input.conversation_id, &input.run_id,
            Some(&assistant_message_id), true, &foundational_bindings)?;
        apply_memory_usage_entries(&transaction, &input.memory_usage_entries)?;
        model_attempts::apply(&transaction, &input.run_id, &input.model_attempt_mutations)?;
        delegated_invocations::apply(
            &transaction,
            &input.run_id,
            &input.delegated_invocation_mutations,
        )?;
        delegated_permissions::apply(
            &transaction,
            &input.run_id,
            &input.delegated_permission_mutations,
        )?;
        model_attempts::append_initial_events(
            &transaction,
            &input.run_id,
            &input.model_attempt_mutations,
            input.created_at_ms,
        )?;
        delegated_invocations::append_initial_events(
            &transaction,
            &input.run_id,
            &input.delegated_invocation_mutations,
            input.created_at_ms,
        )?;
        delegated_permissions::append_initial_events(
            &transaction,
            &input.run_id,
            &input.delegated_permission_mutations,
            input.created_at_ms,
        )?;
        crate::initial_run_events::append(&transaction, &input.run_id, &input.initial_events, input.created_at_ms, input.checkpoint_event_cursor, input.checkpoint_revision)?;
        assistant_turn_ownership::apply(
            &transaction,
            &input.conversation_id,
            &input.run_id,
            Some(&assistant_message_id),
            true,
            &input.assistant_turn_bindings,
        )?;

        insert_outbox_effects(
            &transaction,
            &input.run_id,
            &input.outbox_effects,
            input.created_at_ms,
        )?;
        let conversation_revision: i64 = transaction.query_row(
            "SELECT revision FROM conversations WHERE id=?1",
            params![input.conversation_id],
            |row| row.get(0),
        )?;
        conversation_append_sync::enqueue_resume(
            &transaction,
            input,
            conversation_revision,
            &assistant_message_id,
            &previous_run_id,
        )?;
        crate::run_message_admission::publish(&transaction, &input.conversation_id, &input.run_id)?;
        transaction.commit()?;

        if self.has_write_unit_of_work() {
            self.applied_admission_stamps
                .insert(input.run_id.clone(), input.checkpoint_revision);
        }
        Ok(BeginTurnResult {
            conversation_id: input.conversation_id.clone(),
            user_message_id,
            assistant_message_id,
            run_id: input.run_id.clone(),
            user_ordinal,
            assistant_ordinal,
        })
    }

    pub fn begin_maintenance_run(
        &mut self,
        input: &BeginMaintenanceRunInput,
    ) -> StoreResult<BeginMaintenanceRunResult> {
        require_id("conversation_id", &input.conversation_id)?;
        require_id("run_id", &input.run_id)?;
        validate_json("checkpoint_json", &input.checkpoint_json)?;
        validate_json("invocation_config_json", &input.invocation_config_json)?;
        require_id("invocation_config_digest", &input.invocation_config_digest)?;
        let initial_status = if input.outbox_effects.is_empty() {
            let checkpoint: serde_json::Value = serde_json::from_str(&input.checkpoint_json)?;
            let turn = &checkpoint["state"]["execution"]["active_turn"];
            let no_effects = checkpoint.pointer("/state/host/pending_host_effects/effects")
                .and_then(serde_json::Value::as_object).is_some_and(|effects| effects.is_empty());
            let mut noop_cursor = None;
            let mut terminal_cursor = None;
            for event in &input.initial_events {
                let payload: serde_json::Value = serde_json::from_str(&event.payload_json)?;
                if event.event_kind == "compaction_completed"
                    && payload["summary"] == "" && payload["replaced_item_count"] == 0
                    && payload["source_item_count"] == 0 && payload["original_token_count"] == 0
                    && payload["compaction_item_id"].is_null() && payload["source_fingerprint"].is_null()
                    // CompactionCompleted is a session-level event; the later
                    // TurnCompleted and exact checkpoint bind its owning Run.
                    && event.core_turn_id.is_none() && payload["reason"] == "manual" {
                    if noop_cursor.replace(event.core_cursor).is_some() {
                        return Err(StoreError::Invariant("duplicate maintenance no-op evidence".into()));
                    }
                }
                if event.event_kind == "turn_completed" && payload["turn_id"] == input.run_id
                    && event.core_turn_id.as_deref() == Some(input.run_id.as_str()) {
                    if terminal_cursor.replace(event.core_cursor).is_some() {
                        return Err(StoreError::Invariant("duplicate maintenance terminal evidence".into()));
                    }
                }
            }
            if turn["turn_id"] != input.run_id || turn["status"] != "completed"
                || turn["context"]["input_origin"] != "maintenance" || !no_effects
                || !checkpoint["state"]["execution"]["requirements"]["scope"].is_null()
                || !checkpoint["state"]["execution"]["requirements"]["decision"].is_null()
                || !matches!((noop_cursor,terminal_cursor), (Some(noop),Some(terminal)) if noop < terminal)
                || !input.model_attempt_mutations.is_empty()
                || !input.delegated_invocation_mutations.is_empty()
                || !input.delegated_permission_mutations.is_empty()
            {
                return Err(StoreError::Invariant("maintenance without Host effects requires exact no-op completion evidence".into()));
            }
            "sealed"
        } else { "queued" };
        let transaction = self.write_transaction()?;
        let conversation_exists: bool = transaction.query_row(
            "SELECT EXISTS(SELECT 1 FROM conversations WHERE id = ?1)",
            params![input.conversation_id],
            |row| row.get(0),
        )?;
        if !conversation_exists {
            return Err(StoreError::Invariant(format!(
                "unknown conversation {}",
                input.conversation_id
            )));
        }
        let active_exists: bool = transaction.query_row(
            "SELECT EXISTS(
                SELECT 1 FROM runs
                WHERE conversation_id = ?1
                  AND status NOT IN ('sealed')
            )",
            params![input.conversation_id],
            |row| row.get(0),
        )?;
        if active_exists {
            return Err(StoreError::Conflict(
                "maintenance Run cannot overlap another active Run".to_string(),
            ));
        }
        let stored_revision: Option<i64> = transaction
            .query_row(
                "SELECT revision FROM core_checkpoints WHERE conversation_id = ?1",
                params![input.conversation_id],
                |row| row.get(0),
            )
            .optional()?;
        if stored_revision.is_some_and(|revision| revision >= input.checkpoint_revision) {
            return Err(StoreError::Conflict(
                "maintenance checkpoint revision did not advance".to_string(),
            ));
        }
        transaction.execute(
            "INSERT INTO runs(
                id, conversation_id, kind, user_message_id, assistant_message_id,
                context_checkpoint_id, invocation_config_digest,
                status, next_event_sequence, created_at_ms, updated_at_ms
             ) VALUES (?1, ?2, 'maintenance', NULL, NULL, ?3, ?4, ?6, 1, ?5, ?5)",
            params![
                input.run_id,
                input.conversation_id,
                input.context_checkpoint_id,
                input.invocation_config_digest,
                input.created_at_ms,
                initial_status
            ],
        )?;
        transaction.execute(
            "INSERT INTO run_invocation_payloads(run_id,config_json) VALUES(?1,?2)",
            params![input.run_id,input.invocation_config_json],
        )?;
        checkpoint_store::write_baseline(&transaction, &input.conversation_id,
            input.checkpoint_revision, input.checkpoint_event_cursor,
            &input.checkpoint_json, input.created_at_ms)?;
        checkpoint_store::bind_initial_run_baseline(&transaction, &input.conversation_id,
            &input.run_id, input.checkpoint_revision)?;
        apply_memory_usage_entries(&transaction, &input.memory_usage_entries)?;
        model_attempts::apply(&transaction, &input.run_id, &input.model_attempt_mutations)?;
        delegated_invocations::apply(
            &transaction,
            &input.run_id,
            &input.delegated_invocation_mutations,
        )?;
        delegated_permissions::apply(
            &transaction,
            &input.run_id,
            &input.delegated_permission_mutations,
        )?;
        model_attempts::append_initial_events(
            &transaction,
            &input.run_id,
            &input.model_attempt_mutations,
            input.created_at_ms,
        )?;
        delegated_invocations::append_initial_events(
            &transaction,
            &input.run_id,
            &input.delegated_invocation_mutations,
            input.created_at_ms,
        )?;
        delegated_permissions::append_initial_events(
            &transaction,
            &input.run_id,
            &input.delegated_permission_mutations,
            input.created_at_ms,
        )?;
        crate::initial_run_events::append(&transaction, &input.run_id, &input.initial_events, input.created_at_ms, input.checkpoint_event_cursor, input.checkpoint_revision)?;
        insert_outbox_effects(
            &transaction,
            &input.run_id,
            &input.outbox_effects,
            input.created_at_ms,
        )?;
        transaction.commit()?;
        Ok(BeginMaintenanceRunResult {
            conversation_id: input.conversation_id.clone(),
            run_id: input.run_id.clone(),
        })
    }

    pub(crate) fn admit_remote_submission_replica(
        &mut self,
        input: &AdmittedReplicaTurnInput,
    ) -> StoreResult<BeginTurnResult> {
        require_id("source_host_id", &input.source_host_id)?;
        require_id("conversation_id", &input.conversation_id)?;
        require_id("user_message_id", &input.user_message_id)?;
        require_id("assistant_message_id", &input.assistant_message_id)?;
        require_id("run_id", &input.run_id)?;
        require_id("user_part_id", &input.user_part_id)?;
        require_id("assistant_part_id", &input.assistant_part_id)?;
        let transaction = self.write_transaction()?;
        let active:bool=transaction.query_row("SELECT EXISTS(SELECT 1 FROM runs WHERE conversation_id=?1 AND status NOT IN ('sealed')) OR EXISTS(SELECT 1 FROM remote_submission_intents WHERE conversation_id=?1 AND submission_id<>?2 AND phase IN ('prepared','bound'))",params![input.conversation_id,input.run_id],|r|r.get(0))?;
        if active {
            return Err(StoreError::Conflict(
                "remote turn cannot overlap an active Run".into(),
            ));
        }
        transaction.execute(
            "INSERT INTO conversations(id, title, revision, created_at_ms, updated_at_ms)
             VALUES (?1, ?2, 1, ?3, ?3)
             ON CONFLICT(id) DO UPDATE SET
               title = CASE
                   WHEN conversations.title = '' OR conversations.title = conversations.id
                   THEN excluded.title ELSE conversations.title END,
               revision = conversations.revision + 1,
               updated_at_ms = excluded.updated_at_ms",
            params![
                input.conversation_id,
                input.conversation_title,
                input.created_at_ms
            ],
        )?;
        let user_ordinal: i64 = transaction.query_row(
            "SELECT COALESCE(MAX(ordinal), 0) + 1 FROM messages WHERE conversation_id = ?1",
            params![input.conversation_id],
            |row| row.get(0),
        )?;
        let assistant_ordinal = user_ordinal + 1;
        transaction.execute(
            "INSERT INTO messages(id, conversation_id, run_id, role, status, ordinal, created_at_ms)
             VALUES (?1, ?2, ?3, 'user', 'completed', ?4, ?5)",
            params![input.user_message_id, input.conversation_id, input.run_id, user_ordinal, input.created_at_ms],
        )?;
        transaction.execute(
            "INSERT INTO messages(id, conversation_id, run_id, role, status, ordinal, created_at_ms)
             VALUES (?1, ?2, ?3, 'assistant', 'queued', ?4, ?5)",
            params![input.assistant_message_id, input.conversation_id, input.run_id, assistant_ordinal, input.created_at_ms],
        )?;
        transaction.execute(
            "INSERT INTO message_parts(id, message_id, kind, ordinal, state, content_text, created_at_ms, updated_at_ms)
             VALUES (?1, ?2, 'text', 1, 'open', '', ?3, ?3)",
            params![input.assistant_part_id, input.assistant_message_id, input.created_at_ms],
        )?;
        transaction.execute(
            "INSERT INTO runs(id, conversation_id, kind, user_message_id, assistant_message_id, context_checkpoint_id, status, next_event_sequence, created_at_ms, updated_at_ms)
             VALUES (?1, ?2, 'conversation', ?3, ?4, ?5, 'queued', 1, ?6, ?6)",
            params![input.run_id, input.conversation_id, input.user_message_id, input.assistant_message_id, input.context_checkpoint_id, input.created_at_ms],
        )?;
        transaction.execute(
            "INSERT INTO remote_run_sources(run_id,source_host_id) VALUES(?1,?2)",
            params![input.run_id, input.source_host_id],
        )?;
        transaction.execute(
            "INSERT INTO canonical_run_initial_parts(run_id,part_id) VALUES(?1,?2)",
            params![input.run_id, input.assistant_part_id],
        )?;
        let revision: i64 = transaction.query_row(
            "SELECT revision FROM conversations WHERE id=?1",
            params![input.conversation_id],
            |row| row.get(0),
        )?;
        if conversation_append_sync::needs_initial_turn_publication(
            &transaction,
            &input.conversation_id,
            user_ordinal,
        )? {
            publish_conversation_content(
                &transaction,
                &input.conversation_id,
                input.created_at_ms,
            )?;
        } else {
            conversation_append_sync::enqueue_remote_turn(
                &transaction,
                input,
                revision,
                user_ordinal,
            )?;
        }
        transaction.commit()?;
        Ok(BeginTurnResult {
            conversation_id: input.conversation_id.clone(),
            user_message_id: input.user_message_id.clone(),
            assistant_message_id: input.assistant_message_id.clone(),
            run_id: input.run_id.clone(),
            user_ordinal,
            assistant_ordinal,
        })
    }
}

fn apply_initial_assistant_parts(
    connection: &Connection,
    conversation: &str,
    run: &str,
    message: &str,
    mutations: &[PartMutation],
    bindings: &[nola_domain::AssistantTurnBinding],
) -> StoreResult<()> {
    for mutation in mutations {
        let part = part_mutation_id(mutation);
        let bound = bindings.iter().any(|binding| {
            matches!(binding,
            nola_domain::AssistantTurnBinding::Part { part_id, assistant_message_id, turn_id }
            if part_id == part && assistant_message_id == message
                && bindings.iter().any(|admission| matches!(admission,
                    nola_domain::AssistantTurnBinding::Admission {
                        turn_id: admitted_turn, assistant_message_id: admitted_message,
                    } if admitted_turn == turn_id && admitted_message == message)))
        });
        if !bound {
            return Err(StoreError::Invariant(
                "initial Part mutation lacks exact admitted typed ownership".into(),
            ));
        }
        assistant_turn_ownership::validate_part_write_owner(
            connection, conversation, run, message, part)?;
        if let PartMutation::Stream { operation, .. } = mutation {
            let owner = operation.owner();
            if owner.conversation_id != conversation
                || owner.run_id != run
                || owner.message_id != message
            {
                return Err(StoreError::Invariant(
                    "initial stream owner differs from admission".into(),
                ));
            }
        }
        apply_part_mutation(connection, message, mutation)?;
    }
    Ok(())
}
