// SPDX-License-Identifier: Apache-2.0
use super::*;
use crate::assistant_journal::{AssistantJournalCursor, AssistantTextChunkV1};
use std::cell::Cell;

#[test]
fn execution_source_binary_roundtrip_and_unknown_tag_rejection() {
    use crate::code_mode::{CodeCellIdentity, CodeItemSource};
    for source in [CodeItemSource::Model, CodeItemSource::CodeCell { cell: CodeCellIdentity {
        host_id: "host".into(), conversation_id: "session".into(), turn_id: "turn".into(),
        parent_call_id: "parent".into(), cell_id: "cell".into(), generation: "generation".into(),
    }}] {
        let mut bytes = Vec::new();
        source.write(&mut bytes).unwrap();
        let mut reader = Reader { bytes: &bytes, offset: 0 };
        assert_eq!(CodeItemSource::read(&mut reader).unwrap(), source);
        assert_eq!(reader.offset, bytes.len());
        if bytes.len() > 1 {
            assert!(CodeItemSource::read(&mut Reader { bytes: &bytes[..bytes.len()-1], offset: 0 }).is_err());
        }
    }
    assert_eq!(CodeItemSource::read(&mut Reader { bytes: &[2], offset: 0 }), Err(OperationError::InvalidWire));
}

#[test]
fn activity_state_binary_tags_roundtrip_without_conflating_sealed_and_completed() {
    for (state, tag) in [
        (ActivityState::Pending, 0), (ActivityState::Running, 1),
        (ActivityState::WaitingForPermission, 2), (ActivityState::Completed, 3),
        (ActivityState::Cancelled, 4), (ActivityState::Failed, 5),
        (ActivityState::WaitingForInput, 6), (ActivityState::Sealed, 7),
    ] {
        let mut bytes = Vec::new();
        state.write(&mut bytes).unwrap();
        assert_eq!(bytes, vec![tag]);
        let mut reader = Reader { bytes: &bytes, offset: 0 };
        assert_eq!(ActivityState::read(&mut reader).unwrap(), state);
        assert_eq!(reader.offset, bytes.len());
    }
    assert_eq!(ActivityState::read(&mut Reader { bytes: &[8], offset: 0 }),
        Err(OperationError::InvalidWire));
}

fn identity() -> AssistantJournalIdentity {
    AssistantJournalIdentity {
        source_store_epoch: "epoch".into(),
        conversation_id: "conversation".into(),
        run_id: "run".into(),
        assistant_message_id: "assistant".into(),
    }
}
fn head(id: &str) -> JournalPartHead {
    JournalPartHead {
        identity: identity(),
        part_id: id.into(),
        generation: JournalNumber(1),
        revision: JournalNumber(1),
        metadata_revision: JournalNumber(1),
        order_key: JournalOrderKey {
            section: JournalSection::Transcript,
            position: JournalNumber(1),
        },
        metadata: JournalPartMetadata::Text(JournalTextMetadata {
            is_final: false,
            visibility: JournalVisibility::Public,
            review: None,
        }),
        bodies: vec![empty_journal_content(
            &identity(),
            id,
            JournalContentField::Markdown,
            JournalNumber(1),
        )
        .unwrap()],
    }
}
fn batch(operations: Vec<AssistantJournalOperationV1>) -> AssistantOperationBatchV1 {
    AssistantOperationBatchV1 {
        protocol_version: 1,
        identity: identity(),
        operations,
    }
}
fn declare(head: JournalPartHead) -> AssistantJournalOperationV1 {
    AssistantJournalOperationV1::Declare(DeclareJournalPart { part: head })
}
fn chunk(text: &str) -> JournalChunkRef {
    let c = AssistantTextChunkV1::new(text.into()).unwrap();
    JournalChunkRef {
        digest: c.digest(),
        utf8_bytes: JournalNumber(c.byte_length() as u64),
    }
}
fn append(h: &JournalPartHead, c: JournalChunkRef) -> AssistantJournalOperationV1 {
    AssistantJournalOperationV1::Append(AppendJournalText {
        part_id: h.part_id.clone(),
        expected_generation: h.generation,
        expected_revision: h.revision,
        expected: h.bodies[0].clone(),
        resulting: append_journal_content(&h.bodies[0], std::slice::from_ref(&c)).unwrap(),
        chunks: vec![c],
    })
}
#[derive(Default)]
struct State {
    heads: BTreeMap<String, JournalPartHead>,
    chunks: Vec<JournalChunkRef>,
    manifests: Vec<JournalContentRef>,
    retired: BTreeSet<String>,
    loads: Cell<usize>,
}
impl AssistantJournalStateReader for State {
    fn load_part_head(&self, id: &str) -> Result<Option<JournalPartHead>> {
        self.loads.set(self.loads.get() + 1);
        if self.retired.contains(id) {
            return Err(OperationError::RetiredPart);
        }
        Ok(self.heads.get(id).cloned())
    }
    fn verify_chunk_reference(&self, c: &JournalChunkRef) -> Result<()> {
        if self.chunks.contains(c) {
            Ok(())
        } else {
            Err(OperationError::ManifestUnavailable)
        }
    }
    fn verify_content_reference(
        &self,
        owner: &AssistantJournalIdentity,
        id: &str,
        c: &JournalContentRef,
    ) -> Result<()> {
        if owner != &identity() {
            return Err(OperationError::OwnerMismatch);
        }
        if c.chunk_count.0 == 0
            && *c == empty_journal_content(owner, id, c.field, c.content_generation)?
            || self.manifests.contains(c)
        {
            Ok(())
        } else {
            Err(OperationError::ManifestUnavailable)
        }
    }
}

#[test]
fn same_part_operations_are_validated_in_order_with_one_keyed_read() {
    let initial = head("text");
    let c = chunk("你好🙂");
    let mut replaced = head("text");
    replaced.generation = JournalNumber(2);
    replaced.revision = JournalNumber(4);
    replaced.bodies = vec![empty_journal_content(
        &identity(),
        "text",
        JournalContentField::Markdown,
        JournalNumber(4),
    )
    .unwrap()];
    let state = State {
        chunks: vec![c.clone()],
        ..Default::default()
    };
    let value = batch(vec![
        declare(initial.clone()),
        append(&initial, c),
        AssistantJournalOperationV1::Update(UpdateJournalMetadata {
            part_id: "text".into(),
            expected_generation: JournalNumber(1),
            expected_revision: JournalNumber(2),
            metadata: JournalPartMetadata::Text(JournalTextMetadata {
                is_final: true,
                visibility: JournalVisibility::Public,
                review: None,
            }),
        }),
        AssistantJournalOperationV1::Replace(ReplaceJournalPart {
            part_id: "text".into(),
            expected_generation: JournalNumber(1),
            expected_revision: JournalNumber(3),
            part: replaced.clone(),
        }),
        AssistantJournalOperationV1::Remove(RemoveJournalPart {
            part_id: "text".into(),
            expected_generation: JournalNumber(2),
            expected_revision: JournalNumber(4),
        }),
    ]);
    let changed = validate_assistant_batch(&state, &value).unwrap();
    assert_eq!(state.loads.get(), 1);
    assert_eq!(changed.len(), 1);
    assert_eq!(changed[0].expected, None);
    assert_eq!(changed[0].replacement, None);
    assert_eq!(changed[0].removed_head, Some(replaced));
    assert!(
        state.heads.is_empty(),
        "planning cannot publish partial content"
    );
}

#[test]
fn field_replace_delete_and_readd_use_monotonic_head_clock() {
    let initial = head("text");
    let replacement = empty_journal_content(
        &identity(),
        "text",
        JournalContentField::Markdown,
        JournalNumber(4),
    )
    .unwrap();
    let operation = |revision, expected, resulting| {
        AssistantJournalOperationV1::ReplaceContent(ReplaceJournalContent {
            part_id: "text".into(),
            expected_generation: JournalNumber(1),
            expected_revision: JournalNumber(revision),
            field: JournalContentField::Markdown,
            expected,
            resulting,
        })
    };
    let recreated = empty_journal_content(
        &identity(),
        "text",
        JournalContentField::Markdown,
        JournalNumber(3),
    )
    .unwrap();
    let operations = vec![
        declare(initial.clone()),
        operation(1, Some(initial.bodies[0].clone()), None),
        operation(2, None, Some(recreated.clone())),
        operation(3, Some(recreated), Some(replacement.clone())),
    ];
    let value = batch(operations);
    assert_eq!(
        AssistantOperationBatchV1::decode_canonical(&value.canonical_bytes().unwrap()).unwrap(),
        value
    );
    let changes = validate_assistant_batch(&State::default(), &value).unwrap();
    let result = changes[0].replacement.as_ref().unwrap();
    assert_eq!(result.generation, JournalNumber(1));
    assert_eq!(result.revision, JournalNumber(4));
    assert_eq!(result.bodies, vec![replacement]);
    let stale = batch(vec![
        declare(initial.clone()),
        operation(1, Some(initial.bodies[0].clone()), None),
        operation(2, None, Some(initial.bodies[0].clone())),
    ]);
    assert!(
        validate_assistant_batch(&State::default(), &stale).is_err(),
        "deleted content cannot reuse an old generation even if its manifest exists"
    );
}

#[test]
fn whole_part_replacement_preserves_only_exact_existing_field_references() {
    let initial = head("text");
    let mut replaced = initial.clone();
    replaced.generation = JournalNumber(2);
    replaced.revision = JournalNumber(2);
    let replace = |part| {
        AssistantJournalOperationV1::Replace(ReplaceJournalPart {
            part_id: "text".into(),
            expected_generation: JournalNumber(1),
            expected_revision: JournalNumber(1),
            part,
        })
    };
    let good = batch(vec![declare(initial.clone()), replace(replaced.clone())]);
    let changes = validate_assistant_batch(&State::default(), &good).unwrap();
    assert_eq!(
        changes[0].replacement.as_ref().unwrap().bodies,
        initial.bodies
    );
    let foreign_chunk = chunk("different");
    replaced.bodies[0] =
        append_journal_content(&initial.bodies[0], std::slice::from_ref(&foreign_chunk)).unwrap();
    let state = State {
        manifests: replaced.bodies.clone(),
        ..Default::default()
    };
    assert!(
        matches!(
            validate_assistant_batch(&state, &batch(vec![declare(initial), replace(replaced)])),
            Err(OperationError::GenerationConflict)
        ),
        "a changed root is not an exact retained field"
    );
}

#[test]
fn replacement_cannot_move_an_existing_part_between_pages_or_sections() {
    let initial = head("stable-part");
    for section_change in [false, true] {
        let mut replacement = initial.clone();
        replacement.generation = JournalNumber(2);
        replacement.revision = JournalNumber(2);
        if section_change {
            replacement.order_key.section = JournalSection::Tasks;
        } else {
            replacement.order_key.position = JournalNumber(9);
        }
        let operation = AssistantJournalOperationV1::Replace(ReplaceJournalPart {
            part_id: initial.part_id.clone(), expected_generation: initial.generation,
            expected_revision: initial.revision, part: replacement,
        });
        // Both first publication and later replay use the same invariant.
        assert!(matches!(validate_assistant_batch(&State::default(),
            &batch(vec![declare(initial.clone()), operation.clone()])),
            Err(OperationError::GenerationConflict)));
        let mut state = State::default();
        state.heads.insert(initial.part_id.clone(), initial.clone());
        assert!(matches!(validate_assistant_batch(&state, &batch(vec![operation])),
            Err(OperationError::GenerationConflict)));
    }
}

#[test]
fn field_generation_can_differ_from_part_generation_and_still_append() {
    let initial = head("text");
    let field = empty_journal_content(
        &identity(),
        "text",
        JournalContentField::Markdown,
        JournalNumber(2),
    )
    .unwrap();
    let replaced = AssistantJournalOperationV1::ReplaceContent(ReplaceJournalContent {
        part_id: "text".into(),
        expected_generation: JournalNumber(1),
        expected_revision: JournalNumber(1),
        field: JournalContentField::Markdown,
        expected: Some(initial.bodies[0].clone()),
        resulting: Some(field.clone()),
    });
    let mut next = initial.clone();
    next.bodies = vec![field];
    next.revision = JournalNumber(2);
    let c = chunk("ok");
    let state = State {
        chunks: vec![c.clone()],
        ..Default::default()
    };
    let changes = validate_assistant_batch(
        &state,
        &batch(vec![declare(initial), replaced, append(&next, c)]),
    )
    .unwrap();
    let result = changes[0].replacement.as_ref().unwrap();
    assert_eq!(result.generation.0, 1);
    assert_eq!(result.bodies[0].content_generation.0, 2);
    assert_eq!(result.revision.0, 3);
}

#[test]
fn field_replacement_requires_exact_head_and_manifest_proof() {
    let initial = head("text");
    let replacement = append_journal_content(
        &empty_journal_content(
            &identity(),
            "text",
            JournalContentField::Markdown,
            JournalNumber(2),
        )
        .unwrap(),
        &[chunk("new")],
    )
    .unwrap();
    let make = |expected_revision, expected| {
        AssistantJournalOperationV1::ReplaceContent(ReplaceJournalContent {
            part_id: "text".into(),
            expected_generation: JournalNumber(1),
            expected_revision: JournalNumber(expected_revision),
            field: JournalContentField::Markdown,
            expected,
            resulting: Some(replacement.clone()),
        })
    };
    assert!(matches!(
        validate_assistant_batch(
            &State::default(),
            &batch(vec![
                declare(initial.clone()),
                make(1, Some(initial.bodies[0].clone()))
            ])
        ),
        Err(OperationError::ManifestUnavailable)
    ));
    let state = State {
        manifests: vec![replacement.clone()],
        ..Default::default()
    };
    assert!(matches!(
        validate_assistant_batch(
            &state,
            &batch(vec![declare(initial.clone()), make(1, None)])
        ),
        Err(OperationError::ContentConflict)
    ));
    assert!(validate_assistant_batch(
        &state,
        &batch(vec![
            declare(initial.clone()),
            make(2, Some(initial.bodies[0].clone()))
        ])
    )
    .is_err());
}

#[test]
fn append_checks_chunk_proof_lengths_roots_generations_and_revision() {
    let h = head("text");
    let c = chunk("x");
    let state = State {
        heads: [("text".into(), h.clone())].into(),
        chunks: vec![c.clone()],
        ..Default::default()
    };
    let valid = append(&h, c);
    let change = validate_assistant_batch(&state, &batch(vec![valid.clone()])).unwrap();
    assert_eq!(
        change[0].replacement.as_ref().unwrap().bodies[0].utf8_bytes,
        JournalNumber(1)
    );
    for kind in 0..5 {
        let mut bad = valid.clone();
        let AssistantJournalOperationV1::Append(ref mut a) = bad else {
            unreachable!()
        };
        match kind {
            0 => a.expected_revision = JournalNumber(2),
            1 => a.expected_generation = JournalNumber(2),
            2 => a.resulting.utf8_bytes = JournalNumber(2),
            3 => a.expected.content_root = chunk("other").digest,
            _ => a.chunks[0].utf8_bytes = JournalNumber(0),
        };
        assert!(validate_assistant_batch(&state, &batch(vec![bad])).is_err());
    }
    let missing = State {
        heads: state.heads.clone(),
        ..Default::default()
    };
    assert_eq!(
        validate_assistant_batch(&missing, &batch(vec![valid])),
        Err(OperationError::ManifestUnavailable)
    );
}

#[test]
fn local_overlay_never_reads_unaffected_assistant_parts() {
    let mut state = State::default();
    for i in 0..10_000 {
        let h = head(&format!("part-{i}"));
        state.heads.insert(h.part_id.clone(), h);
    }
    let h = state.heads["part-42"].clone();
    let c = chunk("a");
    state.chunks.push(c.clone());
    let changes = validate_assistant_batch(&state, &batch(vec![append(&h, c)])).unwrap();
    assert_eq!(state.loads.get(), 1);
    assert_eq!(changes.len(), 1);
    assert_eq!(changes[0].part_id, "part-42");
}

#[test]
fn owner_mismatch_and_deleted_ids_cannot_reenter_as_fresh_parts() {
    let mut wrong = head("text");
    wrong.identity.run_id = "foreign".into();
    assert!(validate_assistant_batch(&State::default(), &batch(vec![declare(wrong)])).is_err());
    let state = State {
        retired: ["text".into()].into(),
        ..Default::default()
    };
    assert_eq!(
        validate_assistant_batch(&state, &batch(vec![declare(head("text"))])),
        Err(OperationError::RetiredPart)
    );
    let remove = AssistantJournalOperationV1::Remove(RemoveJournalPart {
        part_id: "text".into(),
        expected_generation: JournalNumber(1),
        expected_revision: JournalNumber(1),
    });
    assert_eq!(
        validate_assistant_batch(
            &State::default(),
            &batch(vec![declare(head("text")), remove, declare(head("text"))])
        ),
        Err(OperationError::RetiredPart)
    );
}

#[test]
fn review_visibility_cannot_hide_or_reveal_a_body_through_metadata_only() {
    let h = head("text");
    let state = State {
        heads: [("text".into(), h.clone())].into(),
        ..Default::default()
    };
    let update = AssistantJournalOperationV1::Update(UpdateJournalMetadata {
        part_id: "text".into(),
        expected_generation: JournalNumber(1),
        expected_revision: JournalNumber(1),
        metadata: JournalPartMetadata::Text(JournalTextMetadata {
            is_final: false,
            visibility: JournalVisibility::Withheld,
            review: Some(TextReviewState::Pending),
        }),
    });
    assert!(validate_assistant_batch(&state, &batch(vec![update])).is_err());
    let mut hidden = h.clone();
    hidden.generation = JournalNumber(2);
    hidden.revision = JournalNumber(2);
    hidden.bodies.clear();
    hidden.metadata = JournalPartMetadata::Text(JournalTextMetadata {
        is_final: false,
        visibility: JournalVisibility::Withheld,
        review: Some(TextReviewState::Pending),
    });
    let replace = AssistantJournalOperationV1::Replace(ReplaceJournalPart {
        part_id: "text".into(),
        expected_generation: JournalNumber(1),
        expected_revision: JournalNumber(1),
        part: hidden,
    });
    assert!(validate_assistant_batch(&state, &batch(vec![replace])).is_ok());
    let mut final_head = h;
    final_head.metadata = JournalPartMetadata::Text(JournalTextMetadata {
        is_final: true,
        visibility: JournalVisibility::Public,
        review: None,
    });
    let c = chunk("late");
    let final_state = State {
        heads: [("text".into(), final_head.clone())].into(),
        chunks: vec![c.clone()],
        ..Default::default()
    };
    assert!(validate_assistant_batch(&final_state, &batch(vec![append(&final_head, c)])).is_err());
}

#[test]
fn typed_batch_seals_header_and_ignores_json_layout() {
    let b = batch(vec![declare(head("text"))]);
    let genesis = AssistantJournalCursor::initial(identity()).unwrap();
    let header = b.seal(1, genesis.record_digest, "op".into(), 1).unwrap();
    b.verify_header(&header).unwrap();
    assert_eq!(
        AssistantJournalRecordHeaderV1::seal_from_batch(
            &b,
            1,
            genesis.record_digest,
            "op".into(),
            1
        )
        .unwrap(),
        header
    );
    header.verify_with_batch(&b).unwrap();
    let wire = serde_json::to_string_pretty(&b).unwrap();
    let decoded: AssistantOperationBatchV1 = serde_json::from_str(&wire).unwrap();
    assert_eq!(
        b.canonical_bytes().unwrap(),
        decoded.canonical_bytes().unwrap()
    );
    decoded.verify_header(&header).unwrap();
    let changed = batch(vec![declare(head("different"))]);
    assert_eq!(
        changed.verify_header(&header),
        Err(OperationError::HeaderMismatch)
    );
    let bytes = b.canonical_bytes().unwrap();
    assert_eq!(
        AssistantOperationBatchV1::decode_canonical(&bytes).unwrap(),
        b
    );
}

#[test]
fn canonical_decoder_rejects_unknown_tags_truncation_and_trailing_data() {
    let bytes = batch(vec![declare(head("text"))])
        .canonical_bytes()
        .unwrap();
    for end in 0..bytes.len() {
        assert!(AssistantOperationBatchV1::decode_canonical(&bytes[..end]).is_err());
    }
    let mut extra = bytes.clone();
    extra.push(0);
    assert!(AssistantOperationBatchV1::decode_canonical(&extra).is_err());
    let mut bad = bytes;
    bad[9] = 255;
    assert_eq!(
        AssistantOperationBatchV1::decode_canonical(&bad),
        Err(OperationError::InvalidVersion)
    );
    assert_eq!(
        AssistantOperationBatchV1::decode_canonical(&vec![0; MAX_OPERATION_BATCH_BYTES + 1]),
        Err(OperationError::Budget)
    );
}

#[test]
fn batches_reject_opaque_json_unknown_fields_and_unbounded_metadata() {
    let b = batch(vec![declare(head("text"))]);
    let mut wire = serde_json::to_value(&b).unwrap();
    wire["operations"][0]["data"]["part"]["metadata"]["data"]["arbitrary_json"] =
        serde_json::json!({"execute":true});
    assert!(serde_json::from_value::<AssistantOperationBatchV1>(wire).is_err());
    assert!(batch(vec![]).canonical_bytes().is_err());
    assert!(batch(vec![declare(head("text")); 129])
        .canonical_bytes()
        .is_err());
    let mut too_long = head("text");
    too_long.metadata = JournalPartMetadata::Source(JournalSourceMetadata {
        source_id: "source".into(),
        title: "x".repeat(MAX_METADATA_STRING_BYTES + 1).into(),
        uri: "https://example.invalid".into(),
        mime_type: None,
        evidence: None,
    });
    too_long.bodies.clear();
    assert!(batch(vec![declare(too_long)]).canonical_bytes().is_err());
}

#[test]
fn content_root_binds_owner_field_generation_and_order() {
    let a = empty_journal_content(
        &identity(),
        "a",
        JournalContentField::Markdown,
        JournalNumber(1),
    )
    .unwrap();
    let b = empty_journal_content(
        &identity(),
        "b",
        JournalContentField::Markdown,
        JournalNumber(1),
    )
    .unwrap();
    assert_ne!(a.content_root, b.content_root);
    assert_ne!(
        a.content_root,
        empty_journal_content(
            &identity(),
            "a",
            JournalContentField::Markdown,
            JournalNumber(2)
        )
        .unwrap()
        .content_root
    );
    let x = chunk("x");
    let y = chunk("y");
    assert_ne!(
        append_journal_content(&a, &[x.clone(), y.clone()])
            .unwrap()
            .content_root,
        append_journal_content(&a, &[y, x]).unwrap().content_root
    );
}

#[test]
fn all_metadata_variants_have_canonical_typed_roundtrips() {
    let values = vec![
        JournalPartMetadata::UiSurface(JournalUiSurfaceMetadata {
            surface: crate::generative_ui::UiSurfaceRevisionRef {
                identity: crate::generative_ui::UiSurfaceIdentity {
                    owner: crate::generative_ui::UiSurfaceOwner {
                        authority_id: "epoch".into(), conversation_id: "conversation".into(), participant_id: "assistant".into(),
                        context_id: "conversation".into(), originating_run_id: "run".into(), message_id: "assistant".into(), part_id: "tool".into(),
                    }, surface_id: "form".into(), generation: 1,
                }, revision: 3,
            }, summary: "确认表单".into(),
        }),
        JournalPartMetadata::Reasoning(JournalReasoningMetadata {
            title: "Reasoning".into(),
            state: ActivityState::Running,
        }),
        JournalPartMetadata::Step(JournalStepMetadata {
            title: "执行步骤".into(),
            state: ActivityState::Running,
        }),
        JournalPartMetadata::Tool(JournalToolMetadata {
            execution_source: Default::default(),
            tool_call_id: "call".into(),
            capability_id: "read".into(),
            title: "Read".into(),
            summary: Some("".into()),
            origin: JournalToolOrigin {
                kind: HostCapabilityOriginKind::Builtin,
                provider_id: None,
                provider_title: None,
                namespace: None,
            },
            permission: None,
            state: ActivityState::WaitingForPermission,
            display_key: None,
            affected_resources: vec![].into(),
            artifacts: JournalContentField::ToolArtifacts,
            full_output: None,
            started_at_ms: Some(JournalNumber(u64::MAX)),
            finished_at_ms: None,
        }),
        JournalPartMetadata::Source(JournalSourceMetadata {
            source_id: "source".into(),
            title: "标题".into(),
            uri: "https://example.invalid".into(),
            mime_type: None,
            evidence: None,
        }),
        JournalPartMetadata::Citation(JournalCitationMetadata {
            citation_id: "citation-real-id".into(),
            source_id: "source".into(),
            title: "标题".into(),
            uri: "https://example.invalid".into(),
            mime_type: Some("application/pdf".into()),
            origin_tool_call_id: "call".into(),
            location: Some(JournalCitationLocation {
                page: Some(JournalNumber(1)),
                line_start: None,
                line_end: None,
                section: Some("Section".into()),
            }),
            byte_range: None,
        }),
        JournalPartMetadata::Attachment(JournalAttachmentMetadata {
            attachment_id: "attachment".into(),
            name: "file".into(),
            mime_type: None,
            uri: "file-ref".into(),
            operation: None,
        }),
        JournalPartMetadata::Task(JournalTaskMetadata {
            task_id: "task".into(),
            display_key: "task.background".into(),
            state: ActivityState::Running,
            progress: Some(JournalProgress::from_fraction(0.5).unwrap()),
            cancel_requested: false,
        }),
        JournalPartMetadata::TaskControl(JournalTaskControlMetadata {
            task: JournalTaskMetadata { task_id: "controlled-task".into(), display_key: "task.background".into(),
                state: ActivityState::Running, progress: None, cancel_requested: false },
            supports_guidance: true, supports_cancellation: true,
        }),
        JournalPartMetadata::Skill(JournalSkillMetadata {
            run_id: "skill-run".into(),
            skill_id: "skill".into(),
            display_name: "Skill".into(),
            mode: JournalSkillMode::Inline,
            state: ActivityState::Completed,
            model_turns: JournalNumber(u64::MAX),
            tool_calls: JournalNumber(2),
            elapsed_millis: JournalNumber(10),
        }),
        JournalPartMetadata::ModelAttempt(JournalModelAttemptMetadata {
            attempt_id: "attempt".into(),
            state: ActivityState::Failed,
            provider: None,
            model: None,
            failure_code: "timeout".into(),
            retryable: true,
            http_status: Some(JournalNumber(503)),
            fallback_attempt_id: None,
            fallback_provider: None,
            fallback_model: None,
        }),
        JournalPartMetadata::Error(JournalErrorMetadata {
            code: "error".into(),
            display_key: "error.generic".into(),
            recoverable: true,
        }),
        JournalPartMetadata::Historical(JournalHistoricalMetadata {
            schema: JournalHistoricalSchema::AgentPart,
            canonical_kind: "tool".into(),
            canonical_revision: JournalNumber(2),
        }),
    ];
    for metadata in values {
        let mut h = head("node");
        h.bodies.clear();
        h.metadata = metadata;
        if matches!(h.metadata, JournalPartMetadata::Tool(_)) {
            h.bodies.push(
                empty_journal_content(
                    &identity(),
                    "node",
                    JournalContentField::ToolArtifacts,
                    JournalNumber(1),
                )
                .unwrap(),
            );
        }
        if matches!(h.metadata, JournalPartMetadata::ModelAttempt(_)) {
            h.bodies.push(
                empty_journal_content(
                    &identity(),
                    "node",
                    JournalContentField::FailureMessage,
                    JournalNumber(1),
                )
                .unwrap(),
            );
        }
        let b = batch(vec![declare(h)]);
        let bytes = b.canonical_bytes().unwrap();
        assert_eq!(
            AssistantOperationBatchV1::decode_canonical(&bytes).unwrap(),
            b
        );
        assert!(validate_assistant_batch(&State::default(), &b).is_ok());
    }
}

#[test]
fn invalid_progress_and_nondecimal_counter_wire_are_rejected() {
    for fraction in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY, -0.1, 1.1] {
        assert_eq!(
            JournalProgress::from_fraction(fraction),
            Err(OperationError::InvalidMetadata)
        );
        let wire = serde_json::json!({"fraction_bits":fraction.to_bits().to_string()});
        assert!(serde_json::from_value::<JournalProgress>(wire).is_err());
        let bytes = fraction.to_bits().to_be_bytes();
        assert!(JournalProgress::read(&mut Reader {
            bytes: &bytes,
            offset: 0
        })
        .is_err());
    }
    for wire in [
        serde_json::json!({}),
        serde_json::json!({"fraction_bits":null}),
        serde_json::json!({"fraction_bits":0}),
        serde_json::json!({"fraction_bits":"00"}),
        serde_json::json!({"fraction_bits":"0", "extra":true}),
        serde_json::json!({"numerator":"1", "denominator":"2"}),
    ] {
        assert!(serde_json::from_value::<JournalProgress>(wire).is_err());
    }
    for n in [
        serde_json::json!(1),
        serde_json::json!("01"),
        serde_json::json!("+1"),
        serde_json::json!("1.0"),
        serde_json::json!("18446744073709551616"),
    ] {
        assert!(serde_json::from_value::<JournalNumber>(n).is_err());
    }
    assert_eq!(
        serde_json::to_value(JournalNumber(u64::MAX)).unwrap(),
        "18446744073709551615"
    );
}

#[test]
fn progress_roundtrips_exact_binary64_including_subnormals_and_signed_zero() {
    let edge_bits = [
        0,
        1,
        2,
        f64::MIN_POSITIVE.to_bits() - 1,
        f64::MIN_POSITIVE.to_bits(),
        0.1_f64.to_bits(),
        0.5_f64.to_bits(),
        1.0_f64.to_bits() - 1,
        1.0_f64.to_bits(),
        (-0.0_f64).to_bits(),
    ];
    let mut seed = 0x92112345_u64;
    let samples = (0..1024).map(|_| {
        seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1);
        seed % (1.0_f64.to_bits() + 1)
    });
    for bits in edge_bits.into_iter().chain(samples) {
        let progress = JournalProgress::from_fraction(f64::from_bits(bits)).unwrap();
        assert_eq!(progress.fraction().to_bits(), bits);
        let wire = serde_json::to_value(progress).unwrap();
        assert_eq!(wire, serde_json::json!({"fraction_bits":bits.to_string()}));
        let decoded: JournalProgress = serde_json::from_value(wire).unwrap();
        assert_eq!(decoded.fraction().to_bits(), bits);
        let mut h = head("task");
        h.bodies.clear();
        h.metadata = JournalPartMetadata::Task(JournalTaskMetadata {
            task_id: "task".into(),
            display_key: "task.background".into(),
            state: ActivityState::Running,
            progress: Some(progress),
            cancel_requested: false,
        });
        let b = batch(vec![declare(h)]);
        let bytes = b.canonical_bytes().unwrap();
        assert_eq!(
            AssistantOperationBatchV1::decode_canonical(&bytes).unwrap(),
            b
        );
    }
}

#[test]
fn metadata_field_references_are_typed_owned_and_present() {
    let mut h = head("source");
    h.metadata = JournalPartMetadata::Source(JournalSourceMetadata {
        source_id: "s".into(),
        title: JournalTextValue::Field(JournalContentField::Title),
        uri: "https://example.invalid".into(),
        mime_type: None,
        evidence: None,
    });
    h.bodies.clear();
    assert!(validate_assistant_batch(&State::default(), &batch(vec![declare(h.clone())])).is_err());
    h.bodies.push(
        empty_journal_content(
            &identity(),
            "source",
            JournalContentField::Title,
            JournalNumber(1),
        )
        .unwrap(),
    );
    assert!(validate_assistant_batch(&State::default(), &batch(vec![declare(h.clone())])).is_ok());
    if let JournalPartMetadata::Source(m) = &mut h.metadata {
        m.title = JournalTextValue::Field(JournalContentField::Uri);
    }
    assert!(batch(vec![declare(h)]).canonical_bytes().is_err());
}

#[test]
fn citation_independent_line_bounds_are_preserved() {
    for (start, end) in [
        (Some(JournalNumber(3)), None),
        (None, Some(JournalNumber(4))),
    ] {
        let mut h = head("citation");
        h.bodies.clear();
        h.metadata = JournalPartMetadata::Citation(JournalCitationMetadata {
            citation_id: "citation-real-id".into(),
            source_id: "s".into(),
            title: "title".into(),
            uri: "uri".into(),
            mime_type: None,
            origin_tool_call_id: "call".into(),
            location: Some(JournalCitationLocation {
                page: None,
                line_start: start,
                line_end: end,
                section: None,
            }),
            byte_range: None,
        });
        let value = batch(vec![declare(h)]);
        assert_eq!(
            AssistantOperationBatchV1::decode_canonical(&value.canonical_bytes().unwrap()).unwrap(),
            value
        );
        assert!(validate_assistant_batch(&State::default(), &value).is_ok());
    }
}

#[test]
fn tool_output_owner_and_readonly_permission_origin_are_strict() {
    let mut h = head("tool");
    h.metadata = JournalPartMetadata::Tool(JournalToolMetadata {
        execution_source: Default::default(),
        tool_call_id: "call".into(),
        capability_id: "read".into(),
        title: "Read".into(),
        summary: Some("".into()),
        origin: JournalToolOrigin {
            kind: HostCapabilityOriginKind::Mcp,
            provider_id: Some("provider".into()),
            provider_title: Some("".into()),
            namespace: None,
        },
        permission: Some(JournalToolPermission {
            request_id: "permission".into(),
            permission_domain: "files".into(),
            rationale: "Read only".into(),
            decision: Some("".into()),
        }),
        state: ActivityState::Running,
        display_key: None,
        affected_resources: vec![].into(),
        artifacts: JournalContentField::ToolArtifacts,
        full_output: Some(ToolOutputReference {
            message_id: MessageId("assistant".into()),
            part_id: MessagePartId("tool".into()),
            tool_call_id: ToolCallId("call".into()),
        }),
        started_at_ms: None,
        finished_at_ms: None,
    });
    h.bodies = vec![empty_journal_content(
        &identity(),
        "tool",
        JournalContentField::ToolArtifacts,
        JournalNumber(1),
    )
    .unwrap()];
    assert!(validate_assistant_batch(&State::default(), &batch(vec![declare(h.clone())])).is_ok());
    for index in 0..3 {
        let mut wrong = h.clone();
        let JournalPartMetadata::Tool(metadata) = &mut wrong.metadata else {
            unreachable!()
        };
        let output = metadata.full_output.as_mut().unwrap();
        match index {
            0 => output.message_id.0 = "foreign".into(),
            1 => output.part_id.0 = "foreign".into(),
            _ => output.tool_call_id.0 = "foreign".into(),
        }
        assert_eq!(
            batch(vec![declare(wrong)]).canonical_bytes(),
            Err(OperationError::OwnerMismatch)
        );
    }
    let value = batch(vec![declare(h)]);
    let original = serde_json::to_value(&value).unwrap();
    let mut missing_source = original.clone();
    missing_source["operations"][0]["data"]["part"]["metadata"]["data"]
        .as_object_mut().unwrap().remove("execution_source");
    assert!(serde_json::from_value::<AssistantOperationBatchV1>(missing_source).is_err());
    let mut nested_source = original.clone();
    nested_source["operations"][0]["data"]["part"]["metadata"]["data"]["execution_source"] =
        serde_json::json!({"kind":"code_cell","cell":{
            "host_id":"host", "conversation_id":"core-session", "turn_id":"turn",
            "parent_call_id":"parent", "cell_id":"cell", "generation":"generation"
        }});
    let nested: AssistantOperationBatchV1 = serde_json::from_value(nested_source.clone()).unwrap();
    assert!(nested.canonical_bytes().is_ok());
    nested_source["operations"][0]["data"]["part"]["metadata"]["data"]["execution_source"]["cell"]["generation"] = "".into();
    let invalid: AssistantOperationBatchV1 = serde_json::from_value(nested_source).unwrap();
    assert_eq!(invalid.canonical_bytes(), Err(OperationError::InvalidContent));
    for location in ["permission", "origin", "full_output"] {
        for key in ["grant", "lease", "token", "path"] {
            let mut wire = original.clone();
            wire["operations"][0]["data"]["part"]["metadata"]["data"][location][key] =
                "forged".into();
            assert!(serde_json::from_value::<AssistantOperationBatchV1>(wire).is_err());
        }
    }
}

#[test]
fn withheld_review_status_is_independent_but_cannot_publish_any_body() {
    for review in [
        TextReviewState::Pending,
        TextReviewState::Rejected,
        TextReviewState::Interrupted,
    ] {
        let mut h = head("text");
        h.metadata = JournalPartMetadata::Text(JournalTextMetadata {
            is_final: true,
            visibility: JournalVisibility::Withheld,
            review: Some(review),
        });
        assert!(batch(vec![declare(h.clone())]).canonical_bytes().is_err());
        h.bodies.clear();
        let value = batch(vec![declare(h.clone())]);
        let bytes = value.canonical_bytes().unwrap();
        assert_eq!(
            AssistantOperationBatchV1::decode_canonical(&bytes).unwrap(),
            value
        );
        assert!(validate_assistant_batch(&State::default(), &value).is_ok());
        let JournalPartMetadata::Text(metadata) = &mut h.metadata else {
            unreachable!()
        };
        metadata.visibility = JournalVisibility::Public;
        assert!(batch(vec![declare(h)]).canonical_bytes().is_err());
    }
}

#[test]
fn citation_schema_and_ordinary_projection_share_range_and_opaque_location_uri_domain() {
    use crate::{CitationRange, MessagePartView, PublicPartContractError, SourceLocation};
    let locations = [
        None,
        Some(SourceLocation::default()),
        Some(SourceLocation {
            page: Some(u64::MAX),
            line_start: None,
            line_end: Some(0),
            section: Some(String::new()),
        }),
        Some(SourceLocation {
            page: None,
            line_start: Some(9),
            line_end: Some(2),
            section: None,
        }),
    ];
    for uri in ["", " \n\t", "opaque source fact"] {
        for location in &locations {
            for range in [
                None,
                Some(CitationRange {
                    start: 0,
                    end: u64::MAX,
                }),
                Some(CitationRange { start: 4, end: 4 }),
                Some(CitationRange { start: 9, end: 2 }),
            ] {
                let ordinary = MessagePartView::Citation {
                    part_id: crate::MessagePartId("citation".into()),
                    revision: 1,
                    citation_id: "real-citation".into(),
                    source_id: "source".into(),
                    title: String::new(),
                    uri: uri.into(),
                    mime_type: Some(String::new()),
                    origin_tool_call_id: "call".into(),
                    page: location.as_ref().and_then(|value| value.page),
                    line_start: location.as_ref().and_then(|value| value.line_start),
                    line_end: location.as_ref().and_then(|value| value.line_end),
                    section: location.as_ref().and_then(|value| value.section.clone()),
                    location: location.clone(),
                    range_start: range.as_ref().map(|value| value.start),
                    range_end: range.as_ref().map(|value| value.end),
                };
                let mut journal = head("citation");
                journal.bodies.clear();
                journal.metadata = JournalPartMetadata::Citation(JournalCitationMetadata {
                    citation_id: "real-citation".into(),
                    source_id: "source".into(),
                    title: String::new().into(),
                    uri: uri.into(),
                    mime_type: Some(String::new().into()),
                    origin_tool_call_id: "call".into(),
                    location: location.as_ref().map(|value| JournalCitationLocation {
                        page: value.page.map(JournalNumber),
                        line_start: value.line_start.map(JournalNumber),
                        line_end: value.line_end.map(JournalNumber),
                        section: value.section.clone().map(JournalTextValue::Inline),
                    }),
                    byte_range: range.as_ref().map(|value| JournalRange {
                        start: JournalNumber(value.start),
                        end: JournalNumber(value.end),
                    }),
                });
                let batch = batch(vec![declare(journal)]);
                let valid = !range.as_ref().is_some_and(|value| value.start > value.end);
                assert_eq!(
                    ordinary
                        .validate_public_semantics(&crate::MessageId("assistant".into()))
                        .is_ok(),
                    valid
                );
                assert_eq!(batch.canonical_bytes().is_ok(), valid);
                assert_eq!(
                    validate_assistant_batch(&State::default(), &batch).is_ok(),
                    valid
                );
                if !valid {
                    assert_eq!(
                        ordinary.validate_public_semantics(&crate::MessageId("assistant".into())),
                        Err(PublicPartContractError::CitationRangeReversed)
                    );
                } else {
                    let wire = batch.canonical_bytes().unwrap();
                    assert_eq!(
                        AssistantOperationBatchV1::decode_canonical(&wire).unwrap(),
                        batch
                    );
                    assert_eq!(
                        serde_json::from_value::<MessagePartView>(
                            serde_json::to_value(&ordinary).unwrap()
                        )
                        .unwrap(),
                        ordinary
                    );
                }
            }
        }
    }
    assert_eq!(
        CitationRange::from_public_bounds(Some(0), None),
        Err(crate::PublicCitationRangeError::Incomplete)
    );
    assert_eq!(
        CitationRange::from_public_bounds(None, Some(0)),
        Err(crate::PublicCitationRangeError::Incomplete)
    );
}

#[test]
fn operation_and_manifest_hashes_match_independent_golden_bytes() {
    let golden: serde_json::Value = serde_json::from_str(include_str!(
        "../tests/fixtures/assistant_operations_v1.json"
    ))
    .unwrap();
    let value = batch(vec![AssistantJournalOperationV1::Remove(
        RemoveJournalPart {
            part_id: "text".into(),
            expected_generation: JournalNumber(1),
            expected_revision: JournalNumber(2),
        },
    )]);
    let bytes = value.canonical_bytes().unwrap();
    assert_eq!(
        bytes.iter().map(|b| format!("{b:02x}")).collect::<String>(),
        golden["remove_batch_hex"]
    );
    assert_eq!(
        value.digest().unwrap().to_hex(),
        golden["remove_batch_digest"]
    );
    let empty = empty_journal_content(
        &identity(),
        "text",
        JournalContentField::Markdown,
        JournalNumber(1),
    )
    .unwrap();
    assert_eq!(empty.content_root.to_hex(), golden["empty_content_root"]);
    let delete = batch(vec![AssistantJournalOperationV1::ReplaceContent(
        ReplaceJournalContent {
            part_id: "text".into(),
            expected_generation: JournalNumber(1),
            expected_revision: JournalNumber(1),
            field: JournalContentField::Markdown,
            expected: Some(empty.clone()),
            resulting: None,
        },
    )]);
    assert_eq!(
        delete
            .canonical_bytes()
            .unwrap()
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect::<String>(),
        golden["replace_content_delete_hex"]
    );
    assert_eq!(
        delete.digest().unwrap().to_hex(),
        golden["replace_content_delete_digest"]
    );
    assert_eq!(
        append_journal_content(&empty, &[chunk("x")])
            .unwrap()
            .content_root
            .to_hex(),
        golden["appended_x_root"]
    );
}

#[test]
fn field_operation_matches_independent_javascript_golden() {
    let golden: serde_json::Value = serde_json::from_str(include_str!(
        "../tests/fixtures/assistant_field_operations_v1.json"
    ))
    .unwrap();
    let empty = empty_journal_content(
        &identity(),
        "tool",
        JournalContentField::ToolOutput,
        JournalNumber(2),
    )
    .unwrap();
    assert_eq!(
        empty.content_root.to_hex(),
        golden["empty_tool_output_generation_2_root"]
    );
    let value = batch(vec![AssistantJournalOperationV1::ReplaceContent(
        ReplaceJournalContent {
            part_id: "tool".into(),
            expected_generation: JournalNumber(1),
            expected_revision: JournalNumber(3),
            field: JournalContentField::ToolOutput,
            expected: Some(empty),
            resulting: None,
        },
    )]);
    assert_eq!(
        value
            .canonical_bytes()
            .unwrap()
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect::<String>(),
        golden["remove_tool_output_batch_hex"]
    );
    assert_eq!(
        value.digest().unwrap().to_hex(),
        golden["remove_tool_output_batch_digest"]
    );
}

#[test]
fn surface_metadata_rejects_foreign_message_and_revision_regression() {
    use crate::generative_ui::{UiSurfaceIdentity, UiSurfaceOwner, UiSurfaceRevisionRef};
    let metadata = JournalUiSurfaceMetadata {
        surface: UiSurfaceRevisionRef { identity: UiSurfaceIdentity { owner: UiSurfaceOwner {
            authority_id: "epoch".into(), conversation_id: "conversation".into(), participant_id: "assistant".into(),
            context_id: "conversation".into(), originating_run_id: "run".into(), message_id: "assistant".into(), part_id: "tool".into(),
        }, surface_id: "form".into(), generation: 1 }, revision: 2 }, summary: "表单".into(),
    };
    let mut initial = head("surface");
    initial.bodies.clear();
    initial.metadata = JournalPartMetadata::UiSurface(metadata.clone());
    assert!(batch(vec![declare(initial.clone())]).canonical_bytes().is_ok());
    let mut foreign = initial.clone();
    let JournalPartMetadata::UiSurface(value) = &mut foreign.metadata else { unreachable!() };
    value.surface.identity.owner.message_id = "another-assistant".into();
    assert!(batch(vec![declare(foreign)]).canonical_bytes().is_err());
    let mut state = State::default();
    state.heads.insert(initial.part_id.clone(), initial.clone());
    for revision in [1, 2] {
        let mut stale = metadata.clone();
        stale.surface.revision = revision;
        let update = AssistantJournalOperationV1::Update(UpdateJournalMetadata {
            part_id: initial.part_id.clone(), expected_generation: initial.generation,
            expected_revision: initial.revision, metadata: JournalPartMetadata::UiSurface(stale),
        });
        assert!(validate_assistant_batch(&state, &batch(vec![update])).is_err());
    }
    let mut next = metadata;
    next.surface.revision = 3;
    let update = AssistantJournalOperationV1::Update(UpdateJournalMetadata {
        part_id: initial.part_id.clone(), expected_generation: initial.generation,
        expected_revision: initial.revision, metadata: JournalPartMetadata::UiSurface(next),
    });
    assert!(validate_assistant_batch(&state, &batch(vec![update])).is_ok());
}
