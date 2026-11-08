// SPDX-License-Identifier: Apache-2.0
//! Bounded sealed-source preparation and command-owned canonical publication.
use crate::transcript_input::FinalizedTranscriptInput;
use nola_domain::{CanonicalBodyReference, CANONICAL_BODY_CHUNK_MAX_BYTES};
use nola_protocol::uploaded_transcript::UploadedTranscriptManifest;
use nola_store_port::{
    content_upload::{ContentUploadRepository, UploadOwner},
    transcript_publication::{PreparedTranscriptPublication, TranscriptPublicationRepository},
    transcript_write::{AppendMessagesInput, AppendMessagesResult},
};

pub fn publish_uploaded_transcript<
    S: TranscriptPublicationRepository + ?Sized,
    U: ContentUploadRepository + ?Sized,
>(
    store: &mut S,
    uploads: &U,
    manifest: &UploadedTranscriptManifest,
    maximum_bytes: u64,
) -> Result<AppendMessagesResult, String> {
    manifest.validate(maximum_bytes)?;
    let manifest_json = serde_json::to_string(manifest).map_err(|error| error.to_string())?;
    if let Some(result) = store
        .transcript_publication_replay(&manifest.transcript.import_id, &manifest_json)
        .map_err(|error| error.to_string())?
    {
        return Ok(result);
    }
    store
        .preflight_transcript_publication(
            &manifest.transcript.conversation_id,
            manifest.expected_conversation_revision,
            manifest.transcript.expected_message_count,
        )
        .map_err(|error| error.to_string())?;
    let mut references = Vec::with_capacity(manifest.uploads.len());
    for binding in &manifest.uploads {
        let owner = manifest.owner_for(binding);
        let owner = UploadOwner {
            conversation_id: owner.conversation_id,
            import_id: owner.import_id,
            part_id: owner.part_id,
        };
        let progress = uploads
            .progress(&binding.upload_id, &owner)
            .map_err(|error| error.to_string())?;
        if !progress.sealed
            || progress.cancelled
            || progress.expected_bytes != binding.expected_utf8_bytes
            || progress.committed_bytes != binding.expected_utf8_bytes
        {
            return Err("transcript preparation requires exact complete sealed upload".into());
        }
        // Seal makes bytes immutable. Count bounded chunks without buffering the
        // body; a second traversal streams them into one destination write unit.
        let mut count = 0_u32;
        for chunk in source_chunks(
            uploads,
            &binding.upload_id,
            &owner,
            binding.expected_utf8_bytes,
        ) {
            chunk?;
            count = count.checked_add(1).ok_or("upload chunk count overflow")?;
            if count > 16_384 {
                return Err("upload has too many canonical chunks".into());
            }
        }
        let reference = CanonicalBodyReference::for_field(
            binding.upload_id.clone(),
            manifest.transcript.conversation_id.clone(),
            binding.message_id.clone(),
            binding.part_id.clone(),
            nola_domain::PartBodyField::ContentText,
            binding.expected_utf8_bytes,
            count,
        )
        .map_err(|error| error.to_string())?;
        store
            .prepare_transcript_content(
                &reference,
                &mut source_chunks(
                    uploads,
                    &binding.upload_id,
                    &owner,
                    binding.expected_utf8_bytes,
                ),
                maximum_bytes,
            )
            .map_err(|error| error.to_string())?;
        references.push(reference);
    }
    store
        .publish_transcript(&PreparedTranscriptPublication {
            import_id: manifest.transcript.import_id.clone(),
            manifest_json,
            expected_conversation_revision: manifest.expected_conversation_revision,
            append: AppendMessagesInput {
                conversation_id: manifest.transcript.conversation_id.clone(),
                expected_message_count: manifest.transcript.expected_message_count,
                messages: manifest
                    .transcript
                    .messages
                    .iter()
                    .cloned()
                    .map(|message| message.into_store())
                    .collect(),
                updated_at_ms: manifest.transcript.imported_at_ms,
            },
            references,
        })
        .map_err(|error| error.to_string())
}

fn source_chunks<'a, U: ContentUploadRepository + ?Sized>(
    uploads: &'a U,
    id: &'a str,
    owner: &'a UploadOwner,
    expected: u64,
) -> impl Iterator<Item = Result<(u64, String), String>> + 'a {
    let mut offset = 0_u64;
    let mut failed = false;
    std::iter::from_fn(move || {
        if failed || offset == expected {
            return None;
        }
        let next = uploads
            .read_chunk(id, owner, offset)
            .map_err(|error| error.to_string())
            .and_then(|text| text.ok_or_else(|| "sealed upload is missing a chunk".to_string()))
            .and_then(|text| {
                let end = offset
                    .checked_add(text.len() as u64)
                    .ok_or("upload offset overflow")?;
                if text.is_empty()
                    || text.len() as u64 > CANONICAL_BODY_CHUNK_MAX_BYTES
                    || end > expected
                {
                    return Err("sealed upload contains an invalid bounded chunk".into());
                }
                let previous = offset;
                offset = end;
                Ok((previous, text))
            });
        failed = next.is_err();
        Some(next)
    })
}
