// SPDX-License-Identifier: Apache-2.0
//! Immutable presentation directory; retired Parts keep their seek position.
use super::*;

pub(crate) fn initialize(c: &Connection) -> StoreResult<()> {
    c.execute_batch("CREATE TABLE assistant_journal_order (
        owner TEXT NOT NULL, part_id TEXT NOT NULL, section INTEGER NOT NULL,
        position INTEGER NOT NULL CHECK(position>=0), PRIMARY KEY(owner,part_id));
        CREATE INDEX assistant_journal_presentation_order
        ON assistant_journal_order(owner,section,position,part_id);")?;
    super::initialize_owner_index(c)
}

pub(super) fn record(c: &Connection, owner: &str, batch: &AssistantOperationBatchV1) -> StoreResult<()> {
    for op in &batch.operations {
        match op {
            AssistantJournalOperationV1::Declare(value) => {
                let part = &value.part;
                c.execute("INSERT INTO assistant_journal_order(owner,part_id,section,position) VALUES(?1,?2,?3,?4)",
                    params![owner,part.part_id,part.order_key.section as i64,number(part.order_key.position.0)?])?;
            }
            AssistantJournalOperationV1::Replace(value) => {
                let part = &value.part;
                let matches: bool = c.query_row("SELECT EXISTS(SELECT 1 FROM assistant_journal_order
                    WHERE owner=?1 AND part_id=?2 AND section=?3 AND position=?4)",
                    params![owner,part.part_id,part.order_key.section as i64,number(part.order_key.position.0)?], |r|r.get(0))?;
                if !matches { return Err(invalid("Part replacement changed its immutable presentation order")); }
            }
            _ => {}
        }
    }
    Ok(())
}
