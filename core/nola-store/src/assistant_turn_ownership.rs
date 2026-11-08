// SPDX-License-Identifier: Apache-2.0
//! Durable ownership evidence from accepted canonical writer inputs.

use crate::{params, Connection, NolaStore, OptionalExtension, StoreError, StoreResult};
use nola_domain::AssistantTurnBinding;
use nola_store_port::assistant_ownership::{AssistantOwner, AssistantPartOwner};

fn invalid(message: &str) -> StoreError { StoreError::Invariant(message.into()) }

pub(super) fn validate_part_write_owner(
    connection: &Connection, conversation: &str, run: &str, message: &str, part: &str,
) -> StoreResult<()> {
    let conflicting: bool = connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM assistant_turn_part_bindings WHERE part_id=?1
            AND (conversation_id<>?2 OR run_id<>?3 OR message_id<>?4))",
        params![part,conversation,run,message], |row|row.get(0),
    )?;
    if conflicting {
        return Err(StoreError::Conflict("Part mutation belongs to another accepted Run or Message".into()));
    }
    Ok(())
}

pub(super) fn apply(
    connection: &Connection, conversation: &str, run: &str,
    admitted_message: Option<&str>, admission: bool, bindings: &[AssistantTurnBinding],
) -> StoreResult<()> {
    for binding in bindings {
        let is_admission = matches!(binding, AssistantTurnBinding::Admission { .. });
        let (turn, message, part) = match binding {
            AssistantTurnBinding::SegmentStarted { turn_id, assistant_message_id } => {
                let source: Option<i64> = connection.query_row(
                    "SELECT segment.sequence FROM run_events segment WHERE segment.run_id=?1 AND segment.event_kind='assistant_segment_started'
                     AND json_extract(segment.payload_json,'$.turn_id')=?2 AND json_extract(segment.payload_json,'$.assistant_message_id')=?3
                     AND json_array_length(json_extract(segment.payload_json,'$.applications'))>0
                     AND EXISTS(SELECT 1 FROM run_events applied WHERE applied.run_id=segment.run_id AND applied.event_kind='input_applied'
                         AND applied.sequence<segment.sequence AND json_extract(applied.payload_json,'$.turn_id')=?2
                         AND json_extract(applied.payload_json,'$.applications')=json_extract(segment.payload_json,'$.applications'))",
                    params![run,turn_id,assistant_message_id], |row| row.get(0),
                ).optional()?;
                let sequence = source.ok_or_else(||invalid("Segment requires its exact committed applied input frontier"))?;
                let previous: Option<(String,i64)> = connection.query_row(
                    "SELECT turn_id,source_sequence FROM assistant_message_segments WHERE run_id=?1 AND message_id=?2",
                    params![run,assistant_message_id], |row|Ok((row.get(0)?,row.get(1)?)),
                ).optional()?;
                if previous.as_ref().is_some_and(|old| old != &(turn_id.clone(),sequence)) {
                    return Err(StoreError::Conflict("Assistant segment frontier is immutable".into()));
                }
                connection.execute("INSERT OR IGNORE INTO assistant_message_segments(run_id,turn_id,message_id,source_sequence) VALUES(?1,?2,?3,?4)",
                    params![run,turn_id,assistant_message_id,sequence])?;
                (turn_id, assistant_message_id, None)
            }
            AssistantTurnBinding::Activity { turn_id, assistant_message_id, activity_id, activity_kind } => {
                if !matches!(activity_kind.as_str(),"task"|"skill") { return Err(invalid("Invalid activity owner kind")); }
                let source: Option<String> = connection.query_row(
                    "SELECT json_extract(payload_json,'$.assistant_message_id') FROM run_events WHERE run_id=?1
                     AND event_kind='assistant_activity_bound' AND json_extract(payload_json,'$.turn_id')=?2
                     AND json_extract(payload_json,'$.activity_id')=?3 AND json_extract(payload_json,'$.activity_kind')=?4 LIMIT 1",
                    params![run,turn_id,activity_id,activity_kind], |row|row.get(0),
                ).optional()?;
                let source=source.ok_or_else(||invalid("Activity requires exact Core birth evidence"))?;
                if source != *assistant_message_id && !(admitted_message == Some(assistant_message_id.as_str()) && (admission || source == format!("{run}:assistant"))) {
                    return Err(invalid("Activity birth differs from message owner"));
                }
                let previous:Option<(String,String,String)> = connection.query_row(
                    "SELECT conversation_id,turn_id,message_id FROM assistant_activity_message_bindings WHERE run_id=?1 AND activity_kind=?2 AND activity_id=?3",
                    params![run,activity_kind,activity_id], |row|Ok((row.get(0)?,row.get(1)?,row.get(2)?)),
                ).optional()?;
                if previous.as_ref().is_some_and(|old| old != &(conversation.into(),turn_id.clone(),assistant_message_id.clone())) {
                    return Err(StoreError::Conflict("Activity presentation owner is immutable".into()));
                }
                connection.execute("INSERT OR IGNORE INTO assistant_activity_message_bindings(conversation_id,run_id,turn_id,activity_kind,activity_id,message_id)
                    VALUES(?1,?2,?3,?4,?5,?6)",params![conversation,run,turn_id,activity_kind,activity_id,assistant_message_id])?;
                (turn_id,assistant_message_id,None)
            }
            AssistantTurnBinding::Effect { turn_id, assistant_message_id, effect_id } => {
                let source: Option<String> = connection.query_row(
                    "SELECT json_extract(payload_json,'$.assistant_message_id') FROM run_events WHERE run_id=?1
                     AND event_kind='assistant_effect_bound' AND json_extract(payload_json,'$.turn_id')=?2
                     AND json_extract(payload_json,'$.effect_id')=?3 LIMIT 1",
                    params![run,turn_id,effect_id], |row|row.get(0),
                ).optional()?;
                let source = source.ok_or_else(||invalid("Effect binding requires its exact Core declaration"))?;
                if source != *assistant_message_id && !(admitted_message == Some(assistant_message_id.as_str())
                    && (admission || source == format!("{run}:assistant"))) {
                    return Err(invalid("Effect declaration differs from accepted message owner"));
                }
                let previous: Option<(String,String,String,String)> = connection.query_row(
                    "SELECT conversation_id,run_id,turn_id,message_id FROM assistant_effect_message_bindings WHERE effect_id=?1",
                    [effect_id], |row|Ok((row.get(0)?,row.get(1)?,row.get(2)?,row.get(3)?)),
                ).optional()?;
                if previous.as_ref().is_some_and(|old| old != &(conversation.into(),run.into(),turn_id.clone(),assistant_message_id.clone())) {
                    return Err(StoreError::Conflict("Effect presentation owner is immutable".into()));
                }
                // Message membership is checked below in the same transaction.
                connection.execute("INSERT OR IGNORE INTO assistant_effect_message_bindings(conversation_id,run_id,effect_id,turn_id,message_id)
                    VALUES(?1,?2,?3,?4,?5)", params![conversation,run,effect_id,turn_id,assistant_message_id])?;
                (turn_id, assistant_message_id, None)
            }
            AssistantTurnBinding::TurnStarted { turn_id, assistant_message_id } => {
                let accepted: bool = connection.query_row(
                    "SELECT EXISTS(SELECT 1 FROM run_events WHERE run_id=?1
                     AND event_kind='turn_started'
                     AND json_extract(payload_json,'$.turn_id')=?2
                     AND json_extract(payload_json,'$.assistant_message_id')=?3)",
                    params![run,turn_id,assistant_message_id], |row| row.get(0),
                )?;
                if !accepted {
                    return Err(invalid("Turn birth binding requires its exact committed Core event"));
                }
                (turn_id, assistant_message_id, None)
            }
            AssistantTurnBinding::Admission { turn_id, assistant_message_id } => {
                if !admission || admitted_message != Some(assistant_message_id.as_str()) {
                    return Err(invalid("Turn admission binding requires the exact admitted assistant"));
                }
                (turn_id, assistant_message_id, None)
            }
            AssistantTurnBinding::Part { turn_id, assistant_message_id, part_id } => {
                crate::require_id("assistant_binding.part_id", part_id)?;
                (turn_id, assistant_message_id, Some(part_id))
            }
        };
        crate::require_id("assistant_binding.turn_id", turn)?;
        crate::require_id("assistant_binding.message_id", message)?;
        let canonical: bool = connection.query_row(
            "SELECT EXISTS(SELECT 1 FROM messages m JOIN runs r ON r.id=m.run_id
             WHERE m.id=?1 AND m.conversation_id=?2 AND m.role='assistant' AND m.run_id=?3
             AND r.conversation_id=?2 AND r.kind='conversation'
             AND NOT EXISTS(SELECT 1 FROM remote_run_sources remote WHERE remote.run_id=r.id))",
            params![message,conversation,run], |row|row.get(0),
        )?;
        if !canonical { return Err(invalid("Turn binding lacks current local assistant membership")); }
        if let Some(part) = part {
            let member: bool = connection.query_row(
                "SELECT EXISTS(SELECT 1 FROM message_parts WHERE id=?1 AND message_id=?2)",
                params![part,message], |row|row.get(0),
            )?;
            if !member { return Err(invalid("Turn Part binding does not match canonical membership")); }
            let conflicting: bool = connection.query_row(
                "SELECT EXISTS(SELECT 1 FROM assistant_turn_part_bindings WHERE part_id=?1
                    AND (conversation_id<>?2 OR run_id<>?3 OR turn_id<>?4 OR message_id<>?5))",
                params![part,conversation,run,turn,message], |row|row.get(0),
            )?;
            if conflicting {
                return Err(StoreError::Conflict("Turn Part binding is immutable".into()));
            }
            connection.execute(
                "INSERT OR IGNORE INTO assistant_turn_part_bindings(conversation_id,run_id,part_id,turn_id,message_id) VALUES(?1,?2,?3,?4,?5)",
                params![conversation,run,part,turn,message],
            )?;
        } else if is_admission {
            let previous: Option<(String,String)> = connection.query_row(
                "SELECT turn_id,message_id FROM assistant_turn_message_bindings WHERE run_id=?1 AND admission=1",
                [run], |row|Ok((row.get(0)?,row.get(1)?)),
            ).optional()?;
            if previous.as_ref().is_some_and(|(old_turn,old_message)| old_turn != turn || old_message != message) {
                return Err(StoreError::Conflict("Turn admission binding is immutable".into()));
            }
        }
        // This is a relation set. A reused non-admission Turn can have multiple
        // real Messages; retain ambiguity rather than rejecting ordinary text.
        connection.execute(
            "INSERT INTO assistant_turn_message_bindings(conversation_id,run_id,turn_id,message_id,admission) VALUES(?1,?2,?3,?4,?5)
             ON CONFLICT(run_id,turn_id,message_id) DO UPDATE SET admission=MAX(admission,excluded.admission)",
            params![conversation,run,turn,message,is_admission],
        )?;
    }
    Ok(())
}

fn validate_scope(connection: &Connection, conversation: &str, run: &str) -> StoreResult<bool> {
    crate::require_id("assistant_owner.conversation_id", conversation)?;
    crate::require_id("assistant_owner.run_id", run)?;
    let scope: Option<(String,String,bool)> = connection.query_row(
        "SELECT conversation_id,kind,EXISTS(SELECT 1 FROM remote_run_sources remote WHERE remote.run_id=r.id) FROM runs r WHERE id=?1",
        [run], |row|Ok((row.get(0)?,row.get(1)?,row.get(2)?)),
    ).optional()?;
    match scope {
        None => Ok(false),
        Some((actual,kind,remote)) if actual == conversation && kind == "conversation" && !remote => Ok(true),
        Some(_) => Err(invalid("assistant owner is outside the exact local Run scope")),
    }
}

impl NolaStore {
    pub fn assistant_owner_for_core_turn(&self, conversation: &str, turn: &str) -> StoreResult<Option<AssistantOwner>> {
        crate::require_id("assistant_owner.conversation_id", conversation)?;
        crate::require_id("assistant_owner.turn_id", turn)?;
        let mut query = self.connection.prepare(
            "SELECT DISTINCT run_id FROM assistant_turn_message_bindings
             WHERE conversation_id=?1 AND turn_id=?2 ORDER BY run_id LIMIT 2",
        )?;
        let runs = query.query_map(params![conversation,turn], |row|row.get::<_,String>(0))?
            .collect::<Result<Vec<_>,_>>()?;
        match runs.as_slice() {
            [] => Ok(None),
            [run] => self.assistant_owner_for_turn(conversation,run,turn),
            _ => Err(StoreError::Conflict("Core Turn has ambiguous persisted Run ownership".into())),
        }
    }

    pub fn assistant_owner_for_turn(&self, conversation: &str, run: &str, turn: &str) -> StoreResult<Option<AssistantOwner>> {
        crate::require_id("assistant_owner.turn_id", turn)?;
        if !validate_scope(&self.connection, conversation, run)? { return Ok(None); }
        let mut query = self.connection.prepare(
            "SELECT b.message_id,b.admission,m.conversation_id,m.role
             FROM assistant_turn_message_bindings b JOIN messages m ON m.id=b.message_id
             WHERE b.conversation_id=?1 AND b.run_id=?2 AND b.turn_id=?3
             AND NOT EXISTS(SELECT 1 FROM assistant_message_segments segment WHERE segment.run_id=b.run_id AND segment.message_id=b.message_id)
             ORDER BY b.admission DESC,b.message_id LIMIT 2",
        )?;
        let rows = query.query_map(params![conversation,run,turn], |row|
            Ok((row.get::<_,String>(0)?,row.get::<_,bool>(1)?,row.get::<_,String>(2)?,row.get::<_,String>(3)?))
        )?.collect::<Result<Vec<_>,_>>()?;
        let Some((message,admission,actual_conversation,role)) = rows.first() else { return Ok(None); };
        if actual_conversation != conversation || role != "assistant"
            || !crate::canonical_message_ownership::matches_local_read_owner(&self.connection, conversation, message, run)? {
            return Err(invalid("Turn owner lacks accepted assistant read membership"));
        }
        if !admission && rows.len() > 1 {
            return Err(StoreError::Conflict("Turn has ambiguous assistant ownership".into()));
        }
        Ok(Some(AssistantOwner { conversation_id:conversation.into(),run_id:run.into(),assistant_message_id:message.clone() }))
    }

    pub fn assistant_owner_for_activity(&self, conversation: &str, run: &str, kind: &str, id: &str) -> StoreResult<Option<AssistantOwner>> {
        if !validate_scope(&self.connection,conversation,run)? { return Ok(None); }
        let message:Option<String> = self.connection.query_row(
            "SELECT b.message_id FROM assistant_activity_message_bindings b JOIN messages m ON m.id=b.message_id
             WHERE b.conversation_id=?1 AND b.run_id=?2 AND b.activity_kind=?3 AND b.activity_id=?4
             AND m.conversation_id=?1 AND m.role='assistant'",
            params![conversation,run,kind,id],|r|r.get(0),
        ).optional()?;
        if let Some(message) = &message {
            if !crate::canonical_message_ownership::matches_local_read_owner(&self.connection, conversation, message, run)? {
                return Err(invalid("Activity owner lacks exact accepted assistant read membership"));
            }
        }
        Ok(message.map(|assistant_message_id|AssistantOwner {conversation_id:conversation.into(),run_id:run.into(),assistant_message_id}))
    }
    pub fn assistant_owner_for_effect(&self, conversation: &str, run: &str, effect: &str) -> StoreResult<Option<AssistantOwner>> {
        if !validate_scope(&self.connection, conversation, run)? { return Ok(None); }
        let message: Option<String> = self.connection.query_row(
            "SELECT b.message_id FROM assistant_effect_message_bindings b JOIN messages m ON m.id=b.message_id
             WHERE b.conversation_id=?1 AND b.run_id=?2 AND b.effect_id=?3 AND m.conversation_id=?1 AND m.run_id=?2 AND m.role='assistant'",
            params![conversation,run,effect], |row|row.get(0),
        ).optional()?;
        Ok(message.map(|assistant_message_id| AssistantOwner { conversation_id:conversation.into(),run_id:run.into(),assistant_message_id }))
    }

    pub fn assistant_owner_for_part(&self, conversation: &str, run: &str, part: &str) -> StoreResult<Option<AssistantPartOwner>> {
        crate::require_id("assistant_owner.part_id", part)?;
        if !validate_scope(&self.connection, conversation, run)? { return Ok(None); }
        let row: Option<(String,String,Option<String>,String,Option<String>,Option<String>,bool,bool)> = self.connection.query_row(
            "SELECT m.id,m.conversation_id,m.run_id,m.role,b.turn_id,b.message_id,
                EXISTS(SELECT 1 FROM assistant_turn_message_bindings t
                    WHERE t.conversation_id=?3 AND t.run_id=?2 AND t.turn_id=b.turn_id
                    AND t.message_id=m.id AND b.conversation_id=?3),
                EXISTS(SELECT 1 FROM assistant_turn_part_bindings other WHERE other.part_id=p.id)
             FROM message_parts p JOIN messages m ON m.id=p.message_id
             LEFT JOIN assistant_turn_part_bindings b ON b.run_id=?2 AND b.part_id=p.id
             WHERE p.id=?1",
            params![part,run,conversation], |row|Ok((row.get(0)?,row.get(1)?,row.get(2)?,row.get(3)?,row.get(4)?,row.get(5)?,row.get(6)?,row.get(7)?)),
        ).optional()?;
        let Some((message,actual_conversation,actual_run,role,turn,bound_message,has_turn_binding,has_any_binding)) = row else { return Ok(None); };
        let exact_binding = match &bound_message {
            Some(bound) => bound == &message && has_turn_binding,
            // Creation planning may observe a genuinely unbound current Part.
            // Reusing the visible Message cannot claim a predecessor's Part.
            None => actual_run.as_deref() == Some(run) && !has_any_binding,
        };
        if actual_conversation != conversation || role != "assistant" || !exact_binding
            || !crate::canonical_message_ownership::matches_local_read_owner(&self.connection, conversation, &message, run)? {
            return Err(invalid("Part owner lacks exact accepted assistant read membership"));
        }
        Ok(Some(AssistantPartOwner { part_id:part.into(),turn_id:turn,owner:AssistantOwner {
            conversation_id:conversation.into(),run_id:run.into(),assistant_message_id:message,
        }}))
    }
}
