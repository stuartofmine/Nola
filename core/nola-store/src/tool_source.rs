// SPDX-License-Identifier: Apache-2.0
//! Immutable original tool sources in the canonical transaction and body store.
use crate::{params, NolaStore, OptionalExtension, StoreError, StoreResult};
use nola_domain::{CanonicalBodyReference, CanonicalPartOwner, PartBodyField};
use nola_store_port::tool_source::*;
use sha2::{Digest, Sha256};

fn invalid(message: &str) -> StoreError { StoreError::Invariant(message.into()) }
fn kind_name(kind: ToolSourceKind) -> &'static str {
    match kind { ToolSourceKind::ToolCall => "tool_call", ToolSourceKind::ToolResult => "tool_result" }
}

pub(crate) enum ToolSourceImport {
    Archive { restore_id: String, sequence: u64 },
}

fn require_archive_source_entry(connection: &crate::Connection, claim: &ToolSourceClaim,
    restore_id: &str, sequence: u64) -> StoreResult<()> {
    claim.validate().map_err(|e|invalid(&e))?;
    let encoded: String = connection.query_row(
        "SELECT entry_json FROM archive_restore_entries WHERE restore_id=?1 AND sequence=?2",
        params![restore_id, sequence], |row|row.get(0))?;
    match serde_json::from_str::<nola_domain::ArchiveRestoreEntry>(&encoded)? {
        nola_domain::ArchiveRestoreEntry::ToolSource {claim: stored} if stored == *claim => Ok(()),
        _ => Err(invalid("tool source does not match its staged archive entry")),
    }
}

/// Validate the complete immutable object before replacing any destination data.
pub(crate) fn validate_imported_tool_source_body(connection: &crate::Connection,
    claim: &ToolSourceClaim) -> StoreResult<()> {
    claim.validate().map_err(|e|invalid(&e))?;
    crate::canonical_text_store::validate_reference_from_connection(connection,&claim.body)?;
    crate::canonical_text_replica::require_complete(connection,&claim.body)?;
    let mut raw=String::new();
    let mut query=connection.prepare("SELECT text FROM canonical_text_chunks WHERE content_id=?1 ORDER BY offset_bytes")?;
    let mut rows=query.query([claim.body.content_id()])?;
    while let Some(row)=rows.next()? {
        let text:String=row.get(0)?;
        if raw.len().saturating_add(text.len()) as u64>claim.body.total_utf8_bytes() {
            return Err(invalid("tool source body exceeds declared length"));
        }
        raw.push_str(&text);
    }
    if raw.len() as u64!=claim.body.total_utf8_bytes()
        || format!("{:x}",Sha256::digest(raw.as_bytes()))!=claim.body_sha256 {
        return Err(invalid("tool source body digest or length differs"));
    }
    let body:serde_json::Value=serde_json::from_str(&raw)?;
    let fields:&[&str]=match claim.kind {
        ToolSourceKind::ToolCall=>&["type","id","call_id","name","arguments"],
        ToolSourceKind::ToolResult=>&["type","id","call_id","name","output","is_error","full_output_ref","artifacts"],
    };
    let object=body.as_object().ok_or_else(||invalid("tool source must be an object"))?;
    if object.len()!=fields.len() || fields.iter().any(|field|!object.contains_key(*field))
        || body["type"].as_str()!=Some(kind_name(claim.kind))
        || body["id"].as_str()!=Some(claim.source_item_id.as_str())
        || body["call_id"].as_str()!=Some(claim.call_id.as_str())
        || body["name"].as_str()!=Some(claim.name.as_str())
        || (claim.kind==ToolSourceKind::ToolResult && (!body["is_error"].is_boolean()
            || !body["artifacts"].is_array() || !(body["full_output_ref"].is_null() || body["full_output_ref"].is_string()))) {
        return Err(invalid("tool source original item differs from its archive claim"));
    }
    Ok(())
}

pub(crate) fn install_imported_tool_source(connection: &crate::Connection,
    claim: &ToolSourceClaim, provenance: ToolSourceImport) -> StoreResult<()> {
    if connection.is_autocommit() {return Err(invalid("tool source import requires an archive transaction"));}
    validate_imported_tool_source_body(connection,claim)?;
    let ToolSourceImport::Archive {restore_id,sequence}=provenance;
    require_archive_source_entry(connection,claim,&restore_id,sequence)?;
    let owner=&claim.owner;
    let revision:i64=connection.query_row(
        "SELECT c.revision FROM conversations c JOIN messages m ON m.conversation_id=c.id
         JOIN message_parts p ON p.message_id=m.id
         WHERE c.id=?1 AND m.id=?2 AND p.id=?3 AND m.role='assistant' AND m.status='completed' AND p.kind='tool'",
        params![owner.conversation_id(),owner.message_id(),owner.part_id()],|row|row.get(0))?;
    let original=&claim.original_owner;
    let binding=ToolSourceBinding {
        source_item_id:claim.source_item_id.clone(),kind:claim.kind,call_id:claim.call_id.clone(),name:claim.name.clone(),
        owner:nola_store_port::assistant_ownership::AssistantPartOwner {
            part_id:original.part_owner.part_id().into(),turn_id:Some(original.turn_id.clone()),
            owner:nola_store_port::assistant_ownership::AssistantOwner {
                conversation_id:original.part_owner.conversation_id().into(),
                assistant_message_id:original.part_owner.message_id().into(),run_id:original.run_id.clone(),
            },
        },
        provenance:ToolSourceProvenance::Archive {claim:claim.clone(),restore_id,sequence},
        body_sha256:claim.body_sha256.clone(),body:claim.body.clone(),created_revision:revision,
    };
    let mut query=connection.prepare("SELECT binding_json FROM canonical_tool_sources WHERE part_id=?1")?;
    let mut rows=query.query([owner.part_id()])?;
    let mut has_call=false;
    while let Some(row)=rows.next()? {
        let peer:ToolSourceBinding=serde_json::from_str(&row.get::<_,String>(0)?)?;
        if peer.kind==claim.kind || peer.call_id!=claim.call_id || peer.name!=claim.name || peer.owner!=binding.owner {
            return Err(invalid("archive tool source pair has duplicate or inconsistent identity"));
        }
        has_call |= peer.kind==ToolSourceKind::ToolCall;
    }
    if claim.kind==ToolSourceKind::ToolResult && !has_call {return Err(invalid("archive result requires its original call first"));}
    connection.execute("INSERT INTO canonical_tool_sources(source_item_id,conversation_id,run_id,message_id,part_id,kind,binding_json)
        VALUES(?1,?2,NULL,?3,?4,?5,?6)",params![claim.source_item_id,owner.conversation_id(),owner.message_id(),owner.part_id(),kind_name(claim.kind),serde_json::to_string(&binding)?])?;
    Ok(())
}

impl NolaStore {
    pub(crate) fn bind_tool_source(
        &mut self,
        stamp: &nola_store_port::assistant_journal::AssistantSourceCommitStamp,
        identity: &nola_domain::assistant_journal::AssistantJournalIdentity,
        input: &ToolSourceInput,
    ) -> StoreResult<()> {
        crate::assistant_journal::planning_owner_key(self, stamp, identity)?;
        let owner = &input.owner;
        for value in [&input.source_item_id, &input.call_id, &input.name, &owner.part_id] {
            crate::require_id("tool_source.identity", value)?;
        }
        if owner.owner.conversation_id != identity.conversation_id
            || owner.owner.run_id != identity.run_id
            || owner.owner.assistant_message_id != identity.assistant_message_id
            || owner.turn_id.is_none()
            || self.assistant_owner_for_part(&identity.conversation_id, &identity.run_id, &owner.part_id)?.as_ref() != Some(owner)
        { return Err(invalid("tool source lacks its exact accepted Part/Turn owner")); }
        crate::assistant_turn_ownership::validate_part_write_owner(
            &self.connection, &identity.conversation_id, &identity.run_id,
            &identity.assistant_message_id, &owner.part_id,
        )?;
        let canonical: bool = self.connection.query_row(
            "SELECT EXISTS(SELECT 1 FROM message_parts p JOIN messages m ON m.id=p.message_id
             WHERE p.id=?1 AND p.kind='tool' AND m.id=?2 AND m.run_id=?3 AND m.conversation_id=?4)",
            params![owner.part_id, identity.assistant_message_id, identity.run_id, identity.conversation_id], |r|r.get(0),
        )?;
        if !canonical { return Err(invalid("tool source writer is not the current local Tool Part owner")); }
        if input.body_json.len() > 256*1024*1024 {
            return Err(invalid("tool source exceeds canonical body quota"));
        }
        let body: serde_json::Value = serde_json::from_str(&input.body_json)?;
        if body.get("type").and_then(|v|v.as_str()) != Some(kind_name(input.kind))
            || body.get("id").and_then(|v|v.as_str()) != Some(input.source_item_id.as_str())
            || body.get("call_id").and_then(|v|v.as_str()) != Some(input.call_id.as_str())
            || body.get("name").and_then(|v|v.as_str()) != Some(input.name.as_str())
        { return Err(invalid("tool source original item and typed identity disagree")); }
        let fields: &[&str]=match input.kind {
            ToolSourceKind::ToolCall=>&["type","id","call_id","name","arguments"],
            ToolSourceKind::ToolResult=>&["type","id","call_id","name","output","is_error","full_output_ref","artifacts"],
        };
        let object=body.as_object().ok_or_else(||invalid("tool source must be a typed object"))?;
        if object.len()!=fields.len() || fields.iter().any(|field|!object.contains_key(*field))
            || (input.kind==ToolSourceKind::ToolResult && (
                !body["is_error"].is_boolean() || !body["artifacts"].is_array()
                || !(body["full_output_ref"].is_null() || body["full_output_ref"].is_string())))
        { return Err(invalid("tool source does not match the complete current AgentItem shape")); }
        let digest = format!("{:x}", Sha256::digest(input.body_json.as_bytes()));
        let transaction = self.write_transaction()?;
        let foreign:bool=transaction.query_row("SELECT EXISTS(SELECT 1 FROM canonical_tool_sources
            WHERE source_item_id=?1 AND json_extract(binding_json,'$.owner')<>?2)",
            params![input.source_item_id,serde_json::to_string(&input.owner)?],|r|r.get(0))?;
        // JSON object key ordering is not an owner oracle. Compare decoded
        // snapshots below as well; the query only bounds candidate metadata.
        if foreign {
            let mut q=transaction.prepare("SELECT binding_json FROM canonical_tool_sources WHERE source_item_id=?1")?;
            let mut rows=q.query([&input.source_item_id])?;
            while let Some(row)=rows.next()? {
                let previous:ToolSourceBinding=serde_json::from_str(&row.get::<_,String>(0)?)?;
                if previous.owner!=input.owner { return Err(invalid(&format!("original tool item {} belongs to another accepted owner: stored={:?}, requested={:?}", input.source_item_id, previous.owner, input.owner))); }
            }
        }
        let old: Option<String> = transaction.query_row(
            "SELECT binding_json FROM canonical_tool_sources WHERE (conversation_id=?4 AND source_item_id=?1) OR (part_id=?2 AND kind=?3)",
            params![input.source_item_id, owner.part_id, kind_name(input.kind),identity.conversation_id], |r|r.get(0),
        ).optional()?;
        if let Some(old) = old {
            let old: ToolSourceBinding = serde_json::from_str(&old)?;
            if old.source_item_id != input.source_item_id || old.kind != input.kind
                || old.call_id != input.call_id || old.name != input.name
                || old.owner != input.owner || old.body_sha256 != digest
                || old.provenance!=ToolSourceProvenance::AcceptedRun
            { return Err(StoreError::Conflict("tool source identity, original bytes and owner are immutable".into())); }
            transaction.commit()?;
            return Ok(());
        }
        let peers: Vec<String> = {
            let mut q=transaction.prepare("SELECT binding_json FROM canonical_tool_sources WHERE part_id=?1")?;
            let rows=q.query_map([&owner.part_id], |r|r.get(0))?.collect::<Result<Vec<_>,_>>()?;
            rows
        };
        for peer in peers {
            let peer: ToolSourceBinding=serde_json::from_str(&peer)?;
            if peer.owner != input.owner || peer.call_id != input.call_id || peer.name != input.name {
                return Err(invalid("tool call/result source pair has mixed ownership"));
            }
        }
        if input.kind == ToolSourceKind::ToolResult && transaction.query_row(
            "SELECT COUNT(*) FROM canonical_tool_sources WHERE part_id=?1 AND kind='tool_call'", [&owner.part_id], |r|r.get::<_,i64>(0),
        )? != 1 { return Err(invalid("tool result source requires its immutable original call")); }
        // Exact UTF-8 chunks, not a JSON/display re-encoding. The body object is
        // separate from message_part_body_references and cannot replace the Part.
        let mut chunks=Vec::new();
        let mut at=0;
        while at < input.body_json.len() {
            let mut end=(at+32768).min(input.body_json.len());
            while !input.body_json.is_char_boundary(end) { end-=1; }
            chunks.push(Ok((at as u64,input.body_json[at..end].to_owned())));
            at=end;
        }
        let content_id = format!("tool-source:{:x}", Sha256::digest(serde_json::to_vec(&(
            &input.source_item_id, &input.owner, &digest,
        ))?));
        let reference=CanonicalBodyReference::for_field(content_id, identity.conversation_id.clone(),
            identity.assistant_message_id.clone(), owner.part_id.clone(), PartBodyField::ContentJson,
            input.body_json.len() as u64, chunks.len() as u32).map_err(|e|invalid(&e.to_string()))?;
        crate::canonical_text_store::put_chunks(&transaction,&reference,chunks,256*1024*1024)?;
        let revision: i64=transaction.query_row("SELECT revision FROM conversations WHERE id=?1",[&identity.conversation_id],|r|r.get(0))?;
        let binding=ToolSourceBinding { source_item_id:input.source_item_id.clone(),kind:input.kind,
            call_id:input.call_id.clone(),name:input.name.clone(),owner:input.owner.clone(),provenance:ToolSourceProvenance::AcceptedRun,body_sha256:digest,
            body:reference,created_revision:revision };
        transaction.execute("INSERT INTO canonical_tool_sources(source_item_id,conversation_id,run_id,message_id,part_id,kind,binding_json)
            VALUES(?1,?2,?3,?4,?5,?6,?7)", params![input.source_item_id,identity.conversation_id,identity.run_id,
            identity.assistant_message_id,owner.part_id,kind_name(input.kind),serde_json::to_string(&binding)?])?;
        transaction.commit()?;
        Ok(())
    }

}

pub(crate) fn read_binding(connection:&crate::Connection, conversation: &str, source: &str, revision: i64) -> StoreResult<Option<ToolSourceBinding>> {
        crate::require_id("tool_source.conversation",conversation)?;
        crate::require_id("tool_source.item",source)?;
        let current: i64=connection.query_row("SELECT revision FROM conversations WHERE id=?1",[conversation],|r|r.get(0))?;
        if current != revision || revision < 1 { return Err(StoreError::Conflict("tool source conversation revision changed".into())); }
        let row: Option<(String,Option<String>,String,String,String)>=connection.query_row("SELECT binding_json,run_id,message_id,part_id,kind FROM canonical_tool_sources
            WHERE conversation_id=?1 AND source_item_id=?2",params![conversation,source],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?))).optional()?;
        let Some((raw,run,message,part,kind))=row else { return Ok(None); };
        let binding: ToolSourceBinding=serde_json::from_str(&raw)?;
        let owner=&binding.owner;
        let canonical=binding.canonical_owner().map_err(|e|invalid(&e))?;
        if binding.source_item_id != source || canonical.conversation_id() != conversation || binding.created_revision > revision
            || binding.created_revision < 1 || binding.body.conversation_id() != conversation
            || binding.body.message_id() != canonical.message_id() || binding.body.part_id() != canonical.part_id()
            || canonical.message_id()!=message || canonical.part_id()!=part || kind_name(binding.kind)!=kind
            || binding.body_sha256.len()!=64 || !binding.body_sha256.bytes().all(|byte|byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        { return Err(invalid("tool source has foreign or missing canonical ownership")); }
        match &binding.provenance {
            ToolSourceProvenance::AcceptedRun => {
                if run.as_deref()!=Some(owner.owner.run_id.as_str()) || owner.owner.conversation_id!=conversation
                    || owner.owner.assistant_message_id!=message || owner.part_id!=part
                    || !crate::canonical_message_ownership::matches_local_read_owner(connection,conversation,&message,&owner.owner.run_id)? {
                    return Err(invalid("tool source lacks local accepted Run ownership"));
                }
                let exact:bool=connection.query_row("SELECT EXISTS(SELECT 1 FROM assistant_turn_part_bindings p
                    JOIN assistant_turn_message_bindings t ON t.conversation_id=p.conversation_id AND t.run_id=p.run_id
                        AND t.turn_id=p.turn_id AND t.message_id=p.message_id
                    WHERE p.conversation_id=?1 AND p.run_id=?2 AND p.part_id=?3 AND p.message_id=?4 AND p.turn_id=?5)",
                    params![conversation,owner.owner.run_id,part,message,owner.turn_id],|r|r.get(0))?;
                if !exact { return Err(invalid("tool source lacks immutable Part/Turn binding")); }
            }
            ToolSourceProvenance::VerifiedLocalDerivation {operation_id,target_owner,source_owner,source_binding_sha256,source_provenance} => {
                if run.is_some() || target_owner!=&canonical || source_binding_sha256.len()!=64
                    || !source_binding_sha256.bytes().all(|b|b.is_ascii_digit() || (b'a'..=b'f').contains(&b)) {
                    return Err(invalid("tool source has invalid derived provenance"));
                }
                let current=crate::part_derivation::source(connection,canonical.part_id())?;
                let proof=crate::part_derivation::load(connection,&current)?.ok_or_else(||invalid("tool source derivation proof is unavailable"))?;
                if proof.provenance!=*source_provenance
                    || *source_provenance!=nola_domain::PartDerivationProvenance::LocalBranch
                    || proof.derivation.operation_id!=*operation_id || proof.derivation.target.owner!=canonical
                    || proof.derivation.immediate_source.owner!=*source_owner {
                    return Err(invalid("tool source derivation does not match canonical Part lineage"));
                }
            }
            ToolSourceProvenance::Archive {claim,restore_id,sequence} => {
                if run.is_some() || binding.claim().map_err(|e|invalid(&e))? != *claim
                    || binding.source_item_id!=claim.source_item_id || binding.kind!=claim.kind
                    || binding.call_id!=claim.call_id || binding.name!=claim.name
                    || binding.body!=claim.body || binding.body_sha256!=claim.body_sha256
                    || owner.owner.run_id!=claim.original_owner.run_id
                    || owner.turn_id.as_deref()!=Some(claim.original_owner.turn_id.as_str())
                    || owner.part_id!=claim.original_owner.part_owner.part_id()
                    || owner.owner.conversation_id!=claim.original_owner.part_owner.conversation_id()
                    || owner.owner.assistant_message_id!=claim.original_owner.part_owner.message_id()
                { return Err(invalid("archive tool source identity differs")); }
                require_archive_source_entry(connection,claim,restore_id,*sequence)?;
            }
            ToolSourceProvenance::Replica {..} => {
                return Err(invalid("replica tool sources are not supported by the current receiver"));
            }
        }
        let visible: bool=connection.query_row("SELECT EXISTS(SELECT 1 FROM message_parts p JOIN messages m ON m.id=p.message_id
            WHERE p.id=?1 AND m.id=?2 AND m.conversation_id=?3 AND p.kind='tool'
            AND COALESCE((SELECT e.visible FROM message_projection_events e JOIN conversation_projection_operations o
              ON o.operation_id=e.operation_id WHERE e.conversation_id=m.conversation_id AND e.message_id=m.id
              ORDER BY o.resulting_revision DESC LIMIT 1),1)=1)",params![part,message,conversation],|r|r.get(0))?;
        if !visible { return Err(invalid("tool source Part is not visible in the pinned canonical view")); }
        crate::canonical_text_store::validate_reference_from_connection(connection,&binding.body)?;
        Ok(Some(binding))
}

/// Called only after the branch's canonical PartDerivation has been recorded,
/// inside that same transaction. Source Run metadata is retained as provenance,
/// not replaced with a fictional target Run or an executable call identity.
pub(crate) fn copy_branch_sources(connection:&crate::Connection,input:&nola_domain::CanonicalBranchIntent,
    target_message:&str,target_revision:i64) -> StoreResult<()> {
    if connection.is_autocommit() { return Err(invalid("tool source derivation requires its branch transaction")); }
    let parts={ let mut query=connection.prepare("SELECT id FROM message_parts WHERE message_id=?1 AND kind='tool' ORDER BY ordinal")?;
        let rows=query.query_map([target_message],|r|r.get::<_,String>(0))?.collect::<Result<Vec<_>,_>>()?; rows };
    for part in parts {
        let target=crate::part_derivation::source(connection,&part)?;
        let derivation=crate::part_derivation::load(connection,&target)?.ok_or_else(||invalid("tool source branch lacks canonical derivation"))?;
        let proof=&derivation.derivation;
        if derivation.provenance!=nola_domain::PartDerivationProvenance::LocalBranch {
            // Imported claims are not locally authored raw-source authority.
            continue;
        }
        if proof.operation_id!=input.operation_id || proof.target.owner.conversation_id()!=input.conversation.id
            || proof.target.owner.message_id()!=target_message || proof.immediate_source.owner.conversation_id()!=input.source_conversation_id {
            return Err(invalid("tool source branch owner differs from accepted derivation"));
        }
        let ids={let mut query=connection.prepare("SELECT source_item_id FROM canonical_tool_sources WHERE conversation_id=?1 AND part_id=?2 ORDER BY kind")?;
            let rows=query.query_map(params![input.source_conversation_id,proof.immediate_source.owner.part_id()],|r|r.get::<_,String>(0))?.collect::<Result<Vec<_>,_>>()?; rows};
        for id in ids {
            let original=read_binding(connection,&input.source_conversation_id,&id,input.expected_source_revision)?.ok_or_else(||invalid("branch tool source disappeared"))?;
            if original.canonical_owner().map_err(|e|invalid(&e))?!=proof.immediate_source.owner {
                return Err(invalid("branch tool source belongs to another Part"));
            }
            crate::canonical_text_replica::require_complete(connection,&original.body)?;
            let reference=CanonicalBodyReference::for_field(
                crate::conversation_branch::identity(&input.operation_id,"tool-source-body",&id),
                input.conversation.id.clone(),target_message.into(),part.clone(),PartBodyField::ContentJson,
                original.body.total_utf8_bytes(),original.body.chunk_count()).map_err(|e|invalid(&e.to_string()))?;
            let mut digest=Sha256::new();
            {
                let mut query=connection.prepare("SELECT offset_bytes,text FROM canonical_text_chunks WHERE content_id=?1 ORDER BY offset_bytes")?;
                let rows=query.query_map([original.body.content_id()],|r|Ok((r.get::<_,u64>(0)?,r.get::<_,String>(1)?)))?;
                crate::canonical_text_store::put_chunks(connection,&reference,rows.map(|row| {
                    let (offset,text)=row?;
                    digest.update(text.as_bytes());
                    Ok((offset,text))
                }),256*1024*1024)?;
            }
            if format!("{:x}",digest.finalize())!=original.body_sha256 { return Err(invalid("branch tool source original digest differs")); }
            let source_binding_sha256=format!("{:x}",Sha256::digest(serde_json::to_vec(&original)?));
            let mut binding=original;
            binding.body=reference;
            binding.created_revision=target_revision;
            binding.provenance=ToolSourceProvenance::VerifiedLocalDerivation {
                operation_id:input.operation_id.clone(),target_owner:proof.target.owner.clone(),
                source_owner:proof.immediate_source.owner.clone(),source_binding_sha256,
                source_provenance:derivation.provenance.clone(),
            };
            connection.execute("INSERT INTO canonical_tool_sources(source_item_id,conversation_id,run_id,message_id,part_id,kind,binding_json)
                VALUES(?1,?2,NULL,?3,?4,?5,?6)",params![binding.source_item_id,input.conversation.id,target_message,part,kind_name(binding.kind),serde_json::to_string(&binding)?])?;
        }
    }
    Ok(())
}

impl ToolSourceRepository for NolaStore {
    fn tool_sources_for_part(&self, owner: &CanonicalPartOwner, revision:i64) -> Result<Vec<ToolSourceBinding>,String> {
        (|| -> StoreResult<_> {
            let _snapshot=crate::read_snapshot::pin(&self.connection)?;
            let (current,member):(i64,bool)=self.connection.query_row(
                "SELECT revision,EXISTS(SELECT 1 FROM message_parts p JOIN messages m ON m.id=p.message_id
                 WHERE p.id=?2 AND m.id=?3 AND m.conversation_id=c.id AND p.kind='tool')
                 FROM conversations c WHERE c.id=?1",
                params![owner.conversation_id(),owner.part_id(),owner.message_id()],|r|Ok((r.get(0)?,r.get(1)?)))?;
            if current!=revision || revision<1 || !member { return Err(invalid("tool source Part/revision is outside canonical scope")); }
            let ids={ let mut q=self.connection.prepare("SELECT source_item_id FROM canonical_tool_sources WHERE part_id=?1 ORDER BY kind")?;
                let rows=q.query_map([owner.part_id()],|r|r.get::<_,String>(0))?.collect::<Result<Vec<_>,_>>()?; rows };
            let mut bindings=Vec::new();
            for id in ids {
                let binding=read_binding(&self.connection,owner.conversation_id(),&id,revision)?.ok_or_else(||invalid("tool source disappeared"))?;
                if binding.canonical_owner().map_err(|e|invalid(&e))? != *owner { return Err(invalid("tool source requested for foreign Message")); }
                bindings.push(binding);
            }
            Ok(bindings)
        })().map_err(|e|e.to_string())
    }
    fn tool_source_by_id(&self,conversation:&str,source:&str,revision:i64)->Result<Option<ToolSourceBinding>,String> {
        (|| { let _snapshot=crate::read_snapshot::pin(&self.connection)?;
            read_binding(&self.connection,conversation,source,revision) })().map_err(|e|e.to_string())
    }
    fn read_tool_source_range(&self,binding:&ToolSourceBinding,revision:i64,offset:u64,limit:usize)->Result<ToolSourceRange,String> {
        (|| -> StoreResult<_> {
            let _snapshot=crate::read_snapshot::pin(&self.connection)?;
            if read_binding(&self.connection,binding.canonical_owner().map_err(|e|invalid(&e))?.conversation_id(),&binding.source_item_id,revision)?.as_ref() != Some(binding) {
                return Err(invalid("tool source range binding changed"));
            }
            let range=crate::canonical_text_store::read_range_from_connection(&self.connection,&binding.body,offset,limit)?;
            Ok(ToolSourceRange { offset_bytes:range.offset_bytes,next_offset_bytes:range.next_offset_bytes,
                text:range.text,end_of_content:range.end_of_content })
        })().map_err(|e|e.to_string())
    }
}
