// SPDX-License-Identifier: Apache-2.0
//! Real Core/provider-response commits must not multiply frozen configuration
//! bytes in Run rows or full checkpoint bodies on the incremental hot path.
#[path = "support/committed_usage_stream.rs"]
mod committed_usage;
use nola_store::NolaStore;
use nola_store_port::run_recovery_inspection::RunRecoveryInspectionRepository;

fn stream_wal(config_padding: usize) -> u64 {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("run.db");
    let wal = directory.path().join("run.db-wal");
    let mut store = NolaStore::open(&path).unwrap();
    let reader = rusqlite::Connection::open(&path).unwrap();
    let mut start = 0;
    let mut bytes = 0;
    let mut brackets = 0;
    let (_, receipt) = committed_usage::seed_stream_with_observer(
        &mut store,
        Some(5432),
        config_padding,
        64,
        |begin| {
            if begin {
                // A pinned reader prevents autocheckpoint from recycling the WAL
                // during measurement; file growth therefore counts actual frames.
                let busy: i64 = reader
                    .query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |r| r.get(0))
                    .unwrap();
                assert_eq!(busy, 0);
                reader.execute_batch("BEGIN").unwrap();
                reader
                    .query_row("SELECT count(*) FROM runs", [], |r| r.get::<_, i64>(0))
                    .unwrap();
                start = std::fs::metadata(&wal).map(|m| m.len()).unwrap_or(0);
            } else {
                bytes = std::fs::metadata(&wal).unwrap().len() - start;
                reader.execute_batch("ROLLBACK").unwrap();
            }
            brackets += 1;
        },
    );
    assert_eq!(brackets, 2);
    assert!(bytes > 0);
    let original = store
        .load_run_invocation_config("usage-run")
        .unwrap()
        .unwrap();
    assert!(original.invocation_config_json.len() > config_padding);
    assert_eq!(
        reader
            .query_row("SELECT count(*) FROM run_invocation_payloads", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        1
    );
    assert!(!reader.query_row("SELECT EXISTS(SELECT 1 FROM pragma_table_info('runs') WHERE name='invocation_config_json')",[],|r|r.get::<_,bool>(0)).unwrap());
    assert!(!reader.query_row("SELECT EXISTS(SELECT 1 FROM pragma_table_info('core_checkpoints') WHERE name='checkpoint_json')",[],|r|r.get::<_,bool>(0)).unwrap());
    let baseline = store.load_checkpoint("usage").unwrap().unwrap();
    let bodies: i64=reader.query_row("SELECT count(*) FROM core_checkpoint_bodies",[],|r|r.get(0)).unwrap();
    assert_eq!(bodies,1);
    drop(store);
    drop(reader);
    let reopened = NolaStore::open(&path).unwrap();
    let reopened_baseline = reopened.load_checkpoint("usage").unwrap().unwrap();
    assert_eq!(reopened_baseline.checkpoint_json,baseline.checkpoint_json);
    assert_eq!(reopened_baseline.revision,baseline.revision);
    let identity = receipt.admission.unwrap().identity;
    let exact = reopened
        .load_exact_run_invocation(
            &identity.conversation_id,
            &identity.run_id,
            &identity.assistant_message_id,
        )
        .unwrap()
        .unwrap();
    assert_eq!(
        exact.invocation_config_json,
        original.invocation_config_json
    );
    assert_eq!(
        exact.invocation_config_digest,
        original.invocation_config_digest
    );
    assert!(reopened
        .load_exact_run_invocation("foreign", &identity.run_id, &identity.assistant_message_id)
        .is_err());
    bytes
}

#[test]
fn large_frozen_configuration_does_not_multiply_stream_wal_writes() {
    let small = stream_wal(4096);
    let large = stream_wal(2 * 1024 * 1024);
    assert!(
        large <= small + 8 * 1024 * 1024,
        "payload-sized write amplification: small={small},large={large}"
    );
    assert!(
        large < 32 * 1024 * 1024,
        "64 bounded commits exceeded WAL budget: {large}"
    );
}
