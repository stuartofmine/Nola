// SPDX-License-Identifier: Apache-2.0
//! Pending intent never creates canonical execution; only source admission does.
use super::*;
use nola_domain::remote_submission::*;

fn invalid(error: impl ToString) -> StoreError {
    StoreError::Conflict(error.to_string())
}
fn view(
    id: RemoteSubmissionIdentity,
    digest: Option<String>,
    phase: RemoteSubmissionPhase,
) -> RemoteSubmissionIntentView {
    RemoteSubmissionIntentView {
        identity: id,
        request_digest: digest,
        phase,
    }
}
fn phase(value: &str) -> StoreResult<RemoteSubmissionPhase> {
    Ok(serde_json::from_value(serde_json::Value::String(
        value.into(),
    ))?)
}
fn phase_text(value: RemoteSubmissionPhase) -> &'static str {
    match value {
        RemoteSubmissionPhase::Prepared => "prepared",
        RemoteSubmissionPhase::Bound => "bound",
        RemoteSubmissionPhase::Admitted => "admitted",
        RemoteSubmissionPhase::Rejected => "rejected",
        RemoteSubmissionPhase::Cancelled => "cancelled",
    }
}
fn intent(
    c: &Connection,
    id: &RemoteSubmissionIdentity,
) -> StoreResult<(PrepareRemoteSubmissionInput, RemoteSubmissionIntentView)> {
    id.validate().map_err(invalid)?;
    let (identity,input,digest,state):(String,String,Option<String>,String)=c.query_row(
        "SELECT identity_json,input_json,request_digest,phase FROM remote_submission_intents WHERE submission_id=?1",
        [&id.submission_id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?)))?;
    let stored: RemoteSubmissionIdentity = serde_json::from_str(&identity)?;
    if stored != *id {
        return Err(invalid("remote intent identity mismatch"));
    }
    Ok((
        serde_json::from_str(&input)?,
        view(stored, digest, phase(&state)?),
    ))
}
fn source(
    c: &Connection,
    id: &RemoteSubmissionIdentity,
    digest: &str,
) -> StoreResult<Option<SourceSubmissionReceipt>> {
    id.validate().map_err(invalid)?;
    validate_request_digest(digest).map_err(invalid)?;
    let raw: Option<String> = c
        .query_row(
            "SELECT receipt_json FROM source_submission_receipts WHERE submission_id=?1",
            [&id.submission_id],
            |r| r.get(0),
        )
        .optional()?;
    raw.map(|raw| {
        let mut receipt: SourceSubmissionReceipt = serde_json::from_str(&raw)?;
        receipt.validate().map_err(invalid)?;
        if receipt.identity != *id || receipt.request_digest != digest {
            return Err(invalid("source submission identity or request changed"));
        }
        if receipt.phase == RemoteSubmissionPhase::Admitted {
            receipt.admission = Some(crate::run_message_admission::local_head(
                c,
                &id.conversation_id,
                &id.submission_id,
            )?);
        }
        Ok(receipt)
    })
    .transpose()
}
fn write_receipt(
    c: &Connection,
    receipt: &SourceSubmissionReceipt,
    request: Option<&str>,
) -> StoreResult<()> {
    receipt.validate().map_err(invalid)?;
    c.execute("INSERT INTO source_submission_receipts(submission_id,identity_json,request_json,receipt_json)
        VALUES(?1,?2,?3,?4) ON CONFLICT(submission_id) DO UPDATE SET receipt_json=excluded.receipt_json",
        params![receipt.identity.submission_id,serde_json::to_string(&receipt.identity)?,request,serde_json::to_string(receipt)?])?;
    Ok(())
}

impl NolaStore {
    pub fn admitted_remote_run_identity(&self, run: &str) -> StoreResult<Option<RemoteSubmissionIdentity>> {
        require_id("remote Run", run)?;
        let _snapshot = crate::read_snapshot::pin(&self.connection)?;
        let originating: Option<String> = self.connection.query_row(
            "SELECT identity_json FROM remote_submission_intents WHERE submission_id=?1",
            [run], |row| row.get(0),
        ).optional()?;
        if let Some(raw) = originating {
            let identity: RemoteSubmissionIdentity = serde_json::from_str(&raw)?;
            let (_, accepted) = intent(&self.connection, &identity)?;
            if accepted.phase != RemoteSubmissionPhase::Admitted {
                return Err(invalid("remote intent has no admitted source receipt"));
            }
            validate_request_digest(accepted.request_digest.as_deref().ok_or_else(|| invalid("remote intent has no frozen request"))?).map_err(invalid)?;
            return Ok(Some(identity));
        }
        let adopted: Option<String> = self.connection.query_row(
            "SELECT receipt_json FROM remote_observed_run_receipts WHERE run_id=?1",
            [run], |row| row.get(0),
        ).optional()?;
        adopted.map(|raw| {
            let receipt: SourceSubmissionReceipt = serde_json::from_str(&raw)?;
            receipt.validate().map_err(invalid)?;
            if receipt.phase != RemoteSubmissionPhase::Admitted || receipt.identity.submission_id != run {
                return Err(invalid("observed remote Run has no exact admitted source receipt"));
            }
            Ok(receipt.identity)
        }).transpose()
    }

    pub fn observed_source_submission_receipt(
        &self,
        host: &str,
        client: &str,
        conversation: &str,
        run: &str,
        assistant: &str,
    ) -> StoreResult<Option<SourceSubmissionReceipt>> {
        for value in [host, client, conversation, run, assistant] {
            require_id("observed submission owner", value)?;
        }
        let _snapshot = crate::read_snapshot::pin(&self.connection)?;
        let raw: Option<String> = self
            .connection
            .query_row(
                "SELECT receipt_json FROM source_submission_receipts WHERE submission_id=?1",
                [run],
                |r| r.get(0),
            )
            .optional()?;
        let Some(raw) = raw else {
            return Ok(None);
        };
        let receipt: SourceSubmissionReceipt = serde_json::from_str(&raw)?;
        receipt.validate().map_err(invalid)?;
        let id = &receipt.identity;
        if id.source_host_id != host
            || id.client_id != client
            || id.conversation_id != conversation
            || id.submission_id != run
            || id.assistant_message_id != assistant
        {
            return Err(invalid("observed source submission owner mismatch"));
        }
        source(&self.connection, id, &receipt.request_digest)
    }
    pub fn adopt_observed_remote_run(
        &mut self,
        receipt: &SourceSubmissionReceipt,
    ) -> StoreResult<()> {
        if !self.has_write_unit_of_work() {
            return Err(invalid("observed Run adoption requires Application UoW"));
        }
        receipt.validate().map_err(invalid)?;
        if receipt.phase != RemoteSubmissionPhase::Admitted {
            return Err(invalid("only admitted source execution may be adopted"));
        }
        let id = &receipt.identity;
        let has_intent: bool = self.connection.query_row(
            "SELECT EXISTS(SELECT 1 FROM remote_submission_intents WHERE submission_id=?1)",
            [&id.submission_id],
            |r| r.get(0),
        )?;
        if has_intent {
            // Discovery intentionally omits terminal originating intents. A
            // cold observer can still encounter their admitted source receipt;
            // reuse exact bound acceptance, including owner/digest/epoch and
            // settled-phase checks, rather than treating discovery as authority.
            self.accept_remote_submission_receipt(receipt)?;
            return Ok(());
        }
        let old:Option<(String,String,String,String)>=self.connection.query_row(
            "SELECT r.conversation_id,r.user_message_id,r.assistant_message_id,s.source_host_id FROM runs r LEFT JOIN remote_run_sources s ON s.run_id=r.id WHERE r.id=?1",
            [&id.submission_id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).optional()?;
        if let Some(old) = old {
            let persisted: String = self.connection.query_row(
                "SELECT receipt_json FROM remote_observed_run_receipts WHERE run_id=?1",
                [&id.submission_id],
                |r| r.get(0),
            )?;
            let persisted: SourceSubmissionReceipt = serde_json::from_str(&persisted)?;
            if persisted.identity != receipt.identity
                || persisted.request_digest != receipt.request_digest
            {
                return Err(invalid("observed submission client or request changed"));
            }
            if old
                != (
                    id.conversation_id.clone(),
                    id.user_message_id.clone(),
                    id.assistant_message_id.clone(),
                    id.source_host_id.clone(),
                )
            {
                return Err(invalid(
                    "observed Run conflicts with existing source identity",
                ));
            }
        } else {
            self.admit_remote_submission_replica(&AdmittedReplicaTurnInput {
                user_part_kind: "text".into(),
                source_host_id: id.source_host_id.clone(),
                conversation_id: id.conversation_id.clone(),
                conversation_title: id.conversation_id.clone(),
                user_message_id: id.user_message_id.clone(),
                user_part_id: format!("{}:text", id.user_message_id),
                user_text: String::new(),
                user_content_json: None,
                assistant_message_id: id.assistant_message_id.clone(),
                assistant_part_id: format!("{}:text", id.assistant_message_id),
                run_id: id.submission_id.clone(),
                context_checkpoint_id: None,
                created_at_ms: 0,
            })?;
        }
        pin_receipt_source(&self.connection, receipt)?;
        self.connection.execute(
            "INSERT OR IGNORE INTO remote_observed_run_receipts VALUES(?1,?2)",
            params![id.submission_id, serde_json::to_string(receipt)?],
        )?;
        Ok(())
    }
    pub fn abandon_prepared_remote_submission(
        &mut self,
        id: &RemoteSubmissionIdentity,
    ) -> StoreResult<RemoteSubmissionIntentView> {
        let tx = self.write_transaction()?;
        let (_, current) = intent(&tx, id)?;
        if current.request_digest.is_some()
            || !matches!(
                current.phase,
                RemoteSubmissionPhase::Prepared | RemoteSubmissionPhase::Cancelled
            )
        {
            return Err(invalid(
                "only an unbound prepared submission can be abandoned",
            ));
        }
        tx.execute("UPDATE remote_submission_intents SET phase='cancelled' WHERE submission_id=?1 AND request_digest IS NULL AND phase='prepared'", [&id.submission_id])?;
        tx.commit()?;
        Ok(view(id.clone(), None, RemoteSubmissionPhase::Cancelled))
    }
    pub fn prepare_remote_submission(
        &mut self,
        input: &PrepareRemoteSubmissionInput,
    ) -> StoreResult<RemoteSubmissionIntentView> {
        let id = input.identity();
        id.validate().map_err(invalid)?;
        if input.user_text.trim().is_empty()
            || input.user_part_id.is_empty()
            || input.assistant_part_id != format!("{}:text", id.assistant_message_id)
        {
            return Err(invalid(
                "invalid pending remote submission content identity",
            ));
        }
        if let Some(json) = &input.user_content_json {
            let _: serde_json::Value = serde_json::from_str(json)?;
        }
        let tx = self.write_transaction()?;
        let exists: bool = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM remote_submission_intents WHERE submission_id=?1)",
            [&id.submission_id],
            |r| r.get(0),
        )?;
        if exists {
            let (stored, result) = intent(&tx, &id)?;
            if stored != *input {
                return Err(invalid("remote submission input changed"));
            }
            tx.commit()?;
            return Ok(result);
        }
        crate::agent_room_admission::require_active_context(&tx, &id.conversation_id)?;
        let busy:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM runs WHERE conversation_id=?1 AND status NOT IN ('sealed'))
            OR EXISTS(SELECT 1 FROM remote_submission_intents WHERE conversation_id=?1 AND phase IN ('prepared','bound'))",
            [&id.conversation_id],|r|r.get(0))?;
        if busy {
            return Err(invalid(
                "remote submission cannot overlap pending or active execution",
            ));
        }
        tx.execute("INSERT OR IGNORE INTO conversations(id,title,revision,created_at_ms,updated_at_ms) VALUES(?1,?2,1,?3,?3)",
            params![id.conversation_id,input.conversation_title,input.created_at_ms])?;
        tx.execute(
            "INSERT INTO remote_submission_intents VALUES(?1,?2,?3,?4,NULL,NULL,'prepared',?5)",
            params![
                id.submission_id,
                id.conversation_id,
                serde_json::to_string(&id)?,
                serde_json::to_string(input)?,
                input.created_at_ms
            ],
        )?;
        tx.commit()?;
        Ok(view(id, None, RemoteSubmissionPhase::Prepared))
    }
    pub fn bind_remote_submission_request(
        &mut self,
        id: &RemoteSubmissionIdentity,
        request: &serde_json::Value,
    ) -> StoreResult<RemoteSubmissionIntentView> {
        let json = canonical_submission_request(request).map_err(invalid)?;
        let digest = submission_request_digest(request).map_err(invalid)?;
        let tx = self.write_transaction()?;
        let (_, existing) = intent(&tx, id)?;
        if let Some(old) = &existing.request_digest {
            let old_json: String = tx.query_row(
                "SELECT request_json FROM remote_submission_intents WHERE submission_id=?1",
                [&id.submission_id],
                |r| r.get(0),
            )?;
            if old != &digest || old_json != json {
                return Err(invalid("frozen submission request changed"));
            }
            tx.commit()?;
            return Ok(existing);
        }
        if existing.phase != RemoteSubmissionPhase::Prepared {
            return Err(invalid("submission intent is not preparable"));
        }
        // A prepared draft has not been sent yet. Membership can change while
        // the host request is assembled. Frozen requests remain replayable so
        // already dispatched execution can still be recovered and cancelled.
        crate::agent_room_admission::require_active_context(&tx, &id.conversation_id)?;
        tx.execute("UPDATE remote_submission_intents SET request_json=?2,request_digest=?3,phase='bound' WHERE submission_id=?1",
            params![id.submission_id,json,digest])?;
        tx.commit()?;
        Ok(view(id.clone(), Some(digest), RemoteSubmissionPhase::Bound))
    }
    pub fn remote_submission_intent(
        &self,
        conversation: &str,
    ) -> StoreResult<Option<RemoteSubmissionIntentView>> {
        require_id("conversation", conversation)?;
        let raw:Option<(String,Option<String>,String)>=self.connection.query_row(
            "SELECT identity_json,request_digest,phase FROM remote_submission_intents i WHERE conversation_id=?1
            AND (phase IN ('prepared','bound') OR (phase='admitted' AND EXISTS(SELECT 1 FROM runs r WHERE r.id=i.submission_id AND r.status NOT IN ('sealed'))))
            ORDER BY created_at_ms DESC,submission_id DESC LIMIT 1",[conversation],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).optional()?;
        raw.map(|(id, digest, state)| Ok(view(serde_json::from_str(&id)?, digest, phase(&state)?)))
            .transpose()
    }
    pub fn remote_submission_request(
        &self,
        id: &RemoteSubmissionIdentity,
        digest: &str,
    ) -> StoreResult<serde_json::Value> {
        let (_, current) = intent(&self.connection, id)?;
        if current.request_digest.as_deref() != Some(digest)
            || current.phase != RemoteSubmissionPhase::Bound
        {
            return Err(invalid("submission request is not a matching bound intent"));
        }
        let raw: String = self.connection.query_row(
            "SELECT request_json FROM remote_submission_intents WHERE submission_id=?1",
            [&id.submission_id],
            |r| r.get(0),
        )?;
        Ok(serde_json::from_str(&raw)?)
    }
    pub fn register_source_submission(
        &mut self,
        id: &RemoteSubmissionIdentity,
        request: &serde_json::Value,
    ) -> StoreResult<RegisteredSourceSubmission> {
        id.validate().map_err(invalid)?;
        let json = canonical_submission_request(request).map_err(invalid)?;
        let digest = submission_request_digest(request).map_err(invalid)?;
        let tx = self.write_transaction()?;
        if let Some(receipt) = source(&tx, id, &digest)? {
            tx.commit()?;
            return Ok(RegisteredSourceSubmission {
                receipt,
                created: false,
            });
        }
        let occupied: bool = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM runs WHERE id=?1)",
            [&id.submission_id],
            |r| r.get(0),
        )?;
        if occupied {
            return Err(invalid(
                "source Run already exists without this submission receipt",
            ));
        }
        let receipt = SourceSubmissionReceipt {
            protocol_version: 1,
            identity: id.clone(),
            request_digest: digest,
            phase: RemoteSubmissionPhase::Prepared,
            admission: None,
            rejection_code: None,
        };
        write_receipt(&tx, &receipt, Some(&json))?;
        tx.commit()?;
        Ok(RegisteredSourceSubmission {
            receipt,
            created: true,
        })
    }
    pub fn exact_bound_remote_submission(
        &self,
        id: &RemoteSubmissionIdentity,
        digest: &str,
    ) -> StoreResult<RemoteSubmissionIntentView> {
        id.validate().map_err(invalid)?;
        validate_request_digest(digest).map_err(invalid)?;
        let (_, current) = intent(&self.connection, id)?;
        if current.request_digest.as_deref() != Some(digest) {
            return Err(invalid("remote submission does not match the frozen bound request"));
        }
        // A local unbound abandonment has no digest and cannot pass this gate.
        // Rejected/cancelled bound phases are written only by receipt acceptance.
        Ok(current)
    }
    pub fn source_submission_receipt(
        &self,
        id: &RemoteSubmissionIdentity,
        digest: &str,
    ) -> StoreResult<Option<SourceSubmissionReceipt>> {
        let _read = crate::read_snapshot::pin(&self.connection)?;
        source(&self.connection, id, digest)
    }
    pub fn settle_source_submission(
        &mut self,
        id: &RemoteSubmissionIdentity,
        digest: &str,
        rejection: Option<&str>,
    ) -> StoreResult<SourceSubmissionReceipt> {
        id.validate().map_err(invalid)?;
        validate_request_digest(digest).map_err(invalid)?;
        let tx = self.write_transaction()?;
        let old = source(&tx, id, digest)?;
        if old.is_none() {
            let occupied: bool = tx.query_row(
                "SELECT EXISTS(SELECT 1 FROM runs WHERE id=?1)",
                [&id.submission_id],
                |r| r.get(0),
            )?;
            if occupied {
                return Err(invalid(
                    "cannot settle an existing Run without its source submission receipt",
                ));
            }
        }
        if let Some(receipt) = &old {
            if receipt.phase != RemoteSubmissionPhase::Prepared {
                tx.commit()?;
                return Ok(receipt.clone());
            }
        }
        let receipt = SourceSubmissionReceipt {
            protocol_version: 1,
            identity: id.clone(),
            request_digest: digest.into(),
            phase: if rejection.is_some() {
                RemoteSubmissionPhase::Rejected
            } else {
                RemoteSubmissionPhase::Cancelled
            },
            admission: None,
            rejection_code: rejection.map(str::to_owned),
        };
        write_receipt(&tx, &receipt, None)?;
        tx.commit()?;
        Ok(receipt)
    }
    pub fn accept_remote_submission_receipt(
        &mut self,
        receipt: &SourceSubmissionReceipt,
    ) -> StoreResult<RemoteSubmissionIntentView> {
        if !self.has_write_unit_of_work() {
            return Err(invalid(
                "remote receipt acceptance requires Application UoW",
            ));
        }
        receipt.validate().map_err(invalid)?;
        let tx = self.write_transaction()?;
        let (input, current) = intent(&tx, &receipt.identity)?;
        if current.request_digest.as_deref() != Some(receipt.request_digest.as_str()) {
            return Err(invalid("source receipt does not match frozen request"));
        }
        if current.phase == RemoteSubmissionPhase::Admitted
            && receipt.phase != RemoteSubmissionPhase::Admitted
            || matches!(
                current.phase,
                RemoteSubmissionPhase::Cancelled | RemoteSubmissionPhase::Rejected
            ) && current.phase != receipt.phase
        {
            return Err(invalid("source receipt would regress a settled intent"));
        }
        // Release this nested savepoint before calling the existing owned
        // replica creation primitive; the outer Application UoW remains live.
        tx.commit()?;
        if receipt.phase == RemoteSubmissionPhase::Admitted
            && current.phase != RemoteSubmissionPhase::Admitted
        {
            self.admit_remote_submission_replica(&AdmittedReplicaTurnInput {
                user_part_kind: "text".into(),
                source_host_id: input.source_host_id,
                conversation_id: input.conversation_id,
                conversation_title: input.conversation_title,
                user_message_id: input.user_message_id,
                user_part_id: input.user_part_id,
                user_text: input.user_text,
                user_content_json: input.user_content_json,
                assistant_message_id: input.assistant_message_id,
                assistant_part_id: input.assistant_part_id,
                run_id: input.submission_id,
                context_checkpoint_id: input.context_checkpoint_id,
                created_at_ms: input.created_at_ms,
            })?;
        }
        let target = if receipt.phase == RemoteSubmissionPhase::Prepared {
            RemoteSubmissionPhase::Bound
        } else {
            receipt.phase
        };
        pin_receipt_source(&self.connection, receipt)?;
        self.connection.execute(
            "UPDATE remote_submission_intents SET phase=?2 WHERE submission_id=?1",
            params![receipt.identity.submission_id, phase_text(target)],
        )?;
        Ok(view(
            receipt.identity.clone(),
            Some(receipt.request_digest.clone()),
            target,
        ))
    }
}
fn pin_receipt_source(
    connection: &Connection,
    receipt: &SourceSubmissionReceipt,
) -> StoreResult<()> {
    if let Some(admission) = &receipt.admission {
        let old:Option<(String,String,String)>=connection.query_row(
                "SELECT source_host_id,source_store_epoch,assistant_message_id FROM remote_assistant_journal_sources WHERE run_id=?1 AND assistant_message_id=?2",
                params![receipt.identity.submission_id,receipt.identity.assistant_message_id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).optional()?;
        let expected = (
            receipt.identity.source_host_id.clone(),
            admission.identity.source_store_epoch.clone(),
            receipt.identity.assistant_message_id.clone(),
        );
        if old.as_ref().is_some_and(|old| old != &expected) {
            return Err(invalid("admitted source epoch changed"));
        }
        connection.execute("INSERT OR IGNORE INTO remote_assistant_journal_sources(run_id,source_host_id,source_store_epoch,assistant_message_id) VALUES(?1,?2,?3,?4)",
                params![receipt.identity.submission_id,expected.0,expected.1,expected.2])?;
    }
    Ok(())
}

pub(crate) fn gate_source(c: &Connection, input: &BeginTurnInput) -> StoreResult<()> {
    let raw: Option<String> = c
        .query_row(
            "SELECT receipt_json FROM source_submission_receipts WHERE submission_id=?1",
            [&input.run_id],
            |r| r.get(0),
        )
        .optional()?;
    if let Some(raw) = raw {
        let receipt: SourceSubmissionReceipt = serde_json::from_str(&raw)?;
        receipt.validate().map_err(invalid)?;
        if receipt.phase != RemoteSubmissionPhase::Prepared
            || receipt.identity.conversation_id != input.conversation_id
            || receipt.identity.user_message_id != input.user_message_id
            || receipt.identity.assistant_message_id != input.assistant_message_id
        {
            return Err(invalid(
                "source submission is not pending for this exact Run owner",
            ));
        }
    }
    Ok(())
}
pub(crate) fn commit_source_admission(c: &Connection, input: &BeginTurnInput) -> StoreResult<()> {
    let raw: Option<String> = c
        .query_row(
            "SELECT receipt_json FROM source_submission_receipts WHERE submission_id=?1",
            [&input.run_id],
            |r| r.get(0),
        )
        .optional()?;
    if let Some(raw) = raw {
        let mut receipt: SourceSubmissionReceipt = serde_json::from_str(&raw)?;
        receipt.phase = RemoteSubmissionPhase::Admitted;
        receipt.admission = Some(crate::run_message_admission::local_head(
            c,
            &input.conversation_id,
            &input.run_id,
        )?);
        write_receipt(c, &receipt, None)?;
    }
    Ok(())
}
