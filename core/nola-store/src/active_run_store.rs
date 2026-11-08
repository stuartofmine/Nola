// SPDX-License-Identifier: Apache-2.0
//! Active run and immutable conversation/assistant ownership lookup.

use super::*;
#[cfg(test)]
#[path = "../tests/support/terminal_sources.rs"]
mod terminal_sources;

impl NolaStore {
    pub fn canonical_run_terminal(
        &self,
        conversation_id: &str,
        run_id: &str,
    ) -> StoreResult<Option<nola_store_port::remote_session::CanonicalRunTerminal>> {
        use nola_store_port::remote_session::{CanonicalRunTerminal, CanonicalRunTerminalStatus};
        require_id("conversation_id", conversation_id)?;
        require_id("run_id", run_id)?;
        let _snapshot = crate::read_snapshot::pin(&self.connection)?;
        let row: Option<(String,String,String,i64)> = self
            .connection
            .query_row(
                "SELECT conversation_id,id,assistant_message_id,completed_at_ms FROM runs
             WHERE conversation_id=?1 AND id=?2 AND kind='conversation'
             AND status='sealed'
             AND completed_at_ms IS NOT NULL AND assistant_message_id IS NOT NULL",
                params![conversation_id, run_id],
                |row| Ok((row.get(0)?,row.get(1)?,row.get(2)?,row.get(3)?)),
            )
            .optional()?;
        let Some((conversation_id,run_id,assistant_message_id,completed_at_ms))=row else { return Ok(None) };
        let mut statement = self.connection.prepare(
            "SELECT seal_json,1,NULL FROM assistant_run_terminal_seals WHERE run_id=?1
             UNION ALL SELECT seal_json,manifest_complete,source_host_id FROM remote_assistant_run_terminal_stages WHERE run_id=?1 AND committed=1")?;
        let mut sources = statement.query([&run_id])?;
        let source = sources.next()?.ok_or_else(|| StoreError::Invariant(
            "sealed Run has no committed terminal source".into()))?;
        let raw: String = source.get(0)?;
        let manifest_complete: i64 = source.get(1)?;
        let source_host: Option<String> = source.get(2)?;
        if manifest_complete != 1 || sources.next()?.is_some() {
            return Err(StoreError::Invariant(
                "canonical terminal requires exactly one complete committed source".into()));
        }
        let seal:nola_domain::assistant_run_terminal::AssistantRunTerminalSeal=serde_json::from_str(&raw)?;
        seal.validate().map_err(StoreError::Invariant)?;
        crate::assistant_journal::validate_terminal_source(self, &seal, source_host.as_deref())?;
        if seal.identity.conversation_id!=conversation_id || seal.identity.run_id!=run_id
            || seal.identity.assistant_message_id!=assistant_message_id || seal.completed_at_ms!=completed_at_ms {
            return Err(StoreError::Invariant("canonical terminal differs from its immutable result seal".into()));
        }
        Ok(Some(CanonicalRunTerminal { conversation_id,run_id,assistant_message_id,completed_at_ms,
            status:CanonicalRunTerminalStatus::Sealed,result:seal.result,result_digest:seal.result_digest }))
    }

    pub fn active_run(&self, conversation_id: &str) -> StoreResult<Option<StoredActiveRun>> {
        Ok(self
            .connection
            .query_row(
                "SELECT r.id, r.kind, r.assistant_message_id, p.id, r.next_event_sequence,
                        r.invocation_config_digest
                 FROM runs r
                 LEFT JOIN canonical_run_initial_parts initial ON initial.run_id = r.id
                 LEFT JOIN message_parts p ON p.id = initial.part_id
                    AND p.message_id = r.assistant_message_id
                 WHERE r.conversation_id = ?1
                   AND r.status<>'sealed'
                 ORDER BY r.created_at_ms DESC
                 LIMIT 1",
                params![conversation_id],
                |row| {
                    Ok(StoredActiveRun {
                        run_id: row.get(0)?,
                        kind: row.get(1)?,
                        assistant_message_id: row.get(2)?,
                        assistant_part_id: row.get(3)?,
                        next_event_sequence: row.get(4)?,
                        invocation_config_digest: row.get(5)?,
                    })
                },
            )
            .optional()?)
    }

    pub fn conversation_id_for_run(&self, run_id: &str) -> StoreResult<Option<String>> {
        require_id("run_id", run_id)?;
        Ok(self
            .connection
            .query_row(
                "SELECT conversation_id FROM runs WHERE id = ?1",
                params![run_id],
                |row| row.get(0),
            )
            .optional()?)
    }

    /// Resolves immutable assistant ownership for one already-opened Run.
    /// Terminal Runs remain addressable for idempotent Host projection replay,
    /// while persistence row structure stays private to Store.
    pub fn assistant_message_id_for_run(
        &self,
        run_id: &str,
        conversation_id: &str,
    ) -> StoreResult<Option<String>> {
        Ok(self
            .connection
            .query_row(
                "SELECT r.assistant_message_id
                 FROM runs r
                 WHERE r.id = ?1 AND r.conversation_id = ?2",
                params![run_id, conversation_id],
                |row| row.get(0),
            )
            .optional()?)
    }
}

#[cfg(test)]
mod canonical_terminal_tests {
    use super::*;

    #[test]
    fn active_run_never_returns_an_initial_part_from_another_message() {
        let store = NolaStore::open(":memory:").unwrap();
        store.connection.execute_batch("INSERT INTO conversations(id,title,revision,created_at_ms,updated_at_ms) VALUES('c','fixture',1,1,1);
            INSERT INTO messages(id,conversation_id,run_id,role,status,ordinal,created_at_ms)
            VALUES('a','c','r','assistant','streaming',1,1),
                  ('foreign','c','other','assistant','completed',2,1);
            INSERT INTO runs(id,conversation_id,kind,assistant_message_id,status,next_event_sequence,created_at_ms,updated_at_ms)
            VALUES('r','c','conversation','a','running',1,1,1);
            INSERT INTO message_parts(id,message_id,kind,ordinal,state,content_text,created_at_ms,updated_at_ms)
            VALUES('own-part','a','text',1,'open','',1,1),
                  ('foreign-part','foreign','text',1,'completed','foreign',1,1);
            INSERT INTO canonical_run_initial_parts(run_id,part_id) VALUES('r','foreign-part');").unwrap();
        let active = store.active_run("c").unwrap().unwrap();
        assert_eq!(active.assistant_message_id.as_deref(), Some("a"));
        assert!(active.assistant_part_id.is_none());
        store
            .connection
            .execute(
                "UPDATE canonical_run_initial_parts SET part_id='own-part' WHERE run_id='r'",
                [],
            )
            .unwrap();
        assert_eq!(
            store
                .active_run("c")
                .unwrap()
                .unwrap()
                .assistant_part_id
                .as_deref(),
            Some("own-part")
        );
        store
            .connection
            .execute(
                "DELETE FROM canonical_run_initial_parts WHERE run_id='r'",
                [],
            )
            .unwrap();
        assert!(
            store
                .active_run("c")
                .unwrap()
                .unwrap()
                .assistant_part_id
                .is_none(),
            "absence never falls back to the message's first Part"
        );
    }

    #[test]
    fn terminal_evidence_requires_exact_run_and_committed_terminal_row() {
        let mut store = NolaStore::open(":memory:").unwrap();
        let mut admission = crate::input_facts::tests::input(vec![]);
        admission.conversation_id = "c".into();
        admission.run_id = "r".into();
        admission.user_message_id = "u".into();
        admission.assistant_message_id = "a".into();
        admission.assistant_turn_bindings = vec![nola_domain::AssistantTurnBinding::Admission {
            turn_id:"r".into(),assistant_message_id:"a".into(),
        }];
        admission.checkpoint_event_cursor = 0;
        admission.checkpoint_json = "{}".into();
        store.begin_turn(&admission).unwrap();
        store.connection.execute("UPDATE messages SET status='completed' WHERE id='a'", []).unwrap();
        assert!(
            store.canonical_run_terminal("c", "r").unwrap().is_none(),
            "completed message is not a terminal Run"
        );
        for status in ["interrupted", "completed", "failed", "cancelled"] {
            store
                .connection
                .execute("UPDATE runs SET status=?1,completed_at_ms=NULL", [status])
                .unwrap();
            assert!(store.canonical_run_terminal("c", "r").unwrap().is_none());
            store
                .connection
                .execute("UPDATE runs SET completed_at_ms=42", [])
                .unwrap();
            let evidence = store.canonical_run_terminal("c", "r").unwrap();
            assert!(evidence.is_none(), "a status/timestamp without a canonical seal is not evidence");
            assert!(store
                .canonical_run_terminal("other", "r")
                .unwrap()
                .is_none());
            assert!(store
                .canonical_run_terminal("c", "other")
                .unwrap()
                .is_none());
        }
        store.connection.execute("UPDATE runs SET status='sealed',completed_at_ms=42", []).unwrap();
        assert!(store.canonical_run_terminal("c", "r").is_err(), "even a sealed label needs its committed seal");
        store.connection.execute("UPDATE runs SET status='running',completed_at_ms=NULL", []).unwrap();
        terminal_sources::settle_fixture(&mut store, "c", "r", "r", "u");
        let evidence = store.canonical_run_terminal("c", "r").unwrap().unwrap();
        assert_eq!(evidence.run_id, "r");
        assert_eq!(evidence.conversation_id, "c");
        assert_eq!(evidence.assistant_message_id, "a");
        assert_eq!(evidence.completed_at_ms, 2);
        assert_eq!(serde_json::to_value(evidence.status).unwrap(), "sealed");
        // An uncommitted remote stage is not another terminal authority. Once
        // committed, even byte-identical dual sources are ambiguous, not a
        // reason to silently prefer whichever UNION branch appears first.
        store.connection.execute_batch("SAVEPOINT conflicting_source").unwrap();
        store.connection.execute(
            "INSERT INTO remote_assistant_run_terminal_stages(run_id,source_host_id,seal_json,manifest_complete,committed)
             SELECT run_id,'fixture-remote',seal_json,1,0 FROM assistant_run_terminal_seals WHERE run_id='r'", []
        ).unwrap();
        assert!(store.canonical_run_terminal("c", "r").unwrap().is_some());
        store.connection.execute("UPDATE remote_assistant_run_terminal_stages SET committed=1 WHERE run_id='r'", []).unwrap();
        assert!(store.canonical_run_terminal("c", "r").is_err());
        store.connection.execute_batch("ROLLBACK TO conflicting_source; RELEASE conflicting_source").unwrap();
        assert!(store.canonical_run_terminal("c", "r").unwrap().is_some());
        // Deliberately corrupt only this isolated fixture to exercise an
        // incomplete remote source without a local source masking the error.
        store.connection.execute_batch("SAVEPOINT incomplete_remote_source").unwrap();
        store.connection.execute(
            "INSERT INTO remote_assistant_run_terminal_stages(run_id,source_host_id,seal_json,manifest_complete,committed)
             SELECT run_id,'fixture-remote',seal_json,0,1 FROM assistant_run_terminal_seals WHERE run_id='r'", []
        ).unwrap();
        store.connection.execute("DELETE FROM assistant_run_terminal_seals WHERE run_id='r'", []).unwrap();
        assert!(store.canonical_run_terminal("c", "r").is_err());
        store.connection.execute("UPDATE remote_assistant_run_terminal_stages SET manifest_complete=1 WHERE run_id='r'", []).unwrap();
        assert!(store.canonical_run_terminal("c", "r").is_err(),
            "a valid copied seal and complete label do not establish remote Host ownership");
        store.connection.execute_batch("ROLLBACK TO incomplete_remote_source; RELEASE incomplete_remote_source").unwrap();
        assert!(store.canonical_run_terminal("c", "r").unwrap().is_some());
        store
            .connection
            .execute("UPDATE runs SET kind='maintenance'", [])
            .unwrap();
        assert!(store.canonical_run_terminal("c", "r").unwrap().is_none());
    }
}
