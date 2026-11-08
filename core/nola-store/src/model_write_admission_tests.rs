// SPDX-License-Identifier: Apache-2.0
//! Real second-connection contention at the first model read, not a timing race.
use super::*;
use std::sync::{Arc, Mutex, atomic::{AtomicBool, AtomicUsize, Ordering}};

#[test]
fn multi_step_model_mutations_reserve_writer_before_the_first_profile_read() {
    for operation in ["endpoint", "replace", "configure", "reset"] {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("writer-admission.sqlite");
        let mut store = NolaStore::open_archive_mode(&path, false).unwrap();
        let profile = StoredModelProfile {
            id: "model".into(), display_name: "Model".into(), provider: "test".into(),
            model_name: "test".into(), runtime: "remote".into(),
            source_json: r#"{"endpointID":"endpoint"}"#.into(), parameters_json: "{}".into(),
            selected: true, enabled: true, revision: 0, created_at_ms: 1,
            updated_at_ms: 1, deleted_at_ms: None,
        };
        store.upsert_model_profile(&profile).unwrap();
        let profile = store.model_profile("model").unwrap().unwrap();
        let revisions = std::collections::BTreeMap::from([("model".into(), profile.revision)]);
        let competitor = Connection::open(&path).unwrap();
        competitor.busy_timeout(std::time::Duration::ZERO).unwrap();
        let competitor = Arc::new(Mutex::new(competitor));
        let attempted = Arc::new(AtomicBool::new(false));
        let observation = Arc::new(AtomicUsize::new(0));
        let writer = competitor.clone();
        let once = attempted.clone();
        let observed = observation.clone();
        store.connection.authorizer(Some(move |context: rusqlite::hooks::AuthContext<'_>| {
            if matches!(context.action, rusqlite::hooks::AuthAction::Read { table_name: "model_profiles", .. })
                && !once.swap(true, Ordering::SeqCst) {
                let result = writer.lock().unwrap().execute(
                    "UPDATE work_archive_publication SET last_attempt_at_ms=123 WHERE singleton=1", []);
                observed.store(match result {
                    Err(rusqlite::Error::SqliteFailure(error, _)) if error.code == rusqlite::ErrorCode::DatabaseBusy => 1,
                    Ok(_) => 2,
                    Err(_) => 3,
                }, Ordering::SeqCst);
            }
            rusqlite::hooks::Authorization::Allow
        }));
        match operation {
            "endpoint" => store.configure_model_endpoint("endpoint", &revisions, &[profile.clone()]),
            "replace" => store.replace_model_profiles("test", "remote", &[profile.clone()], 2, &revisions),
            "configure" => {
                let mut input = profile.clone();
                input.selected = false;
                store.configure_model_parameters(&input)
            }
            "reset" => store.reset_model_parameters(&["model".into()], "{}", 2),
            _ => unreachable!(),
        }.unwrap();
        assert!(attempted.load(Ordering::SeqCst), "{operation}: fixture must hit a profile read");
        assert_eq!(observation.load(Ordering::SeqCst), 1, "{operation}: second connection must be blocked before any model snapshot");
        assert!(store.connection.is_autocommit());
        competitor.lock().unwrap().execute(
            "UPDATE work_archive_publication SET last_attempt_at_ms=456 WHERE singleton=1", []).unwrap();
    }
}
