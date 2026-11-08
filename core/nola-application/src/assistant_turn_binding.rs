// SPDX-License-Identifier: Apache-2.0
//! Explicit accepted Core ownership facts for the canonical write transaction.
//! A display string, checkpoint scan, or guessed message suffix is not a fact.

use std::collections::BTreeMap;

use nola_domain::AssistantTurnBinding;

use crate::canonical_part_projection::CanonicalPartProjection;

#[derive(Default)]
pub(crate) struct AssistantTurnBindings {
    boundaries: Vec<AssistantTurnBinding>,
    parts: BTreeMap<String, AssistantTurnBinding>,
    turns: BTreeMap<String, AssistantTurnBinding>,
}

impl AssistantTurnBindings {
    pub(crate) fn born_message(&self, turn_id: &str) -> Option<&str> {
        match self.turns.get(turn_id) {
            Some(AssistantTurnBinding::TurnStarted { assistant_message_id, .. }) => Some(assistant_message_id),
            _ => None,
        }
    }
    pub(crate) fn record_turn(&mut self, turn_id: &str, message_id: &str) -> Result<(), String> {
        if turn_id.is_empty() || message_id.is_empty() {
            return Err("accepted Turn birth has empty ownership".into());
        }
        let binding = AssistantTurnBinding::TurnStarted {
            turn_id: turn_id.into(), assistant_message_id: message_id.into(),
        };
        if self.turns.get(turn_id).is_some_and(|previous| previous != &binding) {
            return Err("accepted Turn birth ownership changed within the transition".into());
        }
        self.turns.insert(turn_id.into(), binding);
        Ok(())
    }
    pub(crate) fn record_segment(&mut self, turn: &str, message: &str) {
        self.boundaries.push(AssistantTurnBinding::SegmentStarted {
            turn_id: turn.into(), assistant_message_id: message.into(),
        });
    }
    pub(crate) fn record_activity(&mut self, turn: &str, message: &str, id: &str, kind: &str) {
        self.boundaries.push(AssistantTurnBinding::Activity { turn_id:turn.into(), assistant_message_id:message.into(),activity_id:id.into(),activity_kind:kind.into() });
    }
    pub(crate) fn record_effect(&mut self, turn: &str, message: &str, effect: &str) {
        self.boundaries.push(AssistantTurnBinding::Effect {
            turn_id: turn.into(), assistant_message_id: message.into(), effect_id: effect.into(),
        });
    }
    pub(crate) fn record_part(
        &mut self,
        turn_id: &str,
        canonical: &CanonicalPartProjection<'_>,
    ) -> Result<(), String> {
        let part = canonical.part();
        if part.turn_id() != turn_id {
            return Err("accepted Core Part and event have different Turn owners".into());
        }
        let binding = AssistantTurnBinding::Part {
            turn_id: turn_id.to_owned(),
            assistant_message_id: canonical.message_id().to_owned(),
            part_id: part.id().to_owned(),
        };
        if let Some(previous) = self.parts.get(part.id()) {
            if previous != &binding {
                return Err("accepted Core Part ownership changed within the transition".into());
            }
        } else {
            self.parts.insert(part.id().to_owned(), binding);
        }
        Ok(())
    }

    pub(crate) fn into_bindings(self) -> Vec<AssistantTurnBinding> {
        self.turns.into_values().chain(self.boundaries).chain(self.parts.into_values()).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nola_agent_engine::agent::parts::{AgentPart, PartOwnership, TextPart};

    #[test]
    fn turn_birth_is_exact_idempotent_and_independent_of_parts() {
        let mut bindings = AssistantTurnBindings::default();
        bindings.record_turn("successor", "opaque-message").unwrap();
        assert_eq!(bindings.born_message("successor"), Some("opaque-message"));
        assert_eq!(bindings.born_message("unknown"), None);
        bindings.record_turn("successor", "opaque-message").unwrap();
        assert!(bindings.record_turn("successor", "different-message").is_err());
        assert!(bindings.record_turn("", "opaque-message").is_err());
        assert_eq!(bindings.into_bindings(), vec![AssistantTurnBinding::TurnStarted {
            turn_id: "successor".into(), assistant_message_id: "opaque-message".into(),
        }]);
    }

    fn text(id: &str, turn: &str, message: &str, body: String) -> AgentPart {
        AgentPart::Text(TextPart {
            id: id.into(),
            ownership: PartOwnership {
                message_id: message.into(),
                turn_id: turn.into(),
            },
            text: body,
            is_complete: false,
            review: None,
        })
    }

    #[test]
    fn binding_uses_accepted_owner_without_copying_body_or_repeating_delta_entries() {
        let part = text(
            "arbitrary-part",
            "child-turn",
            "actual-successor",
            "中".repeat(1024 * 1024),
        );
        let AgentPart::Text(source) = &part else {
            unreachable!()
        };
        let pointer = source.text.as_ptr();
        let canonical =
            CanonicalPartProjection::new(&part, "source-run", Some("admission")).unwrap();
        let mut bindings = AssistantTurnBindings::default();
        for _ in 0..1000 {
            bindings.record_part("child-turn", &canonical).unwrap();
        }
        assert_eq!(canonical.part(), &part);
        assert_eq!(source.text.as_ptr(), pointer);
        assert_eq!(
            bindings.into_bindings(),
            vec![AssistantTurnBinding::Part {
                turn_id: "child-turn".into(),
                assistant_message_id: "actual-successor".into(),
                part_id: "arbitrary-part".into(),
            }]
        );
    }

    #[test]
    fn foreign_event_turn_is_not_an_ownership_binding() {
        let part = text("part", "actual-turn", "assistant", String::new());
        let canonical =
            CanonicalPartProjection::new(&part, "source-run", Some("admission")).unwrap();
        let mut bindings = AssistantTurnBindings::default();
        assert!(bindings.record_part("another-turn", &canonical).is_err());
        assert!(bindings.into_bindings().is_empty());
    }

    #[test]
    fn same_part_cannot_rebind_its_turn_or_message_in_one_commit() {
        let first = text("part", "turn", "assistant-a", "first".into());
        let second = text("part", "turn", "assistant-b", "second".into());
        let mut bindings = AssistantTurnBindings::default();
        bindings
            .record_part(
                "turn",
                &CanonicalPartProjection::new(&first, "run", None).unwrap(),
            )
            .unwrap();
        assert!(bindings
            .record_part(
                "turn",
                &CanonicalPartProjection::new(&second, "run", None).unwrap()
            )
            .is_err());
        assert_eq!(bindings.into_bindings().len(), 1);
    }
}
