// SPDX-License-Identifier: Apache-2.0
//! Shell dispatch facts live in the same transactions as leases and recovery.
use super::*;
use nola_domain::runtime_task_shell_delivery::{ShellWorkerDispatchPhase as Phase,
    ShellWorkerDispatchWitness as Witness, SHELL_WORKER_DISPATCH_METADATA_KEY as KEY};
use nola_store_port::runtime_task::{RuntimeTaskAttempt, RuntimeTaskProgressMutation};

fn witness(metadata: &str) -> StoreResult<Option<Witness>> {
    let metadata: serde_json::Value=serde_json::from_str(metadata)?;
    metadata.get(KEY).cloned().map(serde_json::from_value).transpose().map_err(StoreError::Json)
}

pub(super) fn initialize(connection: &Connection, attempt: &RuntimeTaskAttempt) -> StoreResult<()> {
    let (kind, raw):(String,String)=connection.query_row("SELECT kind,metadata_json FROM durable_tasks WHERE id=?1",
        [&attempt.task_id], |row|Ok((row.get(0)?,row.get(1)?)))?;
    if kind!="shell_process" { return Ok(()); }
    let mut metadata:serde_json::Value=serde_json::from_str(&raw)?;
    let previous=witness(&raw)?;
    let prepared=Witness::new(&attempt.task_id,&attempt.id,&attempt.worker_id,&attempt.worker_generation,&attempt.lease_token,Phase::Prepared)
        .map_err(StoreError::Invariant)?;
    prepared.follows(previous.as_ref()).map_err(StoreError::Invariant)?;
    metadata.as_object_mut().ok_or_else(||StoreError::Invariant("Shell metadata is not an object".into()))?
        .insert(KEY.into(),serde_json::to_value(prepared)?);
    connection.execute("UPDATE durable_tasks SET metadata_json=?2 WHERE id=?1",
        params![attempt.task_id,serde_json::to_string(&metadata)?])?;
    Ok(())
}

pub(super) fn validate_shell_dispatch_progress(connection:&Connection, mutation:&RuntimeTaskProgressMutation) -> StoreResult<()> {
    let previous=witness(&mutation.expected_metadata_json)?;
    let next=witness(&mutation.metadata_json)?;
    let Some(next)=next else {
        if previous.is_some() { return Err(StoreError::Invariant("Task progress cannot erase Shell dispatch evidence".into())); }
        return Ok(());
    };
    if previous.is_none() || (next.phase==Phase::Prepared && previous.as_ref()!=Some(&next)) {
        return Err(StoreError::Invariant("only the claim transaction can initialize Shell Prepared evidence".into()));
    }
    next.follows(previous.as_ref()).map_err(StoreError::Invariant)?;
    if next.task_id != mutation.task_id {
        return Err(StoreError::Invariant("Shell dispatch evidence belongs to another Task".into()));
    }
    // Continuation progress may retain an older attempt's immutable evidence;
    // it cannot manufacture a new admission. Only a phase transition must bind
    // the current attempt; the ordinary progress CAS still fences this write.
    if previous.as_ref() == Some(&next) { return Ok(()); }
    let identity:Option<(String,String)>=connection.query_row(
        "SELECT t.kind,a.worker_id FROM durable_tasks t JOIN runtime_task_attempts a ON a.task_id=t.id
         WHERE t.id=?1 AND a.id=?2",params![mutation.task_id,mutation.attempt_id],|row|Ok((row.get(0)?,row.get(1)?))).optional()?;
    if !identity.is_some_and(|(kind,worker)|kind=="shell_process" && next.matches(&mutation.task_id,
        &mutation.attempt_id,&worker,&mutation.worker_generation,&mutation.lease_token)) {
        return Err(StoreError::Invariant("Shell dispatch evidence does not bind the current progress attempt".into()));
    }
    Ok(())
}

pub(super) fn shell_dispatch_allows_expired_recovery(connection:&Connection, task_id:&str) -> StoreResult<bool> {
    let task:Option<(String,String)>=connection.query_row("SELECT kind,metadata_json FROM durable_tasks WHERE id=?1",
        [task_id],|row|Ok((row.get(0)?,row.get(1)?))).optional()?;
    let Some((kind,raw))=task else {return Ok(true)};
    if kind!="shell_process" {return Ok(true)};
    let proof=match witness(&raw) {Ok(Some(proof))=>proof,_=>return Ok(false)};
    if proof.phase!=Phase::Prepared {return Ok(false)};
    let mut statement=connection.prepare(
        "SELECT a.id,a.worker_id,a.worker_generation,a.lease_token FROM runtime_task_attempts a JOIN durable_tasks t ON t.id=a.task_id
         WHERE t.id=?1 AND a.state='leased' AND a.worker_id=t.worker_id AND a.lease_token=t.ownership_token
           AND a.lease_expires_at_ms=t.lease_until_ms LIMIT 2")?;
    let rows=statement.query_map([task_id],|row|Ok((row.get::<_,String>(0)?,row.get::<_,String>(1)?,row.get::<_,String>(2)?,row.get::<_,String>(3)?)))?
        .collect::<Result<Vec<_>,_>>()?;
    Ok(rows.len()==1 && proof.matches(task_id,&rows[0].0,&rows[0].1,&rows[0].2,&rows[0].3))
}

pub(super) fn require_unchanged(expected:&str, supplied:&str) -> StoreResult<()> {
    crate::terminal_input_store::require_unchanged(expected, supplied)?;
    if witness(expected)?!=witness(supplied)? {
        return Err(StoreError::Invariant("Task final progress cannot rewrite Shell dispatch evidence".into()));
    }
    Ok(())
}
