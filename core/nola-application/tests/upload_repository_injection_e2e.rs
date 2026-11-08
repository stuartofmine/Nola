// SPDX-License-Identifier: Apache-2.0
//! Runtime command boundary with a non-SQLite upload repository. This is not a
//! substitute for real Service IPC/restart and SQLite quota tests.
type ApplicationRuntime = nola_application::ApplicationRuntime<nola_store::NolaStore>;
use nola_application::{ApplicationCommand, CreateConversation};
use nola_protocol::content_upload::{BeginContentUpload, ContentUploadOwner, ContentUploadRequest};
use nola_store_port::content_upload::{ContentUploadRepository, UploadOwner, UploadProgress};
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};

struct UploadFixture(Arc<AtomicUsize>);

impl ContentUploadRepository for UploadFixture {
    type Error = String;
    fn begin(&mut self, _: &str, _: &UploadOwner, _: u64) -> Result<UploadProgress, String> {
        self.0.fetch_add(1, Ordering::Relaxed);
        Err("fixture refuses mutation".into())
    }
    fn progress(&self, id: &str, owner: &UploadOwner) -> Result<UploadProgress, String> {
        assert_eq!(id, "u");
        assert_eq!(owner.conversation_id, "c");
        self.0.fetch_add(1, Ordering::Relaxed);
        Ok(UploadProgress {
            expected_bytes: 12,
            committed_bytes: 4,
            sealed: false,
            cancelled: false,
        })
    }
    fn append(
        &mut self,
        _: &str,
        _: &UploadOwner,
        _: u64,
        _: &str,
    ) -> Result<UploadProgress, String> {
        panic!("unexpected append")
    }
    fn seal(&mut self, _: &str, _: &UploadOwner) -> Result<UploadProgress, String> {
        panic!("unexpected seal")
    }
    fn cancel(&mut self, _: &str, _: &UploadOwner) -> Result<UploadProgress, String> {
        panic!("unexpected cancel")
    }
    fn read_chunk(&self, _: &str, _: &UploadOwner, _: u64) -> Result<Option<String>, String> {
        panic!("unexpected content read")
    }
}

fn owner() -> ContentUploadOwner {
    ContentUploadOwner {
        conversation_id: "c".into(),
        import_id: "i".into(),
        part_id: "p".into(),
    }
}

#[test]
fn injected_repository_is_used_without_creating_sqlite_staging() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("nola.db");
    let calls = Arc::new(AtomicUsize::new(0));
    let mut runtime = ApplicationRuntime::from_store(
        nola_store::NolaStore::open(&path).unwrap(),
        Box::new(UploadFixture(calls.clone())),
    );
    assert_eq!(calls.load(Ordering::Relaxed), 0);
    let progress = || ApplicationCommand::ContentUploadProgress {
        input: ContentUploadRequest {
            upload_id: "u".into(),
            owner: owner(),
        },
    };
    assert_eq!(
        runtime.execute(progress()).unwrap_err(),
        "content upload owner has neither a live conversation nor an active archive profile"
    );
    assert_eq!(calls.load(Ordering::Relaxed), 0);
    runtime
        .execute(ApplicationCommand::CreateConversation {
            input: CreateConversation {
                id: "c".into(),
                title: "c".into(),
                settings_json: None,
                created_at_ms: 1,
                updated_at_ms: 1,
            },
        })
        .unwrap();
    let result = runtime.execute(progress()).unwrap();
    assert_eq!(result["committed_utf8_bytes"], 4);
    assert_eq!(result["expected_utf8_bytes"], 12);
    assert_eq!(calls.load(Ordering::Relaxed), 1);
    let begin = |size| ApplicationCommand::BeginContentUpload {
        input: BeginContentUpload {
            upload_id: "u".into(),
            owner: owner(),
            expected_utf8_bytes: size,
        },
    };
    assert!(runtime.execute(begin(0)).is_err());
    assert!(runtime
        .execute(begin(nola_application::MAXIMUM_CONTENT_UPLOAD_BYTES + 1))
        .is_err());
    assert_eq!(calls.load(Ordering::Relaxed), 1);
    assert_eq!(
        runtime.execute(begin(12)).unwrap_err(),
        "fixture refuses mutation"
    );
    assert_eq!(calls.load(Ordering::Relaxed), 2);
    assert!(!root.path().join("nola.db.uploads.sqlite").exists());
}
