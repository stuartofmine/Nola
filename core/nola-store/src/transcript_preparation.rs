// SPDX-License-Identifier: Apache-2.0
//! Bounded staging-to-canonical adapter; no access to transcript publication.
use crate::{
    canonical_text_store::CanonicalTextStore, content_upload::ContentUploadStore, StoreError,
};
use nola_domain::CanonicalBodyReference;
use nola_store_port::{
    content_upload::UploadOwner, transcript_preparation::TranscriptContentPreparationRepository,
};

pub struct TranscriptContentPreparationStore {
    uploads: ContentUploadStore,
    canonical: CanonicalTextStore,
}

impl TranscriptContentPreparationStore {
    pub fn new(uploads: ContentUploadStore, canonical: CanonicalTextStore) -> Self {
        Self { uploads, canonical }
    }
}

impl TranscriptContentPreparationRepository for TranscriptContentPreparationStore {
    type Error = String;

    fn prepare_part(
        &mut self,
        upload_id: &str,
        owner: &UploadOwner,
        message_id: &str,
        expected_utf8_bytes: u64,
    ) -> Result<CanonicalBodyReference, Self::Error> {
        // Upload identities are unique and immutable, including across imports.
        // Reusing one for another message conflicts with the canonical reference.
        let reference = self
            .uploads
            .sealed_content_reference(
                upload_id,
                owner,
                upload_id.to_owned(),
                message_id.to_owned(),
            )
            .map_err(|error| error.to_string())?;
        if reference.total_utf8_bytes() != expected_utf8_bytes {
            return Err("sealed upload length does not match transcript manifest".into());
        }
        self.uploads
            .copy_sealed_to_canonical(upload_id, owner, &reference, &mut self.canonical)
            .map_err(|error: StoreError| error.to_string())?;
        Ok(reference)
    }
}
