// SPDX-License-Identifier: Apache-2.0
use super::*;

#[test]
fn record_chunk_replays_intermediate_roots_without_relaxing_snapshot_visibility() {
    let directory = tempfile::tempdir().unwrap();
    let database = directory.path().join("record-content.sqlite");
    let mut store = NolaStore::open(&database).unwrap();
    let id = begin(&mut store, "record-content");
    declare(&mut store, &id);
    let part = format!("{}-journal-part", id.run_id);
    store.begin_write_unit_of_work().unwrap();
    let declaration_snapshot = store.open_assistant_journal_snapshot(&id).unwrap();
    transition(&mut store, &id, 3);
    let mut input = append_input(&store, &id, 3, "第一段🙂");
    let AssistantJournalOperationV1::Append(first) = &input.batch.operations[0] else {panic!()};
    let intermediate = first.resulting.clone();
    let second_chunk = AssistantTextChunkV1::new("第二段".into()).unwrap();
    let references = vec![JournalChunkRef {digest:second_chunk.digest(),utf8_bytes:JournalNumber(second_chunk.byte_length() as u64)}];
    let final_content = append_journal_content(&intermediate, &references).unwrap();
    input.batch.operations.push(AssistantJournalOperationV1::Append(AppendJournalText {
        part_id: part.clone(), expected_generation: first.expected_generation,
        expected_revision: JournalNumber(first.expected_revision.0 + 1),
        expected: intermediate.clone(), chunks: references, resulting: final_content.clone(),
    }));
    input.supplied_chunks.push(second_chunk);
    store.commit_assistant_journal(&input).unwrap();
    let snapshot = store.open_assistant_journal_snapshot(&id).unwrap();
    store.commit_write_unit_of_work().unwrap();
    drop(store);
    let mut store = NolaStore::open(&database).unwrap();
    assert!(store.read_assistant_snapshot_chunk(&snapshot,&part,&intermediate,0).is_err(),
        "intermediate operation roots must stay invisible to ordinary snapshot readers");
    assert_eq!(store.read_assistant_record_chunk(&snapshot,&part,&intermediate,0).unwrap(),input.supplied_chunks[0]);
    assert_eq!(store.read_assistant_record_chunk(&snapshot,&part,&final_content,1).unwrap(),input.supplied_chunks[1]);
    assert!(store.read_assistant_record_chunk(&declaration_snapshot,&part,&intermediate,0).is_err());
    assert!(store.read_assistant_record_chunk(&snapshot,"another-part",&intermediate,0).is_err());
    assert!(store.read_assistant_record_chunk(&snapshot,&part,&intermediate,1).is_err());
    let mut forged=snapshot.clone(); forged.at.sequence+=1;
    assert!(store.read_assistant_record_chunk(&forged,&part,&intermediate,0).is_err());
    let after=AssistantJournalCursor::initial(id.clone()).unwrap();
    let AssistantJournalPage::Records {records,..}=store.read_assistant_journal_page(
        &AssistantJournalReadWindow {after,through:snapshot.at.clone()},128,262144,
    ).unwrap() else {panic!()};
    let downloaded=vec![
        store.read_assistant_record_chunk(&snapshot,&part,&intermediate,0).unwrap(),
        store.read_assistant_record_chunk(&snapshot,&part,&final_content,1).unwrap(),
    ];
    let mut receiver=NolaStore::open(directory.path().join("receiver.sqlite")).unwrap();
    receiver.admit_remote_submission_replica(&nola_store_port::remote_session::AdmittedReplicaTurnInput {
        user_part_kind: "text".into(),
        source_host_id:"source".into(),conversation_id:id.conversation_id.clone(),conversation_title:"Receive".into(),
        user_message_id:"remote-user".into(),user_part_id:"remote-user-body".into(),user_text:"Work".into(),user_content_json:None,
        assistant_message_id:id.assistant_message_id.clone(),assistant_part_id:"remote-placeholder".into(),
        run_id:id.run_id.clone(),context_checkpoint_id:None,created_at_ms:1,
    }).unwrap();
    receiver.begin_write_unit_of_work().unwrap();
    receiver.receive_assistant_journal_record("source",&records[0],&[]).unwrap();
    receiver.receive_assistant_journal_record("source",&records[1],&downloaded).unwrap();
    let received_snapshot=receiver.open_assistant_journal_snapshot(&id).unwrap();
    receiver.commit_write_unit_of_work().unwrap();
    assert_eq!(received_snapshot.at,snapshot.at);
    assert_eq!(receiver.read_assistant_snapshot_range(&received_snapshot,&part,&final_content,0,1024).unwrap().text,"第一段🙂第二段");
    store.begin_write_unit_of_work().unwrap();
    transition(&mut store,&id,4);
    let newer=append_input(&store,&id,4,"后续");
    store.commit_assistant_journal(&newer).unwrap();
    let newer_snapshot=store.open_assistant_journal_snapshot(&id).unwrap();
    store.commit_write_unit_of_work().unwrap();
    assert!(store.read_assistant_record_chunk(&newer_snapshot,&part,&intermediate,0).is_err(),
        "a stored digest is not membership in a different record");
    assert_eq!(store.read_assistant_record_chunk(&snapshot,&part,&intermediate,0).unwrap(),input.supplied_chunks[0]);
    store.begin_write_unit_of_work().unwrap();
    store.release_assistant_journal_snapshot(&snapshot).unwrap();
    store.commit_write_unit_of_work().unwrap();
    drop(store);
    let store=NolaStore::open(&database).unwrap();
    assert!(store.read_assistant_record_chunk(&snapshot,&part,&intermediate,0).is_err());
}

#[test]
fn large_declaration_stages_invisibly_restarts_and_publishes_atomically_after_reopen() {
    let directory=tempfile::tempdir().unwrap();
    let source_path=directory.path().join("large-source.sqlite");
    let target_path=directory.path().join("large-target.sqlite");
    let mut source=NolaStore::open(&source_path).unwrap();
    let id=begin(&mut source,"large");
    let part="large-run-journal-part";
    let chunks:Vec<_>=["甲","乙","丙"].iter().map(|value|AssistantTextChunkV1::new(value.repeat(20000)).unwrap()).collect();
    let base=empty_journal_content(&id,part,JournalContentField::Markdown,JournalNumber(1)).unwrap();
    let mut final_content=base.clone();
    source.begin_write_unit_of_work().unwrap();
    let stamp=transition(&mut source,&id,2);
    for chunk in &chunks {
        final_content=source.prepare_assistant_content(&id,part,&final_content,std::slice::from_ref(chunk)).unwrap();
    }
    let mut batch=declaration(&id);
    let AssistantJournalOperationV1::Declare(v)=&mut batch.operations[0] else {panic!()};
    v.part.bodies=vec![final_content.clone()];
    source.commit_assistant_journal(&CommitAssistantJournal {
        stamp:AssistantSourceCommitStamp::Transition {transition_id:stamp},
        expected:AssistantJournalCursor::initial(id.clone()).unwrap(),batch,supplied_chunks:vec![],
    }).unwrap();source.commit_write_unit_of_work().unwrap();
    let window=source.open_assistant_journal_read(&AssistantJournalCursor::initial(id.clone()).unwrap()).unwrap();
    let AssistantJournalPage::Records {mut records,..}=source.read_assistant_journal_page(&window,1,262144).unwrap() else {panic!()};
    let record=records.remove(0);
    let mut target=NolaStore::open(&target_path).unwrap();
    target.admit_remote_submission_replica(&nola_store_port::remote_session::AdmittedReplicaTurnInput {
        user_part_kind: "text".into(),
        source_host_id:"host".into(),conversation_id:id.conversation_id.clone(),conversation_title:"Large".into(),
        user_message_id:"target-u".into(),user_part_id:"target-u-part".into(),user_text:"Receive".into(),user_content_json:None,
        assistant_message_id:id.assistant_message_id.clone(),assistant_part_id:"placeholder".into(),run_id:id.run_id.clone(),
        context_checkpoint_id:None,created_at_ms:1,
    }).unwrap();
    let input=|prefix:JournalContentRef,chunk:AssistantTextChunkV1,restart|StageAssistantJournalContent {
        source_host_id:"host".into(),record:record.clone(),part_id:part.into(),content:final_content.clone(),
        expected_prefix:prefix,chunks:vec![chunk],restart,
    };
    assert!(target.next_assistant_journal_content("wrong-host",&record).is_err());
    let need=target.next_assistant_journal_content("host",&record).unwrap().unwrap();
    assert_eq!(need.part_id,part); assert_eq!(need.content,final_content); assert_eq!(need.expected_prefix,base);
    let bindings:i64=target.connection.query_row("SELECT count(*) FROM remote_assistant_journal_sources",[],|r|r.get(0)).unwrap();
    assert_eq!(bindings,0,"planning cannot pin source authority or publish state");
    target.begin_write_unit_of_work().unwrap();
    let first=target.stage_assistant_journal_content(&input(base.clone(),chunks[0].clone(),false)).unwrap();
    assert_eq!(target.stage_assistant_journal_content(&input(base.clone(),chunks[0].clone(),false)).unwrap(),first);
    assert_eq!(target.assistant_journal_head(&id).unwrap().sequence,0);
    assert!(target.assistant_journal_part(&id,part).unwrap().is_none());
    assert!(target.receive_assistant_journal_record("host",&record,&[]).is_err(),"incomplete preparation cannot publish");
    target.commit_write_unit_of_work().unwrap();drop(target);
    let mut target=NolaStore::open(&target_path).unwrap();
    assert_eq!(target.next_assistant_journal_content("host",&record).unwrap().unwrap().expected_prefix,first,
        "download recovery comes from committed preparation, not a client cursor");
    target.begin_write_unit_of_work().unwrap();
    let wrong=AssistantTextChunkV1::new("错".repeat(20000)).unwrap();
    let bad_prefix=target.stage_assistant_journal_content(&input(first.clone(),wrong,false)).unwrap();
    assert!(target.stage_assistant_journal_content(&input(bad_prefix,chunks[2].clone(),false)).is_err());
    // Reset only this exact record's preparation, not any public generation.
    let first=target.stage_assistant_journal_content(&input(base.clone(),chunks[0].clone(),true)).unwrap();
    let second=target.stage_assistant_journal_content(&input(first.clone(),chunks[1].clone(),false)).unwrap();
    assert_eq!(target.stage_assistant_journal_content(&input(base,chunks[0].clone(),false)).unwrap(),first,
        "repeated earlier page cannot rewind later prepared pages");
    assert_eq!(target.stage_assistant_journal_content(&input(second,chunks[2].clone(),false)).unwrap(),final_content);
    assert!(target.next_assistant_journal_content("host",&record).unwrap().is_none());
    assert_eq!(target.assistant_journal_head(&id).unwrap().sequence,0,
        "content readiness is not record publication");
    target.commit_write_unit_of_work().unwrap();drop(target);
    let mut target=NolaStore::open(&target_path).unwrap();
    target.begin_write_unit_of_work().unwrap();
    target.connection.execute_batch("CREATE TEMP TRIGGER fail_activation BEFORE INSERT ON assistant_journal_records BEGIN SELECT RAISE(ABORT,'activation failure'); END;").unwrap();
    assert!(target.receive_assistant_journal_record("host",&record,&[]).unwrap_err().to_string().contains("activation failure"));
    assert_eq!(target.assistant_journal_head(&id).unwrap().sequence,0);
    let manifests:i64=target.connection.query_row("SELECT count(*) FROM assistant_journal_manifests",[],|r|r.get(0)).unwrap();
    assert_eq!(manifests,0,"failed activation cannot leave readable/prepared public roots");
    target.connection.execute_batch("DROP TRIGGER fail_activation").unwrap();
    target.receive_assistant_journal_record("host",&record,&[]).unwrap();
    target.commit_write_unit_of_work().unwrap();
    assert_eq!(target.assistant_journal_head(&id).unwrap(),window.through);
    assert!(target.next_assistant_journal_content("host",&record).unwrap().is_none(),
        "committed record replay must not redownload content");
    assert_eq!(target.assistant_journal_part(&id,part).unwrap().unwrap().bodies,vec![final_content.clone()]);
    // A second receiver obtained this need before publication and only now
    // finishes downloading. Its exact page is an ACK, not a new preparation.
    let delayed = input(need.expected_prefix.clone(), chunks[0].clone(), false);
    target.begin_write_unit_of_work().unwrap();
    assert_eq!(target.stage_assistant_journal_content(&delayed).unwrap(), first);
    let mut foreign = delayed.clone();
    foreign.source_host_id = "other-host".into();
    assert!(target.stage_assistant_journal_content(&foreign).is_err());
    let mut wrong_page = delayed.clone();
    wrong_page.chunks = vec![AssistantTextChunkV1::new("伪".repeat(20000)).unwrap()];
    assert!(target.stage_assistant_journal_content(&wrong_page).is_err());
    assert_eq!(target.assistant_journal_head(&id).unwrap(), window.through);
    target.commit_write_unit_of_work().unwrap();
    let stages:i64=target.connection.query_row("SELECT count(*) FROM remote_assistant_content_stages",[],|r|r.get(0)).unwrap();
    let leftovers:i64=target.connection.query_row("SELECT count(*) FROM remote_assistant_content_stage_chunks",[],|r|r.get(0)).unwrap();
    assert_eq!((stages,leftovers),(0,0));
    // Two dependent appends in one record must activate in operation order,
    // without retransmitting the already committed 180 KiB prefix.
    let extra:Vec<_>=["增量一","增量二"].iter().map(|text|AssistantTextChunkV1::new((*text).into()).unwrap()).collect();
    let mut batch=AssistantOperationBatchV1 {protocol_version:1,identity:id.clone(),operations:vec![]};
    let mut prefix=final_content;
    let mut prepared_inputs=Vec::new();
    for (index,chunk) in extra.iter().enumerate() {
        let refs=vec![JournalChunkRef {digest:chunk.digest(),utf8_bytes:JournalNumber(chunk.byte_length() as u64)}];
        let next=append_journal_content(&prefix,&refs).unwrap();
        batch.operations.push(AssistantJournalOperationV1::Append(AppendJournalText {
            part_id:part.into(),expected_generation:JournalNumber(1),expected_revision:JournalNumber(1+index as u64),
            expected:prefix.clone(),chunks:refs,resulting:next.clone(),
        }));
        prepared_inputs.push((prefix,next.clone(),chunk.clone()));prefix=next;
    }
    let record=StoredAssistantJournalRecord {
        header:batch.seal(2,window.through.record_digest,"dependent-append".into(),3).unwrap(),batch,
    };
    target.begin_write_unit_of_work().unwrap();
    for (previous,result,chunk) in prepared_inputs {
        let next=target.next_assistant_journal_content("host",&record).unwrap().unwrap();
        assert_eq!(next.expected_prefix,previous); assert_eq!(next.content,result);
        assert_eq!(target.stage_assistant_journal_content(&StageAssistantJournalContent {
            source_host_id:"host".into(),record:record.clone(),part_id:part.into(),content:result.clone(),
            expected_prefix:previous,chunks:vec![chunk],restart:false,
        }).unwrap(),result);
    }
    assert!(target.next_assistant_journal_content("host",&record).unwrap().is_none());
    target.receive_assistant_journal_record("host",&record,&[]).unwrap();
    target.commit_write_unit_of_work().unwrap();
    assert_eq!(target.assistant_journal_part(&id,part).unwrap().unwrap().bodies,vec![prefix]);
}

#[test]
fn receiver_proves_host_epoch_order_and_cold_replay_without_local_republication() {
    let source_directory=tempfile::tempdir().unwrap();
    let mut source=NolaStore::open(source_directory.path().join("source.sqlite")).unwrap();
    let id=begin(&mut source,"receive");
    declare(&mut source,&id);
    source.begin_write_unit_of_work().unwrap();
    transition(&mut source,&id,3);
    let append=append_input(&source,&id,3,"跨进程🙂正文");
    source.commit_assistant_journal(&append).unwrap();
    source.commit_write_unit_of_work().unwrap();
    let window=source.open_assistant_journal_read(&AssistantJournalCursor::initial(id.clone()).unwrap()).unwrap();
    let AssistantJournalPage::Records {records,..}=source.read_assistant_journal_page(&window,128,262144).unwrap() else {panic!()};
    assert_eq!(records.len(),2);
    let directory=tempfile::tempdir().unwrap();
    let database=directory.path().join("receiver.sqlite");
    let mut target=NolaStore::open(&database).unwrap();
    target.admit_remote_submission_replica(&nola_store_port::remote_session::AdmittedReplicaTurnInput {
        user_part_kind: "text".into(),
        source_host_id:"authenticated-host".into(),conversation_id:id.conversation_id.clone(),
        conversation_title:"Receiver".into(),user_message_id:"remote-user".into(),user_part_id:"remote-user-body".into(),
        user_text:"Receive".into(),user_content_json:None,assistant_message_id:id.assistant_message_id.clone(),
        assistant_part_id:"remote-placeholder".into(),run_id:id.run_id.clone(),context_checkpoint_id:None,created_at_ms:1,
    }).unwrap();
    target.begin_write_unit_of_work().unwrap();
    assert!(target.receive_assistant_journal_record("forged-host",&records[0],&[]).is_err());
    let initial=target.assistant_journal_reception_state("authenticated-host",&window.through).unwrap();
    assert!(!initial.complete);
    assert_eq!(initial.after,AssistantJournalCursor::initial(id.clone()).unwrap());
    assert!(target.assistant_journal_reception_state("forged-host",&window.through).is_err());
    assert!(target.receive_assistant_journal_record("authenticated-host",&records[1],&append.supplied_chunks).is_err());
    target.receive_assistant_journal_record("authenticated-host",&records[0],&[]).unwrap();
    // An insert fault after content preparation must roll back that entire frame.
    target.connection.execute_batch("CREATE TEMP TRIGGER reject_received_record BEFORE INSERT ON assistant_journal_records WHEN NEW.sequence=2 BEGIN SELECT RAISE(ABORT,'receiver failure'); END;").unwrap();
    assert!(target.receive_assistant_journal_record("authenticated-host",&records[1],&append.supplied_chunks).unwrap_err().to_string().contains("receiver failure"));
    let count:i64=target.connection.query_row("SELECT count(*) FROM assistant_journal_chunks",[],|row|row.get(0)).unwrap();
    assert_eq!(count,0);
    assert_eq!(target.assistant_journal_head(&id).unwrap().sequence,1);
    target.connection.execute_batch("DROP TRIGGER reject_received_record").unwrap();
    target.receive_assistant_journal_record("authenticated-host",&records[1],&append.supplied_chunks).unwrap();
    target.commit_write_unit_of_work().unwrap();
    drop(target);
    let mut target=NolaStore::open(&database).unwrap();
    assert_eq!(target.assistant_journal_head(&id).unwrap(),window.through);
    target.begin_write_unit_of_work().unwrap();
    let complete=target.assistant_journal_reception_state("authenticated-host",&window.through).unwrap();
    assert!(complete.complete); assert_eq!(complete.after,window.through);
    let earlier=AssistantJournalCursor {identity:id.clone(),sequence:records[0].header.sequence,record_digest:records[0].header.record_digest};
    assert_eq!(target.assistant_journal_reception_state("authenticated-host",&earlier).unwrap(),complete);
    let mut forged=earlier; forged.record_digest=window.through.record_digest;
    assert!(target.assistant_journal_reception_state("authenticated-host",&forged).is_err(),
        "being behind the current cursor cannot authenticate a forged notification");
    assert_eq!(source.local_assistant_journal_head(&id).unwrap(), window.through);
    for store in [&source, &target] {
        let restored = store.load_conversation_message_window(&id.conversation_id, None, 32, 32768)
            .unwrap().unwrap();
        assert_eq!(restored.assistant_journal_heads, vec![window.through.clone()],
            "cold windows must resolve local and admitted remote provenance without a live notification");
    }
    assert_eq!(source.local_run_assistant_journal_heads(&id.conversation_id, &id.run_id, None, 1).unwrap(), vec![window.through.clone()]);
    assert!(source.local_run_assistant_journal_heads(&id.conversation_id, &id.run_id, Some(&id.assistant_message_id), 1).unwrap().is_empty());
    assert!(source.local_run_assistant_journal_heads("foreign", &id.run_id, None, 1).is_err());
    assert!(source.local_run_assistant_journal_heads(&id.conversation_id, &id.run_id, None, 65).is_err());
    assert!(target.local_run_assistant_journal_heads(&id.conversation_id, &id.run_id, None, 1).is_err(),
        "discovery must not re-export an imported journal as this Host's output");
    assert!(target.local_assistant_journal_head(&id).unwrap_err().to_string()
        .contains("not authored by this source Host"),
        "an imported readable journal must not be re-exported under this Host's identity");
    assert!(matches!(target.receive_assistant_journal_record("authenticated-host",&records[1],&append.supplied_chunks).unwrap(),AssistantJournalCommit::Replay(_)));
    let mut changed_id=id.clone();changed_id.source_store_epoch="new-untrusted-epoch".into();
    let changed=declaration(&changed_id);
    let changed_header=changed.seal(1,AssistantJournalCursor::initial(changed.identity.clone()).unwrap().record_digest,"forged".into(),1).unwrap();
    assert!(target.receive_assistant_journal_record("authenticated-host",&StoredAssistantJournalRecord {header:changed_header,batch:changed},&[]).is_err());
    let snapshot=target.open_assistant_journal_snapshot(&id).unwrap();
    target.commit_write_unit_of_work().unwrap();
    let part=target.assistant_journal_part(&id,"receive-run-journal-part").unwrap().unwrap();
    let text=target.read_assistant_snapshot_range(&snapshot,&part.part_id,&part.bodies[0],0,32768).unwrap();
    assert_eq!(text.text,"跨进程🙂正文");
    let publications:i64=target.connection.query_row("SELECT count(*) FROM assistant_journal_publications",[],|row|row.get(0)).unwrap();
    assert_eq!(publications,0,"receipt import must not manufacture local source publications");
}
