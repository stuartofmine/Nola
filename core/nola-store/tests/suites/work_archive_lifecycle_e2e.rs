// SPDX-License-Identifier: Apache-2.0
//! Consolidated synchronous and asynchronous work archive publication lifecycle suite.

use std::{fs::{self, OpenOptions}, time::{Duration, Instant}};
use nola_store::{
    work_archive::read_verified, CreateConversationInput, NolaStore, SaveCheckpointInput,
};
use serde_json::json;

fn create(store: &mut NolaStore) {
    store
        .create_conversation(&CreateConversationInput {
            id: "../session/秘密".into(),
            title: "archive".into(),
            revision: 1,
            settings_json: None,
            created_at_ms: 1,
            updated_at_ms: 1,
        })
        .unwrap();
}
fn save(store: &mut NolaStore, revision: i64) {
    store.save_checkpoint(&SaveCheckpointInput {
        conversation_id:"../session/秘密".into(),expected_revision:revision-1,revision,event_cursor:revision,updated_at_ms:revision,
        checkpoint_json:json!({"revision":revision,"state":{
            "goal_projection":{"active":{"id":"goal","objective":"保留数据","constraints":["不要部署"],"requirements":{},"work":{"entries":[],"user_updates":[]}}},
            "task_projection":{"tasks":{}},"conversation":{"transcript":[
                {"type":"tool_result","id":"r","call_id":"c","name":"read_file","is_error":false,"output":{"content":"evidence"}},
                {"type":"compaction","id":"compact","summary":"summary","source_item_ids":["r"]}
            ]}}}).to_string()
    }).unwrap();
}

#[test]
fn explicit_flush_publishes_private_consistent_files_and_rollback_never_publishes() {
    let temp = tempfile::tempdir().unwrap();
    let db = temp.path().join("store.db");
    let mut store = NolaStore::open(&db).unwrap();
    create(&mut store);
    save(&mut store, 1);
    store.flush_work_archives().unwrap();
    let path = store.work_archive_directory("../session/秘密").unwrap();
    assert_eq!(path.parent(), store.work_archive_root().as_deref());
    let first = read_verified(&path).unwrap();
    assert_eq!(first.manifest.revision, 1);
    assert!(String::from_utf8_lossy(&first.files["goal.md"]).contains("不要部署"));
    assert!(String::from_utf8_lossy(&first.files["compactions.json"]).contains("source_item_ids"));
    store.begin_write_unit_of_work().unwrap();
    save(&mut store, 2);
    assert_eq!(read_verified(&path).unwrap().manifest, first.manifest);
    store.rollback_write_unit_of_work().unwrap();
    assert_eq!(read_verified(&path).unwrap().manifest, first.manifest);
    store.begin_write_unit_of_work().unwrap();
    save(&mut store, 2);
    store.commit_write_unit_of_work().unwrap();
    store.flush_work_archives().unwrap();
    assert_eq!(read_verified(&path).unwrap().manifest.revision, 2);
    assert_eq!(
        store.work_archive_status("../session/秘密").unwrap()["pending"],
        false
    );
    drop(store);
    let mut reopened = NolaStore::open(&db).unwrap();
    save(&mut reopened, 3);
    reopened.flush_work_archives().unwrap();
    assert_eq!(read_verified(&path).unwrap().manifest.revision, 3);
    assert_eq!(
        std::fs::read_dir(path.join("generations")).unwrap().count(),
        2
    );
}

#[test]
fn corrupt_file_is_rejected_and_partial_staging_cannot_replace_manifest() {
    let temp = tempfile::tempdir().unwrap();
    let mut store = NolaStore::open(temp.path().join("s.db")).unwrap();
    create(&mut store);
    save(&mut store, 1);
    store.flush_work_archives().unwrap();
    let path = store.work_archive_directory("../session/秘密").unwrap();
    let before = read_verified(&path).unwrap();
    std::fs::create_dir(path.join("generations/pending-interrupted")).unwrap();
    assert_eq!(read_verified(&path).unwrap().manifest, before.manifest);
    std::fs::write(
        path.join("generations")
            .join(&before.manifest.generation)
            .join("goal.md"),
        "forged done",
    )
    .unwrap();
    assert!(read_verified(&path).is_err());
    assert!(store.flush_work_archives().is_err(),"explicit verification must run even when published target is current");
    assert_eq!(
        store.work_archive_status("../session/秘密").unwrap()["available"],
        false
    );
}

#[test]
fn unrelated_commits_and_reopens_do_not_republish_unchanged_work() {
    let temp = tempfile::tempdir().unwrap();
    let db = temp.path().join("s.db");
    let mut store = NolaStore::open(&db).unwrap();
    create(&mut store);
    save(&mut store, 1);
    store.flush_work_archives().unwrap();
    let path = store.work_archive_directory("../session/秘密").unwrap();
    let manifest_path = path.join("manifest.json");
    let before = std::fs::metadata(&manifest_path)
        .unwrap()
        .modified()
        .unwrap();
    let original = read_verified(&path).unwrap().manifest;
    for revision in 1..5 {
        store
            .put_product_setting("ui_test", "{}", revision)
            .unwrap();
    }
    drop(store);
    let reopened = NolaStore::open(&db).unwrap();
    reopened.flush_work_archives().unwrap();
    assert_eq!(
        std::fs::metadata(manifest_path)
            .unwrap()
            .modified()
            .unwrap(),
        before
    );
    assert_eq!(read_verified(&path).unwrap().manifest, original);
}

#[cfg(unix)]
#[test]
fn unsafe_archive_path_does_not_rollback_database_and_reopen_retries() {
    let temp = tempfile::tempdir().unwrap();
    let db = temp.path().join("s.db");
    let mut store = NolaStore::open(&db).unwrap();
    create(&mut store);
    let outside = temp.path().join("outside");
    std::fs::create_dir(&outside).unwrap();
    let root = store.work_archive_root().unwrap();
    std::os::unix::fs::symlink(&outside, &root).unwrap();
    save(&mut store, 1);
    assert_eq!(
        store
            .load_checkpoint("../session/秘密")
            .unwrap()
            .unwrap()
            .revision,
        1
    );
    assert_eq!(std::fs::read_dir(&outside).unwrap().count(), 0);
    std::fs::remove_file(&root).unwrap();
    drop(store);
    let reopened = NolaStore::open(&db).unwrap();
    reopened.flush_work_archives().unwrap();
    assert!(read_verified(&reopened.work_archive_directory("../session/秘密").unwrap()).is_ok());
}

#[test]
fn concurrent_publisher_and_reader_never_observe_mixed_generations() {
    let temp = tempfile::tempdir().unwrap();
    let db = temp.path().join("s.db");
    let mut writer = NolaStore::open(&db).unwrap();
    create(&mut writer);
    save(&mut writer, 1);
    let path = writer.work_archive_directory("../session/秘密").unwrap();
    let barrier = std::sync::Arc::new(std::sync::Barrier::new(2));
    let start = barrier.clone();
    let archive_path = path.clone();
    let reader = std::thread::spawn(move || {
        let other = NolaStore::open(&db).unwrap();
        start.wait();
        let mut last = 1;
        for _ in 0..8 {
            other.flush_work_archives().unwrap();
            let snapshot = read_verified(&archive_path).unwrap();
            assert!(snapshot.manifest.revision >= last);
            last = snapshot.manifest.revision;
        }
    });
    barrier.wait();
    for revision in 2..6 {
        save(&mut writer, revision);
    }
    reader.join().unwrap();
    writer.flush_work_archives().unwrap();
    assert_eq!(read_verified(&path).unwrap().manifest.revision, 5);
}

// --- Asynchronous publisher & backlog recovery scenarios ---
fn create_async(store: &mut NolaStore) {
    store
        .create_conversation(&CreateConversationInput {
            id: "async-fixture".into(),
            title: "archive".into(),
            revision: 1,
            settings_json: None,
            created_at_ms: 1,
            updated_at_ms: 1,
        })
        .unwrap();
}
fn save_async(store: &mut NolaStore, revision: i64) {
    store
        .save_checkpoint(&SaveCheckpointInput {
            conversation_id: "async-fixture".into(),
            expected_revision: revision - 1,
            revision,
            event_cursor: revision,
            updated_at_ms: revision,
            checkpoint_json:
                serde_json::json!({"revision":revision,"state":{"conversation":{"transcript":[]}}})
                    .to_string(),
        })
        .unwrap();
}
fn await_status(store: &NolaStore, check: impl Fn(&nola_store::ArchivePublicationStatus) -> bool) {
    let deadline = Instant::now() + Duration::from_secs(8);
    loop {
        let status = store.archive_publication_status().unwrap();
        if check(&status) {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "archive did not settle: {status:?}"
        );
        std::thread::sleep(Duration::from_millis(25));
    }
}
fn make_lock_private(root: &std::path::Path) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(root, fs::Permissions::from_mode(0o700)).unwrap();
        fs::set_permissions(root.join("publish.lock"), fs::Permissions::from_mode(0o600)).unwrap();
    }
}
#[test]
fn ack_does_not_wait_for_publisher_lock_and_retry_needs_no_new_command() {
    let temp = tempfile::tempdir().unwrap();
    let mut store = NolaStore::open(temp.path().join("db.sqlite")).unwrap();
    create_async(&mut store);
    let root = store.work_archive_root().unwrap();
    fs::create_dir_all(&root).unwrap();
    let lock = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(root.join("publish.lock"))
        .unwrap();
    make_lock_private(&root);
    lock.lock().unwrap();
    let capture = nola_store::commit_diagnostics::begin_capture().unwrap();
    store.begin_write_unit_of_work().unwrap();
    save_async(&mut store, 1);
    store.commit_write_unit_of_work().unwrap();
    let report = capture.finish();
    assert!(!report.observations.iter().any(|o| matches!(
        o.stage,
        nola_store::commit_diagnostics::Stage::PostCommitArchive
    )));
    assert_eq!(
        store
            .load_checkpoint("async-fixture")
            .unwrap()
            .unwrap()
            .revision,
        1
    );
    await_status(&store, |s| s.consecutive_failures > 0);
    let failed = store.archive_publication_status().unwrap();
    assert_eq!(
        failed.last_error_code.as_deref(),
        Some("archive_publication_failed")
    );
    assert!(failed.target_generation > failed.published_generation);
    drop(lock);
    // No new writes or notifications; the persisted retry deadline must suffice.
    await_status(&store, |s| {
        s.target_generation == s.published_generation && s.last_error_code.is_none()
    });
    assert_eq!(
        read_verified(&store.work_archive_directory("async-fixture").unwrap())
            .unwrap()
            .manifest
            .revision,
        1
    );
}
#[test]
fn target_is_transactional_and_deleted_owner_is_retired_without_erasing_bytes() {
    let temp = tempfile::tempdir().unwrap();
    let db = temp.path().join("db.sqlite");
    let mut store = NolaStore::open(&db).unwrap();
    create_async(&mut store);
    save_async(&mut store, 1);
    store.flush_work_archives().unwrap();
    let before = store
        .archive_publication_status()
        .unwrap()
        .target_generation;
    store.begin_write_unit_of_work().unwrap();
    save_async(&mut store, 2);
    assert!(
        store
            .archive_publication_status()
            .unwrap()
            .target_generation
            > before
    );
    store.rollback_write_unit_of_work().unwrap();
    assert_eq!(
        store
            .archive_publication_status()
            .unwrap()
            .target_generation,
        before
    );
    let path = store.work_archive_directory("async-fixture").unwrap();
    let raw = rusqlite::Connection::open(&db).unwrap();
    raw.execute(
        "DELETE FROM core_checkpoints WHERE conversation_id='async-fixture'",
        [],
    )
    .unwrap();
    assert_eq!(
        store.work_archive_status("async-fixture").unwrap()["available"],
        false
    );
    store.flush_work_archives().unwrap();
    assert!(path.join("retired.json").is_file());
    assert!(
        path.join("manifest.json").is_file(),
        "recovery bytes are retained"
    );
    assert!(read_verified(&path).is_err());
    let status = store.archive_publication_status().unwrap();
    assert_eq!(status.target_generation, status.published_generation);
}
#[test]
fn reopen_recovers_committed_backlog_after_writer_process_exit() {
    let temp = tempfile::tempdir().unwrap();
    let db = temp.path().join("db.sqlite");
    let store = NolaStore::open(&db).unwrap();
    let root = store.work_archive_root().unwrap();
    drop(store);
    fs::create_dir_all(&root).unwrap();
    let lock = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(root.join("publish.lock"))
        .unwrap();
    make_lock_private(&root);
    lock.lock().unwrap();
    let child = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "writer_process_fixture", "--ignored"])
        .env("NOLA_ARCHIVE_FIXTURE_DB", &db)
        .status()
        .unwrap();
    assert!(child.success());
    drop(lock);
    let reopened = NolaStore::open(&db).unwrap();
    await_status(&reopened, |s| s.target_generation == s.published_generation);
    assert_eq!(
        read_verified(&reopened.work_archive_directory("async-fixture").unwrap())
            .unwrap()
            .manifest
            .revision,
        1
    );
}
