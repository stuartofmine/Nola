// SPDX-License-Identifier: Apache-2.0
//! Software Update manifest checker with mandatory Ed25519 signature verification,
//! lifetime/compatibility/downgrade gates, target detection, safe package downloader,
//! and SHA-256 + exact-size verification.

use std::collections::HashSet;
use std::fs::{self, OpenOptions};
use std::io::{Read, Write};
use std::path::Path;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use base64::{engine::general_purpose::STANDARD as BASE64_STANDARD, Engine as _};
use nola_store::NOLA_SQLITE_USER_VERSION;
use nola_tools::web::resolve_and_validate_public_url;
use nola_types::{new_id, sha256_hex};
use ring::signature::{UnparsedPublicKey, ED25519};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use url::Url;

const MAX_MANIFEST_BYTES: u64 = 1024 * 1024; // 1 MiB signed feed cap
const MAX_PACKAGE_BYTES: u64 = 2 * 1024 * 1024 * 1024; // 2 GiB package cap
const MAX_UPDATE_REDIRECTS: usize = 5;
const CLOCK_SKEW_TOLERANCE_SECS: u64 = 300;
/// Maximum allowed feed lifetime (`expires_at - issued_at`): 31 days.
pub const MAX_FEED_LIFETIME_SECS: u64 = 31 * 24 * 60 * 60;
/// Current local SQLite store schema version for compatibility gating.
pub const CURRENT_STORE_SCHEMA: i64 = NOLA_SQLITE_USER_VERSION as i64;
/// Current local SQLite store epoch for compatibility gating.
pub const CURRENT_STORE_EPOCH: i64 = 1;

const BUNDLED_UPDATE_SOURCE_JSON: &str = include_str!("../../../release/update-source.json");

const ALLOWED_GITHUB_CDN_HOSTS: &[&str] = &[
    "objects.githubusercontent.com",
    "release-assets.githubusercontent.com",
    "github-releases.githubusercontent.com",
];

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UpdateSource {
    pub schema_version: u32,
    pub version: String,
    pub build: u64,
    pub channel: String,
    #[serde(default)]
    pub feed_url: Option<String>,
    #[serde(default)]
    pub public_key: Option<String>,
}

impl UpdateSource {
    pub fn bundled() -> Result<Self, String> {
        Self::parse_bytes(BUNDLED_UPDATE_SOURCE_JSON.as_bytes())
    }

    pub fn parse_bytes(bytes: &[u8]) -> Result<Self, String> {
        let value: Self =
            serde_json::from_slice(bytes).map_err(|e| format!("invalid update source: {e}"))?;
        if value.schema_version != 1 {
            return Err(format!(
                "unsupported update source schema_version {}",
                value.schema_version
            ));
        }
        if !is_valid_semver(&value.version) {
            return Err(format!("invalid update source version '{}'", value.version));
        }
        if value.build == 0 {
            return Err("update source build must be > 0".into());
        }
        if !is_valid_channel(&value.channel) {
            return Err(format!("invalid update source channel '{}'", value.channel));
        }
        match (&value.feed_url, &value.public_key) {
            (None, None) => {}
            (Some(url), Some(key)) => {
                validate_https_url(url)?;
                decode_ed25519_public_key(key)?;
            }
            _ => {
                return Err(
                    "update source feed_url and public_key must be configured together".into(),
                );
            }
        }
        Ok(value)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum SignedArtifact {
    Store {
        url: String,
    },
    Installer {
        url: String,
        filename: String,
        size: u64,
        sha256: String,
    },
    Archive {
        url: String,
        filename: String,
        size: u64,
        sha256: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SignedTargetRelease {
    pub target: String,
    pub artifact: SignedArtifact,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SignedFeedPayload {
    pub schema_version: u32,
    pub channel: String,
    pub version: String,
    pub build: u64,
    pub issued_at: u64,
    pub expires_at: u64,
    pub notes: String,
    pub store_schema: i64,
    pub store_epoch: i64,
    pub targets: Vec<SignedTargetRelease>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct SignedFeedEnvelope {
    payload_base64: String,
    signature_base64: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UpdateManifest {
    pub version: String,
    pub download_url: String,
    pub sha256: String,
    #[serde(default)]
    pub notes: String,
    /// Verified Ed25519 signature (base64) for signed release manifests.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub signature: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub build: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub filename: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub release_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UpdateCheckResult {
    /// Truthful status: `"unconfigured"`, `"up_to_date"`, `"available"`,
    /// `"downgrade_rejected"`, `"incompatible_data"`, or `"unavailable"`.
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

fn parse_semver(s: &str) -> (u64, u64, u64) {
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
}

/// Compare two semver strings (`major.minor.patch`, stripping optional `-prerelease`/`+build`).
pub fn is_newer_version(current: &str, candidate: &str) -> bool {
    parse_semver(candidate) > parse_semver(current)
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

fn is_valid_semver(value: &str) -> bool {
    let parts: Vec<&str> = value.split('.').collect();
    parts.len() == 3
        && parts.iter().all(|p| {
            !p.is_empty()
                && p.len() <= 9
                && p.bytes().all(|c| c.is_ascii_digit())
                && (p.len() == 1 || !p.starts_with('0'))
        })
}

fn is_valid_channel(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 32
        && value
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
}

fn validate_https_url(value: &str) -> Result<Url, String> {
    let url = Url::parse(value.trim()).map_err(|e| format!("invalid update URL: {e}"))?;
    if url.scheme() != "https"
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.fragment().is_some()
    {
        return Err(
            "update URL must use https:// without embedded credentials or fragments".into(),
        );
    }
    Ok(url)
}

fn decode_ed25519_public_key(b64: &str) -> Result<Vec<u8>, String> {
    let bytes = BASE64_STANDARD
        .decode(b64.trim())
        .map_err(|e| format!("invalid base64 Ed25519 public key: {e}"))?;
    if bytes.len() != 32 {
        return Err(format!(
            "Ed25519 public key must be 32 bytes (got {})",
            bytes.len()
        ));
    }
    Ok(bytes)
}

fn now_unix_secs() -> Result<u64, String> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .map_err(|_| "system clock is before UNIX_EPOCH".into())
}

fn validate_signed_artifact(target: &str, artifact: &SignedArtifact) -> Result<(), String> {
    if target.is_empty()
        || target.len() > 64
        || !target
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_'))
    {
        return Err(format!("invalid release target '{target}'"));
    }
    match artifact {
        SignedArtifact::Store { url } => {
            let parsed = validate_https_url(url)?;
            let allowed = if target == "ios" {
                matches!(
                    parsed.host_str(),
                    Some("apps.apple.com" | "testflight.apple.com")
                )
            } else if target.starts_with("android-") {
                parsed.host_str() == Some("play.google.com")
            } else {
                false
            };
            if !allowed {
                return Err(format!(
                    "store artifact URL host is not allowed for target '{target}'"
                ));
            }
        }
        SignedArtifact::Installer {
            url,
            filename,
            size,
            sha256,
        }
        | SignedArtifact::Archive {
            url,
            filename,
            size,
            sha256,
        } => {
            validate_https_url(url)?;
            let safe_name = !filename.is_empty()
                && filename.len() <= 160
                && !filename.starts_with('.')
                && filename
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'));
            if !safe_name {
                return Err(format!("invalid artifact filename '{filename}'"));
            }
            if *size == 0 || *size > MAX_PACKAGE_BYTES {
                return Err(format!("invalid artifact size {size} for '{filename}'"));
            }
            if !is_valid_sha256_hex(sha256) {
                return Err(format!(
                    "artifact '{filename}' sha256 must be a valid 64-char hex digest"
                ));
            }
        }
    }
    Ok(())
}

fn verify_signed_envelope(
    feed_bytes: &[u8],
    public_key_b64: &str,
    expected_channel: Option<&str>,
    now_secs: u64,
) -> Result<(SignedFeedPayload, String, String), String> {
    if feed_bytes.len() as u64 > MAX_MANIFEST_BYTES {
        return Err(format!(
            "update feed exceeds {} byte limit",
            MAX_MANIFEST_BYTES
        ));
    }
    let envelope: SignedFeedEnvelope = serde_json::from_slice(feed_bytes).map_err(|_| {
        "unsigned or malformed update feed: expected signed {payload_base64, signature_base64} envelope"
            .to_string()
    })?;
    let payload_bytes = BASE64_STANDARD
        .decode(envelope.payload_base64.trim())
        .map_err(|e| format!("invalid feed payload_base64: {e}"))?;
    let sig_bytes = BASE64_STANDARD
        .decode(envelope.signature_base64.trim())
        .map_err(|e| format!("invalid feed signature_base64: {e}"))?;
    if sig_bytes.len() != 64 {
        return Err(format!(
            "invalid Ed25519 signature length: expected 64 bytes, got {}",
            sig_bytes.len()
        ));
    }
    let pk_bytes = decode_ed25519_public_key(public_key_b64)?;
    UnparsedPublicKey::new(&ED25519, &pk_bytes)
        .verify(&payload_bytes, &sig_bytes)
        .map_err(|_| "Ed25519 signature verification failed for update feed".to_string())?;

    let payload: SignedFeedPayload = serde_json::from_slice(&payload_bytes)
        .map_err(|e| format!("invalid signed feed payload JSON: {e}"))?;
    if payload.schema_version != 1 {
        return Err(format!(
            "unsupported feed schema_version {}",
            payload.schema_version
        ));
    }
    if let Some(ch) = expected_channel {
        if payload.channel != ch {
            return Err(format!(
                "feed channel '{}' does not match configured channel '{}'",
                payload.channel, ch
            ));
        }
    } else if !is_valid_channel(&payload.channel) {
        return Err(format!("invalid feed channel '{}'", payload.channel));
    }
    if !is_valid_semver(&payload.version) {
        return Err(format!("invalid feed version '{}'", payload.version));
    }
    if payload.build == 0 {
        return Err("feed build number must be > 0".into());
    }
    if payload.notes.len() > 32 * 1024 {
        return Err("feed release notes exceed 32 KiB limit".into());
    }
    if payload.store_schema <= 0 || payload.store_epoch <= 0 {
        return Err("feed store_schema and store_epoch must be positive".into());
    }
    if payload.targets.is_empty() || payload.targets.len() > 32 {
        return Err("feed targets must contain between 1 and 32 entries".into());
    }
    if payload.issued_at > now_secs.saturating_add(CLOCK_SKEW_TOLERANCE_SECS) {
        return Err("feed issued_at is in the future (check system clock)".into());
    }
    if payload.expires_at <= now_secs {
        return Err("update feed has expired".into());
    }
    if payload.expires_at <= payload.issued_at {
        return Err("feed expires_at must be greater than issued_at".into());
    }
    if payload.expires_at - payload.issued_at > MAX_FEED_LIFETIME_SECS {
        return Err(format!(
            "feed lifetime exceeds maximum {} seconds (31 days)",
            MAX_FEED_LIFETIME_SECS
        ));
    }
    let mut seen_targets = HashSet::new();
    for item in &payload.targets {
        if !seen_targets.insert(item.target.as_str()) {
            return Err(format!("duplicate release target '{}'", item.target));
        }
        validate_signed_artifact(&item.target, &item.artifact)?;
    }

    let release_id = hex::encode(Sha256::digest(&payload_bytes));
    Ok((payload, release_id, envelope.signature_base64))
}

/// Verify a signed update feed JSON against an `update-source.json` trust root.
/// Used by `nola update verify-feed` CLI and release automation scripts.
pub fn verify_signed_feed_bytes(source_json: &[u8], feed_json: &[u8]) -> Result<Value, String> {
    let now_secs = now_unix_secs()?;
    verify_signed_feed_bytes_at(source_json, feed_json, now_secs)
}

fn verify_signed_feed_bytes_at(
    source_json: &[u8],
    feed_json: &[u8],
    now_secs: u64,
) -> Result<Value, String> {
    let source = UpdateSource::parse_bytes(source_json)?;
    let pk = source
        .public_key
        .as_deref()
        .ok_or_else(|| "update source is missing public_key".to_string())?;
    let (payload, release_id, _) =
        verify_signed_envelope(feed_json, pk, Some(&source.channel), now_secs)?;
    Ok(json!({
        "verified": true,
        "release_id": release_id,
        "schema_version": payload.schema_version,
        "channel": payload.channel,
        "version": payload.version,
        "build": payload.build,
        "issued_at": payload.issued_at,
        "expires_at": payload.expires_at,
        "store_schema": payload.store_schema,
        "store_epoch": payload.store_epoch,
        "notes": payload.notes,
        "target_count": payload.targets.len(),
        "targets": payload.targets
    }))
}

/// Evaluate a verified signed feed against local version/build/store compatibility and target.
pub fn evaluate_signed_feed(
    feed_bytes: &[u8],
    public_key_b64: &str,
    expected_channel: Option<&str>,
    current_version: &str,
    current_build: u64,
    current_store_schema: i64,
    current_store_epoch: i64,
    target: &str,
    now_secs: u64,
) -> Result<UpdateCheckResult, String> {
    let (payload, release_id, signature_b64) =
        verify_signed_envelope(feed_bytes, public_key_b64, expected_channel, now_secs)?;

    let cur_ver = parse_semver(current_version);
    let cand_ver = parse_semver(&payload.version);

    // Monotonic version/build downgrade protection
    if cand_ver < cur_ver || (cand_ver == cur_ver && payload.build < current_build) {
        return Ok(UpdateCheckResult {
            status: "downgrade_rejected".into(),
            current_version: current_version.to_string(),
            target: target.to_string(),
            update_available: false,
            latest: None,
        });
    }

    // Store schema / epoch compatibility gate
    if payload.store_schema != current_store_schema || payload.store_epoch != current_store_epoch {
        return Ok(UpdateCheckResult {
            status: "incompatible_data".into(),
            current_version: current_version.to_string(),
            target: target.to_string(),
            update_available: false,
            latest: None,
        });
    }

    // Exact same version and build -> up to date
    if cand_ver == cur_ver && payload.build == current_build {
        return Ok(UpdateCheckResult {
            status: "up_to_date".into(),
            current_version: current_version.to_string(),
            target: target.to_string(),
            update_available: false,
            latest: None,
        });
    }

    // Find matching target artifact (exact match first, then `cli-<target>` fallback)
    let matched = payload
        .targets
        .iter()
        .find(|t| t.target == target)
        .or_else(|| {
            let cli_target = format!("cli-{target}");
            payload.targets.iter().find(|t| t.target == cli_target)
        });

    let Some(item) = matched else {
        return Ok(UpdateCheckResult {
            status: "unavailable".into(),
            current_version: current_version.to_string(),
            target: target.to_string(),
            update_available: false,
            latest: None,
        });
    };

    let manifest = match &item.artifact {
        SignedArtifact::Installer {
            url,
            filename,
            size,
            sha256,
        }
        | SignedArtifact::Archive {
            url,
            filename,
            size,
            sha256,
        } => UpdateManifest {
            version: payload.version,
            download_url: url.clone(),
            sha256: sha256.clone(),
            notes: payload.notes,
            signature: Some(signature_b64),
            build: Some(payload.build),
            size: Some(*size),
            filename: Some(filename.clone()),
            release_id: Some(release_id),
        },
        SignedArtifact::Store { url } => UpdateManifest {
            version: payload.version,
            download_url: url.clone(),
            sha256: String::new(),
            notes: payload.notes,
            signature: Some(signature_b64),
            build: Some(payload.build),
            size: None,
            filename: None,
            release_id: Some(release_id),
        },
    };

    Ok(UpdateCheckResult {
        status: "available".into(),
        current_version: current_version.to_string(),
        target: target.to_string(),
        update_available: true,
        latest: Some(manifest),
    })
}

/// Strict redirect host allowlist: only same-origin HTTPS redirects or GitHub release-assets CDN.
pub fn is_allowed_update_redirect(previous: &Url, next: &Url) -> bool {
    if validate_https_url(previous.as_str()).is_err() || validate_https_url(next.as_str()).is_err()
    {
        return false;
    }
    if previous.origin() == next.origin() {
        return true;
    }
    let prev_is_github = matches!(
        previous.host_str(),
        Some("github.com" | "api.github.com" | "raw.githubusercontent.com")
    );
    let next_is_cdn = next
        .host_str()
        .is_some_and(|h| ALLOWED_GITHUB_CDN_HOSTS.contains(&h));
    prev_is_github
        && next_is_cdn
        && previous.port_or_known_default() == Some(443)
        && next.port_or_known_default() == Some(443)
}

fn https_get_with_safe_redirects(
    raw_url: &str,
    timeout: Duration,
    extra_header: Option<(&str, &str)>,
) -> Result<(reqwest::blocking::Response, String), String> {
    let mut current_url = raw_url.to_string();
    for hop in 0..=MAX_UPDATE_REDIRECTS {
        validate_https_url(&current_url)?;
        let (parsed, host, addrs) = resolve_and_validate_public_url(&current_url)?;
        let client = reqwest::blocking::Client::builder()
            .timeout(timeout)
            .redirect(reqwest::redirect::Policy::none())
            .resolve_to_addrs(&host, &addrs)
            .user_agent("Nola-Software-Update/2")
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
            if !is_allowed_update_redirect(&parsed, &next) {
                return Err(format!(
                    "untrusted update redirect from '{}' to '{}'",
                    parsed.host_str().unwrap_or(""),
                    next.host_str().unwrap_or("")
                ));
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

fn resolve_trust_root() -> Result<(String, Option<String>, u64), String> {
    let bundled = UpdateSource::bundled().ok();
    let env_pk = std::env::var("NOLA_UPDATE_PUBLIC_KEY")
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());
    let pk = env_pk
        .or_else(|| bundled.as_ref().and_then(|s| s.public_key.clone()))
        .ok_or_else(|| {
            "missing Ed25519 update public key (set NOLA_UPDATE_PUBLIC_KEY or bundle release/update-source.json)"
                .to_string()
        })?;
    let channel = bundled.as_ref().map(|s| s.channel.clone());
    let build = bundled.as_ref().map(|s| s.build).unwrap_or(1);
    Ok((pk, channel, build))
}

/// Fetch a signed HTTPS update feed (DNS-pinned against SSRF, strict redirect allowlist,
/// mandatory Ed25519 verification, lifetime/compatibility/downgrade gates) and check
/// whether a newer version is available.
///
/// When both `feed_url` and `NOLA_UPDATE_FEED_URL` are empty/unset, returns truthful
/// `status: "unconfigured"` without making outbound network requests.
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

    validate_https_url(effective_url)?;
    let (public_key_b64, expected_channel, current_build) = resolve_trust_root()?;
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
        .map_err(|e| format!("failed reading update feed: {e}"))?;
    if body.len() as u64 > MAX_MANIFEST_BYTES {
        return Err("update feed exceeds 1 MiB limit".into());
    }
    let now_secs = now_unix_secs()?;
    evaluate_signed_feed(
        &body,
        &public_key_b64,
        expected_channel.as_deref(),
        current_version,
        current_build,
        CURRENT_STORE_SCHEMA,
        CURRENT_STORE_EPOCH,
        &target,
        now_secs,
    )
}

fn sanitize_package_filename(url: &str) -> String {
    let raw_name = Url::parse(url)
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

fn persist_noclobber(temp_path: &Path, final_path: &Path) -> Result<(), String> {
    if fs::symlink_metadata(final_path).is_ok() {
        return Err(format!(
            "update package destination '{}' already exists (noclobber)",
            final_path.display()
        ));
    }
    match fs::hard_link(temp_path, final_path) {
        Ok(()) => {
            let _ = fs::remove_file(temp_path);
            Ok(())
        }
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => Err(format!(
            "update package destination '{}' already exists (noclobber)",
            final_path.display()
        )),
        Err(_) => {
            // Fallback on filesystems without hard links (e.g. FAT/certain network mounts):
            // double-check non-existence then rename.
            if fs::symlink_metadata(final_path).is_ok() {
                return Err(format!(
                    "update package destination '{}' already exists (noclobber)",
                    final_path.display()
                ));
            }
            fs::rename(temp_path, final_path)
                .map_err(|e| format!("failed to finalize update package: {e}"))
        }
    }
}

/// Stream an update package download to a staged temp file in `dest_dir`, require a valid
/// `expected_sha256` (rejecting `None` or blank), verify SHA-256 and optional exact byte size,
/// and atomically publish without clobbering existing files.
pub fn download_update_package(
    url: &str,
    expected_sha256: Option<&str>,
    dest_dir: &Path,
) -> Result<Value, String> {
    let exp = expected_sha256
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| {
            "expected_sha256 is required and must be a valid 64-char hex digest".to_string()
        })?;
    download_update_package_with_size(url, exp, None, dest_dir)
}

/// Verified update package downloader with mandatory SHA-256 and optional exact `expected_size`.
pub fn download_update_package_with_size(
    url: &str,
    expected_sha256: &str,
    expected_size: Option<u64>,
    dest_dir: &Path,
) -> Result<Value, String> {
    let trimmed_url = url.trim();
    validate_https_url(trimmed_url)?;
    let exp = expected_sha256.trim();
    if exp.is_empty() || !is_valid_sha256_hex(exp) {
        return Err("expected_sha256 must be a valid 64-char hex digest".into());
    }
    if let Some(sz) = expected_size {
        if sz == 0 || sz > MAX_PACKAGE_BYTES {
            return Err(format!("invalid expected_size {sz}"));
        }
    }

    let (mut resp, final_url) =
        https_get_with_safe_redirects(trimmed_url, Duration::from_secs(120), None)?;
    if !resp.status().is_success() {
        return Err(format!("update package returned HTTP {}", resp.status()));
    }

    fs::create_dir_all(dest_dir)
        .map_err(|e| format!("failed to create dest_dir '{}': {e}", dest_dir.display()))?;

    let canonical_dest = dest_dir
        .canonicalize()
        .unwrap_or_else(|_| dest_dir.to_path_buf());
    let final_path = canonical_dest.join(sanitize_package_filename(&final_url));
    if final_path.parent() != Some(canonical_dest.as_path())
        || !nola_tools::path_within_workspace(&canonical_dest, &final_path)
    {
        return Err("sanitized update package path escapes dest_dir".into());
    }
    if fs::symlink_metadata(&final_path).is_ok() {
        return Err(format!(
            "update package destination '{}' already exists (noclobber)",
            final_path.display()
        ));
    }

    let temp_path = canonical_dest.join(format!(".nola-update-{}.tmp", new_id()));
    let mut temp_file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temp_path)
        .map_err(|e| format!("failed to create temp package file: {e}"))?;

    let max_bytes = expected_size.unwrap_or(MAX_PACKAGE_BYTES);
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
            total_bytes = total_bytes
                .checked_add(n as u64)
                .ok_or_else(|| "update package size overflow".to_string())?;
            if total_bytes > max_bytes {
                return Err(format!(
                    "update package exceeds {} byte limit",
                    max_bytes
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
        temp_file
            .sync_all()
            .map_err(|e| format!("failed syncing update package: {e}"))?;
        Ok(())
    })();

    drop(temp_file);
    if let Err(err) = stream_res {
        let _ = fs::remove_file(&temp_path);
        return Err(err);
    }

    if let Some(exp_size) = expected_size {
        if total_bytes != exp_size {
            let _ = fs::remove_file(&temp_path);
            return Err(format!(
                "update package size mismatch: expected {exp_size} bytes, got {total_bytes}"
            ));
        }
    }

    let computed_sha256 = hex::encode(hasher.finalize());
    if !computed_sha256.eq_ignore_ascii_case(exp) {
        let _ = fs::remove_file(&temp_path);
        return Err(format!(
            "update package SHA-256 mismatch: expected {}, got {}",
            exp, computed_sha256
        ));
    }

    if let Err(e) = persist_noclobber(&temp_path, &final_path) {
        let _ = fs::remove_file(&temp_path);
        return Err(e);
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
    use ring::rand::SystemRandom;
    use ring::signature::{Ed25519KeyPair, KeyPair};

    fn generate_ephemeral_keypair() -> (Ed25519KeyPair, String) {
        let rng = SystemRandom::new();
        let pkcs8 = Ed25519KeyPair::generate_pkcs8(&rng).unwrap();
        let kp = Ed25519KeyPair::from_pkcs8(pkcs8.as_ref()).unwrap();
        let pk_b64 = BASE64_STANDARD.encode(kp.public_key().as_ref());
        (kp, pk_b64)
    }

    fn sign_payload(kp: &Ed25519KeyPair, payload: &Value) -> Vec<u8> {
        let payload_bytes = serde_json::to_vec(payload).unwrap();
        let sig = kp.sign(&payload_bytes);
        serde_json::to_vec(&json!({
            "payload_base64": BASE64_STANDARD.encode(&payload_bytes),
            "signature_base64": BASE64_STANDARD.encode(sig.as_ref())
        }))
        .unwrap()
    }

    fn sample_payload(
        version: &str,
        build: u64,
        issued_at: u64,
        expires_at: u64,
        store_schema: i64,
        store_epoch: i64,
    ) -> Value {
        json!({
            "schema_version": 1,
            "channel": "stable",
            "version": version,
            "build": build,
            "issued_at": issued_at,
            "expires_at": expires_at,
            "notes": "Security and performance release",
            "store_schema": store_schema,
            "store_epoch": store_epoch,
            "targets": [
                {
                    "target": "macos-aarch64",
                    "artifact": {
                        "kind": "installer",
                        "url": "https://github.com/stuartofmine/Nola/releases/download/v2.1.0/Nola-macos-aarch64.dmg",
                        "filename": "Nola-macos-aarch64.dmg",
                        "size": 123456,
                        "sha256": "a".repeat(64)
                    }
                }
            ]
        })
    }

    #[test]
    fn updater_returns_unconfigured_when_feed_url_is_empty() {
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
    fn ed25519_signed_feed_verification_and_rejection_gates() {
        let (kp, pk_b64) = generate_ephemeral_keypair();
        let now = 1_800_000_000u64;
        let source_json = serde_json::to_vec(&json!({
            "schema_version": 1,
            "version": "2.0.0",
            "build": 10,
            "channel": "stable",
            "feed_url": "https://example.com/feed.json",
            "public_key": pk_b64
        }))
        .unwrap();

        // 1. Valid signed newer update -> status "available"
        let valid_payload = sample_payload("2.1.0", 12, now - 60, now + 86_400, 2, 1);
        let valid_feed = sign_payload(&kp, &valid_payload);
        let verified = verify_signed_feed_bytes_at(&source_json, &valid_feed, now).unwrap();
        assert_eq!(verified["verified"], json!(true));
        assert_eq!(verified["version"], json!("2.1.0"));

        let eval = evaluate_signed_feed(
            &valid_feed,
            &pk_b64,
            Some("stable"),
            "2.0.0",
            10,
            2,
            1,
            "macos-aarch64",
            now,
        )
        .unwrap();
        assert_eq!(eval.status, "available");
        assert!(eval.update_available);
        assert_eq!(eval.latest.as_ref().unwrap().version, "2.1.0");
        assert_eq!(eval.latest.as_ref().unwrap().size, Some(123456));

        // 2. Tampered payload rejected
        let mut tampered_env: Value = serde_json::from_slice(&valid_feed).unwrap();
        let tampered_payload = sample_payload("2.9.9", 99, now - 60, now + 86_400, 2, 1);
        tampered_env["payload_base64"] =
            json!(BASE64_STANDARD.encode(serde_json::to_vec(&tampered_payload).unwrap()));
        let tampered_bytes = serde_json::to_vec(&tampered_env).unwrap();
        let err = verify_signed_feed_bytes_at(&source_json, &tampered_bytes, now).unwrap_err();
        assert!(err.contains("signature verification failed"), "got: {err}");

        // 3. Unsigned manifest rejected
        let unsigned_bytes = serde_json::to_vec(&valid_payload).unwrap();
        let err = verify_signed_feed_bytes_at(&source_json, &unsigned_bytes, now).unwrap_err();
        assert!(err.contains("unsigned or malformed"), "got: {err}");

        // 4. Expired feed rejected
        let expired_feed = sign_payload(
            &kp,
            &sample_payload("2.1.0", 12, now - 7_200, now - 10, 2, 1),
        );
        let err = verify_signed_feed_bytes_at(&source_json, &expired_feed, now).unwrap_err();
        assert!(err.contains("expired"), "got: {err}");

        // 5. Lifetime > 31 days rejected
        let too_long_feed = sign_payload(
            &kp,
            &sample_payload(
                "2.1.0",
                12,
                now - 60,
                now + MAX_FEED_LIFETIME_SECS + 1000,
                2,
                1,
            ),
        );
        let err = verify_signed_feed_bytes_at(&source_json, &too_long_feed, now).unwrap_err();
        assert!(err.contains("lifetime"), "got: {err}");

        // 6. Incompatible store_schema / store_epoch -> "incompatible_data"
        let incompat = evaluate_signed_feed(
            &valid_feed,
            &pk_b64,
            Some("stable"),
            "2.0.0",
            10,
            1, // local schema 1 vs feed schema 2
            1,
            "macos-aarch64",
            now,
        )
        .unwrap();
        assert_eq!(incompat.status, "incompatible_data");
        assert!(!incompat.update_available);
        assert!(incompat.latest.is_none());

        // 7. Downgrade rejected
        let downgrade = evaluate_signed_feed(
            &valid_feed,
            &pk_b64,
            Some("stable"),
            "2.2.0",
            15,
            2,
            1,
            "macos-aarch64",
            now,
        )
        .unwrap();
        assert_eq!(downgrade.status, "downgrade_rejected");
        assert!(!downgrade.update_available);

        // 8. Same version + build -> "up_to_date"
        let same = evaluate_signed_feed(
            &valid_feed,
            &pk_b64,
            Some("stable"),
            "2.1.0",
            12,
            2,
            1,
            "macos-aarch64",
            now,
        )
        .unwrap();
        assert_eq!(same.status, "up_to_date");
        assert!(!same.update_available);
    }

    #[test]
    fn bundled_release_update_source_verifies_repo_feed() {
        let bundled_src = UpdateSource::bundled().unwrap();
        assert_eq!(bundled_src.schema_version, 1);
        let feed_bytes = include_bytes!("../../../release/feed.json");
        // Verify using a timestamp inside the bundled feed's issued_at..expires_at window
        let res = verify_signed_feed_bytes_at(
            BUNDLED_UPDATE_SOURCE_JSON.as_bytes(),
            feed_bytes,
            1_795_400_000,
        )
        .unwrap();
        assert_eq!(res["verified"], json!(true));
        assert_eq!(res["version"], json!("1.0.0"));
    }

    #[test]
    fn download_update_package_enforces_https_mandatory_sha_redirects_and_noclobber() {
        let dir = tempfile::tempdir().unwrap();
        // Reject missing/None sha256
        let missing_sha =
            download_update_package("https://example.invalid/pkg.tar.gz", None, dir.path())
                .unwrap_err();
        assert!(missing_sha.contains("expected_sha256 is required"));

        // Reject blank sha256
        let blank_sha =
            download_update_package("https://example.invalid/pkg.tar.gz", Some("   "), dir.path())
                .unwrap_err();
        assert!(blank_sha.contains("expected_sha256 is required"));

        // Reject non-https
        let non_https = download_update_package(
            "http://example.invalid/pkg.tar.gz",
            Some(&"a".repeat(64)),
            dir.path(),
        )
        .unwrap_err();
        assert!(non_https.contains("https://"));

        // Reject bad sha format
        let bad_sha = download_update_package(
            "https://example.invalid/pkg.tar.gz",
            Some("not-a-sha"),
            dir.path(),
        )
        .unwrap_err();
        assert!(bad_sha.contains("64-char"));

        // Redirect allowlist checks
        let gh = Url::parse("https://github.com/stuartofmine/Nola/releases/download/v2/a.dmg")
            .unwrap();
        let gh_cdn = Url::parse("https://objects.githubusercontent.com/assets/a.dmg").unwrap();
        let gh_rel =
            Url::parse("https://release-assets.githubusercontent.com/assets/a.dmg").unwrap();
        let evil = Url::parse("https://evil.example.com/a.dmg").unwrap();
        assert!(is_allowed_update_redirect(&gh, &gh_cdn));
        assert!(is_allowed_update_redirect(&gh, &gh_rel));
        assert!(!is_allowed_update_redirect(&gh, &evil));
        assert!(is_allowed_update_redirect(
            &evil,
            &Url::parse("https://evil.example.com/b.dmg").unwrap()
        ));

        // Atomic noclobber persist test
        let tmp = dir.path().join(".stage.tmp");
        let dst = dir.path().join("pkg.bin");
        fs::write(&tmp, b"payload-1").unwrap();
        persist_noclobber(&tmp, &dst).unwrap();
        assert_eq!(fs::read(&dst).unwrap(), b"payload-1");

        let tmp2 = dir.path().join(".stage2.tmp");
        fs::write(&tmp2, b"payload-2").unwrap();
        let clobber_err = persist_noclobber(&tmp2, &dst).unwrap_err();
        assert!(clobber_err.contains("noclobber"));
        assert_eq!(fs::read(&dst).unwrap(), b"payload-1");

        assert_eq!(
            sanitize_package_filename("https://example.com/...hidden.tar.gz"),
            "hidden.tar.gz"
        );
        assert!(!sanitize_package_filename("https://example.com/...").starts_with('.'));
    }
}
