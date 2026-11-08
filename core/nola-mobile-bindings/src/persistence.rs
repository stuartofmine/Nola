// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Implements persistence logic in the Rust core module.
// - Interfaces:
//   - get_config_base
//   - write_config_file
//   - read_config_file
//   - history_filename_for
// - APIs:
//   - none
//
// Persistence and configuration storage

use directories::ProjectDirs;
use serde::{Deserialize, Serialize};
use std::ffi::{CStr, CString};
use std::os::raw::c_char;
use std::path::PathBuf;
use std::ptr;

pub(crate) fn get_config_base() -> Option<PathBuf> {
    if let Some(dirs) = ProjectDirs::from("com", "nola", "Nola") {
        Some(dirs.config_dir().to_path_buf())
    } else {
        std::env::var("HOME").ok().map(PathBuf::from)
    }
}

pub(crate) fn write_config_file(filename: &str, content: &str) -> Result<(), String> {
    let base = get_config_base().ok_or_else(|| "no base dir".to_string())?;
    let config_dir = base.join("nola");
    std::fs::create_dir_all(&config_dir).map_err(|e| e.to_string())?;
    let fpath = config_dir.join(filename);
    std::fs::write(&fpath, content).map_err(|e| e.to_string())?;
    Ok(())
}

pub(crate) fn read_config_file(filename: &str) -> Result<String, String> {
    let base = get_config_base().ok_or_else(|| "no base dir".to_string())?;
    let fpath = base.join("nola").join(filename);
    std::fs::read_to_string(&fpath).map_err(|e| e.to_string())
}

pub(crate) fn config_file_exists(filename: &str) -> Result<bool, String> {
    let base = get_config_base().ok_or_else(|| "no base dir".to_string())?;
    Ok(base.join("nola").join(filename).is_file())
}

pub(crate) fn remove_config_file(filename: &str) -> Result<(), String> {
    let base = get_config_base().ok_or_else(|| "no base dir".to_string())?;
    let path = base.join("nola").join(filename);
    if !path.exists() {
        return Ok(());
    }
    std::fs::remove_file(path).map_err(|error| error.to_string())
}

#[derive(Serialize, Deserialize, Debug)]
pub(crate) struct URLHistoryEntry {
    pub id: String,
    pub timestamp: String,
    #[serde(rename = "dedupeKey")]
    pub dedupe_key: String,
    pub title: String,
    pub details: Option<String>,
    #[serde(rename = "eventId")]
    pub event_id: Option<String>,
    pub priority: Option<f64>,
}

fn history_filename_for(source_id: &str) -> String {
    format!("url_history_{}.json", source_id)
}

fn decode_history(raw: &str) -> Result<Vec<URLHistoryEntry>, serde_json::Error> {
    serde_json::from_str(raw)
}

/// FFI: Append a history entry JSON to a source-specific history file
#[no_mangle]
pub extern "C" fn nola_core_url_history_append(
    source_id_ptr: *const c_char,
    entry_ptr: *const u8,
    entry_len: usize,
) -> bool {
    if source_id_ptr.is_null() || entry_ptr.is_null() || entry_len == 0 {
        return false;
    }
    unsafe {
        let source_id = match CStr::from_ptr(source_id_ptr).to_str() {
            Ok(s) => s.to_string(),
            Err(_) => return false,
        };
        let slice = std::slice::from_raw_parts(entry_ptr, entry_len);
        let entry_str = match std::str::from_utf8(slice) {
            Ok(s) => s,
            Err(_) => return false,
        };

        // Parse entry JSON
        let entry: URLHistoryEntry = match serde_json::from_str(entry_str) {
            Ok(e) => e,
            Err(_) => return false,
        };

        let filename = history_filename_for(&source_id);
        // Load existing history
        let mut list = match config_file_exists(&filename) {
            Ok(false) => Vec::new(),
            Ok(true) => match read_config_file(&filename)
                .and_then(|raw| decode_history(&raw).map_err(|error| error.to_string()))
            {
                Ok(entries) => entries,
                Err(_) => return false,
            },
            Err(_) => return false,
        };

        list.push(entry);
        // Cap to a reasonable size (e.g., 500 entries)
        if list.len() > 500 {
            let start = list.len() - 500;
            list = list.split_off(start);
        }

        let out = match serde_json::to_string(&list) {
            Ok(s) => s,
            Err(_) => return false,
        };

        write_config_file(&filename, &out).is_ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn url_history_requires_current_entry_array() {
        let valid = r#"[{"id":"entry-1","timestamp":"2026-08-21T00:00:00Z","dedupeKey":"key","title":"title","details":null,"eventId":null,"priority":null}]"#;
        assert_eq!(decode_history(valid).expect("current history").len(), 1);
        assert!(decode_history(r#"{"entries":[]}"#).is_err());
        assert!(decode_history("not-json").is_err());
    }
}

/// FFI: Load history entries for a source as JSON string
#[no_mangle]
pub extern "C" fn nola_core_url_history_load(source_id_ptr: *const c_char) -> *mut c_char {
    if source_id_ptr.is_null() {
        return ptr::null_mut();
    }
    unsafe {
        let source_id = match CStr::from_ptr(source_id_ptr).to_str() {
            Ok(s) => s.to_string(),
            Err(_) => return ptr::null_mut(),
        };
        let filename = history_filename_for(&source_id);
        match read_config_file(&filename) {
            Ok(s) => CString::new(s).unwrap().into_raw(),
            Err(_) => ptr::null_mut(),
        }
    }
}

/// FFI: Get config/data directory path
#[no_mangle]
pub extern "C" fn nola_core_persistence_data_dir() -> *mut c_char {
    match get_config_base() {
        Some(base) => {
            let path_str = base.join("nola").to_string_lossy().to_string();
            CString::new(path_str).unwrap().into_raw()
        }
        None => ptr::null_mut(),
    }
}

/// FFI: Save key-value data
#[no_mangle]
pub extern "C" fn nola_core_persistence_save(
    key_ptr: *const c_char,
    value_ptr: *const u8,
    value_len: usize,
) -> bool {
    if key_ptr.is_null() || value_ptr.is_null() {
        return false;
    }
    unsafe {
        let key = CStr::from_ptr(key_ptr).to_string_lossy().to_string();
        let slice = std::slice::from_raw_parts(value_ptr, value_len);
        if let Ok(value_str) = std::str::from_utf8(slice) {
            return write_config_file(&format!("kv_{}.json", key), value_str).is_ok();
        }
        false
    }
}

/// FFI: Load key-value data
#[no_mangle]
pub extern "C" fn nola_core_persistence_load(key_ptr: *const c_char) -> *mut c_char {
    if key_ptr.is_null() {
        return ptr::null_mut();
    }
    unsafe {
        let key = CStr::from_ptr(key_ptr).to_string_lossy().to_string();
        match read_config_file(&format!("kv_{}.json", key)) {
            Ok(s) => CString::new(s).unwrap().into_raw(),
            Err(_) => ptr::null_mut(),
        }
    }
}

/// FFI: Remove key-value data
#[no_mangle]
pub extern "C" fn nola_core_persistence_clear(key_ptr: *const c_char) -> bool {
    if key_ptr.is_null() {
        return false;
    }
    unsafe {
        let key = CStr::from_ptr(key_ptr).to_string_lossy().to_string();
        let base = match get_config_base() {
            Some(b) => b,
            None => return false,
        };
        let fpath = base.join("nola").join(format!("kv_{}.json", key));
        std::fs::remove_file(&fpath).is_ok()
    }
}

/// FFI: Save model selection list
#[no_mangle]
pub extern "C" fn nola_core_model_selection_save_models(
    models_ptr: *const u8,
    models_len: usize,
) -> bool {
    if models_ptr.is_null() || models_len == 0 {
        return false;
    }
    unsafe {
        let slice = std::slice::from_raw_parts(models_ptr, models_len);
        if let Ok(s) = std::str::from_utf8(slice) {
            return write_config_file("models_selection.json", s).is_ok();
        }
        false
    }
}

/// FFI: Load model selection list
#[no_mangle]
pub extern "C" fn nola_core_model_selection_load_models() -> *mut c_char {
    match read_config_file("models_selection.json") {
        Ok(s) => CString::new(s).unwrap().into_raw(),
        Err(_) => ptr::null_mut(),
    }
}
