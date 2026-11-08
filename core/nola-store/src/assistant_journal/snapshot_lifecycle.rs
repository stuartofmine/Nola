// SPDX-License-Identifier: Apache-2.0
//! Exact-owner snapshot release. Content GC is deliberately a separate contract.
use super::*;

fn proof(c: &Connection, s: &AssistantJournalSnapshot) -> StoreResult<(String, bool)> {
    let key = validate_read_owner(c, &s.at.identity)?;
    let released:Option<bool>=c.query_row(
        "SELECT EXISTS(SELECT 1 FROM assistant_journal_snapshot_releases r WHERE r.snapshot_id=s.id)
         FROM assistant_journal_snapshots s WHERE s.id=?1 AND s.owner=?2 AND s.sequence=?3 AND s.digest=?4",
        params![s.snapshot_id,key,number(s.at.sequence)?,s.at.record_digest.to_hex()],|r|r.get(0)
    ).optional()?;
    Ok((
        key,
        released.ok_or_else(|| invalid("snapshot proof unavailable"))?,
    ))
}

pub(super) fn active_key(c: &Connection, s: &AssistantJournalSnapshot) -> StoreResult<String> {
    let (key, released) = proof(c, s)?;
    if released {
        return Err(invalid("snapshot has been released"));
    }
    Ok(key)
}

pub(super) fn release(
    store: &mut NolaStore,
    s: &AssistantJournalSnapshot,
) -> StoreResult<AssistantSnapshotRelease> {
    require_uow(store)?;
    let tx = store.write_transaction()?;
    let (_, released) = proof(&tx, s)?;
    if released {
        tx.commit()?;
        return Ok(AssistantSnapshotRelease::AlreadyReleased);
    }
    tx.execute(
        "INSERT INTO assistant_journal_snapshot_releases(snapshot_id) VALUES(?1)",
        [&s.snapshot_id],
    )?;
    tx.commit()?;
    Ok(AssistantSnapshotRelease::Released)
}
