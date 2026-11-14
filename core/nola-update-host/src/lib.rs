// SPDX-License-Identifier: Apache-2.0
//! Software distribution is a Host concern, independent of model credentials,
//! conversation schemas, and the Application transaction database.
use base64::{engine::general_purpose::STANDARD, Engine};
use ring::signature::{UnparsedPublicKey, ED25519};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{fs, io::{Read, Write}, path::{Path, PathBuf}, time::{Duration, SystemTime, UNIX_EPOCH}};
use url::Url;

const MAX_FEED: usize = 1024 * 1024;
const MAX_PACKAGE: u64 = 2 * 1024 * 1024 * 1024;
const MAX_LIFETIME: u64 = 31 * 24 * 60 * 60;

#[derive(Debug, Serialize)]
pub struct UpdateError { pub code: &'static str, pub message: String }
impl std::fmt::Display for UpdateError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result { write!(f, "{}: {}", self.code, self.message) }
}
impl std::error::Error for UpdateError {}
fn error(code: &'static str, message: impl ToString) -> UpdateError { UpdateError {code, message: message.to_string()} }
type Result<T> = std::result::Result<T, UpdateError>;

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct UpdateSource {
    pub schema_version: u32,
    pub version: String,
    pub build: u64,
    pub channel: String,
    pub feed_url: Option<String>,
    pub public_key: Option<String>,
}
impl UpdateSource {
    pub fn bundled() -> Result<Self> { Self::parse(include_str!("../../../release/update-source.json")) }
    pub fn parse(text: &str) -> Result<Self> {
        let value: Self = serde_json::from_str(text).map_err(|_| error("source_invalid", "Invalid update source"))?;
        if value.schema_version != 1 || !valid_version(&value.version) || value.build == 0 || !valid_channel(&value.channel) {
            return Err(error("source_invalid", "Unsupported update source or release identity"));
        }
        match (&value.feed_url, &value.public_key) {
            (None, None) => {},
            (Some(url), Some(key)) => { https_url(url)?; public_key(key)?; },
            _ => return Err(error("source_invalid", "Feed URL and public signing key must be configured together")),
        }
        Ok(value)
    }
}
fn valid_version(value: &str) -> bool {
    let parts: Vec<_> = value.split('.').collect();
    parts.len() == 3 && parts.iter().all(|p| !p.is_empty() && p.len() <= 9 && p.bytes().all(|c| c.is_ascii_digit()) && (p.len() == 1 || !p.starts_with('0')))
}
fn valid_channel(value: &str) -> bool { !value.is_empty() && value.len() <= 32 && value.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-') }
fn https_url(value: &str) -> Result<Url> {
    let url = Url::parse(value).map_err(|_| error("url_invalid", "Invalid update URL"))?;
    if url.scheme() != "https" || url.host_str().is_none() || !url.username().is_empty() || url.password().is_some() || url.fragment().is_some() {
        return Err(error("url_invalid", "Updates require an HTTPS URL without embedded credentials or fragments"));
    }
    Ok(url)
}
fn public_key(value: &str) -> Result<Vec<u8>> {
    let bytes = STANDARD.decode(value).map_err(|_| error("source_invalid", "Invalid public signing key"))?;
    if bytes.len() != 32 { return Err(error("source_invalid", "Expected a 32-byte Ed25519 public key")); }
    Ok(bytes)
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Artifact {
    Store { url: String },
    Installer { url: String, filename: String, size: u64, sha256: String },
    Archive { url: String, filename: String, size: u64, sha256: String },
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct TargetRelease { pub target: String, pub artifact: Artifact }
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    pub schema_version: u32,
    pub channel: String,
    pub version: String,
    pub build: u64,
    pub issued_at: u64,
    pub expires_at: u64,
    pub notes: String,
    pub store_schema: i64,
    pub store_epoch: i64,
    pub targets: Vec<TargetRelease>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Envelope { payload_base64: String, signature_base64: String }
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Offer {
    pub release_id: String,
    pub version: String,
    pub build: u64,
    pub notes: String,
    pub artifact: Artifact,
}
#[derive(Debug, Serialize)]
pub struct Check {
    pub status: &'static str,
    pub current_version: String,
    pub current_build: u64,
    pub channel: String,
    pub target: String,
    pub release: Option<Offer>,
}

pub fn native_target(component: &str) -> String {
    let os = std::env::consts::OS;
    let arch = std::env::consts::ARCH;
    if component == "ios" { return "ios".into(); }
    if component == "android" { return format!("android-{arch}"); }
    if component == "desktop" && os == "windows" { return format!("windows-{arch}-cpu"); }
    if component == "desktop" { return format!("{os}-{arch}"); }
    format!("cli-{os}-{arch}")
}
fn known_target(target: &str) -> bool {
    matches!(target, "ios" | "android-store" | "android-aarch64" | "android-x86_64" | "macos-aarch64" | "macos-x86_64" |
        "windows-x86_64-cpu" | "windows-aarch64-cpu" | "windows-x86_64-vulkan" | "windows-x86_64-cuda" |
        "cli-macos-aarch64" | "cli-macos-x86_64" | "cli-linux-aarch64" | "cli-linux-x86_64" | "cli-windows-x86_64" | "cli-windows-aarch64")
}
fn validate_artifact(target: &str, artifact: &Artifact) -> Result<()> {
    if !known_target(target) { return Err(error("target_invalid", "Unknown update target")); }
    match artifact {
        Artifact::Store {url} => {
            let url = https_url(url)?;
            let allowed = if target == "ios" { matches!(url.host_str(), Some("apps.apple.com" | "testflight.apple.com")) }
                else if target.starts_with("android-") { url.host_str() == Some("play.google.com") } else { false };
            if !allowed { return Err(error("artifact_invalid", "Store URL does not match the target platform")); }
        },
        Artifact::Installer {url, filename, size, sha256} | Artifact::Archive {url, filename, size, sha256} => {
            https_url(url)?;
            let safe_name = !filename.is_empty() && filename.len() <= 120 && filename.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b,b'-'|b'_'|b'.')) && !filename.starts_with('.');
            let suffix = if matches!(artifact, Artifact::Archive {..}) {
                target.starts_with("cli-") && (filename.ends_with(".tar.gz") || filename.ends_with(".zip"))
            } else if target.starts_with("macos-") { filename.ends_with(".dmg") || filename.ends_with(".pkg") }
                else if target.starts_with("windows-") { filename.ends_with(".exe") || filename.ends_with(".msi") }
                else { matches!(target, "android-aarch64" | "android-x86_64") && filename.ends_with(".apk") };
            if !safe_name || !suffix || *size == 0 || *size > MAX_PACKAGE || sha256.len() != 64 || !sha256.bytes().all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c)) {
                return Err(error("artifact_invalid", "Invalid package filename, platform, size or digest"));
            }
        },
    }
    Ok(())
}

/// Verify exact signed bytes before interpreting any publication content.
pub fn verify_feed(source: &UpdateSource, bytes: &[u8], target: &str, now: u64) -> Result<(Manifest, String)> {
    if bytes.len() > MAX_FEED { return Err(error("feed_too_large", "Update feed exceeds size limit")); }
    let envelope: Envelope = serde_json::from_slice(bytes).map_err(|_| error("feed_invalid", "Invalid signed update envelope"))?;
    let payload = STANDARD.decode(&envelope.payload_base64).map_err(|_| error("feed_invalid", "Invalid payload encoding"))?;
    let signature = STANDARD.decode(&envelope.signature_base64).map_err(|_| error("signature_invalid", "Invalid signature encoding"))?;
    let key = public_key(source.public_key.as_deref().ok_or_else(|| error("source_invalid", "Signing key missing"))?)?;
    UnparsedPublicKey::new(&ED25519, key).verify(&payload, &signature).map_err(|_| error("signature_invalid", "Update feed signature could not be verified"))?;
    let manifest: Manifest = serde_json::from_slice(&payload).map_err(|_| error("feed_invalid", "Invalid release payload"))?;
    if manifest.schema_version != 1 || manifest.channel != source.channel || !valid_version(&manifest.version) || manifest.build == 0 || manifest.notes.len() > 32768 || manifest.store_schema <= 0 || manifest.store_epoch <= 0 || manifest.targets.is_empty() || manifest.targets.len() > 32 {
        return Err(error("feed_invalid", "Unsupported release format, channel or identity"));
    }
    if manifest.issued_at > now.saturating_add(300) || manifest.expires_at <= now || manifest.expires_at <= manifest.issued_at || manifest.expires_at - manifest.issued_at > MAX_LIFETIME {
        return Err(error("feed_expired", "Release metadata expired or device clock is incorrect"));
    }
    let mut seen = std::collections::HashSet::new();
    for item in &manifest.targets {
        if !seen.insert(&item.target) { return Err(error("feed_invalid", "Duplicate release target")); }
        validate_artifact(&item.target, &item.artifact)?;
    }
    if !known_target(target) { return Err(error("target_invalid", "Unsupported installed target")); }
    Ok((manifest, hex::encode(Sha256::digest(&payload))))
}

/// HTTPS only, same-origin redirects plus GitHub's release asset CDN.
/// No caller credentials are forwarded: this transport sets only User-Agent.
fn update_redirect_allowed(previous: &Url, next: &Url) -> bool {
    if https_url(previous.as_str()).is_err() || https_url(next.as_str()).is_err() { return false; }
    previous.origin() == next.origin() || (
        matches!(previous.host_str(), Some("github.com" | "api.github.com"))
        && next.host_str() == Some("release-assets.githubusercontent.com")
        && previous.port_or_known_default() == Some(443) && next.port_or_known_default() == Some(443)
    )
}

pub trait Transport { fn get(&self, url: &Url) -> Result<Box<dyn Read>>; }
pub struct HttpsTransport;
impl Transport for HttpsTransport {
    fn get(&self, url: &Url) -> Result<Box<dyn Read>> {
        https_url(url.as_str())?;
        let client = reqwest::blocking::Client::builder().connect_timeout(Duration::from_secs(10))
            .timeout(Duration::from_secs(300)).redirect(reqwest::redirect::Policy::custom(|attempt| {
                let previous = attempt.previous();
                if previous.len() > 5 { return attempt.error("Too many update redirects"); }
                if previous.last().is_some_and(|url| update_redirect_allowed(url, attempt.url())) {
                    attempt.follow()
                } else { attempt.error("Untrusted update redirect") }
            }))
            .user_agent("Nola-Software-Update/1").build().map_err(|_| error("network_error", "Could not initialize update transport"))?;
        let response = client.get(url.clone()).send().map_err(|_| error("network_error", "Could not reach update server"))?;
        if !response.status().is_success() { return Err(error("network_error", format!("Update server returned HTTP {}", response.status().as_u16()))); }
        Ok(Box::new(response))
    }
}
fn now() -> Result<u64> { SystemTime::now().duration_since(UNIX_EPOCH).map(|v| v.as_secs()).map_err(|_| error("clock_invalid", "Device clock is invalid")) }
pub fn check(source: &UpdateSource, target: &str, transport: &dyn Transport) -> Result<Check> {
    let mut output = Check {status: "disabled", current_version: source.version.clone(), current_build: source.build,
        channel: source.channel.clone(), target: target.into(), release: None};
    let Some(feed) = &source.feed_url else { return Ok(output); };
    let mut bytes = Vec::new();
    transport.get(&https_url(feed)?)?.take(MAX_FEED as u64 + 1).read_to_end(&mut bytes).map_err(|_| error("network_error", "Update feed transfer failed"))?;
    let (manifest, release_id) = verify_feed(source, &bytes, target, now()?)?;
    if manifest.build < source.build { output.status = "ahead"; return Ok(output); }
    let item = manifest.targets.iter().find(|v| v.target == target);
    let data_matches = manifest.store_schema == nola_domain::STORE_SCHEMA_VERSION && manifest.store_epoch == nola_domain::STORE_SCHEMA_EPOCH;
    if manifest.build == source.build {
        if !data_matches || manifest.version != source.version { return Err(error("identity_conflict", "The same build number names another version")); }
        output.status = if item.is_some() { "up_to_date" } else { "unavailable" }; return Ok(output);
    }
    if !data_matches { output.status = "incompatible_data"; return Ok(output); }
    let Some(item) = item else { output.status = "unavailable"; return Ok(output); };
    output.status = "available";
    output.release = Some(Offer {release_id, version: manifest.version, build: manifest.build, notes: manifest.notes, artifact: item.artifact.clone()});
    Ok(output)
}

/// The offered release is re-fetched and pinned before downloading. A package
/// is never exposed under its final name before its length and SHA-256 verify.
pub fn download(source: &UpdateSource, target: &str, expected_id: &str, directory: &Path, transport: &dyn Transport) -> Result<PathBuf> {
    if !directory.is_absolute() { return Err(error("path_invalid", "Download directory must be absolute")); }
    let offer = check(source, target, transport)?.release.ok_or_else(|| error("release_changed", "Check for updates again"))?;
    if offer.release_id != expected_id { return Err(error("release_changed", "The publication changed; check for updates again")); }
    let (url, filename, size, digest) = match &offer.artifact {
        Artifact::Installer {url, filename, size, sha256} | Artifact::Archive {url, filename, size, sha256} => (url, filename, *size, sha256),
        Artifact::Store {..} => return Err(error("store_managed", "Use the platform store to update")),
    };
    fs::create_dir_all(directory).map_err(|_| error("write_failed", "Cannot create update download directory"))?;
    let final_path = directory.join(format!("{}-{}", &offer.release_id[..16], filename));
    let mut temporary = tempfile::NamedTempFile::new_in(directory).map_err(|_| error("write_failed", "Cannot stage update package"))?;
    let mut reader = transport.get(&https_url(url)?)?;
    let mut hasher = Sha256::new(); let mut total = 0_u64; let mut chunk = [0_u8; 65536];
    loop {
        let count = reader.read(&mut chunk).map_err(|_| error("network_error", "Update download interrupted"))?;
        if count == 0 { break; }
        total = total.checked_add(count as u64).ok_or_else(|| error("package_invalid", "Package length overflow"))?;
        if total > size { return Err(error("package_invalid", "Downloaded package exceeds signed size")); }
        temporary.write_all(&chunk[..count]).map_err(|_| error("write_failed", "Cannot write update package"))?;
        hasher.update(&chunk[..count]);
    }
    if total != size || hex::encode(hasher.finalize()) != *digest { return Err(error("package_invalid", "Downloaded package failed size or SHA-256 verification")); }
    temporary.as_file().sync_all().map_err(|_| error("write_failed", "Cannot flush update package"))?;
    temporary.persist_noclobber(&final_path).map_err(|_| error("write_failed", "A package already exists; choose another directory or remove that update download explicitly"))?;
    Ok(final_path)
}

/// Shared native ABI entry. Hosts call on an IO worker; this never installs,
/// launches, quits a process, opens a conversation database or resolves a key.
pub fn dispatch(input: &str) -> serde_json::Value {
    match UpdateSource::bundled() {
        Ok(source) => dispatch_with_source(input, &source),
        Err(error) => serde_json::json!({"ok":false,"error":error}),
    }
}

// Inject only in Rust unit tests; native JSON callers cannot choose a trust root.
fn dispatch_with_source(input: &str, source: &UpdateSource) -> serde_json::Value {
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Request { operation: String, target: String, expected_release_id: Option<String>, directory: Option<PathBuf> }
    let result = (|| -> Result<serde_json::Value> {
        let request: Request = serde_json::from_str(input).map_err(|_| error("request_invalid", "Invalid update request"))?;
        match request.operation.as_str() {
            "check" if request.expected_release_id.is_none() && request.directory.is_none() => serde_json::to_value(check(source, &request.target, &HttpsTransport)?).map_err(|e| error("internal_error", e)),
            "download" => {
                let id = request.expected_release_id.ok_or_else(|| error("request_invalid", "Expected release identity missing"))?;
                let directory = request.directory.ok_or_else(|| error("request_invalid", "Download directory missing"))?;
                Ok(serde_json::json!({"path": download(source, &request.target, &id, &directory, &HttpsTransport)?}))
            },
            _ => Err(error("request_invalid", "Unknown update operation or arguments")),
        }
    })();
    match result { Ok(value) => serde_json::json!({"ok":true,"value":value}), Err(error) => serde_json::json!({"ok":false,"error":error}) }
}

#[cfg(test)]
mod tests;
