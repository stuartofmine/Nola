// SPDX-License-Identifier: Apache-2.0
//! Software Update manifest checker, target detection, package downloader, and SHA-256 verifier.

use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::Path;
use std::time::Duration;

use nola_tools::web::resolve_and_validate_public_url;
use nola_types::{new_id, sha256_hex};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

const MAX_MANIFEST_BYTES: u64 = 64 * 1024;
const MAX_PACKAGE_BYTES: u64 = 512 * 1024 * 1024; // 512 MiB cap

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UpdateManifest {
    pub version: String,
    pub download_url: String,
    pub sha256: String,
    #[serde(default)]
    pub notes: String,
    /// Optional Ed25519 signature (hex or base64) for signed release manifests.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub signature: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UpdateCheckResult {
    /// Truthful status: `"unconfigured"`, `"up_to_date"`, or `"update_available"`.
    pub status: String,
    pub current_version: String,
    pub target: String,
    pub update_available: bool,
    pub latest: Option<UpdateManifest>,
}

/// Resolve the current platform update target, honoring `NOLA_UPDATE_TARGET` first
/// (sanitized strictly to `[A-Za-z0-9_-]`).
pub fn current_update_target() -> String {
    if let Ok(raw) = std::env::var("NOLA_UPDATE_TARGET") {
        let clean: String = raw
            .trim()
            .chars()
            .filter(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-'))
            .collect();
        if !clean.is_empty() {
            return clean;
        }
    }
    format!("{}-{}", std::env::consts::OS, std::env::consts::ARCH)
}

/// Compare two semver strings (`major.minor.patch`, stripping optional `-prerelease`/`+build`).
pub fn is_newer_version(current: &str, candidate: &str) -> bool {
    let parse = |s: &str| -> (u64, u64, u64) {
        let core = s
            .trim()
            .trim_start_matches('v')
            .split(['-', '+'])
            .next()
            .unwrap_or("");
        let mut parts = core.split('.');
        let a = parts.next().and_then(|v| v.parse().ok()).unwrap_or(0);
        let b = parts.next().and_then(|v| v.parse().ok()).unwrap_or(0);
        let c = parts.next().and_then(|v| v.parse().ok()).unwrap_or(0);
        (a, b, c)
    };
    parse(candidate) > parse(current)
}

/// Verify downloaded update bytes against expected lowercase hex SHA-256 digest.
pub fn verify_artifact_sha256(bytes: &[u8], expected_hex: &str) -> bool {
    let exp = expected_hex.trim();
    exp.len() == 64
        && exp.bytes().all(|b| b.is_ascii_hexdigit())
        && sha256_hex(bytes).eq_ignore_ascii_case(exp)
}

fn is_valid_sha256_hex(s: &str) -> bool {
    let exp = s.trim();
    exp.len() == 64 && exp.bytes().all(|b| b.is_ascii_hexdigit())
}

const MAX_UPDATE_REDIRECTS: usize = 5;

fn https_get_with_safe_redirects(
    raw_url: &str,
    timeout: Duration,
    extra_header: Option<(&str, &str)>,
) -> Result<(reqwest::blocking::Response, String), String> {
    let mut current_url = raw_url.to_string();
    for hop in 0..=MAX_UPDATE_REDIRECTS {
        if !current_url.starts_with("https://") {
            return Err("update URL must use https://".into());
        }
        let (parsed, host, addrs) = resolve_and_validate_public_url(&current_url)?;
        let client = reqwest::blocking::Client::builder()
            .timeout(timeout)
            .redirect(reqwest::redirect::Policy::none())
            .resolve_to_addrs(&host, &addrs)
            .build()
            .map_err(|e| e.to_string())?;
        let mut req = client.get(parsed.clone());
        if let Some((k, v)) = extra_header {
            req = req.header(k, v);
        }
        let resp = req
            .send()
            .map_err(|e| format!("update request failed: {e}"))?;
        let status_code = resp.status().as_u16();
        if matches!(status_code, 301 | 302 | 303 | 307 | 308) {
            if hop == MAX_UPDATE_REDIRECTS {
                return Err(format!(
                    "update request exceeded maximum {MAX_UPDATE_REDIRECTS} redirects"
                ));
            }
            let Some(loc) = resp.headers().get("location").and_then(|v| v.to_str().ok()) else {
                return Ok((resp, current_url));
            };
            let next = parsed
                .join(loc)
                .map_err(|e| format!("invalid redirect Location '{loc}': {e}"))?;
            if next.scheme() != "https" {
                return Err("update redirect target must use https://".into());
            }
            current_url = next.to_string();
            continue;
        }
        return Ok((resp, current_url));
    }
    Err(format!(
        "update request exceeded maximum {MAX_UPDATE_REDIRECTS} redirects"
    ))
}

/// Fetch an HTTPS update feed manifest (DNS-pinned against SSRF, up to 5 safe HTTPS redirects,
/// 64 KiB cap) and check whether a newer version exists.
///
/// When both `feed_url` and `NOLA_UPDATE_FEED_URL` are empty/unset, returns truthful
/// `status: "unconfigured"` (NEVER fakes `"up_to_date"`).
pub fn check_for_update(feed_url: &str, current_version: &str) -> Result<UpdateCheckResult, String> {
    let env_url = std::env::var("NOLA_UPDATE_FEED_URL").unwrap_or_default();
    let effective_url = if !feed_url.trim().is_empty() {
        feed_url.trim()
    } else {
        env_url.trim()
    };
    let target = current_update_target();

    if effective_url.is_empty() {
        return Ok(UpdateCheckResult {
            status: "unconfigured".into(),
            current_version: current_version.to_string(),
            target,
            update_available: false,
            latest: None,
        });
    }

    if !effective_url.starts_with("https://") {
        return Err("update feed URL must use https://".into());
    }
    let (resp, _) = https_get_with_safe_redirects(
        effective_url,
        Duration::from_secs(10),
        Some(("X-Nola-Update-Target", &target)),
    )?;
    if !resp.status().is_success() {
        return Err(format!("update feed returned HTTP {}", resp.status()));
    }
    let mut body = Vec::new();
    resp.take(MAX_MANIFEST_BYTES + 1)
        .read_to_end(&mut body)
        .map_err(|e| format!("failed reading update manifest: {e}"))?;
    if body.len() as u64 > MAX_MANIFEST_BYTES {
        return Err("update manifest exceeds 64 KiB limit".into());
    }
    let manifest: UpdateManifest = serde_json::from_slice(&body).map_err(|e| e.to_string())?;
    if !manifest.download_url.starts_with("https://") {
        return Err("update manifest download_url must use https://".into());
    }
    if !is_valid_sha256_hex(&manifest.sha256) {
        return Err("update manifest sha256 must be a valid 64-char hex digest".into());
    }
    let newer = is_newer_version(current_version, &manifest.version);
    Ok(UpdateCheckResult {
        status: if newer {
            "update_available".into()
        } else {
            "up_to_date".into()
        },
        current_version: current_version.to_string(),
        target,
        update_available: newer,
        latest: Some(manifest),
    })
}

fn sanitize_package_filename(url: &str) -> String {
    let raw_name = url::Url::parse(url)
        .ok()
        .and_then(|u| {
            u.path_segments()
                .and_then(|mut s| s.next_back().map(ToString::to_string))
        })
        .unwrap_or_default();
    let clean: String = raw_name
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
        .collect();
    let clean = clean.trim_start_matches('.');
    if clean.is_empty() || clean == "." || clean == ".." {
        format!("nola-update-{}.pkg", current_update_target())
    } else {
        clean.to_string()
    }
}

/// Stream an update package download to a temporary file in `dest_dir`, verify SHA-256
/// (rejecting mismatch), and atomically rename into `dest_dir`.
pub fn download_update_package(
    url: &str,
    expected_sha256: Option<&str>,
    dest_dir: &Path,
) -> Result<Value, String> {
    let trimmed_url = url.trim();
    if !trimmed_url.starts_with("https://") {
        return Err("update package URL must use https://".into());
    }
    if let Some(exp) = expected_sha256 {
        if !is_valid_sha256_hex(exp) {
            return Err("expected_sha256 must be a valid 64-char hex digest".into());
        }
    }

    let (mut resp, final_url) =
        https_get_with_safe_redirects(trimmed_url, Duration::from_secs(120), None)?;
    if !resp.status().is_success() {
        return Err(format!("update package returned HTTP {}", resp.status()));
    }

    fs::create_dir_all(dest_dir)
        .map_err(|e| format!("failed to create dest_dir '{}': {e}", dest_dir.display()))?;

    let temp_path = dest_dir.join(format!(".nola-update-{}.tmp", new_id()));
    let mut temp_file = File::create(&temp_path)
        .map_err(|e| format!("failed to create temp package file: {e}"))?;

    let mut hasher = Sha256::new();
    let mut total_bytes: u64 = 0;
    let mut buf = [0u8; 32 * 1024];

    let stream_res: Result<(), String> = (|| {
        loop {
            let n = resp
                .read(&mut buf)
                .map_err(|e| format!("failed streaming update package: {e}"))?;
            if n == 0 {
                break;
            }
            total_bytes = total_bytes.saturating_add(n as u64);
            if total_bytes > MAX_PACKAGE_BYTES {
                return Err(format!(
                    "update package exceeds {} byte limit",
                    MAX_PACKAGE_BYTES
                ));
            }
            hasher.update(&buf[..n]);
            temp_file
                .write_all(&buf[..n])
                .map_err(|e| format!("failed writing update package: {e}"))?;
        }
        temp_file
            .flush()
            .map_err(|e| format!("failed flushing update package: {e}"))?;
        Ok(())
    })();

    drop(temp_file);
    if let Err(err) = stream_res {
        let _ = fs::remove_file(&temp_path);
        return Err(err);
    }

    let computed_sha256 = hex::encode(hasher.finalize());
    if let Some(exp) = expected_sha256 {
        if !computed_sha256.eq_ignore_ascii_case(exp.trim()) {
            let _ = fs::remove_file(&temp_path);
            return Err(format!(
                "update package SHA-256 mismatch: expected {}, got {}",
                exp.trim(),
                computed_sha256
            ));
        }
    }

    let canonical_dest = dest_dir
        .canonicalize()
        .unwrap_or_else(|_| dest_dir.to_path_buf());
    let final_path = canonical_dest.join(sanitize_package_filename(&final_url));
    if final_path.parent() != Some(canonical_dest.as_path())
        || !nola_tools::path_within_workspace(&canonical_dest, &final_path)
    {
        let _ = fs::remove_file(&temp_path);
        return Err("sanitized update package path escapes dest_dir".into());
    }
    if let Err(e) = fs::rename(&temp_path, &final_path) {
        let _ = fs::remove_file(&temp_path);
        return Err(format!("failed to finalize update package: {e}"));
    }

    Ok(json!({
        "path": final_path.display().to_string(),
        "bytes": total_bytes,
        "sha256": computed_sha256,
        "target": current_update_target()
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn updater_returns_unconfigured_when_feed_url_is_empty() {
        // Ensure env var is not polluting the test
        let prev = std::env::var("NOLA_UPDATE_FEED_URL").ok();
        std::env::remove_var("NOLA_UPDATE_FEED_URL");

        let res = check_for_update("", "2.0.0").unwrap();
        assert_eq!(res.status, "unconfigured");
        assert!(!res.update_available);
        assert!(res.latest.is_none());
        assert!(!res.target.is_empty());

        if let Some(v) = prev {
            std::env::set_var("NOLA_UPDATE_FEED_URL", v);
        }
    }

    #[test]
    fn download_update_package_rejects_non_https_and_bad_sha() {
        let dir = tempfile::tempdir().unwrap();
        let err = download_update_package("http://example.invalid/pkg.tar.gz", None, dir.path())
            .unwrap_err();
        assert!(err.contains("https://"));

        let bad_sha = download_update_package(
            "https://example.invalid/pkg.tar.gz",
            Some("not-a-sha"),
            dir.path(),
        )
        .unwrap_err();
        assert!(bad_sha.contains("64-char"));

        assert_eq!(sanitize_package_filename("https://example.com/...hidden.tar.gz"), "hidden.tar.gz");
        assert!(!sanitize_package_filename("https://example.com/...").starts_with('.'));
    }
}
