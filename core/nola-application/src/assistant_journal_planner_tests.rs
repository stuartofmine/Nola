// SPDX-License-Identifier: Apache-2.0
use super::*;
use nola_agent_engine::agent::parts::{
    ArtifactPart, ArtifactReference, CitationPart, CompactionPart, PartOwnership, ReasoningPart,
    SourceLocation, SourcePart, StepPart, StepStatus, TextPart, ToolPart, ToolPartState,
    ToolPermissionPart,
};
use std::cell::Cell;
use std::collections::HashMap;

#[path = "assistant_journal_planner/synthetic_tests.rs"]
mod synthetic_tests;

#[derive(Default)]
struct Reader {
    heads: BTreeMap<String, JournalPartHead>,
    manifests: HashMap<nola_domain::assistant_journal::AssistantDigest, Vec<JournalChunkRef>>,
    reads: Cell<usize>,
    manifest_reads: Cell<usize>,
    retired: bool,
}
impl JournalPlanningReader for Reader {
    fn load_head(
        &self,
        _: &AssistantJournalIdentity,
        part_id: &str,
    ) -> Result<AssistantJournalPartState> {
        self.reads.set(self.reads.get() + 1);
        if self.retired {
            return Ok(AssistantJournalPartState::Retired);
        }
        Ok(match self.heads.get(part_id) {
            Some(head) => AssistantJournalPartState::Live(head.clone()),
            None => AssistantJournalPartState::NeverDeclared,
        })
    }
    fn load_manifest_page(
        &self,
        id: &AssistantJournalIdentity,
        part_id: &str,
        content: &JournalContentRef,
        after_chunk: u64,
        max_records: usize,
    ) -> Result<AssistantManifestPage> {
        self.manifest_reads.set(self.manifest_reads.get() + 1);
        if id != &identity()
            || !self
                .heads
                .get(part_id)
                .is_some_and(|head| head.bodies.contains(content))
            || max_records == 0
            || max_records > MAX_BATCH_OPERATIONS
            || after_chunk > content.chunk_count.0
        {
            return Err(PlannerError::Invalid("invalid manifest request".into()));
        }
        let manifest = self
            .manifests
            .get(&content.content_root)
            .ok_or_else(|| PlannerError::Invalid("manifest missing".into()))?;
        let chunks = manifest
            .iter()
            .skip(after_chunk as usize)
            .take(max_records)
            .cloned()
            .collect::<Vec<_>>();
        let next_chunk = after_chunk + chunks.len() as u64;
        Ok(AssistantManifestPage {
            chunks,
            next_chunk,
            complete: next_chunk == content.chunk_count.0,
        })
    }
}
fn prepared_manifests(
    plan: &PreparedAssistantJournal,
) -> HashMap<nola_domain::assistant_journal::AssistantDigest, Vec<JournalChunkRef>> {
    let mut manifests: HashMap<_, Vec<JournalChunkRef>> = HashMap::new();
    for preparation in &plan.preparations {
        let mut refs: Vec<JournalChunkRef> = if preparation.previous.chunk_count.0 == 0 {
            vec![]
        } else {
            manifests
                .get(&preparation.previous.content_root)
                .unwrap()
                .clone()
        };
        refs.extend(chunk_refs(&preparation.chunks));
        manifests.insert(preparation.resulting.content_root, refs);
    }
    manifests
}

#[test]
fn real_core_repeated_tool_state_keeps_appended_bytes_refs_and_revision_unchanged() {
    use nola_agent_engine::agent::parts::ConversationProjection;
    let reader = Reader::default();
    let mut core = ConversationProjection::default();
    let mut planner = AssistantJournalPlanner::new(&reader, &identity()).unwrap();
    let created = PartUpdate::Created {
        part: AgentPart::Tool(tool(serde_json::json!({"input":1}), Some("首🙂"), None)),
    };
    assert!(core.apply(created.clone()).unwrap());
    planner
        .part_update(&created, &core.parts["tool"], order())
        .unwrap();
    let progress = PartUpdate::ToolProgress {
        part_id: "tool".into(),
        title: None,
        detail: None,
        output_delta: Some("尾🙂".into()),
    };
    assert!(core.apply(progress.clone()).unwrap());
    planner
        .part_update(&progress, &core.parts["tool"], order())
        .unwrap();
    let head = planner.head("tool").unwrap().unwrap();
    let count = planner.operations.len();
    let preparations = planner.preparations.len();
    for _ in 0..3 {
        let mut update = tool_state_update();
        if let PartUpdate::ToolState { output_preview, .. } = &mut update {
            *output_preview = Some("首🙂尾🙂".into());
        }
        assert!(core.apply(update.clone()).unwrap());
        planner
            .part_update(&update, &core.parts["tool"], order())
            .unwrap();
        assert_eq!(planner.head("tool").unwrap().unwrap(), head);
        assert_eq!(planner.operations.len(), count);
        assert_eq!(planner.preparations.len(), preparations);
    }
    // These refs were created/appended in this UoW; no persisted read is needed.
    assert_eq!(reader.manifest_reads.get(), 0);
    let plan = changed(planner.finish().unwrap());
    assert_eq!(plan.batch.operations.len(), 2);
    validate_assistant_batch(
        &ProofReader {
            original: &reader,
            plan: &plan,
        },
        &plan.batch,
    )
    .unwrap();
}

#[test]
fn semantic_match_reads_bounded_real_manifest_pages_and_rejects_invalid_facts() {
    let mut reader = Reader::default();
    let chunks = (0..135)
        .map(|index| AssistantTextChunkV1::new(format!("块{index}🙂")).unwrap())
        .collect::<Vec<_>>();
    let mut head = initial_head();
    let mut reference = head.bodies[0].clone();
    let refs = chunk_refs(&chunks);
    for chunk in &refs {
        reference = append_journal_content(&reference, std::slice::from_ref(chunk)).unwrap();
    }
    head.bodies = vec![reference.clone()];
    reader.heads.insert("text".into(), head);
    reader
        .manifests
        .insert(reference.content_root, refs.clone());
    let text = chunks
        .iter()
        .map(AssistantTextChunkV1::text)
        .collect::<String>();
    let planner = AssistantJournalPlanner::new(&reader, &identity()).unwrap();
    assert!(planner.content_matches("text", &reference, &text).unwrap());
    assert_eq!(reader.manifest_reads.get(), 2);
    let mut different = text.clone();
    different.replace_range(.."块".len(), "字");
    assert!(!planner
        .content_matches("text", &reference, &different)
        .unwrap());
    assert!(planner
        .content_matches("foreign", &reference, &text)
        .is_err());
    drop(planner);
    reader.manifests.get_mut(&reference.content_root).unwrap()[0].utf8_bytes = JournalNumber(0);
    let planner = AssistantJournalPlanner::new(&reader, &identity()).unwrap();
    assert!(planner.content_matches("text", &reference, &text).is_err());
}

#[test]
fn real_core_citations_keep_opaque_uris_and_independent_location_facts() {
    use nola_agent_engine::agent::parts::ConversationProjection;
    let reader = Reader::default();
    let mut core = ConversationProjection::default();
    let mut planner = AssistantJournalPlanner::new(&reader, &identity()).unwrap();
    let locations = [
        None,
        Some(SourceLocation::default()),
        Some(SourceLocation {
            page: Some(0),
            line_start: None,
            line_end: Some(u64::MAX),
            section: Some(String::new()),
        }),
        Some(SourceLocation {
            page: None,
            line_start: Some(9),
            line_end: Some(2),
            section: None,
        }),
    ];
    let long_uri = " \n".repeat(2000);
    for (index, uri) in ["", " \n\t", "opaque source fact", long_uri.as_str()]
        .into_iter()
        .enumerate()
    {
        let part = AgentPart::Citation(CitationPart {
            id: format!("citation-{index}"),
            ownership: PartOwnership {
                message_id: "assistant".into(),
                turn_id: "run".into(),
            },
            citation_id: format!("citation-identity-{index}"),
            source_id: "source".into(),
            title: String::new(),
            uri: uri.to_owned(),
            mime_type: Some(String::new()),
            location: locations[index].clone(),
            range: Some(nola_domain::CitationRange {
                start: 0,
                end: u64::MAX,
            }),
            origin_tool_call_id: "call".into(),
        });
        let update = PartUpdate::Created { part };
        assert!(core.apply(update.clone()).unwrap());
        let accepted = &core.parts[update.part_id()];
        planner.part_update(&update, accepted, order()).unwrap();
        let AgentPart::Citation(accepted) = accepted else {
            panic!("Citation")
        };
        let public = nola_domain::MessagePartView::Citation {
            part_id: nola_domain::MessagePartId(accepted.id.clone()),
            revision: core.revision,
            citation_id: accepted.citation_id.clone(),
            source_id: accepted.source_id.clone(),
            title: accepted.title.clone(),
            uri: accepted.uri.clone(),
            mime_type: accepted.mime_type.clone(),
            origin_tool_call_id: accepted.origin_tool_call_id.clone(),
            page: accepted.location.as_ref().and_then(|value| value.page),
            line_start: accepted
                .location
                .as_ref()
                .and_then(|value| value.line_start),
            line_end: accepted.location.as_ref().and_then(|value| value.line_end),
            section: accepted
                .location
                .as_ref()
                .and_then(|value| value.section.clone()),
            location: accepted.location.clone(),
            range_start: Some(0),
            range_end: Some(u64::MAX),
        };
        public
            .validate_public_semantics(&nola_domain::MessageId("assistant".into()))
            .unwrap();
    }
    let plan = changed(planner.finish().unwrap());
    let changes = validate_assistant_batch(
        &ProofReader {
            original: &reader,
            plan: &plan,
        },
        &plan.batch,
    )
    .unwrap();
    for (index, change) in changes.iter().enumerate() {
        let JournalPartMetadata::Citation(metadata) =
            &change.replacement.as_ref().unwrap().metadata
        else {
            panic!("Citation")
        };
        assert_eq!(
            metadata.location.as_ref().map(|location| (
                location.page.map(|value| value.0),
                location.line_start.map(|value| value.0),
                location.line_end.map(|value| value.0)
            )),
            locations[index].as_ref().map(|location| (
                location.page,
                location.line_start,
                location.line_end
            ))
        );
    }
}
fn identity() -> AssistantJournalIdentity {
    AssistantJournalIdentity {
        source_store_epoch: "epoch".into(),
        conversation_id: "conversation".into(),
        run_id: "run".into(),
        assistant_message_id: "assistant".into(),
    }
}
fn order() -> JournalOrderKey {
    JournalOrderKey {
        section: JournalSection::Transcript,
        position: JournalNumber(1),
    }
}
fn text(value: &str, review: Option<TextReviewState>, final_: bool) -> AgentPart {
    AgentPart::Text(TextPart {
        id: "text".into(),
        ownership: PartOwnership {
            message_id: "assistant".into(),
            turn_id: "run".into(),
        },
        text: value.into(),
        review,
        is_complete: final_,
    })
}
fn changed(plan: VisibleJournalPlan) -> PreparedAssistantJournal {
    match plan {
        VisibleJournalPlan::Changed(mut frames) => {
            assert_eq!(frames.len(), 1, "this fixture expects one bounded frame");
            frames.remove(0)
        }
        _ => panic!("expected changed"),
    }
}
fn initial_head() -> JournalPartHead {
    let reader = Reader::default();
    let mut planner = AssistantJournalPlanner::new(&reader, &identity()).unwrap();
    planner.begin_text("text", order()).unwrap();
    match changed(planner.finish().unwrap())
        .batch
        .operations
        .remove(0)
    {
        AssistantJournalOperationV1::Declare(value) => value.part,
        _ => panic!("expected declaration"),
    }
}

/// Exercise Domain validation against proofs derived from actual planner chunks,
/// not a permissive reader that accepts arbitrary body roots.
struct ProofReader<'a> {
    original: &'a Reader,
    plan: &'a PreparedAssistantJournal,
}
impl AssistantJournalStateReader for ProofReader<'_> {
    fn load_part_head(
        &self,
        part_id: &str,
    ) -> std::result::Result<Option<JournalPartHead>, OperationError> {
        Ok(self.original.heads.get(part_id).cloned())
    }
    fn verify_content_reference(
        &self,
        identity: &AssistantJournalIdentity,
        part_id: &str,
        content: &JournalContentRef,
    ) -> std::result::Result<(), OperationError> {
        if content.chunk_count.0 == 0 {
            return if *content
                == empty_journal_content(
                    identity,
                    part_id,
                    content.field,
                    content.content_generation,
                )? {
                Ok(())
            } else {
                Err(OperationError::ContentConflict)
            };
        }
        for prep in &self.plan.preparations {
            if prep.part_id == part_id && prep.resulting == *content {
                let computed = append_journal_content(&prep.previous, &chunk_refs(&prep.chunks))?;
                return if computed == *content {
                    Ok(())
                } else {
                    Err(OperationError::ContentConflict)
                };
            }
        }
        Err(OperationError::ManifestUnavailable)
    }
    fn verify_chunk_reference(
        &self,
        chunk: &JournalChunkRef,
    ) -> std::result::Result<(), OperationError> {
        if self
            .plan
            .supplied_chunks
            .iter()
            .any(|c| c.digest() == chunk.digest && c.byte_length() as u64 == chunk.utf8_bytes.0)
        {
            Ok(())
        } else {
            Err(OperationError::ManifestUnavailable)
        }
    }
}

#[test]
fn begin_created_delta_committed_delta_and_close_use_one_touched_head() {
    let reader = Reader::default();
    let mut planner = AssistantJournalPlanner::new(&reader, &identity()).unwrap();
    planner.begin_text("text", order()).unwrap();
    let empty = text("", None, false);
    planner
        .part_update(
            &PartUpdate::Created {
                part: empty.clone(),
            },
            &empty,
            order(),
        )
        .unwrap();
    planner
        .part_update(
            &PartUpdate::TextDelta {
                part_id: "text".into(),
                delta: "你".into(),
            },
            &text("你", None, false),
            order(),
        )
        .unwrap();
    planner
        .part_update(
            &PartUpdate::TextCommittedDelta {
                part_id: "text".into(),
                delta: "好".into(),
            },
            &text("你好", None, false),
            order(),
        )
        .unwrap();
    planner.close_public_part("text", "completed").unwrap();
    planner.close_public_part("text", "completed").unwrap();
    let plan = changed(planner.finish().unwrap());
    assert_eq!(reader.reads.get(), 1);
    assert_eq!(plan.batch.operations.len(), 4);
    assert!(plan.preparations.is_empty());
    assert_eq!(
        plan.supplied_chunks
            .iter()
            .map(|c| c.text())
            .collect::<String>(),
        "你好"
    );
    let changes = validate_assistant_batch(
        &ProofReader {
            original: &reader,
            plan: &plan,
        },
        &plan.batch,
    )
    .unwrap();
    let head = changes[0].replacement.as_ref().unwrap();
    assert_eq!(head.revision.0, 4);
    assert_eq!(markdown(head).unwrap().utf8_bytes.0, 6);
    assert!(text_metadata(head).unwrap().is_final);
}

#[test]
fn pending_review_never_supplies_private_chunks_then_approval_reveals_once() {
    let reader = Reader::default();
    let mut planner = AssistantJournalPlanner::new(&reader, &identity()).unwrap();
    let pending = text("secret", Some(TextReviewState::Pending), false);
    planner
        .part_update(
            &PartUpdate::Created {
                part: pending.clone(),
            },
            &pending,
            order(),
        )
        .unwrap();
    planner
        .part_update(
            &PartUpdate::TextDelta {
                part_id: "text".into(),
                delta: "!".into(),
            },
            &text("secret!", Some(TextReviewState::Pending), false),
            order(),
        )
        .unwrap();
    assert!(planner.preparations.is_empty());
    assert!(planner.supplied_chunks.is_empty());
    let approved = text("secret!", Some(TextReviewState::Approved), true);
    let approval = PartUpdate::TextReviewChanged {
        part_id: "text".into(),
        state: TextReviewState::Approved,
    };
    planner.part_update(&approval, &approved, order()).unwrap();
    planner.part_update(&approval, &approved, order()).unwrap();
    let plan = changed(planner.finish().unwrap());
    assert_eq!(plan.batch.operations.len(), 2);
    assert_eq!(plan.preparations.len(), 1);
    let changes = validate_assistant_batch(
        &ProofReader {
            original: &reader,
            plan: &plan,
        },
        &plan.batch,
    )
    .unwrap();
    assert_eq!(changes[0].replacement.as_ref().unwrap().generation.0, 2);
}

#[test]
fn review_revocation_replaces_public_generation_without_private_content() {
    let mut reader = Reader::default();
    reader.heads.insert("text".into(), initial_head());
    let mut planner = AssistantJournalPlanner::new(&reader, &identity()).unwrap();
    planner
        .part_update(
            &PartUpdate::TextReviewChanged {
                part_id: "text".into(),
                state: TextReviewState::Rejected,
            },
            &text("unpublished", Some(TextReviewState::Rejected), true),
            order(),
        )
        .unwrap();
    let plan = changed(planner.finish().unwrap());
    assert!(plan.preparations.is_empty() && plan.supplied_chunks.is_empty());
    let changes = validate_assistant_batch(
        &ProofReader {
            original: &reader,
            plan: &plan,
        },
        &plan.batch,
    )
    .unwrap();
    let head = changes[0].replacement.as_ref().unwrap();
    assert!(head.bodies.is_empty());
    assert_eq!(
        text_metadata(head).unwrap().visibility,
        JournalVisibility::Withheld
    );
}

#[test]
fn unchanged_metadata_produces_no_empty_batch_or_revision_growth() {
    let mut reader = Reader::default();
    let mut head = initial_head();
    head.metadata = JournalPartMetadata::Text(JournalTextMetadata {
        is_final: true,
        visibility: JournalVisibility::Public,
        review: None,
    });
    reader.heads.insert("text".into(), head);
    let mut planner = AssistantJournalPlanner::new(&reader, &identity()).unwrap();
    planner
        .part_update(
            &PartUpdate::Completed {
                part_id: "text".into(),
            },
            &text("", None, true),
            order(),
        )
        .unwrap();
    planner.close_public_part("text", "completed").unwrap();
    assert!(matches!(
        planner.finish().unwrap(),
        VisibleJournalPlan::Unchanged
    ));
}

#[test]
fn replacements_split_utf8_preparation_and_preserve_exact_bytes() {
    let mut reader = Reader::default();
    reader.heads.insert("text".into(), initial_head());
    let mut planner = AssistantJournalPlanner::new(&reader, &identity()).unwrap();
    let content = "汉字🙂\u{0}".repeat(50_000);
    planner
        .part_update(
            &PartUpdate::TextReplaced {
                part_id: "text".into(),
                text: content.clone(),
                is_complete: true,
            },
            &text(&content, None, true),
            order(),
        )
        .unwrap();
    let plan = changed(planner.finish().unwrap());
    assert!(plan.preparations.len() > 1);
    for prep in &plan.preparations {
        assert!(
            prep.chunks
                .iter()
                .map(AssistantTextChunkV1::byte_length)
                .sum::<usize>()
                <= 256 * 1024
        );
        assert!(prep
            .chunks
            .iter()
            .all(|c| c.byte_length() <= ASSISTANT_CHUNK_MAX_BYTES));
    }
    let assembled = plan
        .preparations
        .iter()
        .flat_map(|p| &p.chunks)
        .map(|c| c.text())
        .collect::<String>();
    assert_eq!(assembled, content);
    validate_assistant_batch(
        &ProofReader {
            original: &reader,
            plan: &plan,
        },
        &plan.batch,
    )
    .unwrap();
}

#[test]
fn invalid_owner_tombstone_and_unsupported_family_fail_explicitly() {
    let reader = Reader::default();
    let mut planner = AssistantJournalPlanner::new(&reader, &identity()).unwrap();
    let wrong = text("", None, false).with_message_id("foreign");
    assert_eq!(
        planner
            .part_update(
                &PartUpdate::Created {
                    part: wrong.clone()
                },
                &wrong,
                order()
            )
            .unwrap_err(),
        PlannerError::OwnerMismatch
    );
    let other = AgentPart::Compaction(CompactionPart {
        id: "compaction".into(),
        ownership: PartOwnership {
            message_id: "assistant".into(),
            turn_id: "run".into(),
        },
        summary: "context summary".into(),
        retained_item_ids: vec![],
        source_fingerprint: "fingerprint".into(),
        original_token_count: 100,
        compacted_token_count: 10,
    });
    assert!(matches!(
        planner.part_update(
            &PartUpdate::Created {
                part: other.clone()
            },
            &other,
            order()
        ),
        Err(PlannerError::NotYetSupported(_))
    ));
    let retired = Reader {
        retired: true,
        ..Default::default()
    };
    assert_eq!(
        AssistantJournalPlanner::new(&retired, &identity())
            .unwrap()
            .begin_text("text", order())
            .unwrap_err(),
        PlannerError::RetiredPart
    );
}

#[test]
fn step_creation_and_completion_keep_exact_title_and_only_change_control_metadata() {
    for title in ["运行步骤🙂".to_owned(), "长标题🙂\n".repeat(40_000)] {
        let initial = Reader::default();
        let mut step = StepPart {
            id: "step".into(),
            ownership: PartOwnership {
                message_id: "assistant".into(),
                turn_id: "run".into(),
            },
            title: title.clone(),
            status: StepStatus::Started,
        };
        let part = AgentPart::Step(step.clone());
        let mut planner = AssistantJournalPlanner::new(&initial, &identity()).unwrap();
        planner
            .part_update(&PartUpdate::Created { part: part.clone() }, &part, order())
            .unwrap();
        let plan = changed(planner.finish().unwrap());
        let changes = validate_assistant_batch(
            &ProofReader {
                original: &initial,
                plan: &plan,
            },
            &plan.batch,
        )
        .unwrap();
        let head = changes[0].replacement.clone().unwrap();
        let JournalPartMetadata::Step(metadata) = &head.metadata else {
            panic!("Step metadata")
        };
        assert_eq!(metadata.state, nola_domain::ActivityState::Running);
        if title.len() <= 1024 {
            assert_eq!(metadata.title, JournalTextValue::Inline(title.clone()));
            assert!(head.bodies.is_empty());
            assert!(plan.preparations.is_empty());
        } else {
            assert_eq!(
                metadata.title,
                JournalTextValue::Field(JournalContentField::Title)
            );
            assert_eq!(head.bodies.len(), 1);
            assert_eq!(head.bodies[0].utf8_bytes.0, title.len() as u64);
            let actual = plan
                .preparations
                .iter()
                .flat_map(|p| p.chunks.iter())
                .map(|c| c.text())
                .collect::<String>();
            assert_eq!(actual, title);
            assert!(plan.batch.canonical_bytes().unwrap().len() < 1024);
        }
        let reader = Reader {
            heads: BTreeMap::from([(step.id.clone(), head.clone())]),
            manifests: prepared_manifests(&plan),
            ..Default::default()
        };
        for (status, expected) in [
            (StepStatus::Completed, nola_domain::ActivityState::Completed),
            (StepStatus::Failed, nola_domain::ActivityState::Failed),
            (StepStatus::Cancelled, nola_domain::ActivityState::Cancelled),
        ] {
            step.status = status;
            let part = AgentPart::Step(step.clone());
            let mut planner = AssistantJournalPlanner::new(&reader, &identity()).unwrap();
            planner
                .part_update(
                    &PartUpdate::Completed {
                        part_id: step.id.clone(),
                    },
                    &part,
                    order(),
                )
                .unwrap();
            let completion = changed(planner.finish().unwrap());
            assert!(completion.preparations.is_empty());
            assert!(completion.supplied_chunks.is_empty());
            assert_eq!(completion.batch.operations.len(), 1);
            assert!(
                matches!(&completion.batch.operations[0], AssistantJournalOperationV1::Update(u)
                if matches!(&u.metadata, JournalPartMetadata::Step(m) if m.state == expected))
            );
            let changed = validate_assistant_batch(
                &ProofReader {
                    original: &reader,
                    plan: &completion,
                },
                &completion.batch,
            )
            .unwrap();
            assert_eq!(changed[0].replacement.as_ref().unwrap().bodies, head.bodies);
        }
        let mut changed_title = step.clone();
        changed_title
            .title
            .replace_range(..title.chars().next().unwrap().len_utf8(), "异");
        let mut planner = AssistantJournalPlanner::new(&reader, &identity()).unwrap();
        assert!(matches!(
            planner.part_update(
                &PartUpdate::Completed {
                    part_id: step.id.clone()
                },
                &AgentPart::Step(changed_title),
                order()
            ),
            Err(PlannerError::Invalid(_))
        ));
        let mut planner = AssistantJournalPlanner::new(&reader, &identity()).unwrap();
        assert!(matches!(
            planner.part_update(&PartUpdate::Created { part: part.clone() }, &part, order()),
            Err(PlannerError::Invalid(_))
        ));
    }
}

#[test]
fn large_unicode_delta_uses_bounded_frames_without_truncation_or_new_generation() {
    let mut reader = Reader::default();
    let mut planner = AssistantJournalPlanner::new(&reader, &identity()).unwrap();
    planner.begin_text("text", order()).unwrap();
    let content = "界🙂".repeat(80_000);
    planner
        .part_update(
            &PartUpdate::TextDelta {
                part_id: "text".into(),
                delta: content.clone(),
            },
            &text(&content, None, false),
            order(),
        )
        .unwrap();
    let VisibleJournalPlan::Changed(frames) = planner.finish().unwrap() else {
        panic!("a real large delta must produce frames")
    };
    assert!(frames.len() > 1);
    let supplied = frames
        .iter()
        .flat_map(|frame| &frame.supplied_chunks)
        .map(AssistantTextChunkV1::text)
        .collect::<String>();
    assert_eq!(supplied, content);
    let mut append_count = 0;
    for frame in &frames {
        assert!(frame.batch.canonical_bytes().unwrap().len() <= MAX_OPERATION_BATCH_BYTES);
        assert!(frame.supplied_chunks.len() <= MAX_SUPPLIED_FRAME_CHUNKS);
        assert!(
            frame
                .supplied_chunks
                .iter()
                .map(AssistantTextChunkV1::byte_length)
                .sum::<usize>()
                <= MAX_SUPPLIED_FRAME_BYTES
        );
        append_count += frame
            .batch
            .operations
            .iter()
            .filter(|operation| matches!(operation, AssistantJournalOperationV1::Append(_)))
            .count();
        let changes = validate_assistant_batch(
            &ProofReader {
                original: &reader,
                plan: frame,
            },
            &frame.batch,
        )
        .unwrap();
        for change in changes {
            reader
                .heads
                .insert(change.part_id, change.replacement.unwrap());
        }
    }
    let head = &reader.heads["text"];
    assert_eq!(head.generation, JournalNumber(1));
    assert_eq!(head.revision, JournalNumber(1 + append_count as u64));
    assert_eq!(head.bodies[0].utf8_bytes.0, content.len() as u64);
    assert_eq!(reader.manifest_reads.get(), 0);
}

#[test]
fn many_changes_split_on_operation_count_without_losing_touched_head_order() {
    let mut reader = Reader::default();
    let mut planner = AssistantJournalPlanner::new(&reader, &identity()).unwrap();
    for index in 0..260 {
        planner
            .begin_text(
                &format!("text-{index}"),
                JournalOrderKey {
                    section: JournalSection::Transcript,
                    position: JournalNumber(index),
                },
            )
            .unwrap();
    }
    let VisibleJournalPlan::Changed(frames) = planner.finish().unwrap() else {
        panic!("declarations")
    };
    assert_eq!(
        frames
            .iter()
            .map(|frame| frame.batch.operations.len())
            .collect::<Vec<_>>(),
        vec![128, 128, 4]
    );
    let mut positions = Vec::new();
    for frame in &frames {
        assert!(frame.preparations.is_empty() && frame.supplied_chunks.is_empty());
        for operation in &frame.batch.operations {
            let AssistantJournalOperationV1::Declare(declaration) = operation else {
                panic!("ordered declaration")
            };
            positions.push(declaration.part.order_key.position.0);
        }
        let changes = validate_assistant_batch(
            &ProofReader {
                original: &reader,
                plan: frame,
            },
            &frame.batch,
        )
        .unwrap();
        for change in changes {
            let head = change.replacement.unwrap();
            let expected_position = head
                .part_id
                .strip_prefix("text-")
                .unwrap()
                .parse::<u64>()
                .unwrap();
            assert_eq!(head.order_key.position.0, expected_position);
            reader.heads.insert(change.part_id, head);
        }
    }
    assert_eq!(positions, (0..260).collect::<Vec<_>>());
    assert_eq!(reader.heads.len(), 260);
}

#[test]
fn metadata_frame_budget_is_actual_canonical_bytes_not_only_operation_count() {
    let mut reader = Reader::default();
    let mut planner = AssistantJournalPlanner::new(&reader, &identity()).unwrap();
    for index in 0..80 {
        let part = AgentPart::Source(SourcePart {
            id: format!("source-{index}"),
            ownership: PartOwnership {
                message_id: "assistant".into(),
                turn_id: "run".into(),
            },
            source_id: format!("reference-{index}"),
            // Each field stays below the inline threshold, but the combined
            // canonical batch must split before the 128-operation ceiling.
            title: "界".repeat(300),
            uri: format!("https://example.test/{}", "x".repeat(900)),
            mime_type: None,
            evidence: None,
        });
        planner
            .part_update(
                &PartUpdate::Created { part: part.clone() },
                &part,
                JournalOrderKey {
                    section: JournalSection::Transcript,
                    position: JournalNumber(index),
                },
            )
            .unwrap();
    }
    let VisibleJournalPlan::Changed(frames) = planner.finish().unwrap() else {
        panic!("inline Source metadata")
    };
    assert!(frames.len() > 1);
    assert_eq!(
        frames
            .iter()
            .map(|frame| frame.batch.operations.len())
            .sum::<usize>(),
        80
    );
    for frame in &frames {
        assert!(frame.batch.operations.len() < MAX_BATCH_OPERATIONS);
        assert!(frame.batch.canonical_bytes().unwrap().len() <= MAX_OPERATION_BATCH_BYTES);
        assert!(frame.preparations.is_empty() && frame.supplied_chunks.is_empty());
        let changes = validate_assistant_batch(
            &ProofReader {
                original: &reader,
                plan: frame,
            },
            &frame.batch,
        )
        .unwrap();
        for change in changes {
            let head = change.replacement.unwrap();
            assert!(
                matches!(&head.metadata, JournalPartMetadata::Source(metadata)
                if metadata.title == JournalTextValue::Inline("界".repeat(300)))
            );
            reader.heads.insert(change.part_id, head);
        }
    }
    assert_eq!(reader.heads.len(), 80);
}

#[test]
fn escaped_metadata_frames_leave_room_for_verified_chunk_transport() {
    use base64::Engine;
    use nola_domain::assistant_journal::{AssistantJournalCursor, ASSISTANT_FRAME_MAX_BYTES};
    let reader = Reader::default();
    let id = identity();
    let mut planner = AssistantJournalPlanner::new(&reader, &id).unwrap();
    for index in 0..80 {
        let part = AgentPart::Source(SourcePart {
            id: format!("source-{index}"),
            ownership: PartOwnership {
                message_id: "assistant".into(),
                turn_id: "run".into(),
            },
            source_id: format!("reference-{index}"),
            title: "\u{1}".repeat(1024),
            uri: format!("https://example.test/{}", "x".repeat(900)),
            mime_type: None,
            evidence: None,
        });
        planner
            .part_update(
                &PartUpdate::Created { part: part.clone() },
                &part,
                JournalOrderKey {
                    section: JournalSection::Transcript,
                    position: JournalNumber(index),
                },
            )
            .unwrap();
    }
    // This is the previously admitted shape: valid storage encoding, but no
    // reader can fit it into a single 256 KiB JSON record response.
    let old_batch = AssistantOperationBatchV1 {
        protocol_version: 1,
        identity: id.clone(),
        operations: planner.operations.iter().take(40).cloned().collect(),
    };
    assert!(old_batch.canonical_bytes().unwrap().len() < MAX_OPERATION_BATCH_BYTES);
    assert!(serde_json::to_vec(&old_batch).unwrap().len() > ASSISTANT_FRAME_MAX_BYTES);
    let expected_operations = planner.operations.clone();
    let VisibleJournalPlan::Changed(frames) = planner.finish().unwrap() else {
        panic!()
    };
    assert_eq!(
        frames
            .iter()
            .map(|frame| frame.batch.operations.len())
            .sum::<usize>(),
        80
    );
    assert_eq!(
        frames
            .iter()
            .flat_map(|frame| frame.batch.operations.iter().cloned())
            .collect::<Vec<_>>(),
        expected_operations
    );
    let chunk = AssistantTextChunkV1::new("\u{1}".repeat(ASSISTANT_CHUNK_MAX_BYTES)).unwrap();
    let wire_chunk = crate::AssistantJournalChunk {
        digest: chunk.digest(),
        utf8_bytes: JournalNumber(chunk.byte_length() as u64),
        data_base64: base64::engine::general_purpose::STANDARD.encode(chunk.text().as_bytes()),
    };
    for frame in frames {
        assert!(serde_json::to_vec(&frame.batch).unwrap().len() <= MAX_BATCH_JSON_BYTES);
        let header = frame
            .batch
            .seal(
                u64::MAX,
                AssistantJournalCursor::initial(id.clone())
                    .unwrap()
                    .record_digest,
                "\\".repeat(1024),
                u64::MAX,
            )
            .unwrap();
        let record = crate::AssistantJournalRecord {
            header,
            batch: frame.batch,
        };
        // Envelope-sizing proof only: these synthetic preparation fields are
        // not submitted as an authorized operation of the Source-only batch.
        let reference = empty_journal_content(
            &id,
            "sizing",
            JournalContentField::Markdown,
            JournalNumber(1),
        )
        .unwrap();
        let request = crate::StageAssistantJournalContent {
            source_host_id: "\\".repeat(1024),
            record,
            part_id: "\\".repeat(1024),
            content: reference.clone(),
            expected_prefix: reference,
            chunks: vec![wire_chunk.clone()],
            restart: false,
        };
        assert!(serde_json::to_vec(&request).unwrap().len() <= ASSISTANT_FRAME_MAX_BYTES);
    }
}

#[test]
fn a_delta_does_not_read_or_reserialize_the_existing_body() {
    let mut reader = Reader::default();
    let mut head = initial_head();
    // Build a valid immutable prefix once; the planning reader intentionally
    // has no body-reading method, so the operation cannot reread that prefix.
    let prefix = "a".repeat(1_000_000);
    head.bodies[0] =
        append_journal_content(&head.bodies[0], &chunk_refs(&chunk_text(&prefix).unwrap()))
            .unwrap();
    reader.heads.insert("text".into(), head);
    let mut planner = AssistantJournalPlanner::new(&reader, &identity()).unwrap();
    let full = format!("{prefix}z");
    planner
        .part_update(
            &PartUpdate::TextDelta {
                part_id: "text".into(),
                delta: "z".into(),
            },
            &text(&full, None, false),
            order(),
        )
        .unwrap();
    let plan = changed(planner.finish().unwrap());
    assert!(plan.preparations.is_empty());
    assert_eq!(plan.supplied_chunks[0].text(), "z");
    assert_eq!(reader.reads.get(), 1);
}

fn reasoning(detail: &str, final_: bool) -> AgentPart {
    AgentPart::Reasoning(ReasoningPart {
        id: "reason".into(),
        ownership: PartOwnership {
            message_id: "assistant".into(),
            turn_id: "run".into(),
        },
        title: "Reasoning title".into(),
        detail: detail.into(),
        is_complete: final_,
    })
}

#[test]
fn reasoning_deltas_and_completion_are_incremental_and_domain_validated() {
    let reader = Reader::default();
    let mut planner = AssistantJournalPlanner::new(&reader, &identity()).unwrap();
    let empty = reasoning("", false);
    planner
        .part_update(
            &PartUpdate::Created {
                part: empty.clone(),
            },
            &empty,
            order(),
        )
        .unwrap();
    planner
        .part_update(
            &PartUpdate::ReasoningDelta {
                part_id: "reason".into(),
                title: None,
                detail_delta: "思考".into(),
            },
            &reasoning("思考", false),
            order(),
        )
        .unwrap();
    planner
        .part_update(
            &PartUpdate::Completed {
                part_id: "reason".into(),
            },
            &reasoning("思考", true),
            order(),
        )
        .unwrap();
    planner
        .part_update(
            &PartUpdate::Completed {
                part_id: "reason".into(),
            },
            &reasoning("思考", true),
            order(),
        )
        .unwrap();
    let plan = changed(planner.finish().unwrap());
    assert_eq!(plan.batch.operations.len(), 3);
    assert_eq!(reader.reads.get(), 1);
    assert!(plan.preparations.is_empty());
    let changes = validate_assistant_batch(
        &ProofReader {
            original: &reader,
            plan: &plan,
        },
        &plan.batch,
    )
    .unwrap();
    let head = changes[0].replacement.as_ref().unwrap();
    assert_eq!(head.bodies[0].utf8_bytes.0, 6);
    assert_eq!(
        head.metadata,
        JournalPartMetadata::Reasoning(JournalReasoningMetadata {
            title: "Reasoning title".into(),
            state: nola_domain::ActivityState::Completed
        })
    );
}

#[test]
fn reasoning_title_only_update_is_a_real_visible_change() {
    let mut reader = Reader::default();
    let mut head = initial_head();
    head.part_id = "reason".into();
    head.metadata = JournalPartMetadata::Reasoning(JournalReasoningMetadata {
        title: "old title".into(),
        state: nola_domain::ActivityState::Running,
    });
    head.bodies = vec![empty_journal_content(
        &identity(),
        "reason",
        JournalContentField::ReasoningSummary,
        JournalNumber(1),
    )
    .unwrap()];
    reader.heads.insert("reason".into(), head);
    let mut planner = AssistantJournalPlanner::new(&reader, &identity()).unwrap();
    planner
        .part_update(
            &PartUpdate::ReasoningDelta {
                part_id: "reason".into(),
                title: Some("Reasoning title".into()),
                detail_delta: String::new(),
            },
            &reasoning("", false),
            order(),
        )
        .unwrap();
    let plan = changed(planner.finish().unwrap());
    assert_eq!(plan.batch.operations.len(), 1);
    let changes = validate_assistant_batch(
        &ProofReader {
            original: &reader,
            plan: &plan,
        },
        &plan.batch,
    )
    .unwrap();
    assert!(matches!(&changes[0].replacement.as_ref().unwrap().metadata,
        JournalPartMetadata::Reasoning(metadata) if metadata.title == JournalTextValue::Inline("Reasoning title".into())));
}

#[test]
fn reasoning_replacement_uses_new_generation_and_terminal_append_is_rejected() {
    let reader = Reader::default();
    let mut planner = AssistantJournalPlanner::new(&reader, &identity()).unwrap();
    let original = reasoning("old", false);
    planner
        .part_update(
            &PartUpdate::Created {
                part: original.clone(),
            },
            &original,
            order(),
        )
        .unwrap();
    planner
        .part_update(
            &PartUpdate::ReasoningReplaced {
                part_id: "reason".into(),
                title: "Reasoning title".into(),
                detail: "new".into(),
                is_complete: true,
            },
            &reasoning("new", true),
            order(),
        )
        .unwrap();
    let plan = changed(planner.finish().unwrap());
    let changes = validate_assistant_batch(
        &ProofReader {
            original: &reader,
            plan: &plan,
        },
        &plan.batch,
    )
    .unwrap();
    let mut persisted = Reader::default();
    persisted
        .heads
        .insert("reason".into(), changes[0].replacement.clone().unwrap());
    let mut planner = AssistantJournalPlanner::new(&persisted, &identity()).unwrap();
    assert!(matches!(
        planner.part_update(
            &PartUpdate::ReasoningDelta {
                part_id: "reason".into(),
                title: None,
                detail_delta: " late".into()
            },
            &reasoning("new late", true),
            order()
        ),
        Err(PlannerError::Invalid(_))
    ));
}

fn tool(
    input: serde_json::Value,
    preview: Option<&str>,
    structured: Option<serde_json::Value>,
) -> ToolPart {
    ToolPart {
        execution_source: Default::default(),
        id: "tool".into(),
        ownership: PartOwnership {
            message_id: "assistant".into(),
            turn_id: "run".into(),
        },
        call_id: "call".into(),
        capability_id: "execute".into(),
        title: "display title".into(),
        summary: None,
        origin: Default::default(),
        state: ToolPartState::Running,
        input,
        raw_input: "private incomplete input".into(),
        permission: None,
        output_preview: preview.map(str::to_owned),
        structured_output: structured,
        full_output_ref: None,
        artifacts: vec![],
        started_at_ms: None,
        finished_at_ms: None,
    }
}
fn tool_state_update() -> PartUpdate {
    PartUpdate::ToolState {
        part_id: "tool".into(),
        state: ToolPartState::Running,
        title: None,
        summary: None,
        output_preview: None,
        structured_output: None,
        full_output_ref: None,
        artifacts: vec![],
        timestamp_ms: None,
    }
}

#[test]
fn tool_progress_appends_only_new_preview_and_preserves_large_input_manifest() {
    let reader = Reader::default();
    let mut planner = AssistantJournalPlanner::new(&reader, &identity()).unwrap();
    let mut value = tool(
        serde_json::json!({"large": "i".repeat(600_000)}),
        Some("start"),
        None,
    );
    let created = AgentPart::Tool(value.clone());
    planner
        .part_update(
            &PartUpdate::Created {
                part: created.clone(),
            },
            &created,
            order(),
        )
        .unwrap();
    let input = planner
        .head("tool")
        .unwrap()
        .unwrap()
        .bodies
        .iter()
        .find(|r| r.field == JournalContentField::ToolInput)
        .unwrap()
        .clone();
    let prepared_count = planner.preparations.len();
    value.output_preview = Some("start新".into());
    planner
        .part_update(
            &PartUpdate::ToolProgress {
                part_id: "tool".into(),
                title: None,
                detail: None,
                output_delta: Some("新".into()),
            },
            &AgentPart::Tool(value),
            order(),
        )
        .unwrap();
    let plan = changed(planner.finish().unwrap());
    assert_eq!(plan.preparations.len(), prepared_count);
    assert_eq!(
        plan.supplied_chunks
            .iter()
            .map(|c| c.text())
            .collect::<String>(),
        "新"
    );
    let changes = validate_assistant_batch(
        &ProofReader {
            original: &reader,
            plan: &plan,
        },
        &plan.batch,
    )
    .unwrap();
    assert!(changes[0]
        .replacement
        .as_ref()
        .unwrap()
        .bodies
        .contains(&input));
}

#[test]
fn tool_raw_input_stays_private_while_structured_result_and_preview_are_independent() {
    let reader = Reader::default();
    let mut planner = AssistantJournalPlanner::new(&reader, &identity()).unwrap();
    let mut value = tool(
        serde_json::json!({"n":u64::MAX}),
        Some("ignored"),
        Some(serde_json::json!({"ok":true})),
    );
    let created = AgentPart::Tool(value.clone());
    planner
        .part_update(
            &PartUpdate::Created {
                part: created.clone(),
            },
            &created,
            order(),
        )
        .unwrap();
    let count = planner.operations.len();
    value.raw_input.push_str("raw fragment");
    value.output_preview = Some("ignoredchanged".into());
    planner
        .part_update(
            &PartUpdate::ToolInputDelta {
                part_id: "tool".into(),
                delta: "raw fragment".into(),
            },
            &AgentPart::Tool(value.clone()),
            order(),
        )
        .unwrap();
    assert_eq!(planner.operations.len(), count);
    planner
        .part_update(
            &PartUpdate::ToolProgress {
                part_id: "tool".into(),
                title: None,
                detail: None,
                output_delta: Some("changed".into()),
            },
            &AgentPart::Tool(value),
            order(),
        )
        .unwrap();
    assert_eq!(planner.operations.len(), count + 1);
    let plan = changed(planner.finish().unwrap());
    let changes = validate_assistant_batch(
        &ProofReader {
            original: &reader,
            plan: &plan,
        },
        &plan.batch,
    )
    .unwrap();
    let head = changes[0].replacement.as_ref().unwrap();
    assert!(head
        .bodies
        .iter()
        .any(|r| r.field == JournalContentField::ToolOutput));
    assert!(head
        .bodies
        .iter()
        .any(|r| r.field == JournalContentField::ToolOutputText));
    assert!(plan
        .preparations
        .iter()
        .flat_map(|p| &p.chunks)
        .any(|c| c.text().contains("18446744073709551615")));
}

#[test]
fn tool_output_kind_and_resource_reference_switches_validate_each_ordered_operation() {
    let reader = Reader::default();
    let mut planner = AssistantJournalPlanner::new(&reader, &identity()).unwrap();
    let mut value = tool(serde_json::json!(null), Some("plain"), None);
    let created = AgentPart::Tool(value.clone());
    planner
        .part_update(
            &PartUpdate::Created {
                part: created.clone(),
            },
            &created,
            order(),
        )
        .unwrap();
    value.structured_output = Some(serde_json::json!([true, 2]));
    value.artifacts = vec![ArtifactReference {
        uri: "uri".repeat(2000),
        name: "name".into(),
        mime_type: None,
        operation: None,
    }];
    planner
        .part_update(
            &tool_state_update(),
            &AgentPart::Tool(value.clone()),
            order(),
        )
        .unwrap();
    value.structured_output = None;
    value.output_preview = Some("plain again".into());
    value.artifacts.clear();
    planner
        .part_update(&tool_state_update(), &AgentPart::Tool(value), order())
        .unwrap();
    let plan = changed(planner.finish().unwrap());
    let changes = validate_assistant_batch(
        &ProofReader {
            original: &reader,
            plan: &plan,
        },
        &plan.batch,
    )
    .unwrap();
    let head = changes[0].replacement.as_ref().unwrap();
    assert!(head
        .bodies
        .iter()
        .any(|r| r.field == JournalContentField::ToolOutputText));
    assert!(!head.bodies.iter().any(|r| matches!(
        r.field,
        JournalContentField::ToolOutput | JournalContentField::ToolResources
    )));
}

#[test]
fn tool_preview_tail_replacement_and_permission_do_not_copy_input() {
    let reader = Reader::default();
    let mut planner = AssistantJournalPlanner::new(&reader, &identity()).unwrap();
    let mut value = tool(serde_json::json!({"x":1}), Some(&"a".repeat(16_384)), None);
    let created = AgentPart::Tool(value.clone());
    planner
        .part_update(
            &PartUpdate::Created {
                part: created.clone(),
            },
            &created,
            order(),
        )
        .unwrap();
    value.output_preview = Some(format!("{}b", "a".repeat(16_383)));
    planner
        .part_update(
            &PartUpdate::ToolProgress {
                part_id: "tool".into(),
                title: None,
                detail: None,
                output_delta: Some("b".into()),
            },
            &AgentPart::Tool(value.clone()),
            order(),
        )
        .unwrap();
    value.state = ToolPartState::WaitingForPermission;
    value.permission = Some(ToolPermissionPart {
        request_id: "p".into(),
        permission_domain: "file".into(),
        rationale: "Read-only permission display".into(),
        decision: None,
    });
    planner
        .part_update(
            &PartUpdate::ToolPermission {
                part_id: "tool".into(),
                permission: value.permission.clone().unwrap(),
            },
            &AgentPart::Tool(value),
            order(),
        )
        .unwrap();
    let plan = changed(planner.finish().unwrap());
    assert!(plan.supplied_chunks.is_empty());
    assert_eq!(
        plan.preparations
            .iter()
            .filter(|p| p.resulting.field == JournalContentField::ToolInput)
            .count(),
        1
    );
    let changes = validate_assistant_batch(
        &ProofReader {
            original: &reader,
            plan: &plan,
        },
        &plan.batch,
    )
    .unwrap();
    assert!(
        matches!(&changes[0].replacement.as_ref().unwrap().metadata, JournalPartMetadata::Tool(m) if m.state == nola_domain::ActivityState::WaitingForPermission)
    );
}

#[test]
fn source_citation_and_attachment_keep_large_text_and_single_line_bounds() {
    let reader = Reader::default();
    let mut planner = AssistantJournalPlanner::new(&reader, &identity()).unwrap();
    let owner = PartOwnership {
        message_id: "assistant".into(),
        turn_id: "run".into(),
    };
    let large = "文字".repeat(20_000);
    let parts = [
        AgentPart::Source(SourcePart {
            id: "source".into(),
            ownership: owner.clone(),
            source_id: "s".into(),
            title: large.clone(),
            uri: "https://example.invalid".into(),
            mime_type: None,
            evidence: None,
        }),
        AgentPart::Citation(CitationPart {
            id: "citation".into(),
            ownership: owner.clone(),
            citation_id: "c".into(),
            source_id: "s".into(),
            title: "cite".into(),
            uri: large.clone(),
            mime_type: None,
            location: Some(SourceLocation {
                page: None,
                line_start: Some(9),
                line_end: None,
                section: None,
            }),
            range: None,
            origin_tool_call_id: "call".into(),
        }),
        AgentPart::Artifact(ArtifactPart {
            id: "artifact".into(),
            ownership: owner,
            artifact: ArtifactReference {
                uri: "file-ref".into(),
                name: large.clone(),
                mime_type: None,
                operation: None,
            },
        }),
    ];
    for part in &parts {
        planner
            .part_update(&PartUpdate::Created { part: part.clone() }, part, order())
            .unwrap();
    }
    let plan = changed(planner.finish().unwrap());
    let changes = validate_assistant_batch(
        &ProofReader {
            original: &reader,
            plan: &plan,
        },
        &plan.batch,
    )
    .unwrap();
    assert_eq!(changes.len(), 3);
    let citation = changes
        .iter()
        .find(|c| c.part_id == "citation")
        .unwrap()
        .replacement
        .as_ref()
        .unwrap();
    assert!(
        matches!(&citation.metadata, JournalPartMetadata::Citation(m) if m.location.as_ref().is_some_and(|location| location.line_start == Some(JournalNumber(9)) && location.line_end.is_none()))
    );
    assert_eq!(
        plan.preparations
            .iter()
            .flat_map(|p| &p.chunks)
            .map(|c| c.byte_length())
            .sum::<usize>(),
        large.len() * 3
    );
}

#[test]
fn withheld_review_state_changes_are_public_metadata_without_candidate_bytes() {
    let reader = Reader::default();
    let mut planner = AssistantJournalPlanner::new(&reader, &identity()).unwrap();
    let pending = text(
        "never disclose this candidate",
        Some(TextReviewState::Pending),
        false,
    );
    planner
        .part_update(
            &PartUpdate::Created {
                part: pending.clone(),
            },
            &pending,
            order(),
        )
        .unwrap();
    for review in [TextReviewState::Rejected, TextReviewState::Interrupted] {
        planner
            .part_update(
                &PartUpdate::TextReviewChanged {
                    part_id: "text".into(),
                    state: review,
                },
                &text("never disclose this candidate", Some(review), true),
                order(),
            )
            .unwrap();
    }
    let plan = changed(planner.finish().unwrap());
    assert!(plan.preparations.is_empty() && plan.supplied_chunks.is_empty());
    assert_eq!(plan.batch.operations.len(), 3);
    let changes = validate_assistant_batch(
        &ProofReader {
            original: &reader,
            plan: &plan,
        },
        &plan.batch,
    )
    .unwrap();
    let final_head = changes[0].replacement.as_ref().unwrap();
    assert!(final_head.bodies.is_empty());
    assert!(
        matches!(&final_head.metadata, JournalPartMetadata::Text(metadata)
        if metadata.review == Some(TextReviewState::Interrupted) && metadata.visibility == JournalVisibility::Withheld)
    );
}

fn prepared_body(
    plan: &PreparedAssistantJournal,
    part: &str,
    field: JournalContentField,
) -> Option<String> {
    let preparations: Vec<_> = plan
        .preparations
        .iter()
        .filter(|preparation| preparation.part_id == part && preparation.previous.field == field)
        .collect();
    if preparations.is_empty() {
        return None;
    }
    Some(
        preparations
            .into_iter()
            .flat_map(|preparation| &preparation.chunks)
            .map(|chunk| chunk.text())
            .collect(),
    )
}

#[test]
fn tool_result_absence_null_empty_and_independent_display_facts_survive_journal() {
    use nola_domain::assistant_journal_content::{
        decode_assistant_artifact_content, decode_assistant_structured_content,
    };
    for result in [
        None,
        Some(serde_json::Value::Null),
        Some(serde_json::json!("")),
        Some(serde_json::json!([])),
        Some(serde_json::json!({})),
    ] {
        let reader = Reader::default();
        let mut planner = AssistantJournalPlanner::new(&reader, &identity()).unwrap();
        let mut tool = tool(serde_json::json!({"n": u64::MAX}), Some(""), result.clone());
        tool.title = "真实工具标题🙂".repeat(400);
        tool.summary = Some(String::new());
        tool.origin = nola_domain::HostCapabilityOrigin {
            kind: nola_domain::HostCapabilityOriginKind::Mcp,
            provider_id: Some("provider".into()),
            provider_title: Some("来源🙂".repeat(400)),
            namespace: Some("".into()),
        };
        tool.permission = Some(ToolPermissionPart {
            request_id: "permission".into(),
            permission_domain: "files".into(),
            rationale: "用户审批说明🙂".repeat(400),
            decision: Some("".into()),
        });
        tool.full_output_ref = Some("/private/host/never-public".into());
        tool.started_at_ms = Some(u64::MAX);
        tool.finished_at_ms = Some(0);
        tool.artifacts = vec![ArtifactReference {
            uri: "file-ref".into(),
            name: "原始名字".into(),
            mime_type: Some("".into()),
            operation: None,
        }];
        let part = AgentPart::Tool(tool.clone());
        planner
            .part_update(&PartUpdate::Created { part: part.clone() }, &part, order())
            .unwrap();
        let plan = changed(planner.finish().unwrap());
        let changes = validate_assistant_batch(
            &ProofReader {
                original: &reader,
                plan: &plan,
            },
            &plan.batch,
        )
        .unwrap();
        let head = changes[0].replacement.as_ref().unwrap();
        let JournalPartMetadata::Tool(metadata) = &head.metadata else {
            panic!("Tool metadata")
        };
        assert_eq!(
            metadata.title,
            JournalTextValue::Field(JournalContentField::Title)
        );
        assert_eq!(
            metadata.summary,
            Some(JournalTextValue::Inline(String::new()))
        );
        assert_eq!(metadata.started_at_ms, Some(JournalNumber(u64::MAX)));
        assert_eq!(metadata.finished_at_ms, Some(JournalNumber(0)));
        assert_eq!(
            metadata.full_output.as_ref().unwrap().message_id.0,
            "assistant"
        );
        assert_eq!(metadata.full_output.as_ref().unwrap().part_id.0, "tool");
        assert_eq!(
            metadata.full_output.as_ref().unwrap().tool_call_id.0,
            "call"
        );
        assert_eq!(
            metadata.permission.as_ref().unwrap().rationale,
            JournalTextValue::Field(JournalContentField::PermissionRationale)
        );
        assert_eq!(
            metadata.permission.as_ref().unwrap().decision,
            Some(JournalTextValue::Inline(String::new()))
        );
        assert!(head.bodies.iter().any(
            |body| body.field == JournalContentField::ToolOutputText && body.utf8_bytes.0 == 0
        ));
        let structured = prepared_body(&plan, "tool", JournalContentField::ToolOutput)
            .map(|wire| decode_assistant_structured_content(&wire).unwrap());
        assert_eq!(
            structured,
            result.map(|value| serde_json::from_value(value).unwrap())
        );
        assert_eq!(
            decode_assistant_artifact_content(
                &prepared_body(&plan, "tool", JournalContentField::ToolArtifacts).unwrap()
            )
            .unwrap(),
            tool.artifacts
        );
        assert_eq!(
            prepared_body(&plan, "tool", JournalContentField::Title).as_deref(),
            Some(tool.title.as_str())
        );
        let wire = serde_json::to_string(&plan.batch).unwrap();
        assert!(!wire.contains("never-public") && !wire.contains("raw_input"));
    }
}

#[test]
fn tool_title_and_summary_only_updates_preserve_large_input_and_independent_result_refs() {
    let reader = Reader::default();
    let mut planner = AssistantJournalPlanner::new(&reader, &identity()).unwrap();
    let mut value = tool(
        serde_json::json!({"large": "i".repeat(8 * 1024 * 1024)}),
        Some("preview"),
        Some(serde_json::Value::Null),
    );
    value.title = "长标题🙂".repeat(400);
    value.summary = Some("原摘要🙂".repeat(400));
    let part = AgentPart::Tool(value.clone());
    planner
        .part_update(&PartUpdate::Created { part: part.clone() }, &part, order())
        .unwrap();
    let old = planner.head("tool").unwrap().unwrap();
    let input = old
        .bodies
        .iter()
        .find(|body| body.field == JournalContentField::ToolInput)
        .unwrap()
        .clone();
    let output = old
        .bodies
        .iter()
        .find(|body| body.field == JournalContentField::ToolOutput)
        .unwrap()
        .clone();
    let preview = old
        .bodies
        .iter()
        .find(|body| body.field == JournalContentField::ToolOutputText)
        .unwrap()
        .clone();
    let preparations = planner.preparations.len();
    value.title = "短标题".into();
    planner
        .part_update(
            &PartUpdate::ToolProgress {
                part_id: "tool".into(),
                title: Some(value.title.clone()),
                detail: None,
                output_delta: None,
            },
            &AgentPart::Tool(value.clone()),
            order(),
        )
        .unwrap();
    value.summary = Some(String::new());
    planner
        .part_update(
            &PartUpdate::ToolState {
                part_id: "tool".into(),
                state: ToolPartState::Running,
                title: None,
                summary: Some(String::new()),
                output_preview: None,
                structured_output: None,
                full_output_ref: None,
                artifacts: vec![],
                timestamp_ms: None,
            },
            &AgentPart::Tool(value.clone()),
            order(),
        )
        .unwrap();
    assert_eq!(planner.preparations.len(), preparations);
    let plan = changed(planner.finish().unwrap());
    assert!(plan.supplied_chunks.is_empty());
    let changes = validate_assistant_batch(
        &ProofReader {
            original: &reader,
            plan: &plan,
        },
        &plan.batch,
    )
    .unwrap();
    let head = changes[0].replacement.as_ref().unwrap();
    assert!(
        head.bodies.contains(&input)
            && head.bodies.contains(&output)
            && head.bodies.contains(&preview)
    );
    assert!(!head.bodies.iter().any(|body| matches!(
        body.field,
        JournalContentField::Title | JournalContentField::ToolSummary
    )));
    assert!(matches!(&head.metadata, JournalPartMetadata::Tool(metadata)
        if metadata.title == JournalTextValue::Inline("短标题".into()) && metadata.summary == Some(JournalTextValue::Inline(String::new()))));
}

#[test]
fn real_core_progress_detail_is_preview_not_summary_and_delta_has_precedence() {
    use nola_agent_engine::agent::parts::ConversationProjection;
    let reader = Reader::default();
    let mut planner = AssistantJournalPlanner::new(&reader, &identity()).unwrap();
    let mut core = ConversationProjection::default();
    let mut value = tool(
        serde_json::json!({"x": 1}),
        Some("old"),
        Some(serde_json::Value::Null),
    );
    value.summary = Some("Human summary".into());
    let created = PartUpdate::Created {
        part: AgentPart::Tool(value),
    };
    assert!(core.apply(created.clone()).unwrap());
    planner
        .part_update(&created, &core.parts["tool"], order())
        .unwrap();
    let input = planner
        .head("tool")
        .unwrap()
        .unwrap()
        .bodies
        .into_iter()
        .find(|body| body.field == JournalContentField::ToolInput)
        .unwrap();
    let updates = [
        PartUpdate::ToolProgress {
            part_id: "tool".into(),
            title: Some(String::new()),
            detail: Some("replacement detail".into()),
            output_delta: None,
        },
        PartUpdate::ToolProgress {
            part_id: "tool".into(),
            title: None,
            detail: Some("must not replace appended output".into()),
            output_delta: Some("新增".into()),
        },
        PartUpdate::ToolProgress {
            part_id: "tool".into(),
            title: None,
            detail: Some("second replacement".into()),
            output_delta: Some(String::new()),
        },
        PartUpdate::ToolState {
            part_id: "tool".into(),
            state: ToolPartState::Running,
            title: None,
            summary: Some(String::new()),
            output_preview: None,
            structured_output: None,
            full_output_ref: None,
            artifacts: vec![],
            timestamp_ms: None,
        },
    ];
    for (index, update) in updates.iter().enumerate() {
        assert!(core.apply(update.clone()).unwrap());
        planner
            .part_update(update, &core.parts["tool"], order())
            .unwrap();
        let head = planner.head("tool").unwrap().unwrap();
        let JournalPartMetadata::Tool(metadata) = &head.metadata else {
            panic!("Tool")
        };
        assert_eq!(
            metadata.summary,
            Some(JournalTextValue::Inline(if index == 3 {
                String::new()
            } else {
                "Human summary".into()
            }))
        );
        assert_eq!(
            metadata.title,
            JournalTextValue::Inline("display title".into())
        );
        assert!(head.bodies.contains(&input));
    }
    let plan = changed(planner.finish().unwrap());
    let changes = validate_assistant_batch(
        &ProofReader {
            original: &reader,
            plan: &plan,
        },
        &plan.batch,
    )
    .unwrap();
    assert!(changes[0]
        .replacement
        .as_ref()
        .unwrap()
        .bodies
        .contains(&input));
    assert!(plan
        .preparations
        .iter()
        .any(
            |preparation| preparation.previous.field == JournalContentField::ToolOutputText
                && preparation
                    .chunks
                    .iter()
                    .map(|chunk| chunk.text())
                    .collect::<String>()
                    == "second replacement"
        ));
}

#[test]
fn real_core_bounded_deep_input_is_the_public_journal_input() {
    use nola_domain::assistant_journal_content::decode_assistant_structured_content;
    use nola_agent_engine::agent::parts::ConversationProjection;
    let reader = Reader::default();
    let mut planner = AssistantJournalPlanner::new(&reader, &identity()).unwrap();
    let mut core = ConversationProjection::default();
    let created = PartUpdate::Created {
        part: AgentPart::Tool(tool(serde_json::Value::Null, None, None)),
    };
    core.apply(created.clone()).unwrap();
    planner
        .part_update(&created, &core.parts["tool"], order())
        .unwrap();
    let mut input = serde_json::json!({"leaf": "original"});
    for _ in 0..50 {
        input = serde_json::Value::Array(vec![input]);
    }
    let update = PartUpdate::ToolInputResolved {
        part_id: "tool".into(),
        input,
    };
    assert!(core.apply(update.clone()).unwrap());
    planner
        .part_update(&update, &core.parts["tool"], order())
        .unwrap();
    let AgentPart::Tool(tool) = &core.parts["tool"] else {
        panic!("Tool")
    };
    let plan = changed(planner.finish().unwrap());
    let preparation = plan
        .preparations
        .iter()
        .rev()
        .find(|preparation| preparation.previous.field == JournalContentField::ToolInput)
        .unwrap();
    let wire = preparation
        .chunks
        .iter()
        .map(|chunk| chunk.text())
        .collect::<String>();
    assert_eq!(
        serde_json::to_value(decode_assistant_structured_content(&wire).unwrap()).unwrap(),
        tool.input
    );
    validate_assistant_batch(
        &ProofReader {
            original: &reader,
            plan: &plan,
        },
        &plan.batch,
    )
    .unwrap();
}

#[test]
fn long_reasoning_title_survives_detail_append_and_inline_switch_each_op_valid() {
    let reader = Reader::default();
    let mut planner = AssistantJournalPlanner::new(&reader, &identity()).unwrap();
    let mut part = reasoning("start", false);
    let long_title = "推理标题🙂".repeat(300);
    if let AgentPart::Reasoning(part) = &mut part {
        part.title = long_title.clone();
    }
    planner
        .part_update(&PartUpdate::Created { part: part.clone() }, &part, order())
        .unwrap();
    let title_ref = planner
        .head("reason")
        .unwrap()
        .unwrap()
        .bodies
        .into_iter()
        .find(|body| body.field == JournalContentField::Title)
        .unwrap();
    if let AgentPart::Reasoning(part) = &mut part {
        part.detail.push_str("新增");
    }
    planner
        .part_update(
            &PartUpdate::ReasoningDelta {
                part_id: "reason".into(),
                title: None,
                detail_delta: "新增".into(),
            },
            &part,
            order(),
        )
        .unwrap();
    assert!(planner
        .head("reason")
        .unwrap()
        .unwrap()
        .bodies
        .contains(&title_ref));
    if let AgentPart::Reasoning(part) = &mut part {
        part.title = "短标题".into();
    }
    planner
        .part_update(
            &PartUpdate::ReasoningDelta {
                part_id: "reason".into(),
                title: Some("短标题".into()),
                detail_delta: String::new(),
            },
            &part,
            order(),
        )
        .unwrap();
    let plan = changed(planner.finish().unwrap());
    let changes = validate_assistant_batch(
        &ProofReader {
            original: &reader,
            plan: &plan,
        },
        &plan.batch,
    )
    .unwrap();
    let head = changes[0].replacement.as_ref().unwrap();
    assert!(!head
        .bodies
        .iter()
        .any(|body| body.field == JournalContentField::Title));
    assert!(head
        .bodies
        .iter()
        .any(|body| body.field == JournalContentField::ReasoningSummary));
    assert!(
        matches!(&head.metadata, JournalPartMetadata::Reasoning(metadata)
        if metadata.title == JournalTextValue::Inline("短标题".into()))
    );
}

#[test]
fn source_evidence_citation_identity_mime_section_and_optional_location_are_lossless() {
    use nola_domain::assistant_journal_content::decode_assistant_source_evidence_content;
    use nola_domain::{MemoryUsage, SourceEvidence};
    let evidence = SourceEvidence::Memory {
        usage: MemoryUsage {
            record_id: "record".into(),
            component_id: "component".into(),
            title: Some("脱敏证据🙂".repeat(400)),
            usage_count: u64::MAX,
            last_used_at_epoch_ms: Some(i64::MIN),
            requires_current_state_verification: true,
            truncated: true,
            supersedes: vec!["old".into()],
            ..Default::default()
        },
    };
    let owner = PartOwnership {
        message_id: "assistant".into(),
        turn_id: "run".into(),
    };
    let source = AgentPart::Source(SourcePart {
        id: "source".into(),
        ownership: owner.clone(),
        source_id: "source-real".into(),
        title: "Source".into(),
        uri: "source-ref".into(),
        mime_type: None,
        evidence: Some(evidence.clone()),
    });
    let reader = Reader::default();
    let mut planner = AssistantJournalPlanner::new(&reader, &identity()).unwrap();
    planner
        .part_update(
            &PartUpdate::Created {
                part: source.clone(),
            },
            &source,
            order(),
        )
        .unwrap();
    let locations = [
        None,
        Some(SourceLocation::default()),
        Some(SourceLocation {
            page: Some(u64::MAX),
            line_start: None,
            line_end: Some(9),
            section: Some("节标题🙂".repeat(400)),
        }),
    ];
    for (index, location) in locations.iter().enumerate() {
        let part = AgentPart::Citation(CitationPart {
            id: format!("citation-part-{index}"),
            ownership: owner.clone(),
            citation_id: format!("real-citation-{index}"),
            source_id: "source-real".into(),
            title: "Citation".into(),
            uri: "citation-ref".into(),
            mime_type: Some("application/pdf".into()),
            location: location.clone(),
            range: Some(nola_domain::CitationRange {
                start: 0,
                end: u64::MAX,
            }),
            origin_tool_call_id: "call".into(),
        });
        planner
            .part_update(&PartUpdate::Created { part: part.clone() }, &part, order())
            .unwrap();
    }
    let plan = changed(planner.finish().unwrap());
    let changes = validate_assistant_batch(
        &ProofReader {
            original: &reader,
            plan: &plan,
        },
        &plan.batch,
    )
    .unwrap();
    assert_eq!(
        decode_assistant_source_evidence_content(
            &prepared_body(&plan, "source", JournalContentField::SourceEvidence).unwrap()
        )
        .unwrap(),
        evidence
    );
    for (index, location) in locations.iter().enumerate() {
        let head = changes
            .iter()
            .find(|change| change.part_id == format!("citation-part-{index}"))
            .unwrap()
            .replacement
            .as_ref()
            .unwrap();
        let JournalPartMetadata::Citation(metadata) = &head.metadata else {
            panic!("Citation")
        };
        assert_eq!(metadata.citation_id, format!("real-citation-{index}"));
        assert_eq!(
            metadata.mime_type,
            Some(JournalTextValue::Inline("application/pdf".into()))
        );
        assert_eq!(metadata.location.is_some(), location.is_some());
        assert_eq!(
            metadata.byte_range.as_ref().unwrap().end,
            JournalNumber(u64::MAX)
        );
        if index == 2 {
            assert_eq!(
                metadata.location.as_ref().unwrap().section,
                Some(JournalTextValue::Field(
                    JournalContentField::CitationSection
                ))
            );
        }
    }
}
