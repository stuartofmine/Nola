// SPDX-License-Identifier: Apache-2.0
//! Pure admission for immutable original fields; no body materialization or IO.
use nola_domain::{CanonicalBodyReference, MessagePartInput, PartBodyField};
use std::collections::{HashMap, HashSet};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BodyBindingError {
    InvalidIdentity,
    DuplicatePart,
    MissingPart,
    OwnerMismatch,
    NotFinalized,
    InlineConflict,
    DuplicateField,
    ReusedObject,
}

/// Publication intent, before any chunk geometry or durable object exists.
#[derive(Debug, Clone, Copy)]
pub struct BodyBindingDeclaration<'a> {
    pub content_id: &'a str,
    pub conversation_id: &'a str,
    pub message_id: &'a str,
    pub part_id: &'a str,
    pub field: PartBodyField,
}

/// The caller supplies bounded declarations and separately proves that every
/// referenced object is durable and complete in the publishing transaction.
/// The two original fields are independent, including for non-text Part kinds.
pub fn validate_body_bindings<'a>(
    conversation_id: &str,
    parts: impl IntoIterator<Item = (&'a str, &'a str, &'a MessagePartInput)>,
    references: &[CanonicalBodyReference],
) -> Result<(), BodyBindingError> {
    let declarations: Vec<_> = references
        .iter()
        .map(|reference| BodyBindingDeclaration {
            content_id: reference.content_id(),
            conversation_id: reference.conversation_id(),
            message_id: reference.message_id(),
            part_id: reference.part_id(),
            field: reference.field(),
        })
        .collect();
    validate_body_binding_declarations(conversation_id, parts, &declarations)
}

pub fn validate_body_binding_declarations<'a>(
    conversation_id: &str,
    parts: impl IntoIterator<Item = (&'a str, &'a str, &'a MessagePartInput)>,
    references: &[BodyBindingDeclaration<'_>],
) -> Result<(), BodyBindingError> {
    if references.is_empty() {
        return Ok(());
    }
    let mut declared = HashMap::new();
    for (message_id, status, part) in parts {
        if declared
            .insert(part.id.as_str(), (message_id, status, part))
            .is_some()
        {
            return Err(BodyBindingError::DuplicatePart);
        }
    }
    let mut fields = HashSet::new();
    let mut objects = HashSet::new();
    for reference in references {
        for id in [
            reference.content_id,
            reference.conversation_id,
            reference.message_id,
            reference.part_id,
        ] {
            if id.is_empty()
                || id.len() > 128
                || id.trim() != id
                || id.chars().any(char::is_control)
            {
                return Err(BodyBindingError::InvalidIdentity);
            }
        }
        let (message_id, status, part) = declared
            .get(reference.part_id)
            .ok_or(BodyBindingError::MissingPart)?;
        if reference.conversation_id != conversation_id || reference.message_id != *message_id {
            return Err(BodyBindingError::OwnerMismatch);
        }
        if *status != "completed" || !matches!(part.state.as_str(), "complete" | "completed") {
            return Err(BodyBindingError::NotFinalized);
        }
        if match reference.field {
            PartBodyField::ContentText => !part.content_text.is_empty(),
            PartBodyField::ContentJson => part.content_json.is_some(),
        } {
            return Err(BodyBindingError::InlineConflict);
        }
        if !fields.insert((reference.part_id, reference.field)) {
            return Err(BodyBindingError::DuplicateField);
        }
        if !objects.insert(reference.content_id) {
            return Err(BodyBindingError::ReusedObject);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn unprepared_binding_validates_identity_without_inventing_geometry() {
        let part = part();
        let declaration = BodyBindingDeclaration {
            content_id: "upload",
            conversation_id: "c",
            message_id: "m",
            part_id: "p",
            field: PartBodyField::ContentJson,
        };
        assert_eq!(
            validate_body_binding_declarations("c", [("m", "completed", &part)], &[declaration]),
            Ok(())
        );
        for bad in ["", " upload", "upload\n", &"x".repeat(129)] {
            let invalid = BodyBindingDeclaration {
                content_id: bad,
                ..declaration
            };
            assert_eq!(
                validate_body_binding_declarations("c", [("m", "completed", &part)], &[invalid]),
                Err(BodyBindingError::InvalidIdentity)
            );
        }
    }
    fn part() -> MessagePartInput {
        MessagePartInput {
            id: "p".into(),
            kind: "tool_result".into(),
            ordinal: 1,
            state: "completed".into(),
            content_text: String::new(),
            content_json: None,
            content_digest: None,
            created_at_ms: 1,
            updated_at_ms: 2,
        }
    }
    fn reference(id: &str, field: PartBodyField) -> CanonicalBodyReference {
        CanonicalBodyReference::for_field(
            id.into(),
            "c".into(),
            "m".into(),
            "p".into(),
            field,
            4,
            1,
        )
        .unwrap()
    }
    #[test]
    fn fields_are_independent_and_object_identity_is_not_shared() {
        let mut part = part();
        let text = reference("text", PartBodyField::ContentText);
        let json = reference("json", PartBodyField::ContentJson);
        assert_eq!(
            validate_body_bindings(
                "c",
                [("m", "completed", &part)],
                &[text.clone(), json.clone()]
            ),
            Ok(())
        );
        assert_eq!(
            validate_body_bindings(
                "c",
                [("m", "completed", &part)],
                &[text.clone(), text.clone()]
            ),
            Err(BodyBindingError::DuplicateField)
        );
        assert_eq!(
            validate_body_bindings(
                "c",
                [("m", "completed", &part)],
                &[text, reference("text", PartBodyField::ContentJson)]
            ),
            Err(BodyBindingError::ReusedObject)
        );
        part.content_text = "inline text remains alongside JSON".into();
        assert_eq!(
            validate_body_bindings("c", [("m", "completed", &part)], &[json.clone()]),
            Ok(())
        );
        part.content_json = Some("null".into());
        assert_eq!(
            validate_body_bindings("c", [("m", "completed", &part)], &[json]),
            Err(BodyBindingError::InlineConflict)
        );
    }
    #[test]
    fn owner_finality_and_unambiguous_declaration_are_required() {
        let mut part = part();
        let refs = [reference("text", PartBodyField::ContentText)];
        assert_eq!(
            validate_body_bindings("other", [("m", "completed", &part)], &refs),
            Err(BodyBindingError::OwnerMismatch)
        );
        assert_eq!(
            validate_body_bindings("c", [("other", "completed", &part)], &refs),
            Err(BodyBindingError::OwnerMismatch)
        );
        assert_eq!(
            validate_body_bindings("c", [("m", "open", &part)], &refs),
            Err(BodyBindingError::NotFinalized)
        );
        assert_eq!(
            validate_body_bindings(
                "c",
                [("m", "completed", &part), ("m2", "completed", &part)],
                &refs
            ),
            Err(BodyBindingError::DuplicatePart)
        );
        part.state = "failed".into();
        assert_eq!(
            validate_body_bindings("c", [("m", "completed", &part)], &refs),
            Err(BodyBindingError::NotFinalized)
        );
        part.id = "other".into();
        assert_eq!(
            validate_body_bindings("c", [("m", "completed", &part)], &refs),
            Err(BodyBindingError::MissingPart)
        );
    }
}
