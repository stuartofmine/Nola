// SPDX-License-Identifier: Apache-2.0
//! Host-local view preferences, never a second conversation or execution store.
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::path::PathBuf;

#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ViewState {
    pub draft: String,
    pub cursor: usize,
    pub viewport: crate::terminal_ui::ViewportBookmark,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Record {
    format: u32,
    state: ViewState,
}

#[derive(Clone)]
pub(crate) struct ViewStore(std::sync::Arc<std::sync::Mutex<ViewFile>>);

struct ViewFile {
    path: PathBuf,
    last_bytes: Vec<u8>,
}

impl ViewStore {
    pub fn open(session: &str) -> Result<(Self, ViewState), String> {
        let database = crate::platform_paths::application_database_path()?;
        let database = database.canonicalize().map_err(|error| error.to_string())?;
        let scope = format!("{:x}", Sha256::digest(database.as_os_str().as_encoded_bytes()));
        let name = format!("{:x}.json", Sha256::digest(session.as_bytes()));
        let directory = database.parent().ok_or("view state database directory missing")?
            .join("CliViewState").join(scope);
        std::fs::create_dir_all(&directory).map_err(|error| error.to_string())?;
        #[cfg(unix)] {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&directory, std::fs::Permissions::from_mode(0o700))
                .map_err(|error| error.to_string())?;
        }
        let path = directory.join(name);
        let bytes = match std::fs::read(&path) {
            Ok(bytes) => Some(bytes),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
            Err(error) => return Err(error.to_string()),
        };
        let state = if let Some(bytes) = &bytes {
            let record: Record = serde_json::from_slice(bytes).map_err(|error| format!("Invalid CLI view state: {error}"))?;
            if record.format != 1 { return Err("Unsupported CLI view state format; file preserved".into()); }
            if record.state.cursor > record.state.draft.len() || !record.state.draft.is_char_boundary(record.state.cursor) {
                return Err("Invalid CLI draft cursor; file preserved".into());
            }
            record.state
        } else { ViewState::default() };
        Ok((Self(std::sync::Arc::new(std::sync::Mutex::new(ViewFile { path, last_bytes: bytes.unwrap_or_default() }))), state))
    }

    pub fn save(&mut self, state: ViewState) -> Result<(), String> {
        use std::io::Write;
        let mut file_state = self.0.lock().map_err(|_| "CLI view state lock poisoned")?;
        let bytes = serde_json::to_vec(&Record { format: 1, state }).map_err(|error| error.to_string())?;
        if bytes == file_state.last_bytes { return Ok(()); }
        let parent = file_state.path.parent().ok_or("view state directory missing")?;
        let mut file = tempfile::NamedTempFile::new_in(parent).map_err(|error| error.to_string())?;
        file.write_all(&bytes).map_err(|error| error.to_string())?;
        file.persist(&file_state.path).map_err(|error| error.to_string())?;
        file_state.last_bytes = bytes;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn draft_cursor_roundtrips_and_sessions_and_stores_are_isolated() {
        let _lock = crate::store::test_env_lock();
        let root = tempfile::tempdir().unwrap();
        let previous = std::env::var_os("NOLA_DB_PATH");
        struct Restore(Option<std::ffi::OsString>);
        impl Drop for Restore {
            fn drop(&mut self) {
                match &self.0 { Some(path) => std::env::set_var("NOLA_DB_PATH", path), None => std::env::remove_var("NOLA_DB_PATH") }
            }
        }
        let _restore = Restore(previous);
        let db = root.path().join("one.db");
        std::fs::write(&db, b"").unwrap();
        std::env::set_var("NOLA_DB_PATH", &db);
        let (mut first, _) = ViewStore::open("session-a").unwrap();
        first.save(ViewState { draft: "甲乙 draft".into(), cursor: 3, ..Default::default() }).unwrap();
        let (_, restored) = ViewStore::open("session-a").unwrap();
        assert_eq!(restored.draft, "甲乙 draft");
        assert_eq!(restored.cursor, 3);
        assert!(ViewStore::open("session-b").unwrap().1.draft.is_empty());
        // The listener captures the current session, even if the foreground
        // later binds another one. Both writers share their deduplication state.
        let binding = ActiveView::bind(Some(first.clone()));
        let mut listener = active_store().unwrap();
        let (other_session, _) = ViewStore::open("session-b").unwrap();
        ActiveView::replace(Some(other_session));
        listener.save(ViewState { draft: "执行草稿".into(), cursor: 6, ..Default::default() }).unwrap();
        assert_eq!(ViewStore::open("session-a").unwrap().1.draft, "执行草稿");
        assert!(ViewStore::open("session-b").unwrap().1.draft.is_empty());
        first.save(ViewState { draft: "甲乙 draft".into(), cursor: 3, ..Default::default() }).unwrap();
        assert_eq!(ViewStore::open("session-a").unwrap().1.draft, "甲乙 draft");
        drop(binding);
        assert!(active_store().is_none());
        let first_path = first.0.lock().unwrap().path.clone();
        #[cfg(unix)] {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(std::fs::metadata(&first_path).unwrap().permissions().mode() & 0o777, 0o600);
        }
        let other = root.path().join("two.db");
        std::fs::write(&other, b"").unwrap();
        std::env::set_var("NOLA_DB_PATH", other);
        assert!(ViewStore::open("session-a").unwrap().1.draft.is_empty());
        std::env::set_var("NOLA_DB_PATH", db);
        let mut invalid: serde_json::Value = serde_json::from_slice(&std::fs::read(&first_path).unwrap()).unwrap();
        invalid["format"] = serde_json::json!(99);
        let bytes = serde_json::to_vec(&invalid).unwrap();
        std::fs::write(&first_path, &bytes).unwrap();
        assert!(ViewStore::open("session-a").err().unwrap().contains("Unsupported"));
        assert_eq!(std::fs::read(&first_path).unwrap(), bytes);
        invalid["format"] = serde_json::json!(1);
        invalid["state"]["cursor"] = serde_json::json!(1);
        std::fs::write(&first_path, serde_json::to_vec(&invalid).unwrap()).unwrap();
        assert!(ViewStore::open("session-a").err().unwrap().contains("cursor"));
    }
}

// Only the interactive session thread binds this sink. A turn listener captures
// its own handle before spawning, so later session switches cannot redirect it.
thread_local! {
    static ACTIVE: std::cell::RefCell<Option<ViewStore>> = const { std::cell::RefCell::new(None) };
}
pub(crate) struct ActiveView;
impl ActiveView {
    pub fn bind(store: Option<ViewStore>) -> Self { Self::replace(store); Self }
    pub fn replace(store: Option<ViewStore>) { ACTIVE.with(|active| *active.borrow_mut() = store); }
}
impl Drop for ActiveView {
    fn drop(&mut self) { Self::replace(None); }
}
pub(crate) fn active_store() -> Option<ViewStore> { ACTIVE.with(|active| active.borrow().clone()) }
