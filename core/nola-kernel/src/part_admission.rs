// SPDX-License-Identifier: Apache-2.0
//! Canonical Part mutation admission from bounded identity metadata.
use nola_domain::PartMutation;

pub struct CanonicalPartIdentity<'a> {
    pub part_id: &'a str,
    pub message_id: &'a str,
    pub kind: &'a str,
    pub state: &'a str,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PartAdmission {
    Apply,
    InvalidIdentity,
    MissingPart,
    OwnershipConflict,
    KindConflict,
    NotOpen,
}

pub fn evaluate_part_mutation(
    message_id: &str,
    mutation: &PartMutation,
    existing: Option<&CanonicalPartIdentity<'_>>,
) -> PartAdmission {
    let part_id = match mutation {
        PartMutation::Upsert { part_id, .. }
        | PartMutation::AppendText { part_id, .. }
        | PartMutation::Metadata { part_id, .. }
        | PartMutation::Close { part_id, .. } => part_id,
        PartMutation::Stream { operation, .. } => &operation.owner().part_id,
    };
    if message_id.trim().is_empty() || part_id.trim().is_empty() {
        return PartAdmission::InvalidIdentity;
    }
    if let PartMutation::Stream { operation, .. } = mutation {
        if operation.owner().validate().is_err() {
            return PartAdmission::InvalidIdentity;
        }
        if operation.owner().message_id != message_id {
            return PartAdmission::OwnershipConflict;
        }
        // This bounded identity does not carry conversation/run facts: Store
        // must check those against persisted ownership before applying the CAS.
    }
    if let Some(stored) = existing {
        if stored.part_id != part_id || stored.message_id != message_id {
            return PartAdmission::OwnershipConflict;
        }
    }
    match (mutation, existing) {
        (PartMutation::Upsert { kind, .. }, Some(stored)) if kind != stored.kind => {
            PartAdmission::KindConflict
        }
        (PartMutation::Upsert { .. }, _) => PartAdmission::Apply,
        (_, None) => PartAdmission::MissingPart,
        (PartMutation::Stream { .. }, Some(stored)) if stored.kind != "text" => {
            PartAdmission::KindConflict
        }
        (PartMutation::Stream { .. }, Some(stored)) if stored.state != "open" => {
            PartAdmission::NotOpen
        }
        (PartMutation::AppendText { .. }, Some(stored)) if stored.state != "open" => {
            PartAdmission::NotOpen
        }
        _ => PartAdmission::Apply,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn append() -> PartMutation {
        PartMutation::AppendText {
            part_id: "p".into(),
            expected_content_digest: "prefix".into(),
            text: "next".into(),
            updated_at_ms: 1,
        }
    }
    fn part() -> CanonicalPartIdentity<'static> {
        CanonicalPartIdentity {
            part_id: "p",
            message_id: "m",
            kind: "text",
            state: "open",
        }
    }
    #[test]
    fn append_requires_an_existing_open_owned_part() {
        assert_eq!(
            evaluate_part_mutation("m", &append(), None),
            PartAdmission::MissingPart
        );
        let mut stored = part();
        assert_eq!(
            evaluate_part_mutation("m", &append(), Some(&stored)),
            PartAdmission::Apply
        );
        for state in ["completed", "failed", "cancelled"] {
            stored.state = state;
            assert_eq!(
                evaluate_part_mutation("m", &append(), Some(&stored)),
                PartAdmission::NotOpen
            );
        }
        stored.state = "open";
        stored.message_id = "other";
        assert_eq!(
            evaluate_part_mutation("m", &append(), Some(&stored)),
            PartAdmission::OwnershipConflict
        );
        stored.message_id = "m";
        stored.part_id = "other";
        assert_eq!(
            evaluate_part_mutation("m", &append(), Some(&stored)),
            PartAdmission::OwnershipConflict
        );
    }
    #[test]
    fn upsert_cannot_repurpose_an_existing_part_kind() {
        let mutation = PartMutation::Upsert {
            part_id: "p".into(),
            kind: "reasoning".into(),
            ordinal: 1,
            state: "open".into(),
            content_text: "Evidence".into(),
            content_json: None,
            content_digest: None,
            created_at_ms: 1,
            updated_at_ms: 1,
        };
        assert_eq!(
            evaluate_part_mutation("m", &mutation, None),
            PartAdmission::Apply
        );
        assert_eq!(
            evaluate_part_mutation("m", &mutation, Some(&part())),
            PartAdmission::KindConflict
        );
    }
    #[test]
    fn close_checks_ownership_even_without_a_digest() {
        let mutation = PartMutation::Close {
            part_id: "p".into(),
            state: "completed".into(),
            content_json: None,
            content_digest: None,
            updated_at_ms: 1,
        };
        assert_eq!(
            evaluate_part_mutation("m", &mutation, None),
            PartAdmission::MissingPart
        );
        assert_eq!(
            evaluate_part_mutation("m", &mutation, Some(&part())),
            PartAdmission::Apply
        );
        assert_eq!(
            evaluate_part_mutation("other", &mutation, Some(&part())),
            PartAdmission::OwnershipConflict
        );
        assert_eq!(
            evaluate_part_mutation("", &mutation, Some(&part())),
            PartAdmission::InvalidIdentity
        );
    }

    #[test]
    fn stream_requires_owned_open_text_and_valid_full_owner_shape() {
        let owner = nola_domain::CanonicalStreamOwner {
            conversation_id: "c".into(),
            run_id: "r".into(),
            message_id: "m".into(),
            part_id: "p".into(),
            field: nola_domain::PartBodyField::ContentText,
        };
        let stream = |owner| PartMutation::Stream {
            operation: nola_domain::CanonicalStreamOperation::Declare { owner },
            updated_at_ms: 1,
        };
        assert_eq!(
            evaluate_part_mutation("m", &stream(owner.clone()), None),
            PartAdmission::MissingPart
        );
        let mut stored = part();
        assert_eq!(
            evaluate_part_mutation("m", &stream(owner.clone()), Some(&stored)),
            PartAdmission::Apply
        );
        stored.kind = "reasoning";
        assert_eq!(
            evaluate_part_mutation("m", &stream(owner.clone()), Some(&stored)),
            PartAdmission::KindConflict
        );
        stored.kind = "text";
        stored.state = "completed";
        assert_eq!(
            evaluate_part_mutation("m", &stream(owner.clone()), Some(&stored)),
            PartAdmission::NotOpen
        );
        stored.state = "open";
        let mut invalid = owner.clone();
        invalid.message_id = "foreign".into();
        assert_eq!(
            evaluate_part_mutation("m", &stream(invalid), Some(&stored)),
            PartAdmission::OwnershipConflict
        );
        let mut invalid = owner.clone();
        invalid.part_id = "foreign".into();
        assert_eq!(
            evaluate_part_mutation("m", &stream(invalid), Some(&stored)),
            PartAdmission::OwnershipConflict
        );
        let mut invalid = owner.clone();
        invalid.run_id.clear();
        assert_eq!(
            evaluate_part_mutation("m", &stream(invalid), Some(&stored)),
            PartAdmission::InvalidIdentity
        );
        let mut invalid = owner;
        invalid.field = nola_domain::PartBodyField::ContentJson;
        assert_eq!(
            evaluate_part_mutation("m", &stream(invalid), Some(&stored)),
            PartAdmission::InvalidIdentity
        );
    }

    #[test]
    fn metadata_needs_existing_owned_part_without_a_kind_replacement() {
        let mutation = PartMutation::Metadata {
            part_id: "p".into(),
            state: "completed".into(),
            content_json: None,
            updated_at_ms: 1,
        };
        assert_eq!(
            evaluate_part_mutation("m", &mutation, None),
            PartAdmission::MissingPart
        );
        assert_eq!(
            evaluate_part_mutation("m", &mutation, Some(&part())),
            PartAdmission::Apply
        );
        assert_eq!(
            evaluate_part_mutation("foreign", &mutation, Some(&part())),
            PartAdmission::OwnershipConflict
        );
        // No implicit Close authorization: the Store must reject this metadata
        // when an existing canonical stream has not yet been explicitly closed.
    }
}
