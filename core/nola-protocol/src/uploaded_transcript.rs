// SPDX-License-Identifier: Apache-2.0
//! Bounded finalized-import manifest for the published transcript command.
//! Application resolves sealed uploads; the canonical commit binds original bodies to Parts.
use crate::{content_upload::ContentUploadOwner, ImportConversationTranscript};
use serde::{Deserialize, Serialize};
use std::{
    collections::{HashMap, HashSet},
    io::Write,
};

pub const UPLOADED_TRANSCRIPT_MANIFEST_MAX_BYTES: usize = 128 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct TranscriptPartUpload {
    pub upload_id: String,
    pub message_id: String,
    pub part_id: String,
    pub expected_utf8_bytes: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct UploadedTranscriptManifest {
    pub expected_conversation_revision: i64,
    pub transcript: ImportConversationTranscript,
    pub uploads: Vec<TranscriptPartUpload>,
}

impl UploadedTranscriptManifest {
    /// Structural admission only. Application must still verify persisted owner,
    /// seal state, bytes, idle conversation, replay and publication preconditions.
    pub fn validate(&self, maximum_upload_bytes: u64) -> Result<(), String> {
        let transcript = &self.transcript;
        if self.expected_conversation_revision < 1 {
            return Err("uploaded transcript requires a positive conversation revision".into());
        }
        valid_id(&transcript.import_id)?;
        valid_id(&transcript.conversation_id)?;
        if transcript.messages.is_empty()
            || transcript.messages.len() > 128
            || self.uploads.is_empty()
            || self.uploads.len() > 512
        {
            return Err("uploaded transcript exceeds bounded manifest cardinality".into());
        }
        let mut budget = EncodingBudget(UPLOADED_TRANSCRIPT_MANIFEST_MAX_BYTES);
        serde_json::to_writer(&mut budget, self).map_err(|_| {
            "uploaded transcript manifest exceeds its encoded byte budget".to_string()
        })?;
        transcript
            .expected_message_count
            .checked_add(transcript.messages.len())
            .and_then(|count| i64::try_from(count).ok())
            .ok_or("uploaded transcript ordinal overflow")?;
        let mut messages = HashSet::new();
        let mut parts = HashMap::new();
        for (index, message) in transcript.messages.iter().enumerate() {
            valid_id(&message.id)?;
            message.validate_finalized()?;
            if !messages.insert(message.id.as_str())
                || message.ordinal != (transcript.expected_message_count + index + 1) as i64
            {
                return Err(
                    "uploaded transcript has duplicate messages or invalid ordinals".into(),
                );
            }
            for part in &message.parts {
                valid_id(&part.id)?;
                if parts
                    .insert(part.id.as_str(), (message.id.as_str(), part))
                    .is_some()
                    || parts.len() > 1024
                {
                    return Err("uploaded transcript has duplicate or too many Parts".into());
                }
            }
        }
        let mut upload_ids = HashSet::new();
        let mut bound_parts = HashSet::new();
        for upload in &self.uploads {
            valid_id(&upload.upload_id)?;
            valid_id(&upload.message_id)?;
            valid_id(&upload.part_id)?;
            if upload.expected_utf8_bytes == 0 || upload.expected_utf8_bytes > maximum_upload_bytes
            {
                return Err("uploaded transcript content exceeds quota".into());
            }
            if !upload_ids.insert(upload.upload_id.as_str())
                || !bound_parts.insert(upload.part_id.as_str())
            {
                return Err("uploaded transcript contains duplicate upload bindings".into());
            }
            let (message_id, part) = parts
                .get(upload.part_id.as_str())
                .ok_or("upload refers to an undeclared Part")?;
            if *message_id != upload.message_id
                || part.kind != "text"
                || !part.content_text.is_empty()
                || part.content_json.is_some()
            {
                return Err(
                    "upload must bind its declared text Part without an inline body".into(),
                );
            }
        }
        Ok(())
    }

    pub fn owner_for(&self, binding: &TranscriptPartUpload) -> ContentUploadOwner {
        ContentUploadOwner {
            conversation_id: self.transcript.conversation_id.clone(),
            import_id: self.transcript.import_id.clone(),
            part_id: binding.part_id.clone(),
        }
    }
}

fn valid_id(value: &str) -> Result<(), String> {
    if value.is_empty()
        || value.trim() != value
        || value.len() > 128
        || value.chars().any(char::is_control)
    {
        return Err("invalid uploaded transcript identity".into());
    }
    Ok(())
}

struct EncodingBudget(usize);
impl Write for EncodingBudget {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.0 = self
            .0
            .checked_sub(bytes.len())
            .ok_or_else(|| std::io::Error::other("manifest byte budget exceeded"))?;
        Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn manifest() -> UploadedTranscriptManifest {
        serde_json::from_value(json!({
            "expected_conversation_revision":1,
            "transcript": {"import_id":"import","source":"backup_restore","conversation_id":"c",
                "expected_message_count":0,"imported_at_ms":2,"messages":[{
                    "id":"m","run_id":null,"role":"user","status":"completed","ordinal":1,
                    "created_at_ms":1,"completed_at_ms":2,"parts":[{
                        "id":"p","kind":"text","ordinal":1,"state":"completed","content_text":"",
                        "content_json":null,"content_digest":null,"created_at_ms":1,"updated_at_ms":2
                    }]
                }]},
            "uploads":[{"upload_id":"u","message_id":"m","part_id":"p","expected_utf8_bytes":10000000}]
        })).unwrap()
    }

    #[test]
    fn ten_million_byte_body_is_represented_by_a_small_owner_bound_manifest() {
        let value = manifest();
        value.validate(256 * 1024 * 1024).unwrap();
        let wire = serde_json::to_vec(&value).unwrap();
        assert!(wire.len() < 1024);
        assert_eq!(
            serde_json::from_slice::<UploadedTranscriptManifest>(&wire).unwrap(),
            value
        );
        let owner = value.owner_for(&value.uploads[0]);
        assert_eq!(
            (
                owner.conversation_id.as_str(),
                owner.import_id.as_str(),
                owner.part_id.as_str()
            ),
            ("c", "import", "p")
        );
    }

    #[test]
    fn uploaded_publication_requires_an_explicit_positive_conversation_revision() {
        for revision in [0, -1] {
            let mut value = manifest();
            value.expected_conversation_revision = revision;
            assert!(value.validate(256 * 1024 * 1024).is_err());
        }
        let mut wire = serde_json::to_value(manifest()).unwrap();
        wire.as_object_mut()
            .unwrap()
            .remove("expected_conversation_revision");
        assert!(serde_json::from_value::<UploadedTranscriptManifest>(wire).is_err());
    }

    #[test]
    fn binding_cannot_redirect_or_duplicate_a_part_or_smuggle_an_inline_body() {
        for mutation in 0..7 {
            let mut value = manifest();
            match mutation {
                0 => value.uploads[0].message_id = "other".into(),
                1 => value.uploads[0].part_id = "missing".into(),
                2 => value.uploads.push(value.uploads[0].clone()),
                3 => value.transcript.messages[0].parts[0].content_text = "inline".into(),
                4 => value.transcript.messages[0].parts[0].kind = "tool_call".into(),
                5 => value.transcript.messages[0].status = "streaming".into(),
                _ => value.transcript.expected_message_count = usize::MAX,
            }
            assert!(
                value.validate(256 * 1024 * 1024).is_err(),
                "mutation {mutation}"
            );
        }
    }

    #[test]
    fn encoded_budget_accounts_for_json_escaping_and_rejects_external_paths() {
        let mut value = manifest();
        value.transcript.messages[0].parts[0].content_json = Some("\0".repeat(24_000));
        assert!(value.validate(256 * 1024 * 1024).is_err());
        let mut wire = serde_json::to_value(manifest()).unwrap();
        wire["uploads"][0]["path"] = "/tmp/untrusted.db".into();
        assert!(serde_json::from_value::<UploadedTranscriptManifest>(wire).is_err());
        assert!(manifest().validate(100).is_err());
    }

    #[test]
    fn uploaded_text_rejects_a_second_body_even_when_the_json_is_small() {
        for body in ["{}", "null", "{\"text\":\"different inline body\"}"] {
            let mut value = manifest();
            value.transcript.messages[0].parts[0].content_json = Some(body.into());
            assert!(value.validate(256 * 1024 * 1024).is_err(), "{body}");
        }
    }

    #[test]
    fn unbound_inline_parts_still_obey_the_manifest_identity_contract() {
        for bad_id in [" ", " trailing ", "control\n", &"x".repeat(129)] {
            let mut value = manifest();
            let mut inline = value.transcript.messages[0].parts[0].clone();
            inline.id = bad_id.into();
            inline.ordinal = 2;
            inline.content_text = "small inline body".into();
            value.transcript.messages[0].parts.push(inline);
            assert!(value.validate(256 * 1024 * 1024).is_err());
        }
        let mut value = manifest();
        let mut inline = value.transcript.messages[0].parts[0].clone();
        inline.id = "inline-part".into();
        inline.ordinal = 2;
        inline.content_text = "small inline body".into();
        value.transcript.messages[0].parts.push(inline);
        value.validate(256 * 1024 * 1024).unwrap();
    }
}
