// SPDX-License-Identifier: Apache-2.0
//! CLI mention completion. The draft carries the selected expert's exact identity
//! and revision so a restored draft cannot silently pick a replacement expert.
use crate::application_port::CliApplicationPort;
use nola_application::{
    agent_room::AgentRoomCommand, persistent_agent::AgentDirectory, ApplicationCommand,
};
use nola_domain::{
    expert_task_target::{
        ExpertMentionSegment, ExpertMentions, MainExpertSelectionContextV1, MainExpertSelectionV1,
    },
    persistent_agent::AgentAvailability,
};
use std::cell::RefCell;

const PREFIX: &str = "@expert[";
#[derive(Clone)]
pub(crate) struct Candidate {
    pub token: String,
    pub label: String,
    name: String,
    description: String,
    offered: bool,
    selection: MainExpertSelectionV1,
}

impl Candidate {
    fn new(id: &str, revision: i64, name: &str) -> Self {
        let encoded_id = url::form_urlencoded::byte_serialize(id.as_bytes()).collect::<String>();
        Self {
            token: format!("{PREFIX}{encoded_id}#{revision}]"),
            label: format!("@{name} · {}", expert_label()),
            name: name.to_owned(),
            description: String::new(),
            offered: true,
            selection: MainExpertSelectionV1 {
                agent_id: id.into(),
                agent_revision: revision,
                participant_id: None,
                context_conversation_id: None,
            },
        }
    }
}

fn expert_label() -> &'static str {
    use crate::i18n::Language::*;
    match crate::i18n::current_language() {
        ZhHans => "专家",
        ZhHant => "專家",
        En => "expert",
        De => "Experte",
        Es => "experto",
        Fr => "expert",
        Ja => "専門家",
        Ko => "전문가",
        Ru => "эксперт",
    }
}

fn directory(session: Option<&str>) -> Result<Vec<Candidate>, String> {
    use nola_application::agent_room::ConversationRoomAssociation;
    let mut app = CliApplicationPort::persistent_default()?;
    let directory: AgentDirectory = app.execute(ApplicationCommand::AgentRoom {
        command: AgentRoomCommand::Directory,
    })?;
    let mut candidates: Vec<Candidate> = directory
        .agents
        .iter()
        .filter(|agent| agent.availability() == &AgentAvailability::Active)
        .map(|agent| {
            let mut item = Candidate::new(agent.id().as_str(), agent.revision(), agent.name());
            item.description = agent.description().into();
            item
        })
        .collect();
    if let Some(id) = session {
        let association: Option<ConversationRoomAssociation> =
            app.execute(ApplicationCommand::AgentRoom {
                command: AgentRoomCommand::ForConversation {
                    conversation_id: id.to_owned(),
                },
            })?;
        if let Some(association) = association {
            for member in association
                .room
                .room
                .participants()
                .iter()
                .filter(|p| p.active)
            {
                // A draft can still hold the original directory reference after
                // this expert joins. Only offer the pinned member in the picker.
                for item in candidates
                    .iter_mut()
                    .filter(|item| item.selection.agent_id == member.agent_id.as_str())
                {
                    item.offered = false;
                }
                let mut item = Candidate::new(
                    member.agent_id.as_str(),
                    member.agent_revision,
                    &member.display_name,
                );
                let participant =
                    url::form_urlencoded::byte_serialize(member.id.as_bytes()).collect::<String>();
                item.token.pop();
                item.token.push_str(&format!("/{participant}]"));
                item.description.clone_from(&member.description);
                item.selection.participant_id = Some(member.id.clone());
                item.selection.context_conversation_id =
                    Some(member.context_conversation_id.clone());
                candidates.push(item);
            }
        }
    }
    let mut names = std::collections::HashMap::new();
    for item in candidates.iter().filter(|item| item.offered) {
        *names.entry(item.name.clone()).or_insert(0) += 1;
    }
    for item in candidates.iter_mut().filter(|item| item.offered) {
        if names[&item.name] > 1 {
            item.label
                .push_str(&format!(" · {}", item.selection.agent_id));
        }
    }
    candidates.sort_by_key(|item| item.selection.participant_id.is_none());
    Ok(candidates)
}

thread_local! {
    static COMPLETIONS: RefCell<Option<Vec<Candidate>>> = const { RefCell::new(None) };
}

pub(crate) fn begin_composer() {
    COMPLETIONS.with(|cache| cache.replace(None));
}

pub(crate) fn completions(prefix: &str, session: Option<&str>) -> Vec<Candidate> {
    COMPLETIONS.with(|cache| {
        let mut cache = cache.borrow_mut();
        let candidates = cache.get_or_insert_with(|| directory(session).unwrap_or_default());
        let query = prefix.to_lowercase();
        candidates
            .iter()
            .filter(|item| {
                item.offered
                    && (item.name.to_lowercase().contains(&query)
                        || item.description.to_lowercase().contains(&query)
                        || item.selection.agent_id.to_lowercase().contains(&query))
            })
            .take(12)
            .cloned()
            .collect()
    })
}

/// Byte ranges are editor-local. Persistence on the wire uses ordered segments.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub(crate) struct Occurrence {
    pub start: usize,
    pub end: usize,
    pub text: String,
    pub selection: MainExpertSelectionV1,
}

pub(crate) fn completion(token: &str, start: usize) -> Option<Occurrence> {
    COMPLETIONS.with(|cache| {
        let cache = cache.borrow();
        let item = cache
            .as_ref()?
            .iter()
            .find(|item| item.offered && item.token == token)?;
        let text = format!("@{}", item.name);
        Some(Occurrence {
            start,
            end: start + text.len(),
            text,
            selection: item.selection.clone(),
        })
    })
}

/// Keep identities outside the edited range and detach any intersected occurrence.
pub(crate) fn replace(runs: &mut Vec<Occurrence>, start: usize, end: usize, inserted: usize) {
    runs.retain_mut(|r| {
        if r.end <= start {
            return true;
        }
        if r.start < end {
            return false;
        }
        r.start = r.start - (end - start) + inserted;
        r.end = r.end - (end - start) + inserted;
        true
    });
}

pub(crate) fn bind(
    text: &str,
    runs: &[Occurrence],
) -> Result<Option<MainExpertSelectionContextV1>, String> {
    if runs.is_empty() {
        return Ok(None);
    }
    let mut selected_experts = Vec::new();
    let mut segments = Vec::new();
    let mut offset = 0;
    for r in runs {
        if r.start < offset || text.get(r.start..r.end) != Some(r.text.as_str()) {
            return Err("Expert mention changed; select the expert again.".into());
        }
        if r.start > offset {
            segments.push(ExpertMentionSegment {
                text: text[offset..r.start].into(),
                agent_id: None,
            });
        }
        if !selected_experts.contains(&r.selection) {
            selected_experts.push(r.selection.clone());
        }
        segments.push(ExpertMentionSegment {
            text: r.text.clone(),
            agent_id: Some(r.selection.agent_id.clone()),
        });
        offset = r.end;
    }
    if offset < text.len() {
        segments.push(ExpertMentionSegment {
            text: text[offset..].into(),
            agent_id: None,
        });
    }
    if !segments
        .iter()
        .any(|s| s.agent_id.is_none() && !s.text.trim().is_empty())
    {
        return Err(match crate::i18n::current_language() {
            crate::i18n::Language::ZhHans => "写下想请专家做的事。",
            _ => "Add what you would like the expert to do.",
        }
        .into());
    }
    let mentions = ExpertMentions {
        version: 1,
        segments,
    };
    mentions.validate(text, &selected_experts)?;
    Ok(Some(MainExpertSelectionContextV1 {
        version: 1,
        selected_experts,
        expected_source_store_epoch: None,
        expert_mentions: Some(mentions),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn inline_mentions_preserve_repeated_identity_and_plain_pastes() {
        let expert = Candidate::new("first", 3, "审查");
        let mention = "@审查";
        let text = "请@审查。@审查 和 @审查";
        let mut runs: Vec<_> = text
            .match_indices(mention)
            .take(2)
            .map(|(start, _)| Occurrence {
                start,
                end: start + mention.len(),
                text: mention.into(),
                selection: expert.selection.clone(),
            })
            .collect();
        let context = bind(text, &runs).unwrap().unwrap();
        assert_eq!(context.selected_experts.len(), 1);
        assert_eq!(
            context
                .expert_mentions
                .unwrap()
                .segments
                .iter()
                .filter(|r| r.agent_id.is_some())
                .count(),
            2
        );
        replace(&mut runs, 0, "请".len(), 0);
        assert!(bind(&text["请".len()..], &runs).is_ok());
        assert!(bind(mention, &runs[..1]).is_err());
        assert!(bind("@expert[first#3] and @审查", &[]).unwrap().is_none());
    }
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub(crate) struct EditTarget {
    conversation_id: String,
    message_id: String,
    expected_revision: i64,
    expected_message_revision: u64,
}

impl EditTarget {
    pub fn load(
        conversation: &str,
        message_id: Option<&str>,
    ) -> Result<(Self, String, Vec<Occurrence>), String> {
        use nola_domain::{ConversationProjection, MessagePartView, MessageRole, SemanticValue};
        let mut app = CliApplicationPort::persistent_default()?;
        let view: ConversationProjection =
            app.execute(ApplicationCommand::LoadConversationProjectionV1 {
                conversation_id: conversation.into(),
            })?;
        let msg = view
            .messages
            .iter()
            .rev()
            .find(|m| {
                m.role == MessageRole::User && message_id.is_none_or(|id| m.message_id.0 == id)
            })
            .ok_or("No matching user message in this conversation.")?;
        let (text, runs) = match msg.parts.as_slice() {
            [MessagePartView::Text {
                markdown,
                is_final: true,
                ..
            }] => (markdown.clone(), vec![]),
            [MessagePartView::Historical { projection, .. }]
                if projection.schema.is_none() && projection.canonical_kind == "text" =>
            {
                let text = projection
                    .canonical_text
                    .as_ref()
                    .filter(|v| v.text.len() as u64 == v.total_utf8_bytes)
                    .ok_or("Open the complete message before editing it.")?
                    .text
                    .clone();
                let mut runs = vec![];
                if let Some(SemanticValue::Complete { value }) =
                    projection.fields.get("expert_mentions")
                {
                    let frozen: nola_domain::expert_task_target::FrozenExpertMentions =
                        serde_json::from_value(
                            serde_json::to_value(value).map_err(|e| e.to_string())?,
                        )
                        .map_err(|e| e.to_string())?;
                    if !frozen.matches_text(&text) {
                        return Err("The message's expert references are unavailable.".into());
                    }
                    let mut start = 0;
                    for segment in frozen.segments {
                        let end = start + segment.text.len();
                        if let Some(id) = segment.agent_id {
                            let selection = frozen
                                .selected_experts
                                .iter()
                                .find(|s| s.agent_id == id)
                                .ok_or("The selected expert is unavailable.")?
                                .clone();
                            runs.push(Occurrence {
                                start,
                                end,
                                text: segment.text,
                                selection,
                            });
                        }
                        start = end;
                    }
                }
                (text, runs)
            }
            _ => return Err("Only completed text messages can be edited here.".into()),
        };
        let target = Self {
            conversation_id: conversation.into(),
            message_id: msg.message_id.0.clone(),
            expected_revision: view.revision as i64,
            expected_message_revision: msg.revision,
        };
        Ok((target, text, runs))
    }

    pub fn prepare(
        &self,
        id: &str,
        text: &str,
        selection: Option<&MainExpertSelectionContextV1>,
    ) -> Result<(), String> {
        let input = nola_domain::edited_resend::PrepareInPlaceResend {
            defer_until_admission: true,
            operation_id: id.into(),
            conversation_id: self.conversation_id.clone(),
            message_id: self.message_id.clone(),
            expected_revision: self.expected_revision,
            expected_message_revision: self.expected_message_revision,
            edited_text: text.into(),
            created_at_ms: chrono::Utc::now().timestamp_millis(),
            selected_experts: selection
                .map(|v| v.selected_experts.clone())
                .unwrap_or_default(),
            expert_mentions: selection.and_then(|v| v.expert_mentions.clone()),
            expected_source_store_epoch: None,
        };
        let mut app = CliApplicationPort::persistent_default()?;
        let _: nola_domain::edited_resend::PreparedInPlaceResend =
            app.execute(ApplicationCommand::PrepareInPlaceResend { input })?;
        Ok(())
    }
}
