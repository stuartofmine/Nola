// SPDX-License-Identifier: Apache-2.0
//! Archive manifests authorize sealed uploads before staging canonical references.
use nola_domain::{archive_restore::*, CanonicalBodyReference, CANONICAL_BODY_CHUNK_MAX_BYTES};
use nola_store_port::{
    archive_restore::ArchiveRestoreRepository,
    content_upload::{ContentUploadRepository, UploadOwner},
};

pub fn stage_archive_manifest<
    S: ArchiveRestoreRepository + ?Sized,
    U: ContentUploadRepository + ?Sized,
>(
    store: &mut S,
    uploads: &U,
    input: &StageArchiveManifest,
    maximum_bytes: u64,
) -> Result<ArchiveRestoreProgress, String> {
    // Bound the external intent before traversing uploads or touching staging.
    let mut budget = ManifestBudget(128 * 1024);
    serde_json::to_writer(&mut budget, input)
        .map_err(|_| "archive manifest exceeds encoded byte budget".to_string())?;
    let entry = match &input.entry {
        ArchiveRestoreManifestEntry::ToolSource { claim, upload_id } => {
            ArchiveRestoreEntry::ToolSource {
                claim: prepare_tool_source_upload(store, uploads, input, claim, upload_id, maximum_bytes)?,
            }
        }
        ArchiveRestoreManifestEntry::Profile { profile } => ArchiveRestoreEntry::Profile {
            profile: profile.clone(),
        },
        ArchiveRestoreManifestEntry::PartDerivation { record } => {
            let mut budget = ManifestBudget(16 * 1024);
            serde_json::to_writer(&mut budget, record)
                .map_err(|_| "archive Part derivation exceeds encoded byte budget".to_string())?;
            ArchiveRestoreEntry::PartDerivation {
                record: record.clone(),
            }
        }
        ArchiveRestoreManifestEntry::Message {
            conversation_id,
            message,
            uploads: bindings,
        } => {
            if bindings.len() > 512 || message.parts.len() > 512 {
                return Err("archive message has too many body bindings".into());
            }
            let mut bound = std::collections::HashSet::new();
            let mut upload_ids = std::collections::HashSet::new();
            let mut declared = Vec::new();
            // Admit the entire message/binding set before preparing any body.
            let valid_id = |value: &str| {
                !value.is_empty()
                    && value.len() <= 128
                    && value.trim() == value
                    && !value.chars().any(char::is_control)
            };
            if !matches!(message.role.as_str(), "user" | "assistant" | "system")
                || !valid_id(conversation_id)
                || !valid_id(&message.id)
                || message.run_id.as_deref().is_some_and(|run| !valid_id(run))
                || message.status != "completed"
                || message.ordinal < 1
                || !message
                    .completed_at_ms
                    .is_some_and(|at| at >= message.created_at_ms)
                || message.parts.is_empty()
            {
                return Err("archive message must be finalized history".into());
            }
            let mut part_ids = std::collections::HashSet::new();
            for (index, part) in message.parts.iter().enumerate() {
                if !valid_id(&part.id)
                    || part.ordinal != index as i64 + 1
                    || part.state != "completed"
                    || part.updated_at_ms < part.created_at_ms
                    || !part_ids.insert(part.id.as_str())
                {
                    return Err("invalid archive Part identity, order or state".into());
                }
                if let Some(json) = &part.content_json {
                    serde_json::from_str::<serde_json::Value>(json).map_err(|e| e.to_string())?;
                }
            }
            for binding in bindings {
                if binding.message_id != message.id
                    || binding.expected_utf8_bytes == 0
                    || binding.expected_utf8_bytes > maximum_bytes
                    || !bound.insert((binding.part_id.as_str(), binding.field))
                    || !upload_ids.insert(binding.upload_id.as_str())
                {
                    return Err("archive upload owner or body source mismatch".into());
                }
                declared.push(nola_kernel::BodyBindingDeclaration {
                    content_id: &binding.upload_id,
                    conversation_id,
                    message_id: &binding.message_id,
                    part_id: &binding.part_id,
                    field: binding.field,
                });
            }
            nola_kernel::validate_body_binding_declarations(
                conversation_id,
                message
                    .parts
                    .iter()
                    .map(|part| (message.id.as_str(), message.status.as_str(), part)),
                &declared,
            )
            .map_err(|e| format!("invalid archive field binding: {e:?}"))?;
            // Prepared retries must all agree before a new first body is created.
            for binding in bindings {
                if let Some(reference) = store
                    .prepared_archive_reference(&input.restore_id, &binding.upload_id)
                    .map_err(|e| e.to_string())?
                {
                    validate_prepared(&reference, conversation_id, binding)?;
                }
            }
            let mut references = Vec::new();
            for binding in bindings {
                let prepared = store
                    .prepared_archive_reference(&input.restore_id, &binding.upload_id)
                    .map_err(|e| e.to_string())?;
                let reference = if let Some(reference) = prepared {
                    reference
                } else {
                    let owner = UploadOwner {
                        conversation_id: conversation_id.clone(),
                        import_id: input.restore_id.clone(),
                        part_id: binding.part_id.clone(),
                    };
                    let progress = uploads
                        .progress(&binding.upload_id, &owner)
                        .map_err(|e| e.to_string())?;
                    if !progress.sealed
                        || progress.cancelled
                        || progress.expected_bytes != binding.expected_utf8_bytes
                        || progress.committed_bytes != binding.expected_utf8_bytes
                    {
                        return Err("archive requires an exact sealed upload".into());
                    }
                    let mut count = 0u32;
                    for chunk in chunks(
                        uploads,
                        &binding.upload_id,
                        &owner,
                        binding.expected_utf8_bytes,
                    ) {
                        chunk?;
                        count = count.checked_add(1).ok_or("archive chunk count overflow")?;
                        if count > 16384 {
                            return Err("archive body has too many chunks".into());
                        }
                    }
                    let reference = CanonicalBodyReference::for_field(
                        binding.upload_id.clone(),
                        conversation_id.clone(),
                        message.id.clone(),
                        binding.part_id.clone(),
                        binding.field,
                        binding.expected_utf8_bytes,
                        count,
                    )
                    .map_err(|e| e.to_string())?;
                    store
                        .prepare_archive_content(
                            &input.restore_id,
                            &reference,
                            &mut chunks(
                                uploads,
                                &binding.upload_id,
                                &owner,
                                binding.expected_utf8_bytes,
                            ),
                            maximum_bytes,
                        )
                        .map_err(|e| e.to_string())?;
                    reference
                };
                validate_prepared(&reference, conversation_id, binding)?;
                references.push(reference);
            }
            ArchiveRestoreEntry::Message {
                conversation_id: conversation_id.clone(),
                message: message.clone(),
                references,
            }
        }
    };
    store
        .stage_archive_entry(&input.restore_id, input.sequence, &entry)
        .map_err(|e| e.to_string())
}

fn prepare_tool_source_upload<
    S: ArchiveRestoreRepository + ?Sized,
    U: ContentUploadRepository + ?Sized,
>(
    store: &mut S,
    uploads: &U,
    input: &StageArchiveManifest,
    claim: &nola_domain::tool_source::ToolSourceClaim,
    upload_id: &str,
    maximum_bytes: u64,
) -> Result<nola_domain::tool_source::ToolSourceClaim, String> {
    use sha2::{Digest, Sha256};
    claim.validate()?;
    let total = claim.body.total_utf8_bytes();
    if upload_id.is_empty()
        || upload_id.len() > 128
        || upload_id.trim() != upload_id
        || upload_id.chars().any(char::is_control)
        || total == 0
        || total > maximum_bytes
    {
        return Err("invalid archive tool source upload identity or length".into());
    }
    let owner = UploadOwner {
        conversation_id: claim.owner.conversation_id().into(),
        import_id: input.restore_id.clone(),
        part_id: claim.owner.part_id().into(),
    };
    let progress = uploads.progress(upload_id, &owner).map_err(|e| e.to_string())?;
    if !progress.sealed || progress.cancelled
        || progress.expected_bytes != total || progress.committed_bytes != total
    {
        return Err("archive tool source requires an exact sealed upload".into());
    }
    let mut digest = Sha256::new();
    let mut count = 0u32;
    for chunk in chunks(uploads, upload_id, &owner, total) {
        let (_, text) = chunk?;
        digest.update(text.as_bytes());
        count = count.checked_add(1).ok_or("archive source chunk count overflow")?;
        if count > 16384 {
            return Err("archive source has too many chunks".into());
        }
    }
    if format!("{:x}", digest.finalize()) != claim.body_sha256 {
        return Err("archive tool source original digest differs".into());
    }
    // Transport-supplied content IDs never select an existing local body.
    let content_id = format!("archive-source:{:x}", Sha256::digest(
        serde_json::to_vec(&(&input.restore_id, upload_id)).map_err(|e| e.to_string())?
    ));
    let reference = CanonicalBodyReference::for_field(
        content_id.clone(), claim.owner.conversation_id().into(),
        claim.owner.message_id().into(), claim.owner.part_id().into(),
        nola_domain::PartBodyField::ContentJson, total, count,
    ).map_err(|e| e.to_string())?;
    if let Some(prepared) = store.prepared_archive_reference(&input.restore_id, &content_id)
        .map_err(|e| e.to_string())?
    {
        if prepared != reference {
            return Err("archive tool source prepared owner or length differs".into());
        }
    } else {
        store.prepare_archive_content(
            &input.restore_id, &reference,
            &mut chunks(uploads, upload_id, &owner, total), maximum_bytes,
        ).map_err(|e| e.to_string())?;
    }
    let mut prepared = claim.clone();
    prepared.body = reference;
    prepared.validate()?;
    Ok(prepared)
}

fn validate_prepared(
    reference: &CanonicalBodyReference,
    conversation_id: &str,
    binding: &ArchivePartUpload,
) -> Result<(), String> {
    if reference.conversation_id() != conversation_id
        || reference.message_id() != binding.message_id
        || reference.part_id() != binding.part_id
        || reference.field() != binding.field
        || reference.total_utf8_bytes() != binding.expected_utf8_bytes
    {
        return Err("archive prepared owner, field or length differs".into());
    }
    Ok(())
}

fn chunks<'a, U: ContentUploadRepository + ?Sized>(
    uploads: &'a U,
    id: &'a str,
    owner: &'a UploadOwner,
    total: u64,
) -> impl Iterator<Item = Result<(u64, String), String>> + 'a {
    let mut offset = 0;
    let mut failed = false;
    std::iter::from_fn(move || {
        if failed || offset == total {
            return None;
        }
        let next = uploads
            .read_chunk(id, owner, offset)
            .map_err(|e| e.to_string())
            .and_then(|chunk| {
                chunk.ok_or_else(|| "sealed archive upload is missing a chunk".into())
            })
            .and_then(|text| {
                let end = offset
                    .checked_add(text.len() as u64)
                    .ok_or("archive upload offset overflow")?;
                if text.is_empty()
                    || text.len() as u64 > CANONICAL_BODY_CHUNK_MAX_BYTES
                    || end > total
                {
                    return Err("invalid sealed archive chunk geometry".into());
                }
                let start = offset;
                offset = end;
                Ok((start, text))
            });
        failed = next.is_err();
        Some(next)
    })
}
struct ManifestBudget(usize);
impl std::io::Write for ManifestBudget {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.0 = self
            .0
            .checked_sub(bytes.len())
            .ok_or_else(|| std::io::Error::other("archive manifest byte budget"))?;
        Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}
