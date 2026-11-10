// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Adapts native code-intelligence results to Nola's stable Host contract.
// - Boundary: Contains no process discovery, MCP transport, or engine lifecycle.

use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::io::Read;
#[cfg(unix)]
use std::os::unix::ffi::OsStrExt as _;
use std::path::{Path, PathBuf};

const MAX_PROVENANCE_FILE_BYTES: u64 = 8 * 1_024 * 1_024;
const MAX_PROVENANCE_FACTS: usize = 64;

/// Adds independently observed repository evidence to a native code-intelligence
/// result before it becomes model-visible. The provider cannot self-attest
/// freshness: results that conflict with current workspace evidence are
/// quarantined so stale code facts cannot remain model-visible as answers.
pub(crate) fn attach_result_provenance(capability_id: &str, workspace: &Path, output: &mut Value) {
    let Some(operation) = nola_core::code_intelligence::operation_for_capability_id(capability_id)
    else {
        return;
    };
    if !operation.is_read_only() || !operation.requires_code_fact_attestation() {
        return;
    }
    let workspace = workspace
        .canonicalize()
        .unwrap_or_else(|_| workspace.to_path_buf());
    let repository_root = workspace.to_string_lossy().into_owned();
    let current_revision = git_revision(&workspace);
    let provider_revision = find_string(
        output,
        &["repository_revision", "git_commit", "commit_sha"],
        0,
    );
    let mut raw_facts = Vec::new();
    let mut fact_limit_exceeded = false;
    collect_code_facts(output, 0, &mut raw_facts, &mut fact_limit_exceeded);
    let facts = raw_facts
        .into_iter()
        .map(|raw| {
            let candidate = PathBuf::from(&raw.path);
            let absolute = if candidate.is_absolute() {
                candidate
            } else {
                workspace.join(candidate)
            };
            let canonical = absolute.canonicalize().ok();
            let inside_workspace = canonical
                .as_ref()
                .is_some_and(|path| path.starts_with(&workspace));
            let path_exists =
                inside_workspace && canonical.as_ref().is_some_and(|path| path.is_file());
            let bytes = path_exists
                .then(|| canonical.as_ref().expect("checked path"))
                .filter(|path| {
                    path.metadata()
                        .is_ok_and(|metadata| metadata.len() <= MAX_PROVENANCE_FILE_BYTES)
                })
                .and_then(|path| std::fs::read(path).ok());
            let current_digest = bytes
                .as_ref()
                .map(|bytes| format!("{:x}", Sha256::digest(bytes)));
            let evidence_matches = match (raw.evidence.as_deref(), bytes.as_deref()) {
                (Some(evidence), Some(bytes)) => std::str::from_utf8(bytes)
                    .ok()
                    .map(|source| source.contains(evidence.trim())),
                _ => None,
            };
            let symbol_matches = match (raw.symbol.as_deref(), bytes.as_deref()) {
                (Some(symbol), Some(bytes)) => std::str::from_utf8(bytes).ok().and_then(|source| {
                    symbol_leaf(symbol).map(|leaf| contains_symbol(source, leaf))
                }),
                _ => None,
            };
            let relative = canonical
                .as_ref()
                .and_then(|path| path.strip_prefix(&workspace).ok())
                .unwrap_or(Path::new(&raw.path))
                .to_string_lossy()
                .replace(std::path::MAIN_SEPARATOR, "/");
            nola_core::code_intelligence::CodeFactProvenance {
                repository_root: repository_root.clone(),
                path: relative,
                symbol: raw.symbol,
                path_exists,
                current_content_sha256: current_digest,
                provider_content_sha256: raw.provider_digest,
                evidence_matches_current_source: evidence_matches,
                symbol_matches_current_source: symbol_matches,
            }
        })
        .collect();
    let mut provenance = nola_core::code_intelligence::assess_code_intelligence_provenance(
        operation,
        repository_root,
        current_revision,
        provider_revision,
        facts,
    );
    if fact_limit_exceeded {
        if provenance.freshness == nola_core::code_intelligence::CodeFactFreshness::Current {
            provenance.freshness =
                nola_core::code_intelligence::CodeFactFreshness::RequiresVerification;
        }
        provenance.requires_current_state_verification = true;
        provenance
            .reasons
            .push("provider_result_exceeds_provenance_fact_limit".to_string());
        provenance.reasons.sort();
        provenance.reasons.dedup();
    }
    // Keep candidates that require a current-source follow-up visible together
    // with their provenance. Only confirmed conflicts and unverifiable overflow
    // are removed from the model-visible envelope.
    let quarantine = fact_limit_exceeded
        || provenance.freshness == nola_core::code_intelligence::CodeFactFreshness::Stale;
    let requires_verification = provenance.requires_current_state_verification;
    let encoded_provenance = serde_json::to_value(provenance).unwrap_or_else(|_| {
        json!({
            "freshness": "requires_verification",
            "requires_current_state_verification": true,
            "reasons": ["provenance_serialization_failed"]
        })
    });
    if quarantine {
        *output = json!({
            "type": "code_intelligence_result_quarantined",
            "provider_result_quarantined": true,
            "message": "The provider result conflicts with the current workspace and must be refreshed before use.",
            "_nola_code_intelligence_provenance": encoded_provenance
        });
        return;
    }
    if let Value::Object(object) = output {
        object.insert(
            "_nola_code_intelligence_provenance".to_string(),
            encoded_provenance,
        );
        if requires_verification {
            if let Some(capability_id) = operation.verification_follow_up_capability_id() {
                object.insert(
                    "_nola_code_intelligence_follow_up".to_string(),
                    json!({
                        "capability_id": capability_id,
                        "argument": "qualified_name",
                        "source": "provider_result"
                    }),
                );
            }
        }
    }
}

fn symbol_leaf(symbol: &str) -> Option<&str> {
    let without_signature = symbol.split('(').next()?.trim();
    let without_terminal_generics = without_signature
        .strip_suffix('>')
        .and_then(|value| value.rfind("::<").map(|start| &value[..start]))
        .unwrap_or(without_signature);
    let leaf = without_terminal_generics
        .rsplit([':', '.', '#', '/', '\\'])
        .find(|segment| !segment.trim().is_empty())?
        .trim()
        .split('<')
        .next()?
        .trim();
    (!leaf.is_empty()
        && leaf
            .chars()
            .all(|character| character == '_' || character.is_alphanumeric()))
    .then_some(leaf)
}

fn contains_symbol(source: &str, symbol: &str) -> bool {
    source.match_indices(symbol).any(|(start, matched)| {
        let end = start + matched.len();
        let is_identifier = |character: char| character == '_' || character.is_alphanumeric();
        let left_is_clear = source[..start]
            .chars()
            .next_back()
            .is_none_or(|character| !is_identifier(character));
        let right_is_clear = source[end..]
            .chars()
            .next()
            .is_none_or(|character| !is_identifier(character));
        left_is_clear && right_is_clear
    })
}

#[derive(Debug)]
struct RawCodeFact {
    path: String,
    symbol: Option<String>,
    provider_digest: Option<String>,
    evidence: Option<String>,
}

fn collect_code_facts(
    value: &Value,
    depth: usize,
    facts: &mut Vec<RawCodeFact>,
    fact_limit_exceeded: &mut bool,
) {
    if depth > 12 || *fact_limit_exceeded {
        return;
    }
    match value {
        Value::Object(object) => {
            let path = ["file_path", "file", "path"]
                .into_iter()
                .find_map(|key| object.get(key).and_then(Value::as_str))
                .map(str::trim)
                .filter(|path| !path.is_empty());
            if let Some(path) = path {
                if facts.len() >= MAX_PROVENANCE_FACTS {
                    *fact_limit_exceeded = true;
                    return;
                }
                let symbol = ["qualified_name", "qn", "symbol", "name"]
                    .into_iter()
                    .find_map(|key| object.get(key).and_then(Value::as_str))
                    .map(str::trim)
                    .filter(|symbol| !symbol.is_empty())
                    .map(str::to_string);
                let provider_digest = ["content_sha256", "content_digest", "source_digest"]
                    .into_iter()
                    .find_map(|key| object.get(key).and_then(Value::as_str))
                    .map(str::trim)
                    .filter(|digest| {
                        digest.len() == 64 && digest.bytes().all(|byte| byte.is_ascii_hexdigit())
                    })
                    .map(|digest| digest.to_ascii_lowercase());
                let evidence = ["source", "snippet", "code"]
                    .into_iter()
                    .find_map(|key| object.get(key).and_then(Value::as_str))
                    .map(str::trim)
                    .filter(|source| !source.is_empty())
                    .map(str::to_string);
                facts.push(RawCodeFact {
                    path: path.to_string(),
                    symbol,
                    provider_digest,
                    evidence,
                });
            }
            for nested in object.values() {
                collect_code_facts(nested, depth + 1, facts, fact_limit_exceeded);
            }
        }
        Value::Array(values) => {
            for nested in values {
                collect_code_facts(nested, depth + 1, facts, fact_limit_exceeded);
            }
        }
        _ => {}
    }
}

fn find_string(value: &Value, keys: &[&str], depth: usize) -> Option<String> {
    if depth > 12 {
        return None;
    }
    match value {
        Value::Object(object) => keys
            .iter()
            .find_map(|key| object.get(*key).and_then(Value::as_str))
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string)
            .or_else(|| {
                object
                    .values()
                    .find_map(|value| find_string(value, keys, depth + 1))
            }),
        Value::Array(values) => values
            .iter()
            .find_map(|value| find_string(value, keys, depth + 1)),
        _ => None,
    }
}

fn git_revision(workspace: &Path) -> Option<String> {
    let output = std::process::Command::new("git")
        .arg("-C")
        .arg(workspace)
        .args(["rev-parse", "HEAD"])
        .output()
        .ok()?;
    let revision = output
        .status
        .success()
        .then(|| String::from_utf8_lossy(&output.stdout).trim().to_string())
        .filter(|revision| !revision.is_empty())?;
    let status = std::process::Command::new("git")
        .arg("-C")
        .arg(workspace)
        .args(["status", "--porcelain=v1", "-z", "--untracked-files=all"])
        .output()
        .ok()?;
    if !status.status.success() {
        return None;
    }
    if !status.stdout.is_empty() {
        Some(dirty_git_revision(workspace, &revision, &status.stdout))
    } else {
        Some(revision)
    }
}

fn dirty_git_revision(workspace: &Path, revision: &str, porcelain: &[u8]) -> String {
    let mut digest = Sha256::new();
    digest.update(porcelain);
    for record in porcelain
        .split(|byte| *byte == 0)
        .filter(|record| !record.is_empty())
    {
        // In porcelain v1 -z, ordinary records are `XY path`; rename/copy
        // records add a second NUL-delimited path. Hash either shape so the
        // identity follows file contents instead of only the set of paths.
        let path_bytes = if record.len() > 3 && record[2] == b' ' {
            &record[3..]
        } else {
            record
        };
        digest.update((path_bytes.len() as u64).to_le_bytes());
        digest.update(path_bytes);
        #[cfg(unix)]
        let relative_path = PathBuf::from(std::ffi::OsStr::from_bytes(path_bytes));
        #[cfg(not(unix))]
        let relative_path = PathBuf::from(String::from_utf8_lossy(path_bytes).as_ref());
        let path = workspace.join(relative_path);
        let Ok(metadata) = std::fs::symlink_metadata(&path) else {
            digest.update(b"missing");
            continue;
        };
        if metadata.file_type().is_symlink() {
            match std::fs::read_link(&path) {
                Ok(target) => digest.update(target.to_string_lossy().as_bytes()),
                Err(_) => digest.update(b"unreadable-symlink"),
            }
            continue;
        }
        if metadata.is_dir() {
            // A dirty gitlink is reported by the parent repository as the
            // same `M path` record while the nested checkout can move between
            // commits or change its worktree. Bind both cases to the nested
            // repository identity; otherwise an old retained index can appear
            // current after submodule code changes.
            match nested_git_revision(&path) {
                Some(nested_revision) => {
                    digest.update(b"nested-git-revision");
                    digest.update(nested_revision.as_bytes());
                }
                None => digest.update(b"non-file"),
            }
            continue;
        }
        if !metadata.is_file() {
            digest.update(b"non-file");
            continue;
        }
        let Ok(mut file) = std::fs::File::open(&path) else {
            digest.update(b"unreadable-file");
            continue;
        };
        let mut buffer = [0u8; 64 * 1024];
        loop {
            match file.read(&mut buffer) {
                Ok(0) => break,
                Ok(count) => digest.update(&buffer[..count]),
                Err(_) => {
                    digest.update(b"read-failed");
                    break;
                }
            }
        }
    }
    format!("{revision}:dirty:{:x}", digest.finalize())
}

fn nested_git_revision(path: &Path) -> Option<String> {
    let output = std::process::Command::new("git")
        .arg("-C")
        .arg(path)
        .args(["rev-parse", "--show-toplevel"])
        .output()
        .ok()?;
    let top_level = output
        .status
        .success()
        .then(|| PathBuf::from(String::from_utf8_lossy(&output.stdout).trim()))?;
    let expected = path.canonicalize().ok()?;
    let observed = top_level.canonicalize().ok()?;
    (observed == expected).then(|| git_revision(path)).flatten()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{env, time::SystemTime};

    fn provenance_fixture() -> PathBuf {
        static NEXT_FIXTURE_ID: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let unique = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let sequence = NEXT_FIXTURE_ID.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let root = env::temp_dir().join(format!(
            "nola-code-provenance-{}-{unique}-{sequence}",
            std::process::id()
        ));
        std::fs::create_dir_all(root.join("src")).unwrap();
        std::fs::write(
            root.join("src/lib.rs"),
            "pub fn durable_answer() -> u32 { 42 }\n",
        )
        .unwrap();
        root
    }

    fn commit_provenance_fixture(root: &Path) -> String {
        for arguments in [
            vec!["init", "--quiet"],
            vec!["config", "user.email", "nola-test@example.invalid"],
            vec!["config", "user.name", "Nola Test"],
            vec!["add", "src/lib.rs"],
            vec!["commit", "--quiet", "-m", "fixture"],
        ] {
            assert!(std::process::Command::new("git")
                .arg("-C")
                .arg(root)
                .args(arguments)
                .status()
                .unwrap()
                .success());
        }
        git_revision(root).expect("committed repository revision")
    }

    #[test]
    fn provider_result_with_matching_path_symbol_and_digest_is_current() {
        let root = provenance_fixture();
        let revision = commit_provenance_fixture(&root);
        let digest = format!(
            "{:x}",
            Sha256::digest(std::fs::read(root.join("src/lib.rs")).unwrap())
        );
        let mut output = json!({
            "repository_revision": revision,
            "structuredContent": {"results": [{
                "file_path": "src/lib.rs",
                "qualified_name": "fixture::durable_answer",
                "content_digest": digest,
                "source": "pub fn durable_answer() -> u32 { 42 }"
            }]}
        });
        attach_result_provenance("codebase_search", &root, &mut output);
        let provenance = &output["_nola_code_intelligence_provenance"];
        assert_eq!(provenance["freshness"], "current");
        assert_eq!(provenance["requires_current_state_verification"], false);
        assert_eq!(provenance["facts"][0]["path"], "src/lib.rs");
        assert_eq!(provenance["facts"][0]["symbol"], "fixture::durable_answer");
        assert_eq!(
            output["structuredContent"]["results"][0]["source"],
            "pub fn durable_answer() -> u32 { 42 }"
        );
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn provider_result_marks_path_drift_and_source_conflict_stale() {
        let root = provenance_fixture();
        let mut output = json!({
            "structuredContent": {"results": [{
                "file": "src/lib.rs",
                "qn": "fixture::removed_answer",
                "content_digest": "00".repeat(32),
                "snippet": "pub fn removed_answer()"
            }, {
                "file": "src/missing.rs",
                "qn": "fixture::missing"
            }]}
        });
        attach_result_provenance("codebase_read_symbol", &root, &mut output);
        let provenance = &output["_nola_code_intelligence_provenance"];
        assert_eq!(provenance["freshness"], "stale");
        assert_eq!(provenance["requires_current_state_verification"], true);
        let reasons = provenance["reasons"].as_array().unwrap();
        for reason in [
            "code_fact_path_missing",
            "code_fact_content_digest_drifted",
            "code_fact_summary_conflicts_with_current_source",
        ] {
            assert!(reasons.iter().any(|value| value == reason));
        }
        assert_eq!(output["type"], "code_intelligence_result_quarantined");
        assert_eq!(output["provider_result_quarantined"], true);
        assert!(output.get("structuredContent").is_none());
        assert!(!output.to_string().contains("pub fn removed_answer()"));
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn matching_digest_cannot_attest_a_symbol_missing_from_current_source() {
        let root = provenance_fixture();
        let digest = format!(
            "{:x}",
            Sha256::digest(std::fs::read(root.join("src/lib.rs")).unwrap())
        );
        let mut output = json!({
            "structuredContent": {"results": [{
                "file_path": "src/lib.rs",
                "qualified_name": "fixture::hallucinated_answer",
                "content_digest": digest,
                "source": "pub fn durable_answer() -> u32 { 42 }"
            }]}
        });

        attach_result_provenance("codebase_read_symbol", &root, &mut output);

        assert_eq!(output["type"], "code_intelligence_result_quarantined");
        let provenance = &output["_nola_code_intelligence_provenance"];
        assert_eq!(provenance["freshness"], "stale");
        assert_eq!(
            provenance["facts"][0]["symbol_matches_current_source"],
            false
        );
        assert!(provenance["reasons"]
            .as_array()
            .unwrap()
            .iter()
            .any(|reason| reason == "code_fact_symbol_missing_from_current_source"));
        assert!(output.get("structuredContent").is_none());
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn symbol_evidence_uses_the_qualified_leaf_with_identifier_boundaries() {
        assert_eq!(symbol_leaf("crate::module::answer(u32)"), Some("answer"));
        assert_eq!(symbol_leaf("crate::module::answer::<u32>"), Some("answer"));
        assert_eq!(symbol_leaf("Type<T>::answer"), Some("answer"));
        assert!(contains_symbol("fn answer() {}", "answer"));
        assert!(!contains_symbol("fn unanswered() {}", "answer"));
        assert_eq!(symbol_leaf("crate::operator+"), None);
    }

    #[test]
    fn provider_result_quarantines_facts_beyond_the_verification_limit() {
        let root = provenance_fixture();
        let digest = format!(
            "{:x}",
            Sha256::digest(std::fs::read(root.join("src/lib.rs")).unwrap())
        );
        let mut results = (0..MAX_PROVENANCE_FACTS)
            .map(|_| {
                json!({
                    "file_path": "src/lib.rs",
                    "qualified_name": "fixture::durable_answer",
                    "content_digest": digest,
                    "source": "pub fn durable_answer() -> u32 { 42 }"
                })
            })
            .collect::<Vec<_>>();
        results.push(json!({
            "file_path": "src/deleted.rs",
            "qualified_name": "fixture::unvalidated_stale_fact",
            "source": "fn unvalidated_stale_fact()"
        }));
        let mut output = json!({"structuredContent": {"results": results}});

        attach_result_provenance("codebase_search", &root, &mut output);

        assert_eq!(output["type"], "code_intelligence_result_quarantined");
        assert_eq!(output["provider_result_quarantined"], true);
        assert_eq!(
            output["_nola_code_intelligence_provenance"]["freshness"],
            "requires_verification"
        );
        assert_eq!(
            output["_nola_code_intelligence_provenance"]["facts"]
                .as_array()
                .unwrap()
                .len(),
            MAX_PROVENANCE_FACTS
        );
        assert!(output["_nola_code_intelligence_provenance"]["reasons"]
            .as_array()
            .unwrap()
            .iter()
            .any(|reason| reason == "provider_result_exceeds_provenance_fact_limit"));
        assert!(output.get("structuredContent").is_none());
        assert!(!output.to_string().contains("unvalidated_stale_fact"));
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn graph_summary_without_fact_identity_requires_verification() {
        let root = provenance_fixture();
        let mut output = json!({"structuredContent": {"summary": "UNVERIFIED-GRAPH-SUMMARY"}});
        attach_result_provenance("codebase_architecture", &root, &mut output);
        let provenance = &output["_nola_code_intelligence_provenance"];
        assert_eq!(provenance["freshness"], "requires_verification");
        assert_eq!(provenance["requires_current_state_verification"], true);
        assert!(output.get("type").is_none());
        assert!(output.to_string().contains("UNVERIFIED-GRAPH-SUMMARY"));
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn matching_file_evidence_without_optional_provider_revision_remains_visible() {
        let root = provenance_fixture();
        let digest = format!(
            "{:x}",
            Sha256::digest(std::fs::read(root.join("src/lib.rs")).unwrap())
        );
        let mut output = json!({
            "structuredContent": {"results": [{
                "file_path": "src/lib.rs",
                "qualified_name": "fixture::durable_answer",
                "content_digest": digest,
                "source": "pub fn durable_answer() -> u32 { 42 }",
                "provider_only_note": "UNBOUND-INDEX-PAYLOAD"
            }]}
        });

        attach_result_provenance("codebase_search", &root, &mut output);

        assert!(output.get("type").is_none());
        assert_eq!(
            output["_nola_code_intelligence_provenance"]["freshness"],
            "current"
        );
        assert!(output.to_string().contains("UNBOUND-INDEX-PAYLOAD"));
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn code_snippet_shape_without_revision_or_digest_is_host_attested() {
        let root = provenance_fixture();
        let mut output = json!({
            "structuredContent": {
                "qualified_name": "fixture::durable_answer",
                "file_path": "src/lib.rs",
                "source": "pub fn durable_answer() -> u32 { 42 }"
            }
        });

        attach_result_provenance("codebase_read_symbol", &root, &mut output);

        assert!(output.get("type").is_none());
        assert_eq!(
            output["_nola_code_intelligence_provenance"]["freshness"],
            "current"
        );
        assert_eq!(
            output["_nola_code_intelligence_provenance"]["facts"][0]
                ["evidence_matches_current_source"],
            true
        );
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn default_tree_search_shape_stays_available_for_read_symbol_followup() {
        let root = provenance_fixture();
        let mut output = json!({
            "structuredContent": {
                "columns": ["qn", "label", "file", "lines", "in", "out"],
                "groups": [{
                    "prefix": "fixture",
                    "rows": [["durable_answer", "Function", "src/lib.rs", "1-1", 0, 0]]
                }],
                "total": 1,
                "has_more": false
            }
        });

        attach_result_provenance("codebase_search", &root, &mut output);

        assert!(output.get("type").is_none());
        assert_eq!(
            output["_nola_code_intelligence_provenance"]["freshness"],
            "requires_verification"
        );
        assert_eq!(
            output["_nola_code_intelligence_follow_up"]["capability_id"],
            "codebase_read_symbol"
        );
        assert_eq!(
            output["_nola_code_intelligence_follow_up"]["argument"],
            "qualified_name"
        );
        assert_eq!(output["structuredContent"]["total"], 1);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn provider_metadata_operations_are_not_misclassified_as_code_facts() {
        let root = provenance_fixture();
        for capability_id in [
            "codebase_index_status",
            "codebase_projects",
            "codebase_schema",
        ] {
            let mut output = json!({
                "structuredContent": {
                    "path": "provider-cache.sqlite",
                    "name": "provider metadata"
                }
            });
            let expected = output.clone();
            attach_result_provenance(capability_id, &root, &mut output);
            assert_eq!(output, expected, "{capability_id}");
        }
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn matching_revision_and_digest_without_optional_source_excerpt_is_current() {
        let root = provenance_fixture();
        let revision = commit_provenance_fixture(&root);
        let digest = format!(
            "{:x}",
            Sha256::digest(std::fs::read(root.join("src/lib.rs")).unwrap())
        );
        let mut output = json!({
            "repository_revision": revision,
            "structuredContent": {"results": [{
                "file_path": "src/lib.rs",
                "qualified_name": "fixture::durable_answer",
                "content_digest": digest,
                "provider_only_note": "NO-SOURCE-EVIDENCE"
            }]}
        });

        attach_result_provenance("codebase_read_symbol", &root, &mut output);

        assert!(output.get("type").is_none());
        assert_eq!(
            output["_nola_code_intelligence_provenance"]["freshness"],
            "current"
        );
        assert!(output.to_string().contains("NO-SOURCE-EVIDENCE"));
        let _ = std::fs::remove_dir_all(root);
    }
}
