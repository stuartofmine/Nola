// SPDX-License-Identifier: Apache-2.0
//! Bounded branch provenance claims; remote receipt never grants semantic authority.
use crate::{
    params, Connection, OptionalExtension, RemoteSyncChangeInput, StoreError, StoreResult,
};
use nola_domain::PartDerivation;

fn conflict(message: &str) -> StoreError {
    StoreError::Conflict(message.into())
}
fn validate_identity(record: &PartDerivation) -> StoreResult<()> {
    crate::part_derivation::validate_shape(record)?;
    let source = &record.immediate_source.owner;
    let target = &record.target.owner;
    for value in [
        &record.operation_id,
        source.conversation_id(),
        source.message_id(),
        source.part_id(),
        target.conversation_id(),
        target.message_id(),
        target.part_id(),
    ] {
        if value.is_empty()
            || value.len() > 128
            || value.trim() != value
            || value.chars().any(char::is_control)
        {
            return Err(conflict("invalid replica Part derivation identity"));
        }
    }
    if source.conversation_id() == target.conversation_id()
        || record.target.run_id.is_some()
        || crate::conversation_branch::identity(
            &record.operation_id,
            "message",
            source.message_id(),
        ) != target.message_id()
        || crate::conversation_branch::identity(&record.operation_id, "part", source.part_id())
            != target.part_id()
    {
        return Err(conflict(
            "replica Part derivation does not match branch-derived target identity",
        ));
    }
    Ok(())
}

pub(super) fn enqueue_message(
    connection: &Connection,
    operation: &str,
    target_message: &str,
    at: i64,
) -> StoreResult<()> {
    let mut statement = connection.prepare(
        "SELECT CASE WHEN length(CAST(d.target_part_id AS BLOB))<=128 THEN d.target_part_id END,CASE WHEN length(CAST(d.derivation_json AS BLOB))<=16384 THEN d.derivation_json END
         FROM message_part_derivations d JOIN message_parts p ON p.id=d.target_part_id
         WHERE d.operation_id=?1 AND p.message_id=?2 ORDER BY p.ordinal")?;
    let mut rows = statement.query(params![operation, target_message])?;
    while let Some(row) = rows.next()? {
        let part: String = row.get(0)?;
        let encoded: String = row.get(1)?;
        let record: PartDerivation = serde_json::from_str(&encoded)?;
        validate_identity(&record)?;
        let current = crate::part_derivation::source(connection, &part)?;
        if current != record.target {
            return Err(conflict("branch proof target changed before enqueue"));
        }
        crate::enqueue_sync_change(connection, "part_derivation", &part, 1, false, &encoded, at)?;
        remember_local(connection, "part_derivation", &part, &encoded)?;
    }
    Ok(())
}

pub(super) fn apply(connection: &Connection, change: &RemoteSyncChangeInput) -> StoreResult<()> {
    if connection.is_autocommit()
        || change.entity_type != "part_derivation"
        || change.deleted
        || change.revision != 1
        || change.payload_json.len() > 16384
    {
        return Err(conflict("invalid immutable Part derivation fact"));
    }
    if accepted_replay(connection, change)? {
        return Ok(());
    }
    let record: PartDerivation = serde_json::from_str(&change.payload_json)?;
    validate_identity(&record)?;
    if change.entity_id != record.target.owner.part_id() {
        return Err(conflict("replica Part derivation entity identity mismatch"));
    }
    if target_is_tombstoned(connection, record.target.owner.conversation_id())? {
        remember_remote(connection, change)?;
        return Ok(());
    }
    let current = crate::part_derivation::source(connection, change.entity_id.as_str())?;
    if record.target != current {
        return Err(conflict(
            "replica Part derivation target bytes or metadata differ",
        ));
    }
    if let Some(previous) = crate::part_derivation::load(connection, &current)? {
        if previous.derivation != record {
            return Err(conflict("immutable Part derivation replay differs"));
        }
        // A separately verified local record stays local; remote claims cannot upgrade it.
        remember_remote(connection, change)?;
        return Ok(());
    }
    connection.execute("INSERT INTO message_part_derivations(target_part_id,operation_id,derivation_json,provenance) VALUES(?1,?2,?3,'replica')",
        params![change.entity_id, record.operation_id, serde_json::to_string(&record)?])?;
    remember_remote(connection, change)?;
    Ok(())
}

/// Only the final lineage waits; its earlier append/proof pairs can share a page.
pub(super) fn dependencies_acknowledged(
    connection: &Connection,
    payload: &str,
) -> StoreResult<bool> {
    let lineage: crate::conversation_branch::BranchLineage = serde_json::from_str(payload)?;
    let mut statement = connection.prepare(
        "SELECT CASE WHEN length(CAST(entity_id AS BLOB))<=128 THEN entity_id END,status,CASE WHEN length(CAST(payload_json AS BLOB))<=16384 THEN payload_json END
         FROM sync_outbox WHERE entity_type='part_derivation' AND revision=1
         AND json_extract(payload_json,'$.operation_id')=?1 ORDER BY local_sequence")?;
    let mut rows = statement.query(params![lineage.intent.operation_id])?;
    let mut count = 0_u64;
    let duplicate: bool = connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM sync_outbox WHERE entity_type='part_derivation' AND revision=1
         AND json_extract(payload_json,'$.operation_id')=?1 GROUP BY entity_id HAVING COUNT(*)>1)",
        params![lineage.intent.operation_id],
        |row| row.get(0),
    )?;
    if duplicate {
        return Err(conflict("branch lineage has duplicate proof dependencies"));
    }
    while let Some(row) = rows.next()? {
        let part: String = row.get(0)?;
        let status: String = row.get(1)?;
        let record: PartDerivation = serde_json::from_str(&row.get::<_, String>(2)?)?;
        validate_identity(&record)?;
        if record.target.owner.part_id() != part
            || record.target.owner.conversation_id() != lineage.intent.conversation.id
            || record.immediate_source.owner.conversation_id()
                != lineage.intent.source_conversation_id
        {
            return Err(conflict("branch lineage proof dependencies differ"));
        }
        if !matches!(status.as_str(), "acknowledged" | "unchanged") {
            return Ok(false);
        }
        count += 1;
    }
    Ok(count == lineage.derivation_count)
}

pub(super) fn verify_lineage_coverage(
    connection: &Connection,
    lineage: &crate::conversation_branch::BranchLineage,
) -> StoreResult<()> {
    let count: u64 = connection.query_row(
        "SELECT COUNT(*) FROM message_part_derivations WHERE operation_id=?1",
        params![lineage.intent.operation_id],
        |row| row.get(0),
    )?;
    if count != lineage.derivation_count {
        return Err(conflict(
            "branch lineage awaits its exact Part derivation count",
        ));
    }
    let mut statement = connection.prepare(
        "SELECT CASE WHEN length(CAST(p.id AS BLOB))<=128 THEN p.id END FROM messages m JOIN message_parts p ON p.message_id=m.id
         WHERE m.conversation_id=?1 AND m.ordinal<=?2 ORDER BY m.ordinal,p.ordinal")?;
    let mut rows = statement.query(params![
        lineage.intent.conversation.id,
        lineage.result.message_count as i64
    ])?;
    let mut covered = 0_u64;
    while let Some(row) = rows.next()? {
        let part: String = row.get(0)?;
        let current = crate::part_derivation::source(connection, &part)?;
        let stored = crate::part_derivation::load(connection, &current)?
            .ok_or_else(|| conflict("branch lineage is missing a target Part derivation"))?;
        let record = stored.derivation;
        validate_identity(&record)?;
        if record.operation_id != lineage.intent.operation_id
            || record.immediate_source.owner.conversation_id()
                != lineage.intent.source_conversation_id
        {
            return Err(conflict(
                "branch lineage Part provenance does not match its operation/source",
            ));
        }
        covered += 1;
    }
    if covered != lineage.derivation_count {
        return Err(conflict("branch lineage Part coverage differs"));
    }
    Ok(())
}

#[cfg(test)]
#[path = "part_derivation_sync_tests.rs"]
mod tests;

// These bounded receipts intentionally outlive canonical target deletion. They
// prove exact acceptance, never authorize recreation or semantic ownership.
fn canonical_digest(kind: &str, payload: &str) -> StoreResult<String> {
    use sha2::{Digest, Sha256};
    let encoded = match kind {
        "part_derivation" => serde_json::to_vec(&serde_json::from_str::<PartDerivation>(payload)?)?,
        "conversation_branch_lineage" => serde_json::to_vec(&serde_json::from_str::<
            crate::conversation_branch::BranchLineage,
        >(payload)?)?,
        _ => return Err(conflict("unknown branch receipt fact")),
    };
    Ok(format!("{:x}", Sha256::digest(encoded)))
}
fn receipt_matches(
    connection: &Connection,
    operation: &str,
    kind: &str,
    entity: &str,
    digest: &str,
) -> StoreResult<bool> {
    let previous: Option<(String, String, String)> = connection.query_row(
        "SELECT entity_type,entity_id,canonical_sha256 FROM branch_fact_receipts WHERE operation_id=?1",
        params![operation], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?))).optional()?;
    if let Some(previous) = previous {
        if previous != (kind.into(), entity.into(), digest.into()) {
            return Err(conflict("immutable branch fact operation replay differs"));
        }
        return Ok(true);
    }
    Ok(false)
}
pub(super) fn accepted_replay(
    connection: &Connection,
    change: &RemoteSyncChangeInput,
) -> StoreResult<bool> {
    receipt_matches(
        connection,
        &change.operation_id,
        &change.entity_type,
        &change.entity_id,
        &canonical_digest(&change.entity_type, &change.payload_json)?,
    )
}
fn remember(
    connection: &Connection,
    operation: &str,
    kind: &str,
    entity: &str,
    payload: &str,
) -> StoreResult<()> {
    let digest = canonical_digest(kind, payload)?;
    if !receipt_matches(connection, operation, kind, entity, &digest)? {
        connection.execute("INSERT INTO branch_fact_receipts(operation_id,entity_type,entity_id,canonical_sha256) VALUES(?1,?2,?3,?4)",
            params![operation, kind, entity, digest])?;
    }
    Ok(())
}
pub(super) fn remember_local(
    connection: &Connection,
    kind: &str,
    entity: &str,
    payload: &str,
) -> StoreResult<()> {
    remember(
        connection,
        &crate::sync_operation_id(kind, entity, 1, false, payload),
        kind,
        entity,
        payload,
    )
}
pub(super) fn remember_remote(
    connection: &Connection,
    change: &RemoteSyncChangeInput,
) -> StoreResult<()> {
    remember(
        connection,
        &change.operation_id,
        &change.entity_type,
        &change.entity_id,
        &change.payload_json,
    )
}

pub(super) fn target_is_tombstoned(
    connection: &Connection,
    conversation: &str,
) -> StoreResult<bool> {
    connection.query_row("SELECT EXISTS(SELECT 1 FROM sync_entity_revisions WHERE entity_type='conversation' AND entity_id=?1 AND deleted=1)",
        params![conversation], |row| row.get(0)).map_err(StoreError::from)
}
