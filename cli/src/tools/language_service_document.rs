// SPDX-License-Identifier: Apache-2.0
//! Versioned document snapshots. The caller authorizes and reads files; this
//! module only encodes snapshots and positions, never accesses the filesystem.

use std::path::Path;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

pub(super) const MAX_DOCUMENT_BYTES: usize = 1024 * 1024;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct DocumentIdentity {
    pub uri: String,
    pub version: i32,
    pub content_digest: String,
}

pub(super) struct Document {
    identity: DocumentIdentity,
    language_id: String,
    text: String,
}

impl Document {
    pub(super) fn open(path: &Path, language_id: &str, text: String) -> Result<(Self, Value), String> {
        if language_id.trim().is_empty() || language_id.len() > 128 || language_id.chars().any(char::is_control) {
            return Err("invalid language ID".into());
        }
        validate_size(&text)?;
        let uri = url::Url::from_file_path(path).map_err(|_| "document path must be absolute")?.to_string();
        let identity = DocumentIdentity { uri, version: 1, content_digest: digest(&text) };
        let message = json!({"jsonrpc":"2.0", "method":"textDocument/didOpen", "params":{
            "textDocument":{"uri":identity.uri, "languageId":language_id, "version":identity.version, "text":text}
        }});
        Ok((Self { identity, language_id: language_id.into(), text }, message))
    }

    pub(super) fn identity(&self) -> DocumentIdentity { self.identity.clone() }

    /// Input coordinates are one-based Unicode scalar positions, not UTF-8
    /// bytes or UTF-16 units. Output is the negotiated zero-based UTF-16 pair.
    pub(super) fn position(&self, line: u32, character: u32) -> Result<Value, String> {
        position(&self.text, line, character)
    }

    pub(super) fn change(&mut self, text: String) -> Result<Option<Value>, String> {
        validate_size(&text)?;
        if text == self.text { return Ok(None); }
        let version = self.identity.version.checked_add(1).ok_or("document versions exhausted")?;
        // Full-document content change is valid for the LSP content-change
        // union; it avoids deriving incremental ranges from stale text.
        let message = json!({"jsonrpc":"2.0", "method":"textDocument/didChange", "params":{
            "textDocument":{"uri":self.identity.uri,"version":version},
            "contentChanges":[{"text":text}]
        }});
        self.identity.version = version;
        self.identity.content_digest = digest(&text);
        self.text = text;
        Ok(Some(message))
    }

    pub(super) fn matches(&self, expected: &DocumentIdentity, current_text: &str) -> bool {
        &self.identity == expected && digest(current_text) == expected.content_digest
    }

    pub(super) fn evidence(&self) -> Value {
        json!({"backend":"language_service", "uri":self.identity.uri,
            "document_version":self.identity.version,"content_digest":self.identity.content_digest,
            "language_id":self.language_id,"position_encoding":"utf-16",
            "build_or_tests_executed":false})
    }

    pub(super) fn close(&self) -> Value {
        json!({"jsonrpc":"2.0","method":"textDocument/didClose",
            "params":{"textDocument":{"uri":self.identity.uri}}})
    }
}

pub(super) fn position(text: &str, line: u32, character: u32) -> Result<Value, String> {
    if line == 0 || character == 0 { return Err("document positions are one-based".into()); }
    let raw = text.split('\n').nth((line - 1) as usize).ok_or("line outside document")?;
    let content = raw.strip_suffix('\r').unwrap_or(raw);
    let scalar_offset = (character - 1) as usize;
    if scalar_offset > content.chars().count() { return Err("character outside line".into()); }
    let utf16_offset: usize = content.chars().take(scalar_offset).map(char::len_utf16).sum();
    Ok(json!({"line":line - 1,"character":utf16_offset}))
}

fn digest(text: &str) -> String { format!("{:x}", Sha256::digest(text.as_bytes())) }

fn validate_size(text: &str) -> Result<(), String> {
    if text.len() > MAX_DOCUMENT_BYTES { return Err("language service document exceeds 1 MiB".into()); }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn positions_handle_unicode_crlf_and_trailing_empty_lines() {
        let (document, _) = Document::open(Path::new("/workspace/a.rs"), "rust", "中🦀a\r\n\n".into()).unwrap();
        assert_eq!(document.position(1, 3).unwrap(), json!({"line":0,"character":3}));
        assert_eq!(document.position(1, 4).unwrap()["character"], 4);
        assert_eq!(document.position(3, 1).unwrap(), json!({"line":2,"character":0}));
        for (line, column) in [(0,1),(1,0),(1,5),(4,1)] {
            assert!(document.position(line, column).is_err());
        }
    }

    #[test]
    fn snapshots_reject_stale_responses_and_oversized_updates() {
        let (mut document, open) = Document::open(Path::new("/workspace/a b.rs"), "rust", "old".into()).unwrap();
        assert!(open["params"]["textDocument"]["uri"].as_str().unwrap().contains("a%20b.rs"));
        let initial = document.identity();
        assert!(document.matches(&initial, "old"));
        assert!(!document.matches(&initial, "new"));
        assert!(document.change("old".into()).unwrap().is_none());
        assert!(document.change("x".repeat(MAX_DOCUMENT_BYTES + 1)).is_err());
        assert_eq!(document.identity(), initial);
        assert_eq!(document.change("new".into()).unwrap().unwrap()["params"]["textDocument"]["version"], 2);
        assert!(!document.matches(&initial, "old"));
        assert_eq!(document.close()["method"], "textDocument/didClose");
    }
}
