// SPDX-License-Identifier: Apache-2.0
//! Single-owner resolution for ordinary projection and incremental publication.
use nola_domain::ModelAttemptRecord;
use nola_store_port::assistant_ownership::{AssistantOwner, AssistantOwnershipFacts};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AssistantOwnerError {
    InvalidFact,
    AmbiguousTurn(String),
    AmbiguousPart(String),
    AttemptLineage(String),
    ForeignSession,
    ForeignRun,
}
impl std::fmt::Display for AssistantOwnerError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{self:?}")
    }
}
impl std::error::Error for AssistantOwnerError {}

pub(crate) struct AssistantOwnerResolver {
    conversation_id: String,
    core_session_id: Option<String>,
    turns: BTreeMap<String, AssistantOwner>,
    activities: BTreeMap<(String,String), (String,AssistantOwner)>,
    parts: BTreeMap<(String, String), AssistantOwner>,
    part_turns: BTreeMap<(String, String), Option<String>>,
    part_ids: BTreeSet<String>,
    admissions: BTreeSet<String>,
    ambiguous_turns: BTreeSet<String>,
}
impl AssistantOwnerResolver {
    pub(crate) fn new(
        conversation_id: &str,
        core_session_id: Option<&str>,
        facts: &AssistantOwnershipFacts,
    ) -> Result<Self, AssistantOwnerError> {
        if conversation_id.trim().is_empty()
            || core_session_id.is_some_and(|id| id.trim().is_empty())
        {
            return Err(AssistantOwnerError::InvalidFact);
        }
        let mut result = Self {
            conversation_id: conversation_id.into(),
            core_session_id: core_session_id.map(str::to_owned),
            turns: BTreeMap::new(),
            activities: BTreeMap::new(),
            parts: BTreeMap::new(),
            part_turns: BTreeMap::new(),
            part_ids: BTreeSet::new(),
            admissions: BTreeSet::new(),
            ambiguous_turns: BTreeSet::new(),
        };
        for owner in &facts.admissions {
            result.validate_owner(owner)?;
        }
        for turn in facts.turns.iter().filter(|turn| turn.admission) {
            result.validate_owner(&turn.owner)?;
            if !facts.admissions.contains(&turn.owner) {
                return Err(AssistantOwnerError::InvalidFact);
            }
            result.insert_turn(&turn.turn_id, &turn.owner)?;
            result.admissions.insert(turn.turn_id.clone());
        }
        for turn in facts.turns.iter().filter(|turn| !turn.admission) {
            result.validate_owner(&turn.owner)?;
            if !result.admissions.contains(&turn.turn_id) {
                result.insert_turn(&turn.turn_id, &turn.owner)?;
            }
        }
        for activity in &facts.activities {
            result.validate_owner(&activity.owner)?;
            if !matches!(activity.kind.as_str(),"task"|"skill") || activity.id.is_empty() || activity.turn_id.is_empty()
                || result.activities.insert((activity.kind.clone(),activity.id.clone()),(activity.turn_id.clone(),activity.owner.clone())).is_some() {
                return Err(AssistantOwnerError::InvalidFact);
            }
        }
        for part in &facts.parts {
            result.validate_owner(&part.owner)?;
            if part.part_id.trim().is_empty() {
                return Err(AssistantOwnerError::InvalidFact);
            }
            result.part_ids.insert(part.part_id.clone());
            let key=(part.owner.run_id.clone(),part.part_id.clone());
            if result.part_turns.insert(key,part.turn_id.clone()).is_some_and(|old|old!=part.turn_id) {
                return Err(AssistantOwnerError::AmbiguousPart(part.part_id.clone()));
            }
            if result
                .parts
                .insert(
                    (part.owner.run_id.clone(), part.part_id.clone()),
                    part.owner.clone(),
                )
                .is_some_and(|old| old != part.owner)
            {
                return Err(AssistantOwnerError::AmbiguousPart(part.part_id.clone()));
            }
        }
        Ok(result)
    }
    fn validate_owner(&self, owner: &AssistantOwner) -> Result<(), AssistantOwnerError> {
        if owner.conversation_id != self.conversation_id {
            return Err(AssistantOwnerError::ForeignSession);
        }
        if owner.run_id.trim().is_empty() || owner.assistant_message_id.trim().is_empty() {
            return Err(AssistantOwnerError::InvalidFact);
        }
        Ok(())
    }
    fn insert_turn(
        &mut self,
        turn: &str,
        owner: &AssistantOwner,
    ) -> Result<(), AssistantOwnerError> {
        if turn.trim().is_empty() {
            return Err(AssistantOwnerError::InvalidFact);
        }
        if self
            .turns
            .insert(turn.to_owned(), owner.clone())
            .is_some_and(|old| old != *owner)
        {
            self.ambiguous_turns.insert(turn.to_owned());
        }
        Ok(())
    }
    pub(crate) fn resolve_turn(
        &self,
        session_id: &str,
        turn_id: &str,
    ) -> Result<Option<&AssistantOwner>, AssistantOwnerError> {
        if self.core_session_id.as_deref() != Some(session_id) {
            return Err(AssistantOwnerError::ForeignSession);
        }
        if self.ambiguous_turns.contains(turn_id) {
            return Err(AssistantOwnerError::AmbiguousTurn(turn_id.into()));
        }
        Ok(self.turns.get(turn_id))
    }
    pub(crate) fn resolve_part(&self, run_id: &str, part_id: &str) -> Result<Option<&AssistantOwner>, AssistantOwnerError> {
        let owner = self.parts.get(&(run_id.to_owned(), part_id.to_owned()));
        if owner.is_none() && self.part_ids.contains(part_id) { return Err(AssistantOwnerError::ForeignRun); }
        Ok(owner)
    }
    pub(crate) fn resolve_part_with_turn(&self, run_id: &str, part_id: &str, core_turn: &str) -> Result<Option<&AssistantOwner>, AssistantOwnerError> {
        let owner=self.resolve_part(run_id,part_id)?;
        if self.part_turns.get(&(run_id.to_owned(),part_id.to_owned())).and_then(|turn|turn.as_deref())
            .is_some_and(|turn|turn!=core_turn) { return Err(AssistantOwnerError::InvalidFact); }
        Ok(owner)
    }
    pub(crate) fn resolve_activity(&self, session: &str, turn: &str, kind: &str, id: &str) -> Result<Option<&AssistantOwner>,AssistantOwnerError> {
        let root=self.resolve_turn(session,turn)?;
        let entry=self.activities.get(&(kind.into(),id.into()));
        if entry.is_some_and(|(bound_turn,_)|bound_turn!=turn) { return Err(AssistantOwnerError::InvalidFact); }
        let owner=entry.map(|(_,owner)|owner);
        if let (Some(root),Some(owner))=(root,owner) {
            if root.run_id!=owner.run_id { return Err(AssistantOwnerError::ForeignRun); }
        }
        Ok(owner)
    }
    pub(crate) fn resolve_attempt<'a>(
        &'a self,
        attempt: &ModelAttemptRecord,
        attempts: &BTreeMap<&str, &ModelAttemptRecord>,
    ) -> Result<Option<&'a AssistantOwner>, AssistantOwnerError> {
        let mut current = attempt;
        let mut visited = BTreeSet::new();
        loop {
            if !visited.insert(current.id.as_str()) {
                return Err(AssistantOwnerError::AttemptLineage(current.id.clone()));
            }
            if current.run_id != attempt.run_id
                || current.effect_id != attempt.effect_id
                || current.invocation_id != attempt.invocation_id
            {
                return Err(AssistantOwnerError::AttemptLineage(current.id.clone()));
            }
            if let Some(parent_id) = &current.predecessor_id {
                let parent = attempts
                    .get(parent_id.as_str())
                    .ok_or_else(|| AssistantOwnerError::AttemptLineage(parent_id.clone()))?;
                if parent.run_id != current.run_id
                    || parent.effect_id != current.effect_id
                    || parent.invocation_id != current.invocation_id
                {
                    return Err(AssistantOwnerError::AttemptLineage(parent_id.clone()));
                }
            }
            let text = self
                .parts
                .get(&(current.run_id.clone(), current.text_part_id.clone()));
            let reasoning = self
                .parts
                .get(&(current.run_id.clone(), current.reasoning_part_id.clone()));
            if (text.is_none() && self.part_ids.contains(&current.text_part_id))
                || (reasoning.is_none() && self.part_ids.contains(&current.reasoning_part_id))
            {
                return Err(AssistantOwnerError::ForeignRun);
            }
            if let (Some(a), Some(b)) = (text, reasoning) {
                if a != b {
                    return Err(AssistantOwnerError::AmbiguousPart(current.id.clone()));
                }
            }
            if let Some(owner) = text.or(reasoning) {
                if owner.run_id != attempt.run_id {
                    return Err(AssistantOwnerError::ForeignRun);
                }
                return Ok(Some(owner));
            }
            let Some(parent) = &current.predecessor_id else {
                return Ok(None);
            };
            current = attempts
                .get(parent.as_str())
                .copied()
                .ok_or_else(|| AssistantOwnerError::AttemptLineage(parent.clone()))?;
        }
    }
}

/// Canonical identity comes from the Store row key; Core identity is the
/// matching root/state pair read from that same checkpoint snapshot.
pub(crate) fn validate_checkpoint_identity<'a>(
    requested_conversation: &str,
    stored_conversation: &str,
    checkpoint_session: &'a str,
    state_session: &str,
) -> Result<&'a str, AssistantOwnerError> {
    if requested_conversation.trim().is_empty()
        || requested_conversation != stored_conversation
        || checkpoint_session.trim().is_empty()
        || checkpoint_session != state_session
    {
        return Err(AssistantOwnerError::ForeignSession);
    }
    Ok(checkpoint_session)
}

#[cfg(test)]
#[path = "assistant_owner_tests.rs"]
mod tests;
