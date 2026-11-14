// SPDX-License-Identifier: Apache-2.0
//! Conversation summary listing and identity-scoped lookup.

use super::*;

impl NolaStore {
    pub fn list_conversations(&self) -> StoreResult<Vec<ConversationSummary>> {
        self.conversation_summaries(None, None, false)
    }

    pub fn list_conversation_page(&self, query: &nola_store_port::conversation_query::ConversationListQuery) -> StoreResult<Vec<ConversationSummary>> {
        if query.limit == 0 || query.limit > 1024 || query.offset > i64::MAX as usize {
            return Err(StoreError::Invariant("invalid conversation page bounds".into()));
        }
        self.conversation_summaries(query.conversation_id.as_deref(), Some(query), false)
    }

    pub fn list_root_conversation_page(&self, query: &nola_store_port::conversation_query::ConversationListQuery) -> StoreResult<Vec<ConversationSummary>> {
        if query.limit == 0 || query.limit > 1024 || query.offset > i64::MAX as usize {
            return Err(StoreError::Invariant("invalid conversation page bounds".into()));
        }
        self.conversation_summaries(query.conversation_id.as_deref(), Some(query), true)
    }

    pub fn conversation_summary(
        &self,
        conversation_id: &str,
    ) -> StoreResult<Option<ConversationSummary>> {
        Ok(self.conversation_summaries(Some(conversation_id), None, false)?.pop())
    }

    fn conversation_summaries(
        &self,
        conversation_id: Option<&str>,
        page: Option<&nola_store_port::conversation_query::ConversationListQuery>,
        roots_only: bool,
    ) -> StoreResult<Vec<ConversationSummary>> {
        let _snapshot = read_snapshot::pin(&self.connection)?;
        let mut statement = self.connection.prepare(&format!(
            "SELECT c.id,
                    c.title,
                    c.revision,
                    (SELECT COUNT(*) FROM messages counted WHERE counted.conversation_id = c.id),
                    c.created_at_ms,
                    c.updated_at_ms,
                    json_extract(settings.settings_json, '$.project_root'),
                    NULL,
                    (SELECT last_user.created_at_ms
                       FROM messages last_user
                      WHERE last_user.conversation_id = c.id
                        AND last_user.role = 'user'
                      ORDER BY last_user.ordinal DESC
                      LIMIT 1)
             FROM conversations c
             LEFT JOIN conversation_settings settings ON settings.conversation_id = c.id
             WHERE (?1 IS NULL OR c.id = ?1)
               AND (?2 IS NULL OR json_extract(settings.settings_json, '$.project_root') = ?2)
               AND (?3 = 0 OR EXISTS(SELECT 1 FROM messages present WHERE present.conversation_id = c.id))
               AND (?6=0 OR (json_type(settings.settings_json,'$.agentRoomOwner') IS NULL
                    AND NOT EXISTS(SELECT 1 FROM durable_tasks t WHERE t.kind='agent' AND t.child_session_id=c.id)
                    AND NOT EXISTS(SELECT 1 FROM runtime_task_run_bindings b WHERE b.child_session_id=c.id)
                    AND NOT EXISTS(SELECT 1 FROM remote_conversation_execution_owners o JOIN runs r ON r.id=o.run_id
                        WHERE r.conversation_id=c.id AND json_extract(o.ownership_json,'$.kind')='task_child')))
             ORDER BY c.updated_at_ms DESC, c.revision DESC, c.rowid DESC, c.id ASC
             LIMIT ?4 OFFSET ?5",
        ))?;
        let rows = statement.query_map(rusqlite::params![conversation_id,
            page.and_then(|p| p.project_root.as_deref()), page.is_some_and(|p| p.non_empty),
            page.map(|p| p.limit as i64).unwrap_or(-1), page.map(|p| p.offset as i64).unwrap_or(0), roots_only], |row| {
            Ok(ConversationSummary {
                id: row.get(0)?,
                title: row.get(1)?,
                revision: row.get(2)?,
                message_count: row.get(3)?,
                created_at_ms: row.get(4)?,
                updated_at_ms: row.get(5)?,
                project_root: row.get(6)?,
                first_user_message: row.get(7)?,
                last_user_message_at_ms: row.get(8)?,
            })
        })?;
        let mut summaries = rows.collect::<Result<Vec<_>, _>>()?;
        for summary in &mut summaries {
            let first: Option<(String, String)> = self.connection.query_row(
                "SELECT m.id,p.id FROM messages m JOIN message_parts p ON p.message_id=m.id
                 WHERE m.conversation_id=?1 AND m.role='user'
                   AND COALESCE((SELECT e.visible FROM message_projection_events e
                     JOIN conversation_projection_operations o ON o.operation_id=e.operation_id
                     WHERE e.conversation_id=m.conversation_id AND e.message_id=m.id
                     ORDER BY o.resulting_revision DESC LIMIT 1),1)=1
                 ORDER BY m.ordinal,p.ordinal LIMIT 1",
                [&summary.id], |row| Ok((row.get(0)?, row.get(1)?)),
            ).optional()?;
            if let Some((message, part)) = first {
                summary.first_user_message = Some(
                    crate::canonical_field_reader::CanonicalFieldReader::open(
                        &self.connection, &summary.id, &message, &part,
                        nola_domain::PartBodyField::ContentText,
                    )?.read_text_prefix(512)?,
                );
            }
        }
        Ok(summaries)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(sealed: bool) -> NolaStore {
        let store = NolaStore::open(std::path::Path::new(":memory:")).unwrap();
        store.connection.execute_batch("INSERT INTO conversations(id,title,revision,created_at_ms,updated_at_ms) VALUES('c','Summary',1,1,1);
            INSERT INTO messages VALUES('m','c',NULL,'user','completed',1,1,1);
            INSERT INTO message_parts(id,message_id,kind,ordinal,state,content_text,created_at_ms,updated_at_ms)
            VALUES('p','m','text',1,'completed','',1,1);").unwrap();
        let text = "你🙂".repeat(400);
        if sealed {
            let reference = nola_domain::CanonicalBodyReference::for_field(
                "body".into(), "c".into(), "m".into(), "p".into(), nola_domain::PartBodyField::ContentText, text.len() as u64, 1,
            ).unwrap();
            let encoded = serde_json::to_string(&reference).unwrap();
            store.connection.execute("INSERT INTO canonical_text_objects VALUES('body',?1)", [&encoded]).unwrap();
            store.connection.execute("INSERT INTO canonical_text_chunks VALUES('body',0,?1)", [&text]).unwrap();
            store.connection.execute("INSERT INTO message_part_body_references VALUES('p','content_text',?1)", [&encoded]).unwrap();
        } else {
            store.connection.execute("UPDATE message_parts SET content_text=?1 WHERE id='p'", [&text]).unwrap();
        }
        store
    }

    #[test]
    fn summary_preview_uses_original_field_for_inline_and_sealed_unicode() {
        for sealed in [false, true] {
            let store = fixture(sealed);
            let summary = store.conversation_summary("c").unwrap().unwrap();
            assert_eq!(summary.first_user_message, Some("你🙂".repeat(256)));
            assert_eq!(store.list_conversations().unwrap(), vec![summary]);
        }
    }

    #[test]
    fn summary_skips_hidden_first_user_without_reading_its_body() {
        let store = fixture(true);
        store.connection.execute_batch("INSERT INTO messages VALUES('next','c',NULL,'user','completed',2,2,2);
            INSERT INTO message_parts(id,message_id,kind,ordinal,state,content_text,created_at_ms,updated_at_ms)
            VALUES('next-p','next','text',1,'completed','Visible',2,2);
            INSERT INTO conversation_projection_operations VALUES('hide','c','delete',NULL,2,2,3);
            INSERT INTO message_projection_events VALUES('hidden','hide','c','m',0,'deleted',3);").unwrap();
        assert_eq!(store.conversation_summary("c").unwrap().unwrap().first_user_message.as_deref(), Some("Visible"));
    }

    #[test]
    fn summary_rejects_broken_binding_instead_of_returning_empty_preview() {
        let store = fixture(true);
        store.connection.execute("UPDATE message_parts SET content_text='conflicting inline' WHERE id='p'", []).unwrap();
        assert!(store.conversation_summary("c").is_err());
    }

    #[test]
    fn page_filters_before_touching_unselected_or_out_of_page_bodies() {
        use nola_store_port::conversation_query::ConversationListQuery;
        let store = fixture(true);
        // Broken authority must fail if selected, but cannot force a reader to
        // hydrate an unrelated project or a row outside its requested page.
        store.connection.execute_batch("UPDATE message_parts SET content_text='conflicting' WHERE id='p';
            INSERT INTO conversations(id,title,revision,created_at_ms,updated_at_ms) VALUES('new','New',1,2,2);
            INSERT INTO messages VALUES('new-m','new',NULL,'user','completed',1,2,2);
            INSERT INTO message_parts(id,message_id,kind,ordinal,state,content_text,created_at_ms,updated_at_ms)
            VALUES('new-p','new-m','text',1,'completed','Visible page',2,2);").unwrap();
        let mut query = ConversationListQuery { conversation_id: None, project_root: None, non_empty: true, limit: 1, offset: 0 };
        let page = store.list_conversation_page(&query).unwrap();
        assert_eq!(page.len(), 1);
        assert_eq!(page[0].id, "new");
        assert_eq!(page[0].first_user_message.as_deref(), Some("Visible page"));
        query.offset = 1;
        assert!(store.list_conversation_page(&query).is_err(), "selected broken preview cannot be hidden");
        query.offset = 0;
        query.project_root = Some("unrelated-project".into());
        assert!(store.list_conversation_page(&query).unwrap().is_empty());
        store.connection.execute("INSERT INTO conversation_settings VALUES('new',?1,2)",
            [r#"{"project_root":"unrelated-project"}"#]).unwrap();
        assert_eq!(store.list_conversation_page(&query).unwrap()[0].id, "new");
        query.project_root = None;
        query.conversation_id = Some("new".into());
        assert_eq!(store.list_conversation_page(&query).unwrap()[0].id, "new");
    }

    #[test]
    fn explicit_pages_enumerate_beyond_one_page_and_reject_unbounded_requests() {
        use nola_store_port::conversation_query::ConversationListQuery;
        let store = NolaStore::open(std::path::Path::new(":memory:")).unwrap();
        store.connection.execute_batch("WITH RECURSIVE n(x) AS (VALUES(1) UNION ALL SELECT x+1 FROM n WHERE x<1025)
            INSERT INTO conversations(id,title,revision,created_at_ms,updated_at_ms) SELECT printf('c%04d',x),'Page',1,x,x FROM n;").unwrap();
        let mut query = ConversationListQuery { conversation_id: None, project_root: None, non_empty: false, limit: 1024, offset: 0 };
        let first = store.list_conversation_page(&query).unwrap();
        query.offset = first.len();
        let last = store.list_conversation_page(&query).unwrap();
        assert_eq!(first.len(), 1024);
        assert_eq!(last.len(), 1);
        assert!(!first.iter().any(|row| row.id == last[0].id));
        query.offset += last.len();
        assert!(store.list_conversation_page(&query).unwrap().is_empty());
        query.non_empty = true;
        query.offset = 0;
        assert!(store.list_conversation_page(&query).unwrap().is_empty());
        query.limit = 0;
        assert!(store.list_conversation_page(&query).is_err());
        query.limit = 1025;
        assert!(store.list_conversation_page(&query).is_err());
    }
}
