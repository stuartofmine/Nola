-- Cumulative byte endpoints permit logarithmic seek without walking prior chunks.
CREATE INDEX assistant_journal_manifest_byte_range
ON assistant_journal_manifests(owner,part_id,field,generation,byte_count);
