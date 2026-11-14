// SPDX-License-Identifier: Apache-2.0
//! Persistent expert targets reuse Task leases; metadata alone never grants execution.
use super::*;
use nola_domain::{
    agent_room::{AgentRoom, ContextOwner, CONTEXT_OWNER_FIELD},
    expert_message::ExpertParticipantReference,
    expert_task_target::{ExpertTaskTargetV1, EXPERT_TASK_TARGET_METADATA_KEY},
};

#[derive(serde::Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
struct ParentRoomLink { conversation_id:String, room_id:String }

fn invalid(message: &str) -> StoreError {
    StoreError::Conflict(message.into())
}
fn setting(c: &Connection, key: &str) -> StoreResult<Option<serde_json::Value>> {
    let raw: Option<String> = c
        .query_row(
            "SELECT value_json FROM product_settings WHERE key=?1",
            [key],
            |row| row.get(0),
        )
        .optional()?;
    raw.map(|raw| serde_json::from_str(&raw).map_err(StoreError::from))
        .transpose()
}
fn context_owner(c: &Connection, context: &str) -> StoreResult<ContextOwner> {
    let raw: Option<String> = c
        .query_row(
            "SELECT settings_json FROM conversation_settings WHERE conversation_id=?1",
            [context],
            |row| row.get(0),
        )
        .optional()?
        .flatten();
    let value: serde_json::Value =
        serde_json::from_str(&raw.ok_or_else(|| invalid("expert Task context settings missing"))?)?;
    serde_json::from_value(
        value
            .get(CONTEXT_OWNER_FIELD)
            .cloned()
            .ok_or_else(|| invalid("expert Task requires persistent ContextOwner"))?,
    )
    .map_err(StoreError::from)
}
fn owner_matches(owner: &ContextOwner, room: &str, reference: &ExpertParticipantReference) -> bool {
    owner.room_id == room
        && owner.participant_id == reference.participant_id
        && owner.agent_id == reference.agent_id
}

/// `require_admitted=false` is presentation classification only. The worker's
/// history/Run admission always requests true and still validates its live lease.
pub(crate) fn validated_expert_task_target(
    c: &Connection,
    task_id: &str,
    child: &str,
    require_admitted: bool,
) -> StoreResult<Option<ExpertTaskTargetV1>> {
    let row:Option<(String,Option<String>,Option<String>,Option<String>,String)>=c.query_row(
        "SELECT kind,owner_session_id,parent_tool_call_id,child_session_id,metadata_json FROM durable_tasks WHERE id=?1",[task_id],
        |row|Ok((row.get(0)?,row.get(1)?,row.get(2)?,row.get(3)?,row.get(4)?))).optional()?;
    let Some((kind, owner, call, session, metadata)) = row else {
        return Ok(None);
    };
    let metadata: serde_json::Value = serde_json::from_str(&metadata)?;
    let Some(pin) = metadata.get(EXPERT_TASK_TARGET_METADATA_KEY) else {
        return Ok(None);
    };
    let pin: ExpertTaskTargetV1 = serde_json::from_value(pin.clone())?;
    pin.validate().map_err(StoreError::Conflict)?;
    if kind != "agent"
        || pin.task_id != task_id
        || pin.target.context_conversation_id != child
        || session.as_deref() != Some(child)
        || owner.as_deref() != Some(pin.source.context_conversation_id())
        || call.as_deref() != Some(pin.source_call_id.as_str())
    {
        return Err(invalid("expert Task draft differs from target pin"));
    }
    let actual_parent:Option<String>=c.query_row("SELECT task_id FROM runtime_task_run_bindings WHERE child_session_id=?1 AND run_id=?2",params![pin.source.context_conversation_id(),pin.source_run_id],|r|r.get(0)).optional()?;
    let declared_parent:Option<String>=c.query_row("SELECT parent_task_id FROM durable_tasks WHERE id=?1",[task_id],|r|r.get(0))?;
    if actual_parent!=declared_parent {return Err(invalid("expert Task parent lineage differs from exact source Run binding"));}
    let record = setting(
        c,
        &format!("expert_peer_private/v1/exchange/{}", pin.exchange_id),
    )?
    .ok_or_else(|| invalid("expert Task private peer admission missing"))?;
    let view = &record["material"]["view"];
    let entry: nola_domain::expert_message::ExpertMessageEntry =
        serde_json::from_value(record["material"]["entry"].clone())?;
    let source: nola_domain::expert_task_target::ExpertTaskSourceV1 = serde_json::from_value(view["source"].clone())?;
    let target: ExpertParticipantReference = serde_json::from_value(view["target"].clone())?;
    if view["version"] != 1
        || view["exchange_id"] != pin.exchange_id
        || view["room_id"] != pin.room_id
        || view["parent_conversation_id"] != pin.parent_conversation_id
        || view["source_run_id"] != pin.source_run_id
        || view["source_call_id"] != pin.source_call_id
        || source != pin.source
        || target != pin.target
        || entry.participant != pin.target
        || entry.child_submission_id != pin.exchange_id
        || entry.child_user_message_id != format!("{}:user", pin.exchange_id)
        || entry.child_assistant_message_id != format!("{}:assistant", pin.exchange_id)
    {
        return Err(invalid(
            "expert Task private admission differs from target pin",
        ));
    }
    if let Some(configuration)=record.get("task_configuration").filter(|value|!value.is_null()) {
        if metadata.get("expert_target_configuration")!=Some(configuration)
            || metadata.get("expert_target_prompt_definitions")!=record.pointer("/material/prompt_definitions")
            || metadata.get("expert_exchange_context")!=record.get("task_exchange_context") {
            return Err(invalid("expert Task configuration metadata differs from private target capture"));
        }
    } else if require_admitted {return Err(invalid("expert Task executable configuration was not Core-prepared"));}
    let authority = crate::agent_task_admission_store::authority(c, task_id)?
        .ok_or_else(|| invalid("expert Task prepared authority missing"))?;
    if let Some(admission) = authority.admission {
        admission.validate().map_err(StoreError::Invariant)?;
        if admission.task_id != task_id
            || admission.parent_conversation_id != pin.source.context_conversation_id()
            || admission.parent_run_id != pin.source_run_id
            || admission.parent_tool_call_id != pin.source_call_id
            || admission.child_conversation_id != child
        {
            return Err(invalid(
                "expert Task Core source admission differs from peer pin",
            ));
        }
    } else if require_admitted {
        return Err(invalid("expert Task is not yet Core-admitted"));
    }
    if pin.source.is_main_agent() {
        let source_settings:Option<String>=c.query_row("SELECT settings_json FROM conversation_settings WHERE conversation_id=?1",[pin.source.context_conversation_id()],|row|row.get(0)).optional()?.flatten();
        let settings:serde_json::Value=source_settings.as_deref().map(serde_json::from_str).transpose()?.unwrap_or(serde_json::Value::Null);
        let source_task:bool=c.query_row("SELECT EXISTS(SELECT 1 FROM runtime_task_run_bindings WHERE child_session_id=?1)",[pin.source.context_conversation_id()],|row|row.get(0))?;
        if settings.get(CONTEXT_OWNER_FIELD).is_some() || source_task || declared_parent.is_some() {
            return Err(invalid("main expert Task source cannot be an expert or task child"));
        }
    }
    let target_owner = context_owner(c, child)?;
    let source_owner = pin.source.legacy_expert().map(|_|context_owner(c, pin.source.context_conversation_id())).transpose()?;
    if !owner_matches(&target_owner, &pin.room_id, &pin.target)
        || pin.source.legacy_expert().is_some_and(|reference| !source_owner.as_ref().is_some_and(|owner|owner_matches(owner, &pin.room_id, reference)))
    {
        return Err(invalid(
            "expert Task ContextOwner differs from frozen target",
        ));
    }
    let forward = setting(
        c,
        &format!(
            "agent_chat_conversation_room/{}",
            pin.parent_conversation_id
        ),
    )?
    .ok_or_else(|| invalid("expert Task parent link missing"))?;
    let reverse = setting(c, &format!("agent_chat_room_conversation/{}", pin.room_id))?
        .ok_or_else(|| invalid("expert Task reverse link missing"))?;
    let forward:ParentRoomLink=serde_json::from_value(forward)?;
    let reverse:ParentRoomLink=serde_json::from_value(reverse)?;
    if forward != reverse
        || forward.room_id != pin.room_id
        || forward.conversation_id != pin.parent_conversation_id
    {
        return Err(invalid("expert Task parent Room links differ"));
    }
    let room: AgentRoom = serde_json::from_value(
        setting(c, &format!("agent_chat_room/{}", pin.room_id))?
            .ok_or_else(|| invalid("expert Task Room missing"))?,
    )?;
    if room.id() != pin.room_id {
        return Err(invalid("expert Task Room identity differs"));
    }
    let matches = |reference: &ExpertParticipantReference| {
        room.participants().iter().any(|member| {
            member.active
                && member.id == reference.participant_id
                && member.agent_id == reference.agent_id
                && member.agent_revision == reference.agent_revision
                && member.context_conversation_id == reference.context_conversation_id
        })
    };
    let bound:Option<(String,String,String)>=c.query_row(
        "SELECT owner_session_id,child_session_id,run_id FROM runtime_task_run_bindings WHERE task_id=?1",[task_id],
        |row|Ok((row.get(0)?,row.get(1)?,row.get(2)?))).optional()?;
    if let Some((owner, context, run)) = bound {
        if owner != pin.source.context_conversation_id() || context != child || run != pin.exchange_id
        {
            return Err(invalid("expert Task immutable Run binding differs"));
        }
    } else if pin.source.legacy_expert().is_some_and(|source|!matches(source)) || !matches(&pin.target) {
        return Err(invalid("expert Task unbound target is no longer current"));
    }
    Ok(Some(pin))
}

pub(crate) fn validate_persistent_expert_history(
    c: &Connection,
    task_id: &str,
    child: &str,
    admitted_run: Option<&str>,
) -> StoreResult<bool> {
    let Some(pin) = validated_expert_task_target(c, task_id, child, true)? else {
        return Ok(false);
    };
    let foreign_active:bool=c.query_row("SELECT EXISTS(SELECT 1 FROM runs WHERE conversation_id=?1 AND status<>'sealed' AND (?2 IS NULL OR id<>?2))",
        params![child,admitted_run],|row|row.get(0))?;
    if foreign_active {
        return Err(invalid("expert Task context has a foreign active Run"));
    }
    let competing:bool=c.query_row("SELECT EXISTS(SELECT 1 FROM durable_tasks WHERE kind='agent' AND child_session_id=?1 AND id<>?2 AND status NOT IN ('sealed','completed','failed','cancelled'))",
        params![child,task_id],|row|row.get(0))?;
    if competing {
        return Err(invalid("expert Task context already has a pending owner"));
    }
    let mut statement = c.prepare(
        "SELECT task_id FROM runtime_task_run_bindings WHERE child_session_id=?1 AND task_id<>?2",
    )?;
    for old in statement.query_map(params![child, task_id], |row| row.get::<_, String>(0))? {
        let old = old?;
        let old_pin = validated_expert_task_target(c, &old, child, true)?
            .ok_or_else(|| invalid("expert context contains a foreign Task binding"))?;
        if old_pin.room_id != pin.room_id
            || old_pin.parent_conversation_id != pin.parent_conversation_id
            || old_pin.target != pin.target
        {
            return Err(invalid("expert context previous Task target differs"));
        }
        let binding = crate::runtime_task_run_binding::binding(c, &old)?
            .ok_or_else(|| invalid("expert previous Task binding missing"))?;
        let raw: Option<String> = c
            .query_row(
                "SELECT receipt_json FROM runtime_task_terminal_receipts WHERE task_id=?1",
                [&old],
                |row| row.get(0),
            )
            .optional()?;
        let receipt: nola_store_port::runtime_task::RuntimeTaskTerminalReceipt =
            serde_json::from_str(
                &raw.ok_or_else(|| invalid("expert previous Task terminal missing"))?,
            )?;
        let result = receipt
            .journal_result
            .as_ref()
            .ok_or_else(|| invalid("expert previous Task has no canonical Run result"))?;
        result.seal.validate().map_err(StoreError::Invariant)?;
        let exact:bool=c.query_row("SELECT EXISTS(SELECT 1 FROM durable_tasks t JOIN runs r ON r.id=?2 WHERE t.id=?1 AND t.kind='agent' AND t.status='sealed'
            AND r.conversation_id=?3 AND r.status='sealed' AND r.completed_at_ms=?4)",params![old,binding.run_id,child,result.seal.completed_at_ms],|row|row.get(0))?;
        if !exact
            || receipt.task_id != old
            || receipt.status != "sealed"
            || receipt.owner_session_id != old_pin.source.context_conversation_id()
            || receipt.child_session_id.as_deref() != Some(child)
            || result.seal.identity.run_id != binding.run_id
            || result.seal.result.conversation_id != child
            || !matches!(&receipt.source,nola_store_port::runtime_task::RuntimeTaskTerminalSource::BoundRunTerminal{binding:source,..} if source==&binding)
        {
            return Err(invalid("expert context previous Task terminal differs"));
        }
    }
    Ok(true)
}

impl NolaStore {
    pub fn validated_expert_task_target(
        &self,
        task_id: &str,
        child: &str,
        require_admitted: bool,
    ) -> StoreResult<Option<ExpertTaskTargetV1>> {
        let _snapshot = crate::read_snapshot::pin(&self.connection)?;
        validated_expert_task_target(&self.connection, task_id, child, require_admitted)
    }
    pub fn expert_task_is_terminal_without_run(&self,context:&str,id:&str,at:i64)->StoreResult<bool>{
        let Some(pin)=self.validated_expert_task_target(id,context,false)? else {return Ok(false)};
        let Some(task)=self.runtime_task(id)? else {return Ok(false)};
        let Some(receipt)=self.runtime_task_terminal_receipt(id)? else {return Ok(false)};
        let privately_prepared:bool=self.connection.query_row("SELECT EXISTS(SELECT 1 FROM product_settings WHERE key=?1 AND json_type(value_json,'$.task_configuration')='object')",[format!("expert_peer_private/v1/exchange/{id}")],|r|r.get(0))?;
        if !privately_prepared || !matches!(task.status.as_str(),"failed"|"cancelled") || receipt.status!=task.status
            || receipt.task_id!=id || receipt.owner_session_id!=pin.source.context_conversation_id()
            || receipt.child_session_id.as_deref()!=Some(context) || task.ended_at_ms!=Some(receipt.ended_at_ms)
            || receipt.recorded_at_ms>at || receipt.ended_at_ms>at || receipt.journal_result.is_some()
            || matches!(&receipt.source,nola_store_port::runtime_task::RuntimeTaskTerminalSource::BoundRunTerminal{..}) {
            return Ok(false);
        }
        if let nola_store_port::runtime_task::RuntimeTaskTerminalSource::WorkerFinish{lease}=&receipt.source {
            if lease.task_id!=id {return Err(invalid("expert Task terminal lease differs"));}
        }
        let has_run:bool=self.connection.query_row("SELECT EXISTS(SELECT 1 FROM runtime_task_run_bindings WHERE task_id=?1) OR EXISTS(SELECT 1 FROM runs WHERE id=?1 OR (conversation_id=?2 AND status<>'sealed'))",params![id,context],|r|r.get(0))?;
        Ok(!has_run)
    }
    pub fn expert_context_has_pending_task(&self, context: &str) -> StoreResult<bool> {
        self.connection.query_row("SELECT EXISTS(SELECT 1 FROM durable_tasks WHERE kind='agent' AND child_session_id=?1 AND status NOT IN ('sealed','completed','failed','cancelled'))",
            [context],|row|row.get(0)).map_err(StoreError::from)
    }
}
