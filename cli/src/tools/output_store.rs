// SPDX-License-Identifier: Apache-2.0
// Tool result storage and lossless range reads. No permissions, model routing,
// UI formatting, or ToolOutcome construction belongs in this module.

use serde::Serialize;
use sha2::{Digest, Sha256};
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::io;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug)]
pub(crate) struct ToolOutputStore {
    root: PathBuf,
}

#[derive(Debug, Serialize)]
pub(super) struct ToolOutputSlice {
    #[serde(rename = "type")]
    pub kind: &'static str,
    pub path: String,
    pub offset_space: &'static str,
    pub json_pointer: Option<String>,
    pub size_bytes: u64,
    pub offset: u64,
    pub bytes_returned: usize,
    pub next_offset: u64,
    pub has_more: bool,
    pub content: String,
}

#[derive(Debug)]
pub(super) enum ReadError {
    OutsideStore,
    NotAFile,
    OffsetOutOfRange { offset: u64, size: u64 },
    InvalidUtf8Boundary { offset: u64 },
    ResponseBudgetTooSmall,
    ModelEvidenceTooLarge,
    InvalidJsonSelection,
    IntegrityMismatch,
    Io(std::io::Error),
}

impl ReadError {
    pub fn code(&self) -> &'static str {
        match self {
            Self::OutsideStore => "tool_output_outside_store",
            Self::NotAFile => "tool_output_not_a_file",
            Self::OffsetOutOfRange { .. } => "tool_output_offset_out_of_range",
            Self::InvalidUtf8Boundary { .. } => "tool_output_invalid_utf8_boundary",
            Self::ResponseBudgetTooSmall => "tool_output_response_budget_too_small",
            Self::ModelEvidenceTooLarge => "tool_output_model_evidence_too_large",
            Self::InvalidJsonSelection => "tool_output_invalid_json_selection",
            Self::IntegrityMismatch => "tool_output_integrity_mismatch",
            Self::Io(error) if error.kind() == std::io::ErrorKind::NotFound => {
                "tool_output_not_found"
            }
            Self::Io(_) => "tool_output_io_error",
        }
    }
}

impl std::fmt::Display for ReadError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::OffsetOutOfRange { offset, size } => {
                write!(formatter, "Offset {offset} exceeds output size {size}")
            }
            Self::InvalidUtf8Boundary { offset } => write!(
                formatter,
                "Offset {offset} is not a UTF-8 character boundary"
            ),
            Self::Io(error) => write!(formatter, "{error}"),
            _ => write!(formatter, "{}", self.code()),
        }
    }
}

impl From<std::io::Error> for ReadError {
    fn from(error: std::io::Error) -> Self {
        Self::Io(error)
    }
}

impl ToolOutputStore {
    pub fn new(root: PathBuf) -> Self {
        Self { root }
    }

    pub fn for_session(database: &Path, session_id: &str) -> Self {
        // Encode arbitrary identities; neither a session ID nor a database
        // filename may escape the store namespace through path separators.
        let namespace = format!("{:x}", Sha256::digest(session_id.as_bytes()));
        let mut directory = database.as_os_str().to_os_string();
        directory.push(".outputs");
        Self::new(PathBuf::from(directory).join(namespace))
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    #[cfg(test)]
    pub fn for_test() -> Self {
        // A store also contains mutable session witnesses, not only uniquely
        // named tool outputs. Sharing this directory across parallel fixtures
        // lets one host replace another host's compaction-request.json.
        Self::new(std::env::temp_dir().join(format!(
            "nola-cli-tool-outputs-{}",
            uuid::Uuid::new_v4()
        )))
    }

    pub fn persist(&self, text: &str) -> std::io::Result<PathBuf> {
        let evidence = nola_domain::tool_evidence::ModelToolEvidence::text(text);
        self.persist_bytes(evidence.as_bytes(), format!("tool-{:x}.txt", Sha256::digest(evidence.as_bytes())))
    }

    pub fn persist_media(&self, bytes: &[u8]) -> std::io::Result<PathBuf> {
        self.persist_bytes(bytes, format!("media-{:x}.bin", Sha256::digest(bytes)))
    }

    /// Snapshot a tool-owned, already authorized media artifact before publishing
    /// its receipt. Replays must not reopen the mutable source file.
    pub fn snapshot_media(&self, source: &Path) -> std::io::Result<PathBuf> {
        let mut options = OpenOptions::new();
        options.read(true);
        #[cfg(unix)] {
            use std::os::unix::fs::OpenOptionsExt;
            options.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
        }
        let file = options.open(source)?;
        if !file.metadata()?.is_file() {
            return Err(std::io::Error::other("media is not a regular file"));
        }
        const LIMIT: u64 = 30 * 1024 * 1024;
        let mut bytes = Vec::new();
        file.take(LIMIT + 1).read_to_end(&mut bytes)?;
        if bytes.is_empty() || bytes.len() as u64 > LIMIT {
            return Err(std::io::Error::other("media is empty or exceeds 30 MiB"));
        }
        if source.parent() == Some(self.root.as_path())
            && source.file_name().and_then(|value| value.to_str()).is_some_and(|name| name.starts_with("media-"))
            && source.file_name().and_then(|value| value.to_str()) != Some(format!("media-{:x}.bin", Sha256::digest(&bytes)).as_str()) {
            return Err(std::io::Error::other("media snapshot digest mismatch"));
        }
        self.persist_media(&bytes)
    }

    fn persist_bytes(&self, bytes: &[u8], name: String) -> std::io::Result<PathBuf> {
        fs::create_dir_all(&self.root)?;
        let id = uuid::Uuid::new_v4();
        let staging = self.root.join(format!(".{id}.pending"));
        let destination = self.root.join(name);
        let result = (|| {
            let mut file = OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&staging)?;
            file.write_all(bytes)?;
            file.sync_all()?;
            // Publish without replacing a previous receipt's evidence. Concurrent
            // identical results share the blob, never the execution identity.
            match fs::hard_link(&staging, &destination) {
                Ok(()) => {},
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
                    let mut options = OpenOptions::new();
                    options.read(true);
                    #[cfg(unix)] {
                        use std::os::unix::fs::OpenOptionsExt;
                        options.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
                    }
                    if fs::symlink_metadata(&destination)?.file_type().is_symlink() {
                        return Err(io::Error::other("tool output destination is a symbolic link"));
                    }
                    let existing = options.open(&destination)?;
                    if !existing.metadata()?.is_file() {
                        return Err(io::Error::other("tool output destination is not a regular file"));
                    }
                    let mut stored = Vec::new();
                    existing.take(bytes.len() as u64 + 1).read_to_end(&mut stored)?;
                    if stored != bytes {
                        return Err(io::Error::other("tool output content digest mismatch"));
                    }
                },
                Err(error) => return Err(error),
            }
            fs::remove_file(&staging)?;
            // Persist the name-to-content binding before publishing a receipt.
            #[cfg(unix)]
            File::open(&self.root)?.sync_all()?;
            Ok(destination)
        })();
        if result.is_err() {
            let _ = fs::remove_file(&staging);
        }
        result
    }

    pub(super) fn resolve(&self, path: &Path) -> Result<PathBuf, ReadError> {
        if fs::symlink_metadata(path)?.file_type().is_symlink() {
            return Err(ReadError::NotAFile);
        }
        let root = fs::canonicalize(&self.root)?;
        let path = fs::canonicalize(path)?;
        if !path.starts_with(root) {
            return Err(ReadError::OutsideStore);
        }
        if path
            .file_name()
            .is_some_and(|name| name.to_string_lossy().starts_with('.'))
        {
            return Err(ReadError::NotAFile);
        }
        if !path.is_file() {
            return Err(ReadError::NotAFile);
        }
        Ok(path)
    }

    /// Project before slicing/searching. Redacting a slice that begins in the
    /// middle of a control URL cannot identify the credential it contains.
    /// Offsets refer to this immutable model view, never to raw UI controls.
    pub(super) fn model_text(&self, path: &Path) -> Result<String, ReadError> {
        let requested_name = path.file_name().and_then(|name| name.to_str());
        let path = self.resolve(path)?;
        let mut options = OpenOptions::new();
        options.read(true);
        #[cfg(unix)] {
            use std::os::unix::fs::OpenOptionsExt;
            options.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
        }
        let file = options.open(&path)?;
        if !file.metadata()?.is_file() { return Err(ReadError::NotAFile); }
        const MAX_MODEL_EVIDENCE_BYTES: u64 = 32 * 1024 * 1024;
        let mut text = String::new();
        file.take(MAX_MODEL_EVIDENCE_BYTES + 1).read_to_string(&mut text)?;
        if text.len() as u64 > MAX_MODEL_EVIDENCE_BYTES { return Err(ReadError::ModelEvidenceTooLarge); }
        // Verify the same bytes that will be projected/sliced, not a separate
        // reopen. Mutable Host witness files have no content-addressed name.
        if let Some(name) = requested_name {
            let content_address = if name.starts_with("tool-") && name.ends_with(".txt") {
                Some(format!("tool-{:x}.txt", Sha256::digest(text.as_bytes())))
            } else if name.starts_with("media-") && name.ends_with(".bin") {
                Some(format!("media-{:x}.bin", Sha256::digest(text.as_bytes())))
            } else {
                None
            };
            if content_address.is_some_and(|expected| expected != name) {
                return Err(ReadError::IntegrityMismatch);
            }
        }
        Ok(nola_domain::tool_evidence::ModelToolEvidence::text(&text))
    }

    /// Both the content byte budget and serialized response budget apply. JSON
    /// escaping can expand a small slice sixfold; never save a page as another
    /// truncated output document just because it contains quotes/control bytes.
    pub(super) fn read(
        &self,
        path: &Path,
        offset: u64,
        max_bytes: usize,
        max_response_chars: usize,
    ) -> Result<ToolOutputSlice, ReadError> {
        self.read_selection(path, None, offset, max_bytes, max_response_chars)
    }

    pub(super) fn read_selection(
        &self, path: &Path, json_pointer: Option<&str>, offset: u64,
        max_bytes: usize, max_response_chars: usize,
    ) -> Result<ToolOutputSlice, ReadError> {
        let evidence = self.model_text(path)?;
        let path = self.resolve(path)?;
        let evidence = if let Some(pointer) = json_pointer {
            if pointer.len() > 512 || (!pointer.is_empty() && !pointer.starts_with('/')) {
                return Err(ReadError::InvalidJsonSelection);
            }
            let value: serde_json::Value = serde_json::from_str(&evidence).map_err(|_| ReadError::InvalidJsonSelection)?;
            let selected = value.pointer(pointer).ok_or(ReadError::InvalidJsonSelection)?;
            match selected {
                serde_json::Value::String(text) => text.clone(),
                value => serde_json::to_string_pretty(value).expect("JSON selection serializes"),
            }
        } else { evidence };
        let size = evidence.len() as u64;
        if offset > size {
            return Err(ReadError::OffsetOutOfRange { offset, size });
        }
        let text = evidence.get(offset as usize..).ok_or(ReadError::InvalidUtf8Boundary { offset })?;
        let mut end = max_bytes.min(text.len());
        while !text.is_char_boundary(end) {
            end -= 1;
        }
        if end == 0 && !text.is_empty() {
            end = text.chars().next().expect("nonempty text").len_utf8();
        }
        let make_page = |end: usize| ToolOutputSlice {
            kind: "tool_output_slice",
            path: path.to_string_lossy().into_owned(),
            offset_space: if json_pointer.is_some() { "json_selection_utf8" } else { "model_evidence_utf8" },
            json_pointer: json_pointer.map(str::to_owned),
            size_bytes: size,
            offset,
            bytes_returned: end,
            next_offset: offset + end as u64,
            has_more: offset + (end as u64) < size,
            content: text[..end].to_string(),
        };
        let fits = |page: &ToolOutputSlice| {
            serde_json::to_string(page)
                .expect("slice serializes")
                .chars()
                .count()
                <= max_response_chars
        };
        let mut page = make_page(end);
        if !fits(&page) {
            let boundaries: Vec<_> = text[..end]
                .char_indices()
                .map(|(index, _)| index)
                .chain(std::iter::once(end))
                .collect();
            let (mut low, mut high) = (0, boundaries.len());
            while low < high {
                let middle = low + (high - low) / 2;
                if fits(&make_page(boundaries[middle])) {
                    low = middle + 1;
                } else {
                    high = middle;
                }
            }
            if low == 0 || (boundaries[low - 1] == 0 && offset < size) {
                return Err(ReadError::ResponseBudgetTooSmall);
            }
            page = make_page(boundaries[low - 1]);
        }
        Ok(page)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn model_projection_precedes_byte_ranges_without_rewriting_source_evidence() {
        let root = tempfile::tempdir().unwrap();
        let store = ToolOutputStore::new(root.path().to_owned());
        let path = root.path().join("raw-host-evidence.txt");
        let source = "prefix http://127.0.0.1:12345/view?token=credential-fixture suffix";
        fs::write(&path, source).unwrap();
        let safe = store.model_text(&path).unwrap();
        assert!(!safe.contains("credential-fixture"));
        let mut reconstructed = String::new();
        let mut offset = 0;
        loop {
            let page = store.read(&path, offset, 5, 2048).unwrap();
            assert_eq!(page.offset_space, "model_evidence_utf8");
            reconstructed.push_str(&page.content);
            offset = page.next_offset;
            if !page.has_more { break; }
        }
        assert_eq!(reconstructed, safe);
        assert_eq!(fs::read_to_string(&path).unwrap(), source);
        let persisted = store.persist(source).unwrap();
        assert_eq!(fs::read_to_string(persisted).unwrap(), safe);
    }

    #[test]
    fn json_region_reads_return_actionable_refs_without_repeating_large_document_text() {
        let root = tempfile::tempdir().unwrap();
        let store = ToolOutputStore::new(root.path().to_owned());
        let source = serde_json::json!({"accessibility":"document ".repeat(20_000),
            "elements":[{"ref":"generation-page-element-1","role":"button","name":"Save"}],"revision":9});
        let path = store.persist(&source.to_string()).unwrap();
        let page = store.read_selection(&path, Some("/elements/0"), 0, 1024, 2048).unwrap();
        assert_eq!(page.offset_space, "json_selection_utf8");
        assert_eq!(page.json_pointer.as_deref(), Some("/elements/0"));
        assert!(page.content.contains("generation-page-element-1"));
        assert!(!page.content.contains("document"));
        assert!(!page.has_more);
        assert!(matches!(store.read_selection(&path, Some("/elements/10"), 0, 1024, 2048), Err(ReadError::InvalidJsonSelection)));
    }

    #[test]
    fn media_snapshot_survives_source_changes_and_rejects_tampered_snapshot() {
        let root = tempfile::tempdir().unwrap();
        let source = root.path().join("screenshot.png");
        fs::write(&source, b"original image").unwrap();
        let store = ToolOutputStore::new(root.path().join("outputs"));
        let snapshot = store.snapshot_media(&source).unwrap();
        fs::write(&source, b"next screenshot").unwrap();
        assert_eq!(fs::read(&snapshot).unwrap(), b"original image");
        assert_ne!(store.snapshot_media(&source).unwrap(), snapshot);
        fs::write(&snapshot, b"tampered").unwrap();
        assert!(store.snapshot_media(&snapshot).is_err());
    }

    #[test]
    fn test_stores_isolate_session_witness_namespaces() {
        let stores = (0..32).map(|_| ToolOutputStore::for_test()).collect::<Vec<_>>();
        let namespaces = stores.iter()
            .map(|store| store.root().join("compaction-request.json"))
            .collect::<std::collections::HashSet<_>>();
        assert_eq!(namespaces.len(), stores.len());
        // Cloning one fixture intentionally preserves its namespace, unlike
        // constructing another independent fixture.
        assert_eq!(stores[0].clone().root(), stores[0].root());
    }

    #[test]
    fn session_and_database_ownership_survive_reopen_without_cross_session_access() {
        let fixture = tempfile::tempdir().unwrap();
        let database = fixture.path().join("fixture.db");
        let first = ToolOutputStore::for_session(&database, "session/../中文");
        let second = ToolOutputStore::for_session(&database, "other");
        let other_database =
            ToolOutputStore::for_session(&fixture.path().join("other.db"), "session/../中文");
        let reference = first.persist("evidence").unwrap();
        second.persist("other evidence").unwrap();
        other_database.persist("unrelated database").unwrap();
        assert!(matches!(
            second.resolve(&reference),
            Err(ReadError::OutsideStore)
        ));
        assert!(matches!(
            other_database.resolve(&reference),
            Err(ReadError::OutsideStore)
        ));
        let reopened = ToolOutputStore::for_session(&database, "session/../中文");
        assert_eq!(
            reopened.read(&reference, 0, 6000, 8000).unwrap().content,
            "evidence"
        );
        assert!(reference.starts_with(fixture.path().join("fixture.db.outputs")));
        let pending = first.root().join(".unfinished.pending");
        fs::write(&pending, "partial").unwrap();
        assert!(matches!(first.resolve(&pending), Err(ReadError::NotAFile)));
    }

    #[test]
    fn escaped_multilingual_pages_are_bounded_lossless_and_survive_reopen() {
        let fixture = tempfile::tempdir().unwrap();
        let store = ToolOutputStore::new(fixture.path().to_path_buf());
        let original = "中文😀\u{0001}\"\\\n".repeat(2000);
        let path = store.persist(&original).unwrap();
        let reopened = ToolOutputStore::new(fixture.path().to_path_buf());
        for limit in [1, 3, 6000] {
            let (mut offset, mut restored) = (0, String::new());
            loop {
                let page = reopened.read(&path, offset, limit, 8000).unwrap();
                assert!(serde_json::to_string(&page).unwrap().chars().count() <= 8000);
                assert_eq!(page.bytes_returned, page.content.len());
                assert!(page.bytes_returned <= limit + 3);
                restored.push_str(&page.content);
                assert!(page.next_offset > offset);
                offset = page.next_offset;
                if !page.has_more {
                    break;
                }
            }
            assert_eq!(restored, original);
        }
        assert!(matches!(
            reopened.read(&path, 1, 6000, 8000),
            Err(ReadError::InvalidUtf8Boundary { .. })
        ));
        assert!(matches!(
            reopened.read(&path, original.len() as u64 + 1, 6000, 8000),
            Err(ReadError::OffsetOutOfRange { .. })
        ));
    }

    #[test]
    fn out_of_store_paths_and_symlinks_are_not_result_authority() {
        let first = tempfile::tempdir().unwrap();
        let second = tempfile::tempdir().unwrap();
        let store = ToolOutputStore::new(first.path().to_path_buf());
        let other = ToolOutputStore::new(second.path().to_path_buf())
            .persist("private")
            .unwrap();
        assert!(matches!(
            store.resolve(&other),
            Err(ReadError::OutsideStore)
        ));
        #[cfg(unix)]
        {
            let link = first.path().join("escape.txt");
            std::os::unix::fs::symlink(other, &link).unwrap();
            assert!(matches!(store.resolve(&link), Err(ReadError::NotAFile)));
        }
    }

    #[test]
    fn identical_evidence_is_shared_after_reopening_without_rewriting() {
        let fixture = tempfile::tempdir().unwrap();
        let store = ToolOutputStore::new(fixture.path().to_path_buf());
        let first = store.persist("证据\nunchanged").unwrap();
        let reopened = ToolOutputStore::new(fixture.path().to_path_buf());
        assert_eq!(first, reopened.persist("证据\nunchanged").unwrap());
        assert_ne!(first, reopened.persist("different").unwrap());
        assert_eq!(fs::read_dir(fixture.path()).unwrap().count(), 2);
    }

    #[test]
    fn corrupted_existing_blob_is_rejected_not_repaired() {
        let fixture = tempfile::tempdir().unwrap();
        let store = ToolOutputStore::new(fixture.path().to_path_buf());
        for path in [store.persist("evidence").unwrap(), store.persist_media(b"media").unwrap()] {
            fs::write(&path, b"corrupt").unwrap();
        }
        assert!(store.persist("evidence").is_err());
        assert!(store.persist_media(b"media").is_err());
        for entry in fs::read_dir(fixture.path()).unwrap() {
            assert_eq!(fs::read(entry.unwrap().path()).unwrap(), b"corrupt");
        }
    }

    #[test]
    fn reopened_reads_reject_tampered_content_addressed_evidence() {
        let fixture = tempfile::tempdir().unwrap();
        let store = ToolOutputStore::new(fixture.path().to_path_buf());
        for path in [store.persist("original").unwrap(), store.persist_media(b"original").unwrap()] {
            fs::write(&path, b"modified").unwrap();
            let reopened = ToolOutputStore::new(fixture.path().to_path_buf());
            assert_eq!(reopened.model_text(&path).unwrap_err().code(), "tool_output_integrity_mismatch");
            assert!(matches!(reopened.read(&path, 0, 128, 2048), Err(ReadError::IntegrityMismatch)));
            assert_eq!(fs::read(&path).unwrap(), b"modified");
        }
    }

    #[cfg(unix)]
    #[test]
    fn content_address_cannot_be_redirected_to_another_valid_blob() {
        let fixture = tempfile::tempdir().unwrap();
        let store = ToolOutputStore::new(fixture.path().to_path_buf());
        let first = store.persist("first").unwrap();
        let second = store.persist("second").unwrap();
        fs::remove_file(&first).unwrap();
        std::os::unix::fs::symlink(&second, &first).unwrap();
        assert!(matches!(store.read(&first, 0, 128, 2048), Err(ReadError::NotAFile)));
        assert_eq!(store.model_text(&second).unwrap(), "second");
    }

    #[test]
    fn concurrent_publication_keeps_one_complete_blob() {
        let fixture = tempfile::tempdir().unwrap();
        let store = ToolOutputStore::new(fixture.path().to_path_buf());
        let paths = std::thread::scope(|scope| {
            let threads: Vec<_> = (0..8).map(|_| scope.spawn(|| store.persist("same evidence").unwrap())).collect();
            threads.into_iter().map(|thread| thread.join().unwrap()).collect::<Vec<_>>()
        });
        assert!(paths.iter().all(|path| path == &paths[0]));
        assert_eq!(fs::read_to_string(&paths[0]).unwrap(), "same evidence");
        assert_eq!(fs::read_dir(fixture.path()).unwrap().count(), 1);
    }

    #[cfg(unix)]
    #[test]
    fn publication_refuses_a_symlink_even_when_its_content_matches() {
        let fixture = tempfile::tempdir().unwrap();
        let store = ToolOutputStore::new(fixture.path().join("store"));
        let path = store.persist("evidence").unwrap();
        let outside = fixture.path().join("outside");
        fs::rename(&path, &outside).unwrap();
        std::os::unix::fs::symlink(&outside, &path).unwrap();
        assert!(store.persist("evidence").is_err());
        assert!(fs::symlink_metadata(&path).unwrap().file_type().is_symlink());
        assert_eq!(fs::read_to_string(outside).unwrap(), "evidence");
    }
}
