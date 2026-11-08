import { journalIdentityKey } from './assistantJournal.ts';
import type { JournalIdentity, JournalTimelinePart, JournalUpdate } from './assistantJournal.ts';

/** Materialized reader output for reducer-only tests; transport tests use the reader. */
export function journalUpdate(sequence: string, identity: JournalIdentity, values: Record<string, unknown>[], sourceHostId = 'host'): JournalUpdate {
    return { sourceHostId, through: { identity, sequence, record_digest: BigInt(sequence).toString(16).padStart(64, '0') }, initial: false,
        changes: values.map((value, index) => ({ partId: String(value.part_id), part: journalPart(identity, value, String(index)) })) };
}
export function journalPart(identity: JournalIdentity, value: Record<string, unknown>, position = '0'): JournalTimelinePart {
    return { source: 'journal', id: String(value.part_id), journalOwner: journalIdentityKey(identity),
        order: { section: 'transcript', position }, value: { ...value, kind: String(value.kind), part_id: String(value.part_id), revision: String(value.revision ?? '1') } };
}
