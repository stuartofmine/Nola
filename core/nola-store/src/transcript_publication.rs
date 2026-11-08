// SPDX-License-Identifier: Apache-2.0
//! Command-owned publication of prepared immutable transcript bodies.
use crate::{params, NolaStore, OptionalExtension, StoreError, StoreResult};
use nola_domain::CanonicalBodyReference;
use nola_store_port::{
    transcript_publication::{PreparedTranscriptPublication, TranscriptPublicationRepository},
    transcript_write::AppendMessagesResult,
};

impl TranscriptPublicationRepository for NolaStore {
    type Error = StoreError;

    fn transcript_publication_replay(
        &self,
        import_id: &str,
        manifest_json: &str,
    ) -> StoreResult<Option<AppendMessagesResult>> {
        let stored: Option<(String, String)> = self.connection.query_row(
            "SELECT manifest_json,result_json FROM transcript_import_publications WHERE import_id=?1",
            params![import_id], |row| Ok((row.get(0)?, row.get(1)?)),
        ).optional()?;
        stored
            .map(|(manifest, result)| {
                if manifest != manifest_json {
                    return Err(StoreError::Conflict(
                        "import identity already has a different manifest".into(),
                    ));
                }
                Ok(serde_json::from_str(&result)?)
            })
            .transpose()
    }

    fn preflight_transcript_publication(
        &self,
        conversation_id: &str,
        expected_revision: i64,
        expected_count: usize,
    ) -> StoreResult<()> {
        let state: Option<(i64, i64, bool)> = self.connection.query_row(
            "SELECT revision,(SELECT COUNT(*) FROM messages WHERE conversation_id=c.id),
             EXISTS(SELECT 1 FROM runs WHERE conversation_id=c.id AND status NOT IN ('sealed'))
             FROM conversations c WHERE id=?1",
            params![conversation_id], |row| Ok((row.get(0)?,row.get(1)?,row.get(2)?)),
        ).optional()?;
        if !state.is_some_and(|(revision, count, active)| {
            !active
                && revision == expected_revision
                && usize::try_from(count).ok() == Some(expected_count)
        }) {
            return Err(StoreError::Conflict(
                "transcript requires idle conversation with matching revision and message count"
                    .into(),
            ));
        }
        Ok(())
    }

    fn prepare_transcript_content(
        &mut self,
        reference: &CanonicalBodyReference,
        chunks: &mut dyn Iterator<Item = Result<(u64, String), String>>,
        maximum_bytes: u64,
    ) -> StoreResult<()> {
        crate::canonical_body::require_text_reference(reference)?;
        let transaction = self.write_transaction()?;
        crate::canonical_text_store::put_chunks(
            &transaction,
            reference,
            chunks.map(|value| value.map_err(StoreError::Invariant)),
            maximum_bytes,
        )?;
        transaction.commit()?;
        Ok(())
    }

    fn publish_transcript(
        &mut self,
        input: &PreparedTranscriptPublication,
    ) -> StoreResult<AppendMessagesResult> {
        if !self.has_write_unit_of_work() {
            return Err(StoreError::Invariant(
                "transcript publication requires the command receipt write unit".into(),
            ));
        }
        validate_prepared(input)?;
        if let Some(result) =
            self.transcript_publication_replay(&input.import_id, &input.manifest_json)?
        {
            return Ok(result);
        }
        let transaction = self.write_transaction()?;
        let result = crate::conversation_append_store::append_in_transaction(
            &transaction,
            &input.append,
            Some(input.expected_conversation_revision),
            &input.references,
        )?;
        transaction.execute(
            "INSERT INTO transcript_import_publications(import_id,conversation_id,manifest_json,result_json)
             VALUES(?1,?2,?3,?4)",
            params![input.import_id,input.append.conversation_id,input.manifest_json,serde_json::to_string(&result)?],
        )?;
        transaction.commit()?;
        Ok(result)
    }
}

fn validate_prepared(input: &PreparedTranscriptPublication) -> StoreResult<()> {
    let invalid = || StoreError::Invariant("invalid prepared transcript publication".into());
    if input.import_id.trim().is_empty()
        || input.import_id.len() > 128
        || input.expected_conversation_revision < 1
        || input.manifest_json.len() > 128 * 1024
        || input.references.is_empty()
        || input.references.len() > 512
        || input.append.messages.is_empty()
        || input.append.messages.len() > 128
    {
        return Err(invalid());
    }
    crate::validate_json("publication.manifest", &input.manifest_json)?;
    // The opaque replay encoding must describe the actual prepared write. A
    // caller cannot reuse its old encoding while substituting new message data.
    let manifest: serde_json::Value = serde_json::from_str(&input.manifest_json)?;
    let transcript = &manifest["transcript"];
    if manifest["expected_conversation_revision"].as_i64()
        != Some(input.expected_conversation_revision)
        || transcript["import_id"].as_str() != Some(input.import_id.as_str())
        || transcript["conversation_id"].as_str() != Some(input.append.conversation_id.as_str())
        || transcript["expected_message_count"].as_u64()
            != Some(input.append.expected_message_count as u64)
        || transcript["imported_at_ms"].as_i64() != Some(input.append.updated_at_ms)
        || transcript["messages"] != serde_json::to_value(&input.append.messages)?
    {
        return Err(invalid());
    }
    let uploads = manifest["uploads"].as_array().ok_or_else(invalid)?;
    if uploads.len() != input.references.len() {
        return Err(invalid());
    }
    let mut parts = std::collections::HashMap::new();
    let mut messages = std::collections::HashSet::new();
    for message in &input.append.messages {
        if !messages.insert(&message.id)
            || message.status != "completed"
            || !message
                .completed_at_ms
                .is_some_and(|at| at >= message.created_at_ms)
            || message.parts.is_empty()
        {
            return Err(invalid());
        }
        for part in &message.parts {
            if part.state != "completed"
                || part.updated_at_ms < part.created_at_ms
                || parts
                    .insert(part.id.as_str(), (message.id.as_str(), part))
                    .is_some()
            {
                return Err(invalid());
            }
        }
    }
    crate::body_binding::validate_text_bindings(
        &input.append.conversation_id,
        input.append.messages.iter().flat_map(|message| {
            message
                .parts
                .iter()
                .map(move |part| (message.id.as_str(), message.status.as_str(), part))
        }),
        &input.references,
    )?;
    for reference in &input.references {
        // This adapter chooses the immutable upload identity for its object ID;
        // that mapping is server-owned and verified against the complete manifest.
        let matching = uploads
            .iter()
            .filter(|upload| {
                upload["part_id"].as_str() == Some(reference.part_id())
                    && upload["message_id"].as_str() == Some(reference.message_id())
                    && upload["upload_id"].as_str() == Some(reference.content_id())
                    && upload["expected_utf8_bytes"].as_u64() == Some(reference.total_utf8_bytes())
            })
            .count();
        if matching != 1 {
            return Err(invalid());
        }
    }
    Ok(())
}
