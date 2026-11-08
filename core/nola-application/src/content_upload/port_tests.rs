// SPDX-License-Identifier: Apache-2.0
use super::*;
use std::cell::Cell;

struct ReadOnlyFixture {
    queries: Cell<usize>,
    fail: bool,
    progress: UploadProgress,
}

impl ContentUploadRepository for ReadOnlyFixture {
    type Error = &'static str;
    fn begin(&mut self, _: &str, _: &UploadOwner, _: u64) -> Result<UploadProgress, Self::Error> {
        panic!("progress must not begin an upload")
    }
    fn append(
        &mut self,
        _: &str,
        _: &UploadOwner,
        _: u64,
        _: &str,
    ) -> Result<UploadProgress, Self::Error> {
        panic!("progress must not append")
    }
    fn seal(&mut self, _: &str, _: &UploadOwner) -> Result<UploadProgress, Self::Error> {
        panic!("progress must not seal")
    }
    fn cancel(&mut self, _: &str, _: &UploadOwner) -> Result<UploadProgress, Self::Error> {
        panic!("progress must not cancel")
    }
    fn read_chunk(&self, _: &str, _: &UploadOwner, _: u64) -> Result<Option<String>, Self::Error> {
        panic!("progress must not load content")
    }
    fn progress(&self, id: &str, owner: &UploadOwner) -> Result<UploadProgress, Self::Error> {
        assert_eq!(id, "upload");
        assert_eq!(
            (&*owner.conversation_id, &*owner.import_id, &*owner.part_id),
            ("c", "i", "p")
        );
        self.queries.set(self.queries.get() + 1);
        if self.fail {
            Err("unavailable")
        } else {
            Ok(self.progress.clone())
        }
    }
}

#[test]
fn progress_is_fresh_read_only_and_keeps_lifecycle_states() {
    let owner = ContentUploadOwner {
        conversation_id: "c".into(),
        import_id: "i".into(),
        part_id: "p".into(),
    };
    let command = ApplicationCommand::ContentUploadProgress {
        input: ContentUploadRequest {
            upload_id: "upload".into(),
            owner: owner.clone(),
        },
    };
    let mut store = ReadOnlyFixture {
        queries: Cell::new(0),
        fail: false,
        progress: UploadProgress {
            expected_bytes: 10,
            committed_bytes: 4,
            sealed: false,
            cancelled: false,
        },
    };
    for (sealed, cancelled, committed, expected_state) in [
        (false, false, 4, ContentUploadState::Receiving),
        (true, false, 10, ContentUploadState::Sealed),
        (false, true, 0, ContentUploadState::Cancelled),
    ] {
        store.progress.sealed = sealed;
        store.progress.cancelled = cancelled;
        store.progress.committed_bytes = committed;
        let value = execute_staged_upload(&mut store, &command, "upload", &owner).unwrap();
        let result: ContentUploadProgress = serde_json::from_value(value).unwrap();
        assert_eq!(result.state, expected_state);
        assert_eq!(result.committed_utf8_bytes, committed);
        assert_eq!(result.expected_utf8_bytes, 10);
    }
    assert_eq!(store.queries.get(), 3);
    store.fail = true;
    assert_eq!(
        execute_staged_upload(&mut store, &command, "upload", &owner).unwrap_err(),
        "unavailable"
    );
}
