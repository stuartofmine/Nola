// SPDX-License-Identifier: Apache-2.0
//! Store-authored branch provenance. Byte equivalence is not semantic authorization.
use crate::{params, Connection, NolaStore, OptionalExtension, StoreError, StoreResult};
use nola_domain::{CanonicalPartOwner, PartBodyField};
use nola_store_port::part_derivation::{PartDerivationProvenance, StoredPartDerivation};
use sha2::{Digest, Sha256};

pub use nola_domain::{PartDerivation, PartDerivationSource, PartFieldFingerprint};
fn invalid(message: &str) -> StoreError {
    StoreError::Conflict(message.into())
}
fn fingerprint(
    connection: &Connection,
    owner: &CanonicalPartOwner,
    field: PartBodyField,
) -> StoreResult<Option<PartFieldFingerprint>> {
    let Some(total) = crate::published_part_body::resolve_field(
        connection,
        owner.conversation_id(),
        owner.message_id(),
        owner.part_id(),
        field,
    )?
    .total_utf8_bytes() else {
        return Ok(None);
    };
    let mut digest = Sha256::new();
    let mut offset = 0;
    while offset < total {
        let (current_total, range) = crate::published_part_body::field_range(
            connection,
            owner.conversation_id(),
            owner.message_id(),
            owner.part_id(),
            field,
            offset,
            32768,
        )?;
        if current_total != total
            || range.offset_bytes != offset
            || range.next_offset_bytes <= offset
        {
            return Err(invalid("branch source field changed during fingerprinting"));
        }
        digest.update(range.text.as_bytes());
        offset = range.next_offset_bytes;
    }
    Ok(Some(PartFieldFingerprint {
        field,
        total_utf8_bytes: total,
        sha256: format!("{:x}", digest.finalize()),
    }))
}
pub(super) fn source(connection: &Connection, part: &str) -> StoreResult<PartDerivationSource> {
    let (conversation, message, run_id, kind, state): (
        String,
        String,
        Option<String>,
        String,
        String,
    ) = connection.query_row(
        "SELECT CASE WHEN length(CAST(m.conversation_id AS BLOB))<=128 THEN m.conversation_id END,
         CASE WHEN length(CAST(m.id AS BLOB))<=128 THEN m.id END,
         CASE WHEN length(CAST(m.run_id AS BLOB))<=128 THEN m.run_id END,
         CASE WHEN length(CAST(p.kind AS BLOB))<=128 THEN p.kind END,
         CASE WHEN length(CAST(p.state AS BLOB))<=128 THEN p.state END
         FROM message_parts p JOIN messages m ON m.id=p.message_id
         WHERE p.id=?1 AND (m.run_id IS NULL OR length(CAST(m.run_id AS BLOB))<=128)",
        params![part],
        |row| {
            Ok((
                row.get(0)?,
                row.get(1)?,
                row.get(2)?,
                row.get(3)?,
                row.get(4)?,
            ))
        },
    )?;
    let owner = CanonicalPartOwner::new(conversation, message, part.into())
        .map_err(|e| invalid(&e.to_string()))?;
    let fields = [
        fingerprint(connection, &owner, PartBodyField::ContentText)?,
        fingerprint(connection, &owner, PartBodyField::ContentJson)?,
    ];
    Ok(PartDerivationSource {
        owner,
        run_id,
        kind,
        state,
        fields,
    })
}
pub(super) fn validate_shape(record: &PartDerivation) -> StoreResult<()> {
    if record.version != 1 || record.operation_id.is_empty() || record.operation_id.len() > 128 {
        return Err(invalid(
            "invalid branch Part derivation version or operation",
        ));
    }
    for group in [&record.target, &record.immediate_source]
        .into_iter()
        .chain(record.root_source.iter())
    {
        if group.kind.len() > 128
            || group.state.len() > 128
            || group.run_id.as_ref().is_some_and(|v| v.len() > 128)
        {
            return Err(invalid("invalid branch Part derivation metadata"));
        }
        for (value, field) in group
            .fields
            .iter()
            .zip([PartBodyField::ContentText, PartBodyField::ContentJson])
        {
            if let Some(value) = value {
                if value.field != field
                    || value.sha256.len() != 64
                    || !value
                        .sha256
                        .bytes()
                        .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
                {
                    return Err(invalid("invalid branch Part fingerprint"));
                }
            }
        }
    }
    if record.target.fields != record.immediate_source.fields
        || record.target.kind != record.immediate_source.kind
        || record.target.state != record.immediate_source.state
        || record.root_source.as_ref().is_some_and(|root| {
            record.target.fields != root.fields
                || record.target.kind != root.kind
                || record.target.state != root.state
        })
    {
        return Err(invalid(
            "branch Part derivation does not preserve original fields and metadata",
        ));
    }
    Ok(())
}
pub(super) fn load(
    connection: &Connection,
    current: &PartDerivationSource,
) -> StoreResult<Option<StoredPartDerivation>> {
    let stored: Option<(String, String, String)> = connection.query_row(
        "SELECT operation_id,CASE WHEN length(CAST(derivation_json AS BLOB))<=16384 THEN derivation_json END,provenance
         FROM message_part_derivations WHERE target_part_id=?1", params![current.owner.part_id()],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?))).optional()?;
    let Some((operation, encoded, provenance)) = stored else {
        return Ok(None);
    };
    let record: PartDerivation = serde_json::from_str(&encoded)?;
    validate_shape(&record)?;
    if record.operation_id != operation || record.target != *current {
        return Err(invalid(
            "branch Part derivation no longer matches canonical target",
        ));
    }
    let provenance = match provenance.as_str() {
        "local_branch" => PartDerivationProvenance::LocalBranch,
        "replica" => PartDerivationProvenance::Replica,
        "archive" => PartDerivationProvenance::Archive,
        _ => return Err(invalid("unknown Part derivation provenance")),
    };
    Ok(Some(StoredPartDerivation {
        derivation: record,
        provenance,
    }))
}

impl NolaStore {
    /// Receiver-owned origin accompanies the claims and cannot be supplied over the wire.
    pub fn part_derivation_record(
        &self,
        owner: &CanonicalPartOwner,
    ) -> StoreResult<Option<StoredPartDerivation>> {
        let _snapshot = crate::read_snapshot::pin(&self.connection)?;
        let current = source(&self.connection, owner.part_id())?;
        if current.owner != *owner {
            return Err(invalid("branch Part derivation owner mismatch"));
        }
        load(&self.connection, &current)
    }
}

/// Called only by canonical branch after copying its message, inside that writer's transaction.
pub(super) fn record_branch_message(
    connection: &Connection,
    operation: &str,
    source_message: &str,
    target_message: &str,
) -> StoreResult<()> {
    if connection.is_autocommit() {
        return Err(invalid("branch provenance requires its writer transaction"));
    }
    let ids = |message: &str| -> StoreResult<Vec<String>> {
        let mut statement = connection.prepare(
            "SELECT CASE WHEN length(CAST(id AS BLOB))<=128 THEN id END FROM message_parts WHERE message_id=?1 ORDER BY ordinal LIMIT 513",
        )?;
        let rows = statement.query_map(params![message], |row| row.get(0))?;
        let result = rows.collect::<Result<Vec<String>, _>>()?;
        if result.len() > 512 {
            return Err(invalid("branch provenance Part count exceeds budget"));
        }
        Ok(result)
    };
    let originals = ids(source_message)?;
    let targets = ids(target_message)?;
    if originals.len() != targets.len() {
        return Err(invalid("branch provenance Part count differs"));
    }
    for (original, target) in originals.iter().zip(&targets) {
        let immediate_source = source(connection, original)?;
        let target = source(connection, target)?;
        let mut provenance = PartDerivationProvenance::LocalBranch;
        let root_source = if let Some(previous) = load(connection, &immediate_source)? {
            provenance = previous.provenance;
            previous.derivation.root_source
        } else {
            // A replicated/legacy branch may have only conversation lineage.
            // Preserve raw branch behavior without inventing a newly trusted root.
            let known_derived: bool = connection.query_row(
                "SELECT EXISTS(SELECT 1 FROM conversation_branch_operations WHERE json_extract(intent_json,'$.conversation.id')=?1)",
                params![immediate_source.owner.conversation_id()], |row| row.get(0))?;
            if known_derived {
                None
            } else {
                Some(immediate_source.clone())
            }
        };
        let record = PartDerivation {
            version: 1,
            operation_id: operation.into(),
            target,
            immediate_source,
            root_source,
        };
        validate_shape(&record)?;
        let encoded = serde_json::to_string(&record)?;
        if encoded.len() > 16384 {
            return Err(invalid("branch Part derivation exceeds byte budget"));
        }
        if let Some(existing) = load(connection, &record.target)? {
            if existing.derivation != record {
                return Err(invalid("branch Part derivation replay differs"));
            }
        } else {
            connection.execute("INSERT INTO message_part_derivations(target_part_id,operation_id,derivation_json,provenance) VALUES(?1,?2,?3,?4)",
                params![record.target.owner.part_id(), operation, encoded, provenance_name(provenance)])?;
        }
    }
    Ok(())
}

#[cfg(test)]
#[path = "part_derivation_tests.rs"]
mod tests;

pub(super) fn provenance_name(value: PartDerivationProvenance) -> &'static str {
    match value {
        PartDerivationProvenance::LocalBranch => "local_branch",
        PartDerivationProvenance::Replica => "replica",
        PartDerivationProvenance::Archive => "archive",
    }
}

/// Archive is an external claim source even when its bytes match a former local
/// branch. Only the final archive transaction can invoke this constructor.
pub(super) fn import_archive_claim(
    connection: &Connection,
    record: &PartDerivation,
) -> StoreResult<()> {
    if connection.is_autocommit() {
        return Err(invalid(
            "archive Part derivation requires its final writer transaction",
        ));
    }
    validate_shape(record)?;
    let encoded = serde_json::to_string(record)?;
    if encoded.len() > 16384 {
        return Err(invalid("archive Part derivation exceeds byte budget"));
    }
    let current = source(connection, record.target.owner.part_id())?;
    if current != record.target {
        return Err(invalid(
            "archive Part derivation target bytes or metadata differ",
        ));
    }
    if let Some(previous) = load(connection, &current)? {
        if previous.derivation != *record
            || previous.provenance != PartDerivationProvenance::Archive
        {
            return Err(invalid("archive Part derivation replay or origin differs"));
        }
        return Ok(());
    }
    connection.execute("INSERT INTO message_part_derivations(target_part_id,operation_id,derivation_json,provenance) VALUES(?1,?2,?3,'archive')",
        params![record.target.owner.part_id(),record.operation_id,encoded])?;
    Ok(())
}
