// SPDX-License-Identifier: Apache-2.0
//! Immutable request bytes and bounded delivery rebinding share one owned UoW.
use crate::*;
use nola_domain::host_effect::HostRequest;
type HostRequestEnvelope = nola_domain::host_effect::HostRequestEnvelope<HostRequest>;
use nola_store_port::{
    host_replay::CoreHostBindingOverlay,
    run_effect::{OutboxEffectWrite, StoredOutboxEffectIdentity},
};

fn digest(raw: &str) -> String {
    format!("{:x}", Sha256::digest(raw.as_bytes()))
}
fn decode(raw: &str, effect: &str, request: &str) -> StoreResult<HostRequestEnvelope> {
    let value: HostRequestEnvelope = serde_json::from_str(raw)?;
    if value.effect_id.0 != effect || value.request_id != request || value.session_id.is_empty() {
        return Err(StoreError::Invariant(
            "outbox immutable envelope identity mismatch".into(),
        ));
    }
    Ok(value)
}
fn binding_json(binding: &CoreHostBindingOverlay) -> StoreResult<String> {
    let json = serde_json::to_string(binding)?;
    if json.len() > 32768 {
        return Err(StoreError::Invariant(
            "outbox binding exceeds bounded metadata budget".into(),
        ));
    }
    Ok(json)
}
fn target(request: &HostRequestEnvelope) -> Option<&str> {
    match &request.payload {
        HostRequest::ResolveDelegatedPermission {
            target_effect_id, ..
        }
        | HostRequest::Cancel { target_effect_id, .. } => Some(&target_effect_id.0),
        _ => None,
    }
}

pub(super) fn write(
    c: &Connection,
    run: &str,
    effect: &OutboxEffectInput,
    at: i64,
) -> StoreResult<()> {
    if c.is_autocommit() {
        return Err(StoreError::Invariant(
            "outbox publication requires an owned transaction".into(),
        ));
    }
    require_id("outbox.effect_id", &effect.effect_id)?;
    require_id("outbox.request_id", &effect.request_id)?;
    match &effect.write {
        OutboxEffectWrite::Declare { payload_json } => {
            let request = decode(payload_json, &effect.effect_id, &effect.request_id)?;
            let binding = binding_json(&CoreHostBindingOverlay::from_request(&request))?;
            // Declaration is insert-only. A repeat must be a command receipt
            // replay, never an implicit overwrite of immutable execution input.
            c.execute("INSERT INTO effect_outbox(effect_id,run_id,request_id,effect_kind,binding_json,payload_digest,recovery_required,target_effect_id,status,created_at_ms)
                VALUES(?1,?2,?3,?4,?5,?6,0,?7,'pending',?8)",
                params![effect.effect_id,run,effect.request_id,effect.effect_kind,binding,digest(payload_json),target(&request),at])?;
            c.execute(
                "INSERT INTO effect_outbox_payloads(effect_id,payload_json) VALUES(?1,?2)",
                params![effect.effect_id, payload_json],
            )?;
        }
        OutboxEffectWrite::Rebind {
            expected_request_id,
            expected_attempt_id,
            binding,
        } => {
            let previous = identity(c, &effect.effect_id)?
                .ok_or_else(|| StoreError::Invariant("outbox rebind has no declaration".into()))?;
            if previous.run_id != run
                || previous.status != "pending"
                || previous.effect_kind != effect.effect_kind
                || previous.request_id != *expected_request_id
                || previous.binding.attempt_id != *expected_attempt_id
                || binding.effect_id.0 != effect.effect_id
                || binding.request_id != effect.request_id
                || binding.session_id != previous.binding.session_id
                || binding.turn_id != previous.binding.turn_id
            {
                return Err(StoreError::Conflict(
                    "outbox rebind owner or expected delivery changed".into(),
                ));
            }
            match (&previous.binding.payload_binding,&binding.payload_binding) {
                (nola_store_port::host_replay::CoreHostPayloadBinding::CancellationNotStarted { proof:old },
                 nola_store_port::host_replay::CoreHostPayloadBinding::CancellationNotStarted { proof:new }) if old==new =>{},
                (nola_store_port::host_replay::CoreHostPayloadBinding::Unchanged,nola_store_port::host_replay::CoreHostPayloadBinding::Unchanged)=>{},
                (nola_store_port::host_replay::CoreHostPayloadBinding::DelegatedPermissionDelivery { delivery:old },
                 nola_store_port::host_replay::CoreHostPayloadBinding::DelegatedPermissionDelivery { delivery:new }) if old.is_none() || old==new =>{},
                _=>return Err(StoreError::Conflict("outbox rebind changed payload binding authority".into())),
            }
            let encoded = binding_json(binding)?;
            if c.execute("UPDATE effect_outbox SET request_id=?2,binding_json=?3 WHERE effect_id=?1 AND run_id=?4 AND request_id=?5 AND status='pending' AND recovery_required=0",
                params![effect.effect_id,effect.request_id,encoded,run,expected_request_id])?!=1 {
                return Err(StoreError::Conflict("outbox rebind lost its delivery CAS".into()));
            }
        }
    }
    Ok(())
}

pub(super) fn identity(
    c: &Connection,
    id: &str,
) -> StoreResult<Option<StoredOutboxEffectIdentity>> {
    let row:Option<(String,String,String,String,Option<String>,bool)>=c.query_row(
        "SELECT run_id,request_id,effect_kind,status,
            CASE WHEN length(CAST(binding_json AS BLOB))<=32768 THEN binding_json END,
            recovery_required OR NOT EXISTS(SELECT 1 FROM effect_outbox_payloads p WHERE p.effect_id=effect_outbox.effect_id)
         FROM effect_outbox WHERE effect_id=?1",
        [id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?))).optional()?;
    row.map(|(run_id, request_id, effect_kind, status, json, blocked)| {
        if blocked {
            return Err(StoreError::Invariant(
                "outbox historical request requires explicit recovery".into(),
            ));
        }
        let binding: CoreHostBindingOverlay = serde_json::from_str(
            json.as_deref()
                .ok_or_else(|| StoreError::Invariant("outbox binding is missing".into()))?,
        )?;
        if binding.effect_id.0 != id || binding.request_id != request_id {
            return Err(StoreError::Invariant(
                "outbox binding/header mismatch".into(),
            ));
        }
        Ok(StoredOutboxEffectIdentity {
            effect_id: id.into(),
            run_id,
            request_id,
            effect_kind,
            status,
            binding,
        })
    })
    .transpose()
}

pub(super) fn read(c: &Connection, id: &str) -> StoreResult<Option<StoredOutboxEffect>> {
    let _snapshot = read_snapshot::pin(c)?;
    let Some(head) = identity(c, id)? else {
        return Ok(None);
    };
    let (raw,expected,created_at_ms,acknowledged_at_ms):(Option<String>,String,i64,Option<i64>)=c.query_row(
        "SELECT p.payload_json,e.payload_digest,e.created_at_ms,e.acknowledged_at_ms FROM effect_outbox e
         LEFT JOIN effect_outbox_payloads p ON p.effect_id=e.effect_id WHERE e.effect_id=?1",[id],
        |r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?)))?;
    let raw = raw
        .ok_or_else(|| StoreError::Invariant("outbox immutable request body is missing".into()))?;
    if digest(&raw) != expected {
        return Err(StoreError::Invariant(
            "outbox immutable request digest mismatch".into(),
        ));
    }
    let mut request: HostRequestEnvelope = serde_json::from_str(&raw)?;
    head.binding
        .apply_to(&mut request)
        .map_err(StoreError::Invariant)?;
    Ok(Some(StoredOutboxEffect {
        effect_id: head.effect_id,
        run_id: head.run_id,
        request_id: head.request_id,
        effect_kind: head.effect_kind,
        payload_json: serde_json::to_string(&request)?,
        status: head.status,
        created_at_ms,
        acknowledged_at_ms,
    }))
}

pub(super) fn pending(c: &Connection, run: &str) -> StoreResult<Vec<StoredOutboxEffect>> {
    let _snapshot = read_snapshot::pin(c)?;
    let ids:Vec<String>=c.prepare("SELECT effect_id FROM effect_outbox WHERE run_id=?1 AND status='pending' ORDER BY rowid")?
        .query_map([run],|r|r.get(0))?.collect::<Result<_,_>>()?;
    ids.iter()
        .map(|id| {
            read(c, id)?
                .ok_or_else(|| StoreError::Invariant("outbox disappeared inside snapshot".into()))
        })
        .collect()
}

#[cfg(test)]
#[path = "outbox_payload_tests.rs"]
mod tests;
