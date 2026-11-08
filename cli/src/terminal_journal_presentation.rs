// SPDX-License-Identifier: Apache-2.0
//! CLI-only rendering observer. No execution outcome is inferred here.

pub(crate) fn readonly_surface_summary(summary: &str) -> String {
    let summary = summary.trim();
    format!("{}\n\n[只读界面摘要：CLI 不支持图形表单交互，请在 Web 或原生客户端操作。]",
        if summary.is_empty() { "此会话包含交互界面。" } else { summary })
}

#[cfg(test)]
mod text_presentation_tests {
    use super::*;
    use crate::agent::Emitter;
    use nola_core::agent::parts::TextReviewState;
    fn journal_text_update(
        id: &str,
        delta: &str,
        finalized: bool,
        review: Option<TextReviewState>,
    ) -> crate::source_journal::CommittedJournalUpdate {
        use nola_domain::assistant_journal_operations::*;
        crate::source_journal::CommittedJournalUpdate {
            key: id.into(),
            reset: false,
            markdown_delta: delta.into(),
            reasoning_delta: String::new(),
            fields: Default::default(),
            head: Some(JournalPartHead {
                identity: nola_domain::assistant_journal::AssistantJournalIdentity {
                    source_store_epoch: "epoch".into(),
                    conversation_id: "conversation".into(),
                    run_id: "run".into(),
                    assistant_message_id: "assistant".into(),
                },
                part_id: id.into(),
                generation: JournalNumber(1),
                revision: JournalNumber(1),
                metadata_revision: JournalNumber(1),
                order_key: JournalOrderKey {
                    section: JournalSection::Transcript,
                    position: JournalNumber(0),
                },
                metadata: JournalPartMetadata::Text(JournalTextMetadata {
                    is_final: finalized,
                    review,
                    visibility: if review.is_some_and(|s| s != TextReviewState::Approved) {
                        JournalVisibility::Withheld
                    } else {
                        JournalVisibility::Public
                    },
                }),
                bodies: vec![],
            }),
        }
    }

    #[test]
    fn surface_summary_is_read_only_and_replays_or_unchanged_revisions_do_not_duplicate_it() {
        use nola_domain::assistant_journal_operations::{JournalPartMetadata, JournalUiSurfaceMetadata};
        let reference: nola_domain::generative_ui::UiSurfaceRevisionRef = serde_json::from_value(serde_json::json!({
            "identity":{"owner":{"authority_id":"epoch","conversation_id":"conversation",
                "participant_id":"assistant","context_id":"conversation","originating_run_id":"run",
                "message_id":"assistant","part_id":"tool-part"},"surface_id":"form","generation":1},
            "revision":1
        })).unwrap();
        let mut emitter = Emitter::new(false);
        let mut presenter = TerminalJournalPresentation::default();
        let make = |revision, summary: &str| {
            let mut update = journal_text_update("surface-part", "", true, None);
            let mut surface: nola_domain::generative_ui::UiSurfaceRevisionRef = reference.clone();
            surface.revision = revision;
            update.head.as_mut().unwrap().metadata = JournalPartMetadata::UiSurface(JournalUiSurfaceMetadata {
                surface, summary: summary.into(),
            });
            update
        };
        presenter.present_journal_update(&mut emitter, make(1, "Choose a destination"));
        let first = emitter.take_markdown_text();
        assert!(first.contains("Choose a destination"));
        assert!(first.contains("CLI 不支持图形表单交互"));
        presenter.present_journal_update(&mut emitter, make(1, "Choose a destination"));
        presenter.present_journal_update(&mut emitter, make(2, "Choose a destination"));
        presenter.present_journal_update(&mut emitter, make(1, "stale value"));
        assert!(emitter.take_markdown_text().is_empty());
        presenter.present_journal_update(&mut emitter, make(3, "Destination: Paris"));
        assert!(emitter.take_markdown_text().contains("Destination: Paris"));
    }

    #[test]
    fn reviewed_candidate_is_not_committed_to_cli_body_before_approval() {
        let mut emitter = Emitter::new(false);
        let mut presenter = TerminalJournalPresentation::default();
        presenter.present_journal_update(&mut emitter, journal_text_update(
            "candidate",
            "",
            true,
            Some(TextReviewState::Pending),
        ));
        assert!(emitter.take_markdown_text().is_empty());
        presenter.present_journal_update(&mut emitter, journal_text_update(
            "candidate",
            "",
            true,
            Some(TextReviewState::Rejected),
        ));
        assert!(emitter.take_markdown_text().is_empty());
        presenter.present_journal_update(&mut emitter, journal_text_update(
            "candidate",
            "approved draft",
            true,
            Some(TextReviewState::Approved),
        ));
        assert_eq!(emitter.take_markdown_text(), "approved draft");
    }

    #[test]
    fn completed_partial_commits_exact_preview_and_stale_retirement_cannot_drop_successor() {
        let mut emitter = Emitter::new(false);
        let mut presenter = TerminalJournalPresentation::default();
        presenter.present_journal_update(&mut emitter, journal_text_update(
            "accepted-prefix",
            "PREFIX-ALPHA-",
            false,
            None,
        ));
        assert_eq!(
            presenter.preview_text_part.as_deref(),
            Some("accepted-prefix")
        );
        presenter.present_journal_update(&mut emitter, journal_text_update("accepted-prefix", "", true, None));
        assert!(presenter.preview_text_part.is_none());
        presenter.present_journal_update(&mut emitter, journal_text_update("new-sampling", "OMEGA", false, None));
        let mut retired = journal_text_update("accepted-prefix", "", true, None);
        retired.reset = true;
        retired.head = None;
        presenter.present_journal_update(&mut emitter, retired);
        assert_eq!(presenter.preview_text_part.as_deref(), Some("new-sampling"));
        let mut retired = journal_text_update("new-sampling", "", true, None);
        retired.reset = true;
        retired.head = None;
        presenter.present_journal_update(&mut emitter, retired);
        assert!(presenter.preview_text_part.is_none());
    }
}
use crate::tools::ToolRequest;
use serde_json::Value;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

#[derive(Debug, Default)]
pub(crate) struct JournalInbox {
    reader: crate::source_journal::CommittedJournalReader,
    pub(crate) updates: Vec<crate::source_journal::CommittedJournalUpdate>,
}
#[derive(Debug)]
pub(crate) struct JournalObserver(pub(crate) Arc<Mutex<JournalInbox>>);
impl crate::runtime_output::JournalPublicationObserver for JournalObserver {
    fn observe(
        &mut self,
        port: &mut crate::application_port::CliApplicationPort,
        cursor: &nola_domain::assistant_journal::AssistantJournalCursor,
    ) -> Result<(), String> {
        let mut inbox = self
            .0
            .lock()
            .map_err(|_| "journal presentation inbox poisoned")?;
        let updates = inbox.reader.read(port, cursor)?;
        inbox.updates.extend(updates);
        Ok(())
    }
}
#[derive(Debug, Default)]
pub(crate) struct TerminalJournalPresentation {
    active_tools: HashMap<String, ToolRequest>,
    journal_activity_states: HashMap<String, nola_domain::ActivityState>,
    preview_text_part: Option<String>,
    rendered_review_states: HashMap<String, nola_core::agent::parts::TextReviewState>,
    rendered_ui_surfaces: HashMap<String, (nola_domain::generative_ui::UiSurfaceRevisionRef, String)>,
}
impl TerminalJournalPresentation {
    pub(crate) fn present_journal_update(
        &mut self,
        emitter: &mut crate::agent::Emitter,
        update: crate::source_journal::CommittedJournalUpdate,
    ) {
        use nola_domain::assistant_journal_operations::{
            JournalContentField as Field, JournalPartMetadata as Metadata, JournalVisibility,
        };
        use nola_domain::{ActivityState, TextReviewState};
        if update.reset && self.preview_text_part.as_deref() == Some(update.key.as_str()) {
            emitter.discard_text_preview();
            self.preview_text_part = None;
        }
        let Some(head) = &update.head else {
            self.rendered_review_states.remove(&update.key);
            self.journal_activity_states.remove(&update.key);
            self.rendered_ui_surfaces.remove(&update.key);
            return;
        };
        match &head.metadata {
            Metadata::UiSurface(surface) => {
                if let Some((previous, summary)) = self.rendered_ui_surfaces.get(&update.key) {
                    if previous.identity == surface.surface.identity {
                        if previous.revision >= surface.surface.revision { return; }
                        if summary == &surface.summary {
                            self.rendered_ui_surfaces.insert(update.key.clone(), (surface.surface.clone(), surface.summary.clone()));
                            return;
                        }
                    }
                }
                self.rendered_ui_surfaces.insert(update.key.clone(), (surface.surface.clone(), surface.summary.clone()));
                // Keep the Core-provided semantic summary. The terminal never
                // interprets the component tree or turns UI handles into tools.
                emitter.text(&format!("\n\n{}\n", readonly_surface_summary(&surface.summary)));
            }
            Metadata::Text(text) => {
                if let Some(review) = text.review {
                    let previous = self
                        .rendered_review_states
                        .insert(update.key.clone(), review);
                    if previous != Some(review) {
                        emitter.text_review(review);
                    }
                    if review != TextReviewState::Approved {
                        return;
                    }
                    if !update.markdown_delta.is_empty() {
                        emitter.text(&update.markdown_delta);
                    }
                    if text.is_final {
                        emitter.finish_reviewed_text_segment();
                    }
                } else if text.visibility == JournalVisibility::Public {
                    if !update.markdown_delta.is_empty() {
                        self.preview_text_part = Some(update.key.clone());
                        emitter.preview_text(&update.markdown_delta);
                    }
                    if text.is_final
                        && self.preview_text_part.as_deref() == Some(update.key.as_str())
                    {
                        emitter.commit_text_preview();
                        self.preview_text_part = None;
                    }
                }
            }
            Metadata::Reasoning(reasoning) => {
                if !update.reasoning_delta.is_empty() {
                    emitter.reasoning(&update.reasoning_delta);
                }
                self.present_journal_activity(
                    emitter,
                    &update.key,
                    &reasoning.state,
                    &update.text(&reasoning.title),
                );
            }
            Metadata::Step(step) => self.present_journal_activity(
                emitter,
                &update.key,
                &step.state,
                &update.text(&step.title),
            ),
            Metadata::Tool(tool) => {
                if self.journal_activity_states.get(&update.key) == Some(&tool.state) {
                    return;
                }
                let input = match update.structured(Field::ToolInput) {
                    Ok(input) => input,
                    Err(error) => {
                        emitter.error(&error);
                        return;
                    }
                };
                let output = if matches!(
                    tool.state,
                    ActivityState::Completed | ActivityState::Failed | ActivityState::Cancelled
                ) {
                    match update.structured(Field::ToolOutput) {
                        Ok(output) => output,
                        Err(error) => {
                            emitter.error(&error);
                            return;
                        }
                    }
                } else {
                    None
                };
                // A failed read must not consume this activity revision. The
                // exact same committed update may be read and presented again.
                self.journal_activity_states
                    .insert(update.key.clone(), tool.state.clone());
                let request = ToolRequest {
                    id: tool.tool_call_id.clone(),
                    name: tool.capability_id.clone(),
                    args: input
                        .as_ref()
                        .and_then(Value::as_object)
                        .cloned()
                        .unwrap_or_default(),
                };
                if tool.state == ActivityState::Running {
                    if !self.active_tools.contains_key(&request.id) {
                        emitter.tool_use(&request, None, None);
                        self.active_tools.insert(request.id.clone(), request);
                    }
                } else if matches!(
                    tool.state,
                    ActivityState::Completed | ActivityState::Failed | ActivityState::Cancelled
                ) {
                    self.active_tools.remove(&request.id);
                    let summary = tool.summary.as_ref().map(|value| update.text(value));
                    let preview = update.field(Field::ToolOutputText);
                    let outcome = crate::core_agent_runtime::tool_part_outcome(
                        summary.as_deref(),
                        (!preview.is_empty()).then_some(preview),
                        output.as_ref(),
                        tool.state != ActivityState::Completed,
                    );
                    if tool.state == ActivityState::Cancelled {
                        emitter.tool_cancelled(&request);
                    } else {
                        emitter.tool_result(&request, &outcome);
                    }
                }
            }
            Metadata::Task(task) => self.present_journal_activity(
                emitter,
                &update.key,
                &task.state,
                update.field(Field::TaskCurrentStep),
            ),
            Metadata::TaskControl(control) => self.present_journal_activity(
                emitter,
                &update.key,
                &control.task.state,
                update.field(Field::TaskCurrentStep),
            ),
            Metadata::Skill(skill) => self.present_journal_activity(
                emitter,
                &update.key,
                &skill.state,
                &update.text(&skill.display_name),
            ),
            Metadata::ModelAttempt(attempt) => self.present_journal_activity(
                emitter,
                &update.key,
                &attempt.state,
                update.field(Field::FailureMessage),
            ),
            Metadata::Error(error) => {
                let message = update.field(Field::FailureMessage);
                emitter.error(if message.is_empty() {
                    &error.display_key
                } else {
                    message
                });
            }
            // Provenance and attachments are typed journal records, not extra
            // assistant prose. Their inline references remain in Markdown.
            Metadata::Source(_)
            | Metadata::Citation(_)
            | Metadata::Attachment(_)
            | Metadata::Historical(_) => {}
        }
    }

    fn present_journal_activity(
        &mut self,
        emitter: &mut crate::agent::Emitter,
        key: &str,
        state: &nola_domain::ActivityState,
        title: &str,
    ) {
        if self.journal_activity_states.get(key) != Some(state) {
            self.journal_activity_states
                .insert(key.to_owned(), state.clone());
            if !title.is_empty() {
                emitter.status("processing", title);
            }
        }
    }
}

#[cfg(test)]
#[test]
fn malformed_journal_tool_body_does_not_consume_activity_state() {
    use nola_domain::assistant_journal_operations::{JournalContentField, JournalPartMetadata};
    let (_, source) = crate::source_journal::source_tests::completed_source(
        &[("read_file", serde_json::json!({"observed":true}))],
        "verified",
    );
    let part = source
        .parts
        .values()
        .find(|part| matches!(part.head.metadata, JournalPartMetadata::Tool(_)))
        .unwrap();
    let mut update = crate::source_journal::CommittedJournalUpdate {
        key: "exact-read-tool".into(),
        head: Some(part.head.clone()),
        reset: false,
        markdown_delta: String::new(),
        reasoning_delta: String::new(),
        fields: part.fields.clone(),
    };
    let mut emitter = crate::agent::Emitter::with_event_sink(|_| {});
    let mut presenter = TerminalJournalPresentation::default();
    update
        .fields
        .insert(JournalContentField::ToolOutput, "invalid JSON".into());
    presenter.present_journal_update(&mut emitter, update);
    assert!(!presenter
        .journal_activity_states
        .contains_key("exact-read-tool"));
    presenter.present_journal_update(
        &mut emitter,
        crate::source_journal::CommittedJournalUpdate {
            key: "exact-read-tool".into(),
            head: Some(part.head.clone()),
            reset: false,
            markdown_delta: String::new(),
            reasoning_delta: String::new(),
            fields: part.fields.clone(),
        },
    );
    assert_eq!(
        presenter.journal_activity_states.get("exact-read-tool"),
        Some(&nola_domain::ActivityState::Completed)
    );
}
