// SPDX-License-Identifier: Apache-2.0
//! Automatic same-build service startup preserves incompatible user history.
#![cfg(unix)]

use std::{
    fs,
    process::{Command, Stdio},
    thread,
    time::{Duration, Instant},
};

#[test]
fn both_cli_service_entries_reject_incompatible_schema_without_erasing_it() {
    for binary in [
        env!("CARGO_BIN_EXE_nola"),
        env!("CARGO_BIN_EXE_nola-agent-host"),
    ] {
        for previously_nola_owned in [false, true] {
            let root = tempfile::tempdir().unwrap();
            let database = root.path().join("history.db");
            if previously_nola_owned {
                drop(nola_store::NolaStore::open(&database).unwrap());
            }
            {
                let connection = rusqlite::Connection::open(&database).unwrap();
                connection.execute_batch("PRAGMA journal_mode=DELETE; CREATE TABLE retained_history(body TEXT NOT NULL);
                INSERT INTO retained_history VALUES('original private history');
                PRAGMA user_version=1;").unwrap();
            }
            let before = fs::read(&database).unwrap();
            let mut process = Command::new(binary)
                .arg("--nola-application-service")
                .arg("--database")
                .arg(&database)
                .arg("--socket")
                .arg(root.path().join("service.sock"))
                .stdout(Stdio::null())
                .stderr(Stdio::piped())
                .spawn()
                .unwrap();
            let deadline = Instant::now() + Duration::from_secs(10);
            loop {
                if process.try_wait().unwrap().is_some() {
                    break;
                }
                if Instant::now() >= deadline {
                    let _ = process.kill();
                    let _ = process.wait();
                    panic!("automatic service accepted an incompatible database: {binary}");
                }
                thread::sleep(Duration::from_millis(20));
            }
            let output = process.wait_with_output().unwrap();
            assert!(!output.status.success(), "{binary}");
            let error = String::from_utf8_lossy(&output.stderr);
            assert!(
                error.contains("database does not use the current canonical schema"),
                "expected schema refusal, not an unrelated startup failure for {binary}: {error}"
            );
            assert!(
                fs::read(&database).unwrap() == before,
                "database bytes changed: {binary}"
            );
            let connection = rusqlite::Connection::open(&database).unwrap();
            let body: String = connection
                .query_row("SELECT body FROM retained_history", [], |row| row.get(0))
                .unwrap();
            assert_eq!(body, "original private history");
        }
    }
}
