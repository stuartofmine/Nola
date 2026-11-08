// SPDX-License-Identifier: Apache-2.0
//! Prepares immutable bodies only. Publication must still check conversation
//! ownership, idle state, ordinal CAS and commit references in one transaction.
use nola_domain::CanonicalBodyReference;
use nola_protocol::uploaded_transcript::UploadedTranscriptManifest;
use nola_store_port::{
    content_upload::UploadOwner, transcript_preparation::TranscriptContentPreparationRepository,
};

pub fn prepare_uploaded_transcript<R: TranscriptContentPreparationRepository + ?Sized>(
    repository: &mut R,
    manifest: &UploadedTranscriptManifest,
    maximum_upload_bytes: u64,
) -> Result<Vec<CanonicalBodyReference>, String> {
    manifest.validate(maximum_upload_bytes)?;
    let mut references = Vec::with_capacity(manifest.uploads.len());
    for binding in &manifest.uploads {
        let owner = manifest.owner_for(binding);
        let reference = repository
            .prepare_part(
                &binding.upload_id,
                &UploadOwner {
                    conversation_id: owner.conversation_id,
                    import_id: owner.import_id,
                    part_id: owner.part_id,
                },
                &binding.message_id,
                binding.expected_utf8_bytes,
            )
            .map_err(|error| error.to_string())?;
        if reference.conversation_id() != manifest.transcript.conversation_id
            || reference.message_id() != binding.message_id
            || reference.part_id() != binding.part_id
            || reference.total_utf8_bytes() != binding.expected_utf8_bytes
        {
            return Err("prepared content does not match transcript binding".into());
        }
        references.push(reference);
    }
    Ok(references)
}
