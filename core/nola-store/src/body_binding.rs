// SPDX-License-Identifier: Apache-2.0
//! Shared binding admission before any local or replicated publication.
use crate::{StoreError, StoreResult};
use nola_domain::{CanonicalBodyReference, MessagePartInput};

pub(super) fn validate_field_bindings<'a>(
    conversation_id: &str,
    parts: impl IntoIterator<Item = (&'a str, &'a str, &'a MessagePartInput)>,
    references: &[CanonicalBodyReference],
) -> StoreResult<()> {
    nola_kernel::validate_body_bindings(conversation_id, parts, references).map_err(|error| {
        StoreError::Invariant(format!("invalid canonical field binding: {error:?}"))
    })
}

pub(super) fn validate_text_bindings<'a>(
    conversation_id: &str,
    parts: impl IntoIterator<Item = (&'a str, &'a str, &'a MessagePartInput)>,
    references: &[CanonicalBodyReference],
) -> StoreResult<()> {
    if references.is_empty() {
        return Ok(());
    }
    for reference in references {
        crate::canonical_body::require_text_reference(reference)?;
    }
    let parts: Vec<_> = parts.into_iter().collect();
    validate_field_bindings(conversation_id, parts.iter().copied(), references)?;
    // The external transcript-upload contract is still specifically text-only;
    // generic append/archive callers use validate_field_bindings directly.
    for reference in references {
        let (_, _, part) = parts
            .iter()
            .find(|(_, _, part)| part.id == reference.part_id())
            .ok_or_else(|| StoreError::Invariant("undeclared canonical field Part".into()))?;
        if part.kind != "text" || part.content_json.is_some() {
            return Err(StoreError::Invariant(
                "structured Part publication is not enabled".into(),
            ));
        }
    }
    Ok(())
}
