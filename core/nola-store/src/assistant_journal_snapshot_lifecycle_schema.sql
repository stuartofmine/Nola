-- Keep the original exact snapshot proof for idempotent release. This is not GC.
CREATE TABLE assistant_journal_snapshot_releases(
    snapshot_id TEXT PRIMARY KEY REFERENCES assistant_journal_snapshots(id) ON DELETE CASCADE
);
