// SPDX-License-Identifier: Apache-2.0
//! Service-owned upload staging. Canonical publication is not performed here.

use crate::{ApplicationCommand, ApplicationRuntime};
use nola_protocol::content_upload::{
    ContentUploadChunk, ContentUploadOwner, ContentUploadProgress, ContentUploadRequest,
    ContentUploadState,
};
use nola_store_port::archive_restore::ArchiveRestoreRepository;
use nola_store_port::content_upload::{ContentUploadRepository, UploadOwner, UploadProgress};
use nola_store_port::conversation_query::ConversationQueryRepository;
use serde_json::Value;

pub const MAXIMUM_UPLOAD_BYTES: u64 = 256 * 1024 * 1024;

impl<R> ApplicationRuntime<R>
where
    R: ArchiveRestoreRepository + ConversationQueryRepository,
{
    pub(crate) fn execute_content_upload(
        &mut self,
        command: ApplicationCommand,
    ) -> Result<Value, String> {
        let (upload_id, owner) = match &command {
            ApplicationCommand::ReadContentUploadChunk { input } => {
                ContentUploadRequest {
                    upload_id: input.upload_id.clone(),
                    owner: input.owner.clone(),
                }
                .validate()
                .map_err(|e| format!("{e:?}"))?;
                (&input.upload_id, &input.owner)
            }
            ApplicationCommand::BeginContentUpload { input } => {
                input
                    .validate(MAXIMUM_UPLOAD_BYTES)
                    .map_err(|e| format!("{e:?}"))?;
                (&input.upload_id, &input.owner)
            }
            ApplicationCommand::AppendContentChunk { input } => {
                input
                    .validate(MAXIMUM_UPLOAD_BYTES)
                    .map_err(|e| format!("{e:?}"))?;
                (&input.upload_id, &input.owner)
            }
            ApplicationCommand::ContentUploadProgress { input }
            | ApplicationCommand::SealContentUpload { input }
            | ApplicationCommand::CancelContentUpload { input } => {
                input.validate().map_err(|e| format!("{e:?}"))?;
                (&input.upload_id, &input.owner)
            }
            _ => return Err("not a content upload command".into()),
        };
        let staged_owner = self
            .store
            .archive_restore_accepts_upload(&owner.import_id, &owner.conversation_id)
            .map_err(|e| e.to_string())?;
        let archive_session = self
            .store
            .archive_restore_session_exists(&owner.import_id)
            .map_err(|e| e.to_string())?;
        let sealed_replay = archive_session
            && !staged_owner
            && !matches!(
                command,
                ApplicationCommand::AppendContentChunk { .. }
                    | ApplicationCommand::CancelContentUpload { .. }
            )
            && self
                .content_upload_store
                .progress(upload_id, &store_owner(owner))
                .is_ok_and(|progress| progress.sealed && !progress.cancelled);
        if !sealed_replay
            && ((archive_session && !staged_owner)
                || (!staged_owner
                    && self
                        .store
                        .conversation_summary(&owner.conversation_id)
                        .map_err(|e| e.to_string())?
                        .is_none()))
        {
            return Err("content upload owner has neither a live conversation nor an active archive profile".into());
        }
        let store = self.content_upload_store.as_mut();
        execute_staged_upload(store, &command, upload_id, owner)
    }
}

fn execute_staged_upload<R: ContentUploadRepository + ?Sized>(
    store: &mut R,
    command: &ApplicationCommand,
    upload_id: &str,
    owner: &ContentUploadOwner,
) -> Result<Value, String> {
    let stored_owner = store_owner(owner);
    if let ApplicationCommand::ReadContentUploadChunk { input } = command {
        let progress = store
            .progress(upload_id, &stored_owner)
            .map_err(|e| e.to_string())?;
        if !progress.sealed {
            return Err("content upload is not sealed".into());
        }
        let text = store
            .read_chunk(upload_id, &stored_owner, input.offset_bytes)
            .map_err(|e| e.to_string())?
            .unwrap_or_default();
        let next = input
            .offset_bytes
            .checked_add(text.len() as u64)
            .ok_or("content offset overflow")?;
        return serde_json::to_value(ContentUploadChunk {
            upload_id: upload_id.into(),
            owner: owner.clone(),
            offset_bytes: input.offset_bytes,
            next_offset_bytes: next,
            text,
            end_of_content: next == progress.expected_bytes,
        })
        .map_err(|e| e.to_string());
    }
    let progress = match command {
        ApplicationCommand::BeginContentUpload { input } => {
            store.begin(upload_id, &stored_owner, input.expected_utf8_bytes)
        }
        ApplicationCommand::AppendContentChunk { input } => {
            store.append(upload_id, &stored_owner, input.offset_bytes, &input.text)
        }
        ApplicationCommand::ContentUploadProgress { .. } => {
            store.progress(upload_id, &stored_owner)
        }
        ApplicationCommand::SealContentUpload { .. } => store.seal(upload_id, &stored_owner),
        ApplicationCommand::CancelContentUpload { .. } => store.cancel(upload_id, &stored_owner),
        _ => return Err("not a content upload command".into()),
    }
    .map_err(|e| e.to_string())?;
    serde_json::to_value(wire_progress(upload_id, owner, progress)).map_err(|e| e.to_string())
}

#[cfg(test)]
mod port_tests;

fn store_owner(owner: &ContentUploadOwner) -> UploadOwner {
    UploadOwner {
        conversation_id: owner.conversation_id.clone(),
        import_id: owner.import_id.clone(),
        part_id: owner.part_id.clone(),
    }
}
fn wire_progress(
    id: &str,
    owner: &ContentUploadOwner,
    progress: UploadProgress,
) -> ContentUploadProgress {
    ContentUploadProgress {
        upload_id: id.into(),
        owner: owner.clone(),
        state: if progress.cancelled {
            ContentUploadState::Cancelled
        } else if progress.sealed {
            ContentUploadState::Sealed
        } else {
            ContentUploadState::Receiving
        },
        expected_utf8_bytes: progress.expected_bytes,
        committed_utf8_bytes: progress.committed_bytes,
    }
}
