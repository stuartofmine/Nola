// SPDX-License-Identifier: Apache-2.0
//! Software Update manifest checker and SHA-256 artifact verifier.

use std::io::Read;
use std::time::Duration;

use nola_tools::web::resolve_and_validate_public_url;
use nola_types::sha256_hex;
use serde::{Deserialize, Serialize};

const MAX_MANIFEST_BYTES: u64 = 64 * 1024;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UpdateManifest {
    pub version: String,
    pub download_url: String,
    pub sha256: String,
    #[serde(default)]
    pub notes: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UpdateCheckResult {
    pub current_version: String,
    pub update_available: bool,
    pub latest: Option<UpdateManifest>,
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

/// Fetch an HTTPS update feed manifest (DNS-pinned against SSRF, no redirects, 64 KiB cap)
/// and check whether a newer version exists.
pub fn check_for_update(feed_url: &str, current_version: &str) -> Result<UpdateCheckResult, String> {
    if !feed_url.starts_with("https://") {
        return Err("update feed URL must use https://".into());
    }
    let (parsed, host, addrs) = resolve_and_validate_public_url(feed_url)?;
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(10))
        .redirect(reqwest::redirect::Policy::none())
        .resolve_to_addrs(&host, &addrs)
        .build()
        .map_err(|e| e.to_string())?;
    let resp = client
        .get(parsed)
        .send()
        .map_err(|e| format!("update feed fetch failed: {e}"))?;
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
    let sha = manifest.sha256.trim();
    if sha.len() != 64 || !sha.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err("update manifest sha256 must be a valid 64-char hex digest".into());
    }
    let newer = is_newer_version(current_version, &manifest.version);
    Ok(UpdateCheckResult {
        current_version: current_version.to_string(),
        update_available: newer,
        latest: Some(manifest),
    })
}
