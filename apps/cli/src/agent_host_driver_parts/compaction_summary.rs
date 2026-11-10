// SPDX-License-Identifier: Apache-2.0
// Cold compaction is a bounded rolling checkpoint, not a second full-context request.
// Only the final checkpoint may cross the Host/Core completion boundary.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CompactionWitnessRestoreError {
    Missing,
    Unreadable,
    TooLarge,
    Malformed,
    SessionChanged,
    ConfigurationChanged,
    InvalidRequest,
    Superseded,
}

impl CompactionWitnessRestoreError {
    fn code(self) -> &'static str {
        match self {
            Self::Missing => "no_request_snapshot",
            Self::Unreadable => "request_snapshot_unreadable",
            Self::TooLarge => "request_snapshot_too_large",
            Self::Malformed => "request_snapshot_malformed",
            Self::SessionChanged => "session_changed",
            Self::ConfigurationChanged => "configuration_changed",
            Self::InvalidRequest => "request_snapshot_invalid_request",
            Self::Superseded => "request_snapshot_superseded",
        }
    }
}

const MAX_COMPACTION_WITNESS_BYTES: usize = 32 * 1024 * 1024;

struct BoundedSnapshotBytes(Vec<u8>);
impl std::io::Write for BoundedSnapshotBytes {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        if self.0.len().saturating_add(bytes.len()) > MAX_COMPACTION_WITNESS_BYTES {
            return Err(std::io::Error::new(std::io::ErrorKind::InvalidData, "compaction snapshot exceeds restore limit"));
        }
        self.0.extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> { Ok(()) }
}

impl CompactionRequestWitness {
    fn persist(&self, root: &std::path::Path) -> std::io::Result<()> {
        let result = self.persist_current(root);
        if result.is_err() {
            // An older cache witness must not masquerade as the latest request
            // after a large/failed replacement. The tombstone has no body or
            // credentials and is committed using the same atomic durability path.
            if Self::invalidate(root, &self.session_id, &self.configuration_identity).is_err() {
                eprintln!("nola_compaction_snapshot_invalidation_failed: stale disk witness may remain");
            }
        }
        result
    }

    fn invalidate(root: &std::path::Path, session: &str, configuration_identity: &str) -> std::io::Result<()> {
        let tombstone = json!({"state":"superseded", "session_id":session,
            "configuration_identity":configuration_identity});
        persist_compaction_bytes(root, tombstone.to_string().as_bytes())
    }

    fn persist_current(&self, root: &std::path::Path) -> std::io::Result<()> {
        let value = json!({"state":"request", "session_id":self.session_id,
            "configuration_identity":self.configuration_identity,
            "context_window":self.context_window, "request":self.request.durable_value(),
            "usage":self.usage});
        let mut bytes = BoundedSnapshotBytes(Vec::new());
        serde_json::to_writer(&mut bytes, &value).map_err(std::io::Error::other)?;
        persist_compaction_bytes(root, &bytes.0)
    }

    fn restore(root: &std::path::Path, session: &str, config: &LlmConfig) -> Result<Self, CompactionWitnessRestoreError> {
        use std::io::Read;
        use CompactionWitnessRestoreError as Error;
        let path = root.join("compaction-request.json");
        // A single bounded recent request, not an ever-growing request archive.
        const MAX_BYTES: u64 = MAX_COMPACTION_WITNESS_BYTES as u64;
        let file = std::fs::File::open(path).map_err(|error| {
            if error.kind() == std::io::ErrorKind::NotFound { Error::Missing } else { Error::Unreadable }
        })?;
        if file.metadata().map_err(|_| Error::Unreadable)?.len() > MAX_BYTES {
            return Err(Error::TooLarge);
        }
        // Bound the read itself: a file may grow after metadata was observed.
        let mut bytes = Vec::new();
        file.take(MAX_BYTES + 1).read_to_end(&mut bytes).map_err(|_| Error::Unreadable)?;
        if bytes.len() as u64 > MAX_BYTES { return Err(Error::TooLarge); }
        let value: Value = serde_json::from_slice(&bytes).map_err(|_| Error::Malformed)?;
        let stored_session = value["session_id"].as_str().ok_or(Error::Malformed)?;
        let identity = value["configuration_identity"].as_str().ok_or(Error::Malformed)?;
        if stored_session != session { return Err(Error::SessionChanged); }
        if identity != crate::llm::request_configuration_identity(config) {
            return Err(Error::ConfigurationChanged);
        }
        if value["state"] == "superseded" { return Err(Error::Superseded); }
        Ok(Self {
            request: std::sync::Arc::new(crate::llm::PreparedModelRequest::restore(&value["request"], config).ok_or(Error::InvalidRequest)?),
            session_id: session.into(), configuration_identity: identity.into(),
            context_window: value["context_window"].as_u64().and_then(|v| usize::try_from(v).ok())
                .filter(|v| *v > 0).ok_or(Error::Malformed)?,
            usage: serde_json::from_value(value["usage"].clone()).map_err(|_| Error::Malformed)?,
        })
    }
}

fn persist_compaction_bytes(root: &std::path::Path, bytes: &[u8]) -> std::io::Result<()> {
    use std::io::Write;
    std::fs::create_dir_all(root)?;
    let staging = root.join(format!(".snapshot-{}", uuid::Uuid::new_v4()));
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)] {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let result = (|| {
        let mut file = options.open(&staging)?;
        #[cfg(windows)]
        nola_credential_host::windows_permissions::protect_private_path(&staging).map_err(std::io::Error::other)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        drop(file);
        let destination = root.join("compaction-request.json");
        #[cfg(windows)] {
            use std::os::windows::ffi::OsStrExt;
            use windows_sys::Win32::Storage::FileSystem::{MoveFileExW, MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH};
            let source = staging.as_os_str().encode_wide().chain(Some(0)).collect::<Vec<_>>();
            let destination = destination.as_os_str().encode_wide().chain(Some(0)).collect::<Vec<_>>();
            // Windows does not support opening a directory with File::open for
            // fsync. Commit its rename through the native write-through operation.
            if unsafe { MoveFileExW(source.as_ptr(), destination.as_ptr(), MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH) } == 0 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        }
        #[cfg(not(windows))] {
            std::fs::rename(&staging, destination)?;
            std::fs::File::open(root)?.sync_all()
        }
    })();
    if result.is_err() { let _ = std::fs::remove_file(&staging); }
    result
}

use nola_core::compaction_summary::CompactionSummaryPlan;

#[cfg(test)]
mod incremental_compaction_tests {
    use super::*;

    #[test]
    fn witness_write_limit_matches_restore_and_rejects_before_growth() {
        use std::io::Write;
        let mut bytes = BoundedSnapshotBytes(vec![0; MAX_COMPACTION_WITNESS_BYTES - 2]);
        bytes.write_all(b"ok").unwrap();
        assert!(bytes.write_all(b"too large").is_err());
        assert_eq!(bytes.0.len(), MAX_COMPACTION_WITNESS_BYTES);
    }

    #[test]
    fn oversized_latest_witness_atomically_invalidates_old_restart_cache() {
        let root = tempfile::tempdir().unwrap();
        let cfg = super::effect_delivery_start_barrier_tests::test_llm("http://127.0.0.1:1/v1".into());
        let identity = crate::llm::request_configuration_identity(&cfg);
        let make_request = |body: Value| {
            let value = json!({"version":1, "configuration_identity":identity,
                "route_identity":crate::llm::model_route_identity(&cfg),
                "started_at_unix":std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs(),
                "completed_response":[], "body":body,
                "effective":{"provider":cfg.provider,"output_field":cfg.chat_output_token_field,
                    "output":cfg.max_output_tokens,"temperature":cfg.temperature,"top_p":cfg.top_p,
                    "thinking":cfg.thinking_enabled,"reasoning":cfg.reasoning_selection,
                    "generation":cfg.generation_options,"sampling":cfg.sampling_override,
                    "parallel":cfg.parallel_tool_calls,"cache":cfg.openai_prompt_cache,
                    "cache_key":cfg.openai_prompt_cache_key,"cache_prefix":cfg.openai_prompt_cache_key_prefix,
                    "cache_retention":cfg.openai_prompt_cache_retention,"no_fallbacks":true}});
            std::sync::Arc::new(crate::llm::PreparedModelRequest::restore(&value, &cfg).unwrap())
        };
        let mut witness = CompactionRequestWitness {
            request:make_request(json!({"messages":[{"role":"user","content":"old request"}]})),
            session_id:"isolated-session".into(), configuration_identity:identity.clone(),
            context_window:128000, usage:crate::llm::LlmUsage::default(),
        };
        witness.persist(root.path()).unwrap();
        assert!(CompactionRequestWitness::restore(root.path(), "isolated-session", &cfg).is_ok());
        CompactionRequestWitness::invalidate(root.path(), "isolated-session", &identity).unwrap();
        assert_eq!(CompactionRequestWitness::restore(root.path(), "isolated-session", &cfg).err(), Some(CompactionWitnessRestoreError::Superseded));
        witness.persist(root.path()).unwrap();
        assert!(CompactionRequestWitness::restore(root.path(), "isolated-session", &cfg).is_ok());
        witness.request = make_request(json!({"messages":[{"role":"user",
            "content":"x".repeat(MAX_COMPACTION_WITNESS_BYTES)}]}));
        assert!(witness.persist(root.path()).is_err());
        assert_eq!(CompactionRequestWitness::restore(root.path(), "isolated-session", &cfg).err(), Some(CompactionWitnessRestoreError::Superseded));
        let tombstone = std::fs::read(root.path().join("compaction-request.json")).unwrap();
        assert!(tombstone.len() < 1024);
        assert_eq!(std::fs::read_dir(root.path()).unwrap().count(), 1, "no staging leaks");
        witness.request = make_request(json!({"messages":[]}));
        witness.persist(root.path()).unwrap();
        assert!(CompactionRequestWitness::restore(root.path(), "isolated-session", &cfg).is_ok());
    }

}

fn compaction_maintenance_decision(choice: &crate::llm::StreamedChoice, target: u64,
    continuations: usize, rewrites: usize, can_continue: bool, can_rewrite: bool)
    -> nola_domain::compaction_maintenance::CompactionMaintenanceDecision {
    nola_core::compaction_maintenance::evaluate_compaction_maintenance(&nola_domain::compaction_maintenance::CompactionMaintenanceFacts {
        summary: choice.content.clone(), target_tokens: target as usize,
        output_limited: matches!(choice.finish_reason.as_str(), "length" | "max_tokens" | "max_output_tokens"),
        has_tool_calls: !choice.tool_calls.is_empty(), completed: matches!(choice.finish_reason.as_str(), "stop" | "end_turn" | "completed"),
        cancelled: false, continuation_count: continuations, rewrite_count: rewrites, can_continue, can_rewrite,
    })
}

#[cfg(test)]
mod atomic_snapshot_tests {
    use super::*;
    #[test]
    fn compaction_snapshot_replacement_commits_and_leaves_no_staging_files() {
        let directory = tempfile::tempdir().unwrap();
        for body in [b"first".as_slice(), b"replacement".as_slice()] {
            persist_compaction_bytes(directory.path(), body).unwrap();
            assert_eq!(std::fs::read(directory.path().join("compaction-request.json")).unwrap(), body);
            assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 1);
        }
    }
}
