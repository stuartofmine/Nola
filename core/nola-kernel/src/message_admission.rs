// SPDX-License-Identifier: Apache-2.0
//! Canonical message identity admission without persistence access.
use nola_domain::TransitionMessageInput;

pub struct CanonicalMessageIdentity<'a> {
    pub conversation_id: &'a str,
    pub message_id: &'a str,
    pub run_id: Option<&'a str>,
    pub role: &'a str,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MessageAdmission {
    Insert,
    Existing,
    InvalidIdentity,
    UnsupportedRole,
    IdentityConflict,
}

pub fn evaluate_message_declaration(
    conversation_id: &str,
    incoming: &TransitionMessageInput,
    existing: Option<&CanonicalMessageIdentity<'_>>,
) -> MessageAdmission {
    if conversation_id.trim().is_empty() || incoming.id.trim().is_empty() {
        return MessageAdmission::InvalidIdentity;
    }
    if let Some(existing) = existing {
        return if existing.conversation_id == conversation_id
            && existing.message_id == incoming.id
            && existing.run_id == incoming.run_id.as_deref()
            && existing.role == incoming.role
        {
            // A declaration ensures existence. It never overwrites a previously
            // committed status, timestamp, or content merely because it repeats.
            MessageAdmission::Existing
        } else {
            MessageAdmission::IdentityConflict
        };
    }
    if !matches!(incoming.role.as_str(), "user" | "assistant" | "system") {
        return MessageAdmission::UnsupportedRole;
    }
    MessageAdmission::Insert
}

#[cfg(test)]
mod tests {
    use super::*;
    fn message() -> TransitionMessageInput {
        TransitionMessageInput {
            id: "m".into(),
            run_id: Some("r".into()),
            role: "assistant".into(),
            status: "streaming".into(),
            created_at_ms: 1,
            completed_at_ms: None,
            initial_parts: Vec::new(),
        }
    }
    fn identity() -> CanonicalMessageIdentity<'static> {
        CanonicalMessageIdentity {
            conversation_id: "c",
            message_id: "m",
            run_id: Some("r"),
            role: "assistant",
        }
    }
    #[test]
    fn new_messages_require_supported_roles_and_stable_identity() {
        for role in ["user", "assistant", "system"] {
            let mut input = message();
            input.role = role.into();
            assert_eq!(
                evaluate_message_declaration("c", &input, None),
                MessageAdmission::Insert
            );
        }
        let mut input = message();
        input.role = "unknown".into();
        assert_eq!(
            evaluate_message_declaration("c", &input, None),
            MessageAdmission::UnsupportedRole
        );
        input.id = " ".into();
        assert_eq!(
            evaluate_message_declaration("c", &input, None),
            MessageAdmission::InvalidIdentity
        );
        assert_eq!(
            evaluate_message_declaration("", &message(), None),
            MessageAdmission::InvalidIdentity
        );
    }
    #[test]
    fn each_identity_dimension_prevents_reusing_an_existing_message() {
        for field in 0..4 {
            let mut existing = identity();
            match field {
                0 => existing.conversation_id = "other",
                1 => existing.message_id = "other",
                2 => existing.run_id = None,
                _ => existing.role = "user",
            }
            assert_eq!(
                evaluate_message_declaration("c", &message(), Some(&existing)),
                MessageAdmission::IdentityConflict
            );
        }
    }
    #[test]
    fn repeated_declarations_do_not_authorize_overwriting_status_or_time() {
        let mut incoming = message();
        incoming.status = "completed".into();
        incoming.completed_at_ms = Some(99);
        incoming.created_at_ms = 98;
        assert_eq!(
            evaluate_message_declaration("c", &incoming, Some(&identity())),
            MessageAdmission::Existing
        );
    }
}
