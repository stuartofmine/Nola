// SPDX-License-Identifier: Apache-2.0
//! Durable surfaces, with owner checks and revision CAS on the canonical connection.
use super::*;
use nola_domain::generative_ui::*;
use nola_store_port::generative_ui::GenerativeUiRepository;

fn identity_key(identity: &UiSurfaceIdentity) -> StoreResult<String> {
    identity.validate().map_err(|error| StoreError::Invariant(error.to_string()))?;
    Ok(format!("{:x}", Sha256::digest(serde_json::to_vec(identity)?)))
}

impl NolaStore {
    pub(crate) fn context_ui_surfaces(&self, context_id: &str) -> StoreResult<Vec<UiSurfaceState>> {
        // New conversations have no Surface state yet. Existing ones must be
        // active, including task-owned contexts, before any model data leaves.
        let exists: bool = self.connection.query_row(
            "SELECT EXISTS(SELECT 1 FROM conversations WHERE id=?1)",
            [context_id], |row| row.get(0))?;
        if !exists { return Ok(Vec::new()); }
        crate::agent_room_admission::require_active_context(&self.connection, context_id)?;
        let settings: Option<String> = self.connection.query_row(
            "SELECT settings_json FROM conversation_settings WHERE conversation_id=?1",
            [context_id], |row| row.get(0)).optional()?;
        let settings: serde_json::Value = settings.map(|raw| serde_json::from_str(&raw)).transpose()?.unwrap_or_default();
        let participant_id = if let Some(owner) = settings.get(nola_domain::agent_room::CONTEXT_OWNER_FIELD) {
            serde_json::from_value::<nola_domain::agent_room::ContextOwner>(owner.clone())?.participant_id
        } else { "assistant".into() };
        let authority = self.ui_authority_id()?;
        let surfaces = self.list_ui_surfaces(&authority, context_id)?;
        Ok(surfaces.into_iter().filter(|state| {
            let owner = &state.identity.owner;
            owner.authority_id == authority && owner.conversation_id == context_id
                && owner.context_id == context_id && owner.participant_id == participant_id
        }).collect())
    }
}

impl GenerativeUiRepository for NolaStore {
    type Error = StoreError;
    fn ui_authority_id(&self) -> StoreResult<String> {
        self.connection.query_row("SELECT value FROM store_metadata WHERE key='assistant_source_epoch'", [], |row| row.get(0)).map_err(StoreError::from)
    }


    fn ui_tool_owner(&self, run_id: &str, turn_id: &str, call_id: &str) -> StoreResult<UiSurfaceOwner> {
        let mut query = self.connection.prepare("SELECT m.conversation_id,m.id,p.id
            FROM message_parts p JOIN messages m ON m.id=p.message_id JOIN runs r ON r.id=m.run_id
            WHERE r.id=?1 AND r.completed_at_ms IS NULL AND m.role='assistant' AND p.kind='tool'
              AND json_extract(p.content_json,'$.call_id')=?2
              AND json_extract(p.content_json,'$.turn_id')=?3
              AND json_extract(p.content_json,'$.capability_id')='render_ui' LIMIT 2")?;
        let rows = query.query_map(params![run_id,call_id,turn_id], |row| Ok((row.get::<_,String>(0)?,row.get::<_,String>(1)?,row.get::<_,String>(2)?)))?
            .collect::<Result<Vec<_>,_>>()?;
        if rows.len() != 1 { return Err(StoreError::Conflict("UI tool lacks a unique active Part owner".into())); }
        let (conversation_id,message_id,part_id) = rows.into_iter().next().unwrap();
        crate::agent_room_admission::require_active_context(&self.connection, &conversation_id)?;
        let settings: Option<String> = self.connection.query_row("SELECT settings_json FROM conversation_settings WHERE conversation_id=?1",
            [&conversation_id], |row| row.get(0)).optional()?;
        let settings: serde_json::Value = settings.map(|raw| serde_json::from_str(&raw)).transpose()?.unwrap_or_default();
        let participant_id = if let Some(owner) = settings.get(nola_domain::agent_room::CONTEXT_OWNER_FIELD) {
            serde_json::from_value::<nola_domain::agent_room::ContextOwner>(owner.clone())?.participant_id
        } else { "assistant".into() };
        let authority_id = self.connection.query_row("SELECT value FROM store_metadata WHERE key='assistant_source_epoch'", [], |row| row.get(0))?;
        Ok(UiSurfaceOwner { authority_id, context_id: conversation_id.clone(), conversation_id,
            participant_id, originating_run_id: run_id.into(), message_id, part_id })
    }

    fn ui_tool_publication(&self, effect_id: &str) -> StoreResult<Option<(String, String)>> {
        self.connection.query_row("SELECT request_digest,result_json FROM ui_tool_publications WHERE effect_id=?1",
            [effect_id], |row| Ok((row.get(0)?,row.get(1)?))).optional().map_err(StoreError::from)
    }

    fn next_ui_generation(&self, owner: &UiSurfaceOwner, surface_id: &str) -> StoreResult<u64> {
        if self.connection.is_autocommit() {
            return Err(StoreError::Invariant("UI generation requires command transaction".into()));
        }
        let previous: i64 = self.connection.query_row(
            "SELECT COALESCE(MAX(json_extract(state_json,'$.identity.generation')),0)
             FROM ui_surfaces WHERE authority_id=?1 AND conversation_id=?2
             AND json_extract(state_json,'$.identity.owner.context_id')=?3
             AND json_extract(state_json,'$.identity.owner.participant_id')=?4
             AND json_extract(state_json,'$.identity.surface_id')=?5",
            params![owner.authority_id, owner.conversation_id, owner.context_id, owner.participant_id, surface_id],
            |row| row.get(0))?;
        previous.checked_add(1).filter(|next| *next > 0).map(|next| next as u64)
            .ok_or_else(|| StoreError::Invariant("UI generation exhausted".into()))
    }

    fn record_ui_tool_publication(&mut self, effect_id: &str, request_digest: &str, result_json: &str) -> StoreResult<()> {
        if self.connection.is_autocommit() { return Err(StoreError::Invariant("UI receipt requires command transaction".into())); }
        self.connection.execute("INSERT INTO ui_tool_publications(effect_id,request_digest,result_json) VALUES(?1,?2,?3)",
            params![effect_id,request_digest,result_json])?;
        Ok(())
    }

    fn read_ui_surface(&self, identity: &UiSurfaceIdentity) -> StoreResult<Option<UiSurfaceState>> {
        let key = identity_key(identity)?;
        let raw: Option<String> = self.connection.query_row(
            "SELECT state_json FROM ui_surfaces WHERE identity_key=?1 AND authority_id=?2 AND conversation_id=?3",
            params![key, identity.owner.authority_id, identity.owner.conversation_id], |row| row.get(0)).optional()?;
        raw.map(|raw| {
            let state: UiSurfaceState = serde_json::from_str(&raw)?;
            if state.identity != *identity { return Err(StoreError::Invariant("UI surface identity mismatch".into())); }
            Ok(state)
        }).transpose()
    }

    fn read_ui_surface_revision(&self, identity: &UiSurfaceIdentity, revision: u64) -> StoreResult<Option<UiSurfaceState>> {
        let key = identity_key(identity)?;
        let revision = i64::try_from(revision).map_err(|_| StoreError::Invariant("UI revision overflow".into()))?;
        let raw: Option<String> = self.connection.query_row(
            "SELECT r.state_json FROM ui_surface_revisions r JOIN ui_surfaces s ON s.identity_key=r.identity_key
             WHERE r.identity_key=?1 AND r.revision=?2 AND s.authority_id=?3 AND s.conversation_id=?4",
            params![key, revision, identity.owner.authority_id, identity.owner.conversation_id], |row| row.get(0)).optional()?;
        raw.map(|raw| {
            let state: UiSurfaceState = serde_json::from_str(&raw)?;
            if state.identity != *identity || state.revision != revision as u64 {
                return Err(StoreError::Invariant("UI historical version identity mismatch".into()));
            }
            Ok(state)
        }).transpose()
    }

    fn write_ui_surface(&mut self, expected_revision: u64, state: &UiSurfaceState) -> StoreResult<()> {
        if self.connection.is_autocommit() { return Err(StoreError::Invariant("UI surface requires command transaction".into())); }
        if expected_revision.checked_add(1) != Some(state.revision) { return Err(StoreError::Conflict("UI surface revision mismatch".into())); }
        let key = identity_key(&state.identity)?;
        let owner = &state.identity.owner;
        let revision = i64::try_from(state.revision).map_err(|_| StoreError::Invariant("UI revision overflow".into()))?;
        let expected = i64::try_from(expected_revision).map_err(|_| StoreError::Invariant("UI revision overflow".into()))?;
        let raw = serde_json::to_string(state)?;
        if raw.len() > UI_MAX_SURFACE_BYTES { return Err(StoreError::Invariant("UI surface exceeds budget".into())); }
        if expected_revision == 0
            && state.identity.generation != self.next_ui_generation(owner, &state.identity.surface_id)? {
            return Err(StoreError::Conflict("UI surface generation mismatch".into()));
        }
        crate::agent_room_admission::require_active_context(&self.connection, &owner.context_id)?;
        let transaction = self.write_transaction()?;
        let owns: bool = transaction.query_row(
            "SELECT EXISTS(SELECT 1 FROM message_parts p JOIN messages m ON m.id=p.message_id JOIN runs r ON r.id=m.run_id
             WHERE p.id=?1 AND m.id=?2 AND m.conversation_id=?3 AND r.id=?4 AND r.conversation_id=m.conversation_id AND m.role='assistant')",
            params![owner.part_id, owner.message_id, owner.conversation_id, owner.originating_run_id], |row| row.get(0))?;
        if !owns { return Err(StoreError::Conflict("UI surface has no owning assistant part".into())); }
        let source_epoch: String = transaction.query_row("SELECT value FROM store_metadata WHERE key='assistant_source_epoch'", [], |row| row.get(0))?;
        if owner.authority_id != source_epoch { return Err(StoreError::Conflict("UI surface belongs to another authority".into())); }
        let deleted = state.phase == UiSurfacePhase::Deleted;
        if expected_revision == 0 {
            let exists: bool = transaction.query_row(
                "SELECT EXISTS(SELECT 1 FROM ui_surfaces WHERE authority_id=?1 AND conversation_id=?2 AND deleted=0
                 AND json_extract(state_json,'$.identity.owner.context_id')=?3
                 AND json_extract(state_json,'$.identity.owner.participant_id')=?4
                 AND json_extract(state_json,'$.identity.surface_id')=?5)",
                params![owner.authority_id,owner.conversation_id,owner.context_id,owner.participant_id,state.identity.surface_id],
                |row| row.get(0))?;
            if exists { return Err(StoreError::Conflict("UI surface name is already active in this context".into())); }
            let count: i64 = transaction.query_row("SELECT COUNT(*) FROM ui_surfaces WHERE authority_id=?1 AND conversation_id=?2 AND deleted=0",
                params![owner.authority_id, owner.conversation_id], |row| row.get(0))?;
            if count >= UI_MAX_ACTIVE_SURFACES as i64 { return Err(StoreError::Conflict("UI active surface limit".into())); }
            transaction.execute("INSERT INTO ui_surfaces(identity_key,authority_id,conversation_id,run_id,message_id,part_id,revision,deleted,state_json) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9)",
                params![key,owner.authority_id,owner.conversation_id,owner.originating_run_id,owner.message_id,owner.part_id,revision,deleted,raw])?;
        } else {
            let changed = transaction.execute("UPDATE ui_surfaces SET revision=?1,deleted=?2,state_json=?3 WHERE identity_key=?4 AND revision=?5 AND deleted=0",
                params![revision,deleted,raw,key,expected])?;
            if changed != 1 { return Err(StoreError::Conflict("UI surface revision conflict or deleted".into())); }
        }
        transaction.execute("INSERT INTO ui_surface_revisions(identity_key,revision,state_json) VALUES(?1,?2,?3)",
            params![key, revision, raw])?;
        transaction.commit()
    }

    fn write_ui_surface_part(&mut self, snapshot: &UiSnapshot, occurred_at_ms: i64) -> StoreResult<()> {
        if self.connection.is_autocommit() || occurred_at_ms < 0 {
            return Err(StoreError::Invariant("UI Part requires a publication transaction and timestamp".into()));
        }
        let key = identity_key(&snapshot.identity)?;
        let owner = &snapshot.identity.owner;
        if self.read_ui_surface(&snapshot.identity)?.is_none_or(|state| state.revision != snapshot.revision) {
            return Err(StoreError::Conflict("UI Part cannot publish a stale Surface revision".into()));
        }
        let state = self.read_ui_surface_revision(&snapshot.identity, snapshot.revision)?
            .ok_or_else(|| StoreError::Conflict("UI Part references an uncommitted Surface version".into()))?;
        if state.phase != snapshot.phase {
            return Err(StoreError::Conflict("UI Part phase does not match its committed version".into()));
        }
        let part_id = format!("ui-surface:{key}");
        let part = nola_domain::MessagePartView::UiSurface {
            part_id: nola_domain::MessagePartId(part_id.clone()), revision: snapshot.revision,
            surface: UiSurfaceRevisionRef { identity: snapshot.identity.clone(), revision: snapshot.revision },
            summary: snapshot.summary.clone(),
        };
        part.validate_public_semantics(&nola_domain::MessageId(owner.message_id.clone()))
            .map_err(|error| StoreError::Invariant(error.to_string()))?;
        let raw = serde_json::to_string(&part)?;
        let transaction = self.write_transaction()?;
        // A mutable reference remains open even after its originating Run ends.
        // Exact historical component data is immutable in ui_surface_revisions.
        let changed = transaction.execute(
            "INSERT INTO message_parts(id,message_id,kind,ordinal,state,content_text,content_json,content_digest,created_at_ms,updated_at_ms)
             VALUES(?1,?2,'ui_surface',COALESCE((SELECT MAX(ordinal)+1 FROM message_parts WHERE message_id=?2),1),'open',?3,?4,?5,?6,?6)
             ON CONFLICT(id) DO UPDATE SET content_text=excluded.content_text,content_json=excluded.content_json,
               content_digest=excluded.content_digest,updated_at_ms=excluded.updated_at_ms
             WHERE message_parts.message_id=excluded.message_id AND message_parts.kind='ui_surface'",
            params![part_id,owner.message_id,snapshot.summary,raw,format!("{:x}",Sha256::digest(snapshot.summary.as_bytes())),occurred_at_ms])?;
        if changed != 1 { return Err(StoreError::Conflict("UI Part identity collision".into())); }
        transaction.commit()?;
        self.applied_ui_surface_stamps.insert(format!("{key}:{}", snapshot.revision));
        publish_surface_journal(self, &part_id, snapshot)

    }

    fn list_ui_surfaces(&self, authority_id: &str, conversation_id: &str) -> StoreResult<Vec<UiSurfaceState>> {
        let mut query = self.connection.prepare("SELECT state_json FROM ui_surfaces WHERE authority_id=?1 AND conversation_id=?2 AND deleted=0 ORDER BY identity_key LIMIT ?3")?;
        let rows = query.query_map(params![authority_id,conversation_id,UI_MAX_ACTIVE_SURFACES as i64], |row| row.get::<_,String>(0))?;
        rows.map(|row| Ok(serde_json::from_str(&row?)?)).collect()
    }
}

#[cfg(test)]
mod history_tests {
    use super::*;

    #[test]
    fn exact_versions_survive_update_and_reopen_but_not_transaction_rollback() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("ui-history.sqlite");
        let mut store = NolaStore::open(&path).unwrap();
        store.connection.execute_batch("INSERT INTO conversations(id,title,revision,created_at_ms,updated_at_ms) VALUES('c','UI',1,1,1);
            INSERT INTO messages(id,conversation_id,run_id,role,status,ordinal,created_at_ms) VALUES('m','c','r','assistant','streaming',1,1);
            INSERT INTO runs(id,conversation_id,kind,assistant_message_id,status,next_event_sequence,created_at_ms,updated_at_ms) VALUES('r','c','conversation','m','running',1,1,1);
            INSERT INTO message_parts(id,message_id,kind,ordinal,state,created_at_ms,updated_at_ms) VALUES('p','m','text',0,'open',1,1);").unwrap();
        let mut state = UiSurfaceState {
            identity: UiSurfaceIdentity { owner: UiSurfaceOwner {
                authority_id: store.ui_authority_id().unwrap(), conversation_id: "c".into(), context_id: "c".into(),
                participant_id: "assistant".into(), originating_run_id: "r".into(), message_id: "m".into(), part_id: "p".into(),
            }, surface_id: "form".into(), generation: 1 },
            revision: 1, catalog_id: "fixture".into(), catalog_digest: "fixture".into(),
            phase: UiSurfacePhase::Pending, components: Default::default(), data: serde_json::json!({"value":"first"}),
            theme: None, send_data_model: false,
        };
        let snapshot = |state: &UiSurfaceState| UiSnapshot {
            identity: state.identity.clone(), revision: state.revision, catalog_id: state.catalog_id.clone(),
            catalog_digest: state.catalog_digest.clone(), phase: state.phase, root_instance_id: None,
            nodes: Default::default(), summary: format!("revision {}", state.revision),
        };
        store.begin_write_unit_of_work().unwrap();
        store.write_ui_surface(0, &state).unwrap();
        store.write_ui_surface_part(&snapshot(&state), 1).unwrap();
        store.commit_write_unit_of_work().unwrap();
        state.revision = 2;
        state.data = serde_json::json!({"value":"second"});
        store.begin_write_unit_of_work().unwrap();
        store.write_ui_surface(1, &state).unwrap();
        store.write_ui_surface_part(&snapshot(&state), 2).unwrap();
        store.commit_write_unit_of_work().unwrap();
        state.revision = 3;
        state.phase = UiSurfacePhase::Deleted;
        store.begin_write_unit_of_work().unwrap();
        store.write_ui_surface(2, &state).unwrap();
        store.write_ui_surface_part(&snapshot(&state), 3).unwrap();
        store.rollback_write_unit_of_work().unwrap();
        drop(store);
        let store = NolaStore::open(&path).unwrap();
        assert_eq!(store.read_ui_surface_revision(&state.identity, 1).unwrap().unwrap().data["value"], "first");
        assert_eq!(store.read_ui_surface_revision(&state.identity, 2).unwrap().unwrap().data["value"], "second");
        assert!(store.read_ui_surface_revision(&state.identity, 3).unwrap().is_none());
        assert_eq!(store.read_ui_surface(&state.identity).unwrap().unwrap().revision, 2);
        let raw: String = store.connection.query_row("SELECT content_json FROM message_parts WHERE kind='ui_surface'", [], |row| row.get(0)).unwrap();
        let nola_domain::MessagePartView::UiSurface { part_id, surface, summary, .. } = serde_json::from_str(&raw).unwrap() else { panic!("not a Surface Part") };
        assert_eq!(surface.revision, 2);
        assert_eq!(summary, "revision 2");
        assert_ne!(part_id.0, state.identity.owner.part_id);
        use nola_store_port::assistant_journal::AssistantJournalRepository;
        let journal_identity = nola_domain::assistant_journal::AssistantJournalIdentity {
            source_store_epoch: state.identity.owner.authority_id.clone(), conversation_id: "c".into(), run_id: "r".into(), assistant_message_id: "m".into(),
        };
        assert_eq!(store.local_assistant_journal_head(&journal_identity).unwrap().sequence, 2);
        let head = store.assistant_journal_part(&journal_identity, &part_id.0).unwrap().unwrap();
        let nola_domain::assistant_journal_operations::JournalPartMetadata::UiSurface(metadata) = head.metadata else { panic!("not a Surface journal head") };
        assert_eq!(metadata.surface, surface);
        assert_eq!(metadata.summary, summary);

        let mut other = state.identity.clone();
        other.owner.participant_id = "another-agent".into();
        assert!(store.read_ui_surface_revision(&other, 1).unwrap().is_none());
    }
}

/// The existing typed assistant journal carries only a Surface reference, so
/// remote observers and native clients share the same canonical Part stream.
fn publish_surface_journal(store: &mut NolaStore, part_id: &str, snapshot: &UiSnapshot) -> StoreResult<()> {
    use nola_domain::assistant_journal::{AssistantJournalIdentity, ASSISTANT_JOURNAL_VERSION};
    use nola_domain::assistant_journal_operations::*;
    use nola_store_port::assistant_journal::*;
    let owner = &snapshot.identity.owner;
    let identity = AssistantJournalIdentity { source_store_epoch: owner.authority_id.clone(),
        conversation_id: owner.conversation_id.clone(), run_id: owner.originating_run_id.clone(),
        assistant_message_id: owner.message_id.clone() };
    let surface = UiSurfaceRevisionRef { identity: snapshot.identity.clone(), revision: snapshot.revision };
    let stamp = AssistantSourceCommitStamp::UiSurface { surface: surface.clone() };
    let expected = store.local_assistant_journal_head(&identity)?;
    let metadata = JournalPartMetadata::UiSurface(JournalUiSurfaceMetadata { surface, summary: snapshot.summary.clone() });
    let operation = match store.assistant_journal_part_state(&identity, part_id)? {
        AssistantJournalPartState::NeverDeclared => {
            let order_key = store.reserve_assistant_journal_order(&stamp, &identity, JournalSection::Transcript, part_id)?;
            AssistantJournalOperationV1::Declare(DeclareJournalPart { part: JournalPartHead {
                identity: identity.clone(), part_id: part_id.into(), generation: JournalNumber(1),
                revision: JournalNumber(1), metadata_revision: JournalNumber(1), order_key, metadata, bodies: vec![],
            } })
        }
        AssistantJournalPartState::Live(head) => {
            if head.metadata == metadata { return Ok(()); }
            AssistantJournalOperationV1::Update(UpdateJournalMetadata { part_id: part_id.into(),
                expected_generation: head.generation, expected_revision: head.revision, metadata })
        }
        AssistantJournalPartState::Retired => return Err(StoreError::Conflict("Surface Part has been retired".into())),
    };
    store.commit_assistant_publication_set(&CommitAssistantPublicationSet {
        stamp, groups: vec![AssistantPublicationGroup { expected, frames: vec![AssistantJournalFrame {
            batch: AssistantOperationBatchV1 { protocol_version: ASSISTANT_JOURNAL_VERSION, identity,
                operations: vec![operation] }, supplied_chunks: vec![],
        }] }],
    })?;
    Ok(())
}
