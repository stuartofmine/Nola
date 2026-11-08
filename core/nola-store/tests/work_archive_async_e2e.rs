use nola_store::{
    work_archive::read_verified, CreateConversationInput, NolaStore, SaveCheckpointInput,
};
use std::{
    fs::{self, OpenOptions},
    time::{Duration, Instant},
};
fn create(store: &mut NolaStore) {
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
fn save(store: &mut NolaStore, revision: i64) {
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
    create(&mut store);
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
    save(&mut store, 1);
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
    create(&mut store);
    save(&mut store, 1);
    store.flush_work_archives().unwrap();
    let before = store
        .archive_publication_status()
        .unwrap()
        .target_generation;
    store.begin_write_unit_of_work().unwrap();
    save(&mut store, 2);
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
#[ignore = "child process fixture"]
fn writer_process_fixture() {
    let db = std::env::var("NOLA_ARCHIVE_FIXTURE_DB").expect("fixture database");
    let mut store = NolaStore::open(db).unwrap();
    create(&mut store);
    save(&mut store, 1);
    // Simulates process loss after durable ACK; deliberately do not drain worker.
    std::process::exit(0);
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
