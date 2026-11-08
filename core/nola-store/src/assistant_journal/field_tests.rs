// SPDX-License-Identifier: Apache-2.0
//! Real SQLite proof that output replacement does not replace input manifests.
use super::*;

fn tool_with_input(store: &mut NolaStore, id: &AssistantJournalIdentity) -> JournalPartHead {
    store.begin_write_unit_of_work().unwrap();
    let transition_id = transition(store, id, 2);
    let mut input =
        empty_journal_content(id, "tool", JournalContentField::ToolInput, JournalNumber(1))
            .unwrap();
    // 2 MiB input, prepared in bounded calls. Logical manifest entries, not just
    // deduplicated chunk bytes, let the test detect accidental input rewrites.
    let chunks = vec![AssistantTextChunkV1::new("x".repeat(65536)).unwrap(); 4];
    for _ in 0..8 {
        input = store
            .prepare_assistant_content(id, "tool", &input, &chunks)
            .unwrap();
    }
    let artifacts = store
        .prepare_assistant_content(
            id,
            "tool",
            &empty_journal_content(
                id,
                "tool",
                JournalContentField::ToolArtifacts,
                JournalNumber(1),
            )
            .unwrap(),
            &[AssistantTextChunkV1::new(
                nola_domain::assistant_journal_content::encode_assistant_artifact_content(&[])
                    .unwrap(),
            )
            .unwrap()],
        )
        .unwrap();
    let part = JournalPartHead {
        identity: id.clone(),
        part_id: "tool".into(),
        generation: JournalNumber(1),
        revision: JournalNumber(1),
        metadata_revision: JournalNumber(1),
        order_key: JournalOrderKey {
            section: JournalSection::Transcript,
            position: JournalNumber(1),
        },
        metadata: JournalPartMetadata::Tool(JournalToolMetadata {
            execution_source: Default::default(),
            tool_call_id: "call".into(),
            capability_id: "test".into(),
            title: "Tool progress".into(),
            summary: None,
            origin: JournalToolOrigin {
                kind: nola_domain::HostCapabilityOriginKind::Unknown,
                provider_id: None,
                provider_title: None,
                namespace: None,
            },
            permission: None,
            state: nola_domain::ActivityState::Running,
            display_key: None,
            affected_resources: vec![].into(),
            artifacts: JournalContentField::ToolArtifacts,
            full_output: None,
            started_at_ms: None,
            finished_at_ms: None,
        }),
        bodies: vec![input, artifacts],
    };
    let expected = store.assistant_journal_head(id).unwrap();
    store
        .commit_assistant_journal(&CommitAssistantJournal {
            stamp: AssistantSourceCommitStamp::Transition { transition_id },
            expected,
            batch: AssistantOperationBatchV1 {
                protocol_version: 1,
                identity: id.clone(),
                operations: vec![AssistantJournalOperationV1::Declare(DeclareJournalPart {
                    part: part.clone(),
                })],
            },
            supplied_chunks: vec![],
        })
        .unwrap();
    store.commit_write_unit_of_work().unwrap();
    part
}

#[test]
fn tool_output_replacements_keep_large_input_generation_and_manifest() {
    let directory = tempfile::tempdir().unwrap();
    let mut store = NolaStore::open(directory.path().join("fields.db")).unwrap();
    let id = begin(&mut store, "fields");
    let mut part = tool_with_input(&mut store, &id);
    let original_input = part.bodies[0].clone();
    for revision in 2..=33 {
        store.begin_write_unit_of_work().unwrap();
        let transition_id = transition(&mut store, &id, revision + 1);
        let previous = part
            .bodies
            .iter()
            .find(|body| body.field == JournalContentField::ToolOutput)
            .cloned();
        let empty = empty_journal_content(
            &id,
            "tool",
            JournalContentField::ToolOutput,
            JournalNumber(revision as u64),
        )
        .unwrap();
        let output = store
            .prepare_assistant_content(
                &id,
                "tool",
                &empty,
                &[AssistantTextChunkV1::new(format!("progress {revision}")).unwrap()],
            )
            .unwrap();
        let expected = store.assistant_journal_head(&id).unwrap();
        store
            .commit_assistant_journal(&CommitAssistantJournal {
                stamp: AssistantSourceCommitStamp::Transition { transition_id },
                expected,
                batch: AssistantOperationBatchV1 {
                    protocol_version: 1,
                    identity: id.clone(),
                    operations: vec![AssistantJournalOperationV1::ReplaceContent(
                        ReplaceJournalContent {
                            part_id: "tool".into(),
                            expected_generation: part.generation,
                            expected_revision: part.revision,
                            field: JournalContentField::ToolOutput,
                            expected: previous,
                            resulting: Some(output),
                        },
                    )],
                },
                supplied_chunks: vec![],
            })
            .unwrap();
        store.commit_write_unit_of_work().unwrap();
        part = store.assistant_journal_part(&id, "tool").unwrap().unwrap();
        assert_eq!(part.generation.0, 1);
        assert_eq!(part.revision.0, revision as u64);
        assert_eq!(
            part.bodies
                .iter()
                .find(|body| body.field == JournalContentField::ToolInput),
            Some(&original_input)
        );
    }
    let input_entries: i64 = store.connection.query_row(
        "SELECT COUNT(*) FROM assistant_journal_manifest_chunks WHERE owner=?1 AND part_id='tool' AND field=?2",
        params![owner(&id).unwrap(), content::field(JournalContentField::ToolInput).unwrap()], |row| row.get(0),
    ).unwrap();
    assert_eq!(
        input_entries, 32,
        "output updates must not regenerate input manifests"
    );
    store.begin_write_unit_of_work().unwrap();
    let snapshot = store.open_assistant_journal_snapshot(&id).unwrap();
    store.commit_write_unit_of_work().unwrap();
    assert_eq!(
        store
            .read_assistant_snapshot_chunk(&snapshot, "tool", &original_input, 31)
            .unwrap()
            .byte_length(),
        65536
    );
}
