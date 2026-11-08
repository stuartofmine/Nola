// SPDX-License-Identifier: Apache-2.0
//! Preparation commits immutable content, never visible transcript messages.
use crate::content_upload::UploadOwner;
use nola_domain::CanonicalBodyReference;

pub trait TranscriptContentPreparationRepository {
    type Error: std::fmt::Display;

    /// Enforce persisted owner, complete seal and exact length. Copy with bounded
    /// chunks into immutable canonical storage before returning. Identical replay
    /// is safe; a different binding or body must not overwrite existing content.
    /// Failure may leave unreferenced objects, but must never publish a message.
    fn prepare_part(
        &mut self,
        upload_id: &str,
        owner: &UploadOwner,
        message_id: &str,
        expected_utf8_bytes: u64,
    ) -> Result<CanonicalBodyReference, Self::Error>;
}
